use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use talos::executor::dispatch_to_cpp;
use talos::{
    percentile_u64, AdmissionController, ChangeDetectionResult, ChangeDetector, DecisionStatus,
    ExecutionProfile, FeatureEmbedding, GpuLeaseManager, ObservabilityLogger, ObservationStage,
    SchedulerState, StateMachine, SystemTelemetry, TaskObservation, TaskPriority, TaskRequest,
    TaskScheduler, TaskType, TelemetryMonitor, TelemetrySource, VlmGateDecision, VlmGateReason,
    VlmRuntimeMetadata,
};

#[derive(Debug)]
struct Args {
    tasks: usize,
    workload: HitlWorkload,
    telemetry_source: TelemetrySource,
    sample_period_ms: u64,
    inter_task_delay_ms: u64,
    payload_bytes: usize,
    duration_secs: Option<u64>,
    progress_every: usize,
    cpu_burn_threads: CpuBurnThreads,
    target_temp_c: Option<f32>,
    stop_temp_c: Option<f32>,
    memory_pressure_mb: usize,
    vlm_temperature_gate_c: Option<f32>,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HitlWorkload {
    Baseline,
    Heavy,
    Thermal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CpuBurnThreads {
    None,
    Auto,
    Count(usize),
}

impl HitlWorkload {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "baseline" => Some(Self::Baseline),
            "heavy" => Some(Self::Heavy),
            "thermal" => Some(Self::Thermal),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "hitl-baseline",
            Self::Heavy => "hitl-heavy",
            Self::Thermal => "hitl-thermal",
        }
    }

    const fn default_payload_bytes(self) -> usize {
        match self {
            Self::Baseline => 4 * 1024,
            Self::Heavy => 16 * 1024 * 1024,
            Self::Thermal => 1024 * 1024,
        }
    }

    const fn default_sample_period_ms(self) -> u64 {
        match self {
            Self::Baseline => 100,
            Self::Heavy => 20,
            Self::Thermal => 50,
        }
    }

    const fn default_inter_task_delay_ms(self) -> u64 {
        match self {
            Self::Baseline => 25,
            Self::Heavy => 0,
            Self::Thermal => 0,
        }
    }

    const fn default_duration_secs(self) -> Option<u64> {
        match self {
            Self::Baseline => None,
            Self::Heavy => Some(60),
            Self::Thermal => Some(300),
        }
    }

    const fn default_progress_every(self) -> usize {
        match self {
            Self::Baseline => 0,
            Self::Heavy => 5,
            Self::Thermal => 1,
        }
    }

    const fn default_cpu_burn_threads(self) -> CpuBurnThreads {
        match self {
            Self::Baseline | Self::Heavy => CpuBurnThreads::None,
            Self::Thermal => CpuBurnThreads::Auto,
        }
    }

    const fn default_target_temp_c(self) -> Option<f32> {
        match self {
            Self::Baseline | Self::Heavy => None,
            Self::Thermal => Some(70.0),
        }
    }

    const fn default_stop_temp_c(self) -> Option<f32> {
        match self {
            Self::Baseline | Self::Heavy => None,
            Self::Thermal => Some(78.0),
        }
    }

    const fn default_memory_pressure_mb(self) -> usize {
        match self {
            Self::Baseline | Self::Heavy => 0,
            Self::Thermal => 0,
        }
    }
}

#[derive(Debug, Default)]
struct HitlStats {
    admitted: usize,
    deferred: usize,
    rejected: usize,
    executed: usize,
    telemetry_valid_samples: usize,
    telemetry_invalid_samples: usize,
    high_load_samples: usize,
    throttle_samples: usize,
    degraded_samples: usize,
    vlm_admitted: usize,
    vlm_deferred: usize,
    vlm_rejected: usize,
    vlm_thermal_pressure_deferrals: usize,
    vlm_memory_pressure_decisions: usize,
    peak_memory_percent: f32,
    peak_temperature_c: f32,
    peak_gpu_utilization: f32,
    max_queue_pressure: u32,
    execution_times_ms: Vec<u64>,
    runtime_latencies_ms: Vec<u64>,
}

impl HitlStats {
    fn observe_sample(
        &mut self,
        telemetry: SystemTelemetry,
        telemetry_valid: bool,
        queue_pressure: u32,
        state: SchedulerState,
    ) {
        if telemetry_valid {
            self.telemetry_valid_samples += 1;
        } else {
            self.telemetry_invalid_samples += 1;
        }

        self.peak_memory_percent = self.peak_memory_percent.max(telemetry.memory_usage_percent);
        self.peak_temperature_c = self.peak_temperature_c.max(telemetry.temperature_c);
        self.peak_gpu_utilization = self.peak_gpu_utilization.max(telemetry.gpu_utilization);
        self.max_queue_pressure = self.max_queue_pressure.max(queue_pressure);

        match state {
            SchedulerState::HIGH_LOAD => self.high_load_samples += 1,
            SchedulerState::THROTTLE => self.throttle_samples += 1,
            SchedulerState::DEGRADED => self.degraded_samples += 1,
            SchedulerState::NORMAL => {}
        }
    }

    fn observe_decision(
        &mut self,
        task_type: TaskType,
        decision: DecisionStatus,
        vlm_gate: Option<VlmGateDecision>,
    ) {
        if task_type != TaskType::VLM_QUERY {
            return;
        }

        match decision {
            DecisionStatus::ADMIT => self.vlm_admitted += 1,
            DecisionStatus::DEFER => self.vlm_deferred += 1,
            DecisionStatus::REJECT => self.vlm_rejected += 1,
        }

        if vlm_gate
            .map(|gate| gate.reason == VlmGateReason::ThermalPressure)
            .unwrap_or(false)
        {
            self.vlm_thermal_pressure_deferrals += 1;
        }

        if vlm_gate
            .map(|gate| gate.reason == VlmGateReason::MemoryPressure)
            .unwrap_or(false)
        {
            self.vlm_memory_pressure_decisions += 1;
        }
    }
}

struct CpuBurners {
    stop: Arc<AtomicBool>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl Drop for CpuBurners {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        while let Some(handle) = self.handles.pop() {
            let _ = handle.join();
        }
    }
}

impl CpuBurnThreads {
    fn parse(value: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match value {
            "none" | "0" => Ok(Self::None),
            "auto" => Ok(Self::Auto),
            _ => Ok(Self::Count(value.parse()?)),
        }
    }

    fn resolve(self) -> usize {
        match self {
            Self::None => 0,
            Self::Auto => thread::available_parallelism()
                .map(|parallelism| parallelism.get())
                .unwrap_or(1),
            Self::Count(count) => count,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run_hitl_baseline(args).await
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut tasks = 60usize;
    let mut workload = HitlWorkload::Baseline;
    let mut telemetry_source = TelemetrySource::Sysfs;
    let mut sample_period_ms = workload.default_sample_period_ms();
    let mut inter_task_delay_ms = workload.default_inter_task_delay_ms();
    let mut payload_bytes = workload.default_payload_bytes();
    let mut duration_secs = workload.default_duration_secs();
    let mut progress_every = workload.default_progress_every();
    let mut cpu_burn_threads = workload.default_cpu_burn_threads();
    let mut target_temp_c = workload.default_target_temp_c();
    let mut stop_temp_c = workload.default_stop_temp_c();
    let mut memory_pressure_mb = workload.default_memory_pressure_mb();
    let mut vlm_temperature_gate_c = None;
    let mut log_jsonl = PathBuf::from("logs/hitl-orinnano-baseline.jsonl");
    let mut log_csv = Some(PathBuf::from("logs/hitl-orinnano-baseline.csv"));
    let mut log_overridden = false;
    let mut csv_overridden = false;
    let mut sample_period_overridden = false;
    let mut inter_task_delay_overridden = false;
    let mut payload_overridden = false;
    let mut duration_overridden = false;
    let mut progress_overridden = false;
    let mut cpu_burn_threads_overridden = false;
    let mut target_temp_overridden = false;
    let mut stop_temp_overridden = false;
    let mut memory_pressure_overridden = false;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tasks" => {
                let value = args.next().ok_or("--tasks requires a value")?;
                tasks = value.parse()?;
            }
            "--workload" => {
                let value = args.next().ok_or("--workload requires a mode")?;
                workload =
                    HitlWorkload::parse(&value).ok_or("workload must be baseline or heavy")?;
                if !sample_period_overridden {
                    sample_period_ms = workload.default_sample_period_ms();
                }
                if !inter_task_delay_overridden {
                    inter_task_delay_ms = workload.default_inter_task_delay_ms();
                }
                if !payload_overridden {
                    payload_bytes = workload.default_payload_bytes();
                }
                if !duration_overridden {
                    duration_secs = workload.default_duration_secs();
                }
                if !progress_overridden {
                    progress_every = workload.default_progress_every();
                }
                if !cpu_burn_threads_overridden {
                    cpu_burn_threads = workload.default_cpu_burn_threads();
                }
                if !target_temp_overridden {
                    target_temp_c = workload.default_target_temp_c();
                }
                if !stop_temp_overridden {
                    stop_temp_c = workload.default_stop_temp_c();
                }
                if !memory_pressure_overridden {
                    memory_pressure_mb = workload.default_memory_pressure_mb();
                }
                if !log_overridden {
                    log_jsonl = PathBuf::from(format!("logs/{}-orinnano.jsonl", workload.name()));
                }
                if !csv_overridden {
                    log_csv = Some(PathBuf::from(format!(
                        "logs/{}-orinnano.csv",
                        workload.name()
                    )));
                }
            }
            "--telemetry" => {
                let value = args.next().ok_or("--telemetry requires a source")?;
                telemetry_source = TelemetrySource::parse(&value)
                    .ok_or("telemetry source must be sysfs, tegrastats, or jtop")?;
                if telemetry_source == TelemetrySource::Synthetic {
                    return Err("talos_hitl cannot use synthetic telemetry".into());
                }
            }
            "--sample-ms" => {
                let value = args.next().ok_or("--sample-ms requires a value")?;
                sample_period_ms = value.parse()?;
                sample_period_overridden = true;
            }
            "--inter-task-ms" => {
                let value = args.next().ok_or("--inter-task-ms requires a value")?;
                inter_task_delay_ms = value.parse()?;
                inter_task_delay_overridden = true;
            }
            "--payload-bytes" => {
                let value = args.next().ok_or("--payload-bytes requires a value")?;
                payload_bytes = value.parse()?;
                payload_overridden = true;
            }
            "--duration-secs" => {
                let value = args.next().ok_or("--duration-secs requires a value")?;
                duration_secs = Some(value.parse()?);
                duration_overridden = true;
            }
            "--no-duration-limit" => {
                duration_secs = None;
                duration_overridden = true;
            }
            "--progress-every" => {
                let value = args.next().ok_or("--progress-every requires a value")?;
                progress_every = value.parse()?;
                progress_overridden = true;
            }
            "--cpu-burn-threads" => {
                let value = args.next().ok_or("--cpu-burn-threads requires a value")?;
                cpu_burn_threads = CpuBurnThreads::parse(&value)?;
                cpu_burn_threads_overridden = true;
            }
            "--target-temp-c" => {
                let value = args.next().ok_or("--target-temp-c requires a value")?;
                target_temp_c = Some(value.parse()?);
                target_temp_overridden = true;
            }
            "--no-target-temp" => {
                target_temp_c = None;
                target_temp_overridden = true;
            }
            "--stop-temp-c" => {
                let value = args.next().ok_or("--stop-temp-c requires a value")?;
                stop_temp_c = Some(value.parse()?);
                stop_temp_overridden = true;
            }
            "--no-stop-temp" => {
                stop_temp_c = None;
                stop_temp_overridden = true;
            }
            "--memory-pressure-mb" => {
                let value = args.next().ok_or("--memory-pressure-mb requires a value")?;
                memory_pressure_mb = value.parse()?;
                memory_pressure_overridden = true;
            }
            "--vlm-temperature-gate-c" => {
                let value = args
                    .next()
                    .ok_or("--vlm-temperature-gate-c requires a value")?;
                vlm_temperature_gate_c = Some(value.parse()?);
            }
            "--log-jsonl" => {
                let value = args.next().ok_or("--log-jsonl requires a path")?;
                log_jsonl = PathBuf::from(value);
                log_overridden = true;
            }
            "--log-csv" => {
                let value = args.next().ok_or("--log-csv requires a path")?;
                log_csv = Some(PathBuf::from(value));
                csv_overridden = true;
            }
            "--no-csv" => {
                log_csv = None;
                csv_overridden = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: talos_hitl [--workload baseline|heavy|thermal] [--tasks N] [--duration-secs N|--no-duration-limit] [--progress-every N] [--cpu-burn-threads none|auto|N] [--target-temp-c C|--no-target-temp] [--stop-temp-c C|--no-stop-temp] [--memory-pressure-mb N] [--vlm-temperature-gate-c C] [--telemetry sysfs|tegrastats|jtop] [--sample-ms N] [--inter-task-ms N] [--payload-bytes N] [--log-jsonl PATH] [--log-csv PATH] [--no-csv]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        tasks,
        workload,
        telemetry_source,
        sample_period_ms,
        inter_task_delay_ms,
        payload_bytes,
        duration_secs,
        progress_every,
        cpu_burn_threads,
        target_temp_c,
        stop_temp_c,
        memory_pressure_mb,
        vlm_temperature_gate_c,
        log_jsonl,
        log_csv,
    })
}

async fn run_hitl_baseline(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut admission = AdmissionController::default();
    if let Some(vlm_temperature_gate_c) = args.vlm_temperature_gate_c {
        admission.vlm_temperature_gate_c = vlm_temperature_gate_c;
    }
    let state_machine = StateMachine::default();
    let lease_manager = GpuLeaseManager::new();
    let mut change_detector = ChangeDetector::default();
    let mut scheduler = TaskScheduler::new();
    let mut telemetry_monitor = TelemetryMonitor::new(
        Duration::from_millis(args.sample_period_ms),
        args.telemetry_source,
    );
    let mut logger = ObservabilityLogger::new(&args.log_jsonl, args.log_csv.as_ref())?;
    let mut stats = HitlStats::default();
    let _memory_pressure = if args.memory_pressure_mb > 0 {
        Some(allocate_memory_pressure(args.memory_pressure_mb))
    } else {
        None
    };
    let burn_thread_count = args.cpu_burn_threads.resolve();
    let _cpu_burners = if burn_thread_count > 0 {
        println!("cpu_burn_threads={burn_thread_count}");
        Some(spawn_cpu_burners(burn_thread_count))
    } else {
        None
    };
    let started = Instant::now();
    let mut target_temp_reported = false;

    println!(
        "profile={} mode={} telemetry={} sample_ms={} payload_bytes={} duration_secs={} target_temp_c={} stop_temp_c={} memory_pressure_mb={} vlm_temperature_gate_c={}",
        ExecutionProfile::Hitl.name(),
        args.workload.name(),
        args.telemetry_source.name(),
        args.sample_period_ms,
        args.payload_bytes,
        args.duration_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        optional_f32_text(args.target_temp_c),
        optional_f32_text(args.stop_temp_c),
        args.memory_pressure_mb,
        optional_f32_text(args.vlm_temperature_gate_c)
    );

    for index in 0..args.tasks {
        if let Some(duration_secs) = args.duration_secs {
            if started.elapsed() >= Duration::from_secs(duration_secs) {
                println!(
                    "duration_limit_reached elapsed_ms={} completed_tasks={}",
                    started.elapsed().as_millis(),
                    stats.executed + stats.deferred + stats.rejected
                );
                break;
            }
        }

        let sample = telemetry_monitor.tick_sample().await;
        if !sample.valid {
            eprintln!(
                "telemetry source={} unavailable; using last known sample",
                sample.source.name()
            );
        }

        let task = hitl_task(index, args.workload, args.payload_bytes);
        let telemetry = sample.telemetry;
        let queue_pressure = scheduler.queue_pressure();
        let state = state_machine.evaluate(&telemetry, queue_pressure);
        stats.observe_sample(telemetry, sample.valid, queue_pressure, state);

        if let Some(target_temp_c) = args.target_temp_c {
            if !target_temp_reported && telemetry.temperature_c >= target_temp_c {
                target_temp_reported = true;
                println!(
                    "target_temp_reached target_temp_c={target_temp_c:.3} current_temp_c={:.3} elapsed_ms={}",
                    telemetry.temperature_c,
                    started.elapsed().as_millis()
                );
            }
        }

        if let Some(stop_temp_c) = args.stop_temp_c {
            if telemetry.temperature_c >= stop_temp_c {
                println!(
                    "stop_temp_reached stop_temp_c={stop_temp_c:.3} current_temp_c={:.3} elapsed_ms={} completed_tasks={}",
                    telemetry.temperature_c,
                    started.elapsed().as_millis(),
                    stats.executed + stats.deferred + stats.rejected
                );
                break;
            }
        }

        let vlm_gate = if task.task_type == TaskType::VLM_QUERY {
            Some(admission.vlm_gate_decision(
                &task,
                &telemetry,
                state,
                lease_manager.is_active(),
                scheduler.cv_burst_active(),
            ))
        } else {
            None
        };
        let vlm_metadata = admission.vlm_profile.runtime_metadata();
        let decision = admission.decide(
            &task,
            &telemetry,
            state,
            lease_manager.is_active(),
            scheduler.cv_burst_active(),
        );
        stats.observe_decision(task.task_type, decision.status, vlm_gate);

        match decision.status {
            DecisionStatus::ADMIT => {
                stats.admitted += 1;
                if let Some(lease) = lease_manager.try_acquire() {
                    let lease_id = lease.id.to_string();
                    let task_type = task.task_type;
                    let task_id = task.task_id;
                    let pool_slot_id = task.pool_slot_id;
                    let execution_started = Instant::now();
                    let result = {
                        let _lease = lease;
                        dispatch_to_cpp(task).await?
                    };
                    let execution_time_ms = elapsed_execution_ms(execution_started);
                    let change_result = if task_type == TaskType::CHANGE_DETECTION && result.ok {
                        Some(change_detector.evaluate(FeatureEmbedding::from(&result)))
                    } else {
                        None
                    };
                    let vlm_runtime = if task_type == TaskType::VLM_QUERY && result.ok {
                        Some((
                            vlm_metadata,
                            result.vlm_output_tokens,
                            result.vlm_confidence,
                            result.vlm_answer_code,
                        ))
                    } else {
                        None
                    };

                    stats.executed += 1;
                    stats.execution_times_ms.push(execution_time_ms);
                    stats.runtime_latencies_ms.push(result.latency_ms);

                    logger.record(&execution_observation(
                        task_id,
                        task_type,
                        queue_pressure,
                        state,
                        sample.source,
                        sample.valid,
                        telemetry,
                        Some(lease_id),
                        pool_slot_id,
                        execution_time_ms,
                        &result,
                        change_result,
                        vlm_gate,
                        vlm_runtime,
                    ))?;
                } else {
                    stats.deferred += 1;
                    record_decision(
                        &mut logger,
                        &task,
                        DecisionStatus::DEFER,
                        queue_pressure,
                        state,
                        sample.source,
                        sample.valid,
                        telemetry,
                        vlm_gate,
                        vlm_metadata,
                    )?;
                    scheduler.defer(task);
                }
            }
            DecisionStatus::DEFER => {
                stats.deferred += 1;
                record_decision(
                    &mut logger,
                    &task,
                    DecisionStatus::DEFER,
                    queue_pressure,
                    state,
                    sample.source,
                    sample.valid,
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                )?;
                scheduler.defer(task);
            }
            DecisionStatus::REJECT => {
                stats.rejected += 1;
                record_decision(
                    &mut logger,
                    &task,
                    DecisionStatus::REJECT,
                    queue_pressure,
                    state,
                    sample.source,
                    sample.valid,
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                )?;
            }
        }

        if args.inter_task_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(args.inter_task_delay_ms)).await;
        }

        let completed = stats.executed + stats.deferred + stats.rejected;
        if args.progress_every > 0 && completed % args.progress_every == 0 {
            println!(
                "progress completed={} task_limit={} elapsed_ms={} temp_c={:.3} peak_temp_c={:.3} mem_percent={:.3} executed={} vlm_deferred={} vlm_thermal_deferrals={} vlm_memory_pressure_decisions={}",
                completed,
                args.tasks,
                started.elapsed().as_millis(),
                telemetry.temperature_c,
                stats.peak_temperature_c,
                stats.peak_memory_percent,
                stats.executed,
                stats.vlm_deferred,
                stats.vlm_thermal_pressure_deferrals,
                stats.vlm_memory_pressure_decisions
            );
        }
    }

    print_summary(args.tasks, started.elapsed(), &args, &stats);
    Ok(())
}

fn record_decision(
    logger: &mut ObservabilityLogger,
    task: &TaskRequest,
    decision: DecisionStatus,
    queue_pressure: u32,
    state: SchedulerState,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    telemetry: SystemTelemetry,
    vlm_gate: Option<VlmGateDecision>,
    vlm_metadata: VlmRuntimeMetadata,
) -> std::io::Result<()> {
    logger.record(&TaskObservation {
        stage: ObservationStage::Decision,
        task_id: task.task_id,
        task_type: task.task_type,
        decision,
        queue_pressure,
        scheduler_state: state,
        telemetry_source,
        telemetry_valid,
        memory_usage_percent: telemetry.memory_usage_percent,
        temperature_c: telemetry.temperature_c,
        gpu_utilization: telemetry.gpu_utilization,
        lease_id: None,
        pool_slot_id: task.pool_slot_id,
        latency_ms: None,
        execution_time_ms: 0,
        runtime_ok: None,
        feature_dim: None,
        input_bytes: None,
        feature_checksum: None,
        feature_mean: None,
        feature_entropy: None,
        feature_edge_density: None,
        feature_saliency_score: None,
        feature_texture_score: None,
        feature_anomaly_score: None,
        feature_detection_count: None,
        change_baseline_ready: None,
        change_score: None,
        change_detected: None,
        vlm_model: vlm_gate.map(|_| vlm_metadata.model_name.to_string()),
        vlm_quantization_bits: vlm_gate.map(|_| vlm_metadata.quantization_bits),
        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
        vlm_output_tokens: None,
        vlm_confidence: None,
        vlm_answer_code: None,
        real_model_backend: None,
        real_model_name: None,
        real_model_exit_code: None,
        real_model_peak_cuda_mb: None,
    })
}

fn execution_observation(
    task_id: u64,
    task_type: TaskType,
    queue_pressure: u32,
    state: SchedulerState,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    telemetry: SystemTelemetry,
    lease_id: Option<String>,
    pool_slot_id: usize,
    execution_time_ms: u64,
    result: &talos::cxx_bridge::ffi::RuntimeResult,
    change_result: Option<ChangeDetectionResult>,
    vlm_gate: Option<VlmGateDecision>,
    vlm_runtime: Option<(VlmRuntimeMetadata, u32, f32, u32)>,
) -> TaskObservation {
    TaskObservation {
        stage: ObservationStage::Execution,
        task_id,
        task_type,
        decision: DecisionStatus::ADMIT,
        queue_pressure,
        scheduler_state: state,
        telemetry_source,
        telemetry_valid,
        memory_usage_percent: telemetry.memory_usage_percent,
        temperature_c: telemetry.temperature_c,
        gpu_utilization: telemetry.gpu_utilization,
        lease_id,
        pool_slot_id,
        latency_ms: Some(result.latency_ms),
        execution_time_ms,
        runtime_ok: Some(result.ok),
        feature_dim: Some(result.feature_dim),
        input_bytes: Some(result.input_bytes),
        feature_checksum: Some(result.checksum),
        feature_mean: Some(result.mean),
        feature_entropy: Some(result.entropy),
        feature_edge_density: Some(result.edge_density),
        feature_saliency_score: Some(result.saliency_score),
        feature_texture_score: Some(result.texture_score),
        feature_anomaly_score: Some(result.anomaly_score),
        feature_detection_count: Some(result.detection_count),
        change_baseline_ready: change_result.map(|result| result.baseline_ready),
        change_score: change_result.map(|result| result.score),
        change_detected: change_result.map(|result| result.changed),
        vlm_model: vlm_runtime.map(|(metadata, _, _, _)| metadata.model_name.to_string()),
        vlm_quantization_bits: vlm_runtime.map(|(metadata, _, _, _)| metadata.quantization_bits),
        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
        vlm_output_tokens: vlm_runtime.map(|(_, tokens, _, _)| tokens),
        vlm_confidence: vlm_runtime.map(|(_, _, confidence, _)| confidence),
        vlm_answer_code: vlm_runtime.map(|(_, _, _, answer_code)| answer_code),
        real_model_backend: None,
        real_model_name: None,
        real_model_exit_code: None,
        real_model_peak_cuda_mb: None,
    }
}

fn hitl_task(index: usize, workload: HitlWorkload, payload_bytes: usize) -> TaskRequest {
    let (task_type, priority, memory_estimate_mb, frame) = match workload {
        HitlWorkload::Baseline => baseline_task_parts(index),
        HitlWorkload::Heavy => heavy_task_parts(index, payload_bytes),
        HitlWorkload::Thermal => thermal_task_parts(index, payload_bytes),
    };

    TaskRequest {
        task_id: index as u64 + 1,
        task_type,
        priority,
        memory_estimate_mb,
        deadline_ms: if task_type == TaskType::VLM_QUERY {
            1_000
        } else {
            250
        },
        pool_slot_id: index % 5,
        frame,
    }
}

fn thermal_task_parts(
    index: usize,
    payload_bytes: usize,
) -> (TaskType, TaskPriority, u64, Vec<u8>) {
    match index % 6 {
        0 => (
            TaskType::VLM_QUERY,
            TaskPriority::LOW,
            2_304,
            patterned_frame(index, payload_bytes.min(8 * 1024 * 1024)),
        ),
        3 => (
            TaskType::CHANGE_DETECTION,
            TaskPriority::MEDIUM,
            128,
            inspection_frame(index, payload_bytes),
        ),
        _ => (
            TaskType::CV_FEATURES,
            TaskPriority::HIGH,
            128,
            inspection_frame(index, payload_bytes),
        ),
    }
}

fn baseline_task_parts(index: usize) -> (TaskType, TaskPriority, u64, Vec<u8>) {
    match index % 10 {
        0 => (
            TaskType::VLM_QUERY,
            TaskPriority::LOW,
            2_304,
            vec![64 + (index % 32) as u8; 4096],
        ),
        3 | 7 => (
            TaskType::CHANGE_DETECTION,
            TaskPriority::MEDIUM,
            64,
            hitl_change_frame(index),
        ),
        1 | 2 | 4 => (
            TaskType::CV_FEATURES,
            TaskPriority::HIGH,
            32,
            inspection_frame(index, 4096),
        ),
        _ => (
            TaskType::CV_FEATURES,
            TaskPriority::MEDIUM,
            32,
            inspection_frame(index, 4096),
        ),
    }
}

fn heavy_task_parts(index: usize, payload_bytes: usize) -> (TaskType, TaskPriority, u64, Vec<u8>) {
    match index % 8 {
        0 | 4 => (
            TaskType::CHANGE_DETECTION,
            TaskPriority::MEDIUM,
            128,
            inspection_frame(index, payload_bytes),
        ),
        _ => (
            TaskType::CV_FEATURES,
            TaskPriority::HIGH,
            128,
            inspection_frame(index, payload_bytes),
        ),
    }
}

fn hitl_change_frame(index: usize) -> Vec<u8> {
    if index < 3 {
        vec![24; 4096]
    } else if index % 4 == 0 {
        inspection_frame(index, 4096)
    } else {
        vec![36; 4096]
    }
}

fn patterned_frame(index: usize, payload_bytes: usize) -> Vec<u8> {
    let mut frame = vec![0; payload_bytes];
    for (offset, byte) in frame.iter_mut().enumerate() {
        *byte = ((offset.wrapping_mul(31) + index.wrapping_mul(17)) % 256) as u8;
    }
    frame
}

fn inspection_frame(index: usize, payload_bytes: usize) -> Vec<u8> {
    let mut frame = vec![24 + (index % 16) as u8; payload_bytes];
    if payload_bytes == 0 {
        return frame;
    }

    let cell_count = 64usize;
    let cell_len = (payload_bytes / cell_count).max(1);
    let defect_cell = (index.wrapping_mul(19).wrapping_add(11)) % cell_count;
    let start = defect_cell * cell_len;
    let end = (start + cell_len).min(payload_bytes);
    for byte in &mut frame[start..end] {
        *byte = 232u8.saturating_sub((index % 23) as u8);
    }

    frame
}

fn print_summary(tasks: usize, elapsed: Duration, args: &Args, stats: &HitlStats) {
    let elapsed_ms = elapsed.as_millis();
    let throughput = if elapsed_ms == 0 {
        0.0
    } else {
        (stats.executed as f64 / elapsed.as_secs_f64()) as f32
    };

    println!("mode={}", args.workload.name());
    println!("profile={}", ExecutionProfile::Hitl.name());
    println!("telemetry={}", args.telemetry_source.name());
    println!("task_limit={tasks}");
    println!(
        "completed_tasks={}",
        stats.executed + stats.deferred + stats.rejected
    );
    println!(
        "duration_secs={}",
        args.duration_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!("sample_ms={}", args.sample_period_ms);
    println!("inter_task_ms={}", args.inter_task_delay_ms);
    println!("payload_bytes={}", args.payload_bytes);
    println!("cpu_burn_threads={}", args.cpu_burn_threads.resolve());
    println!("target_temp_c={}", optional_f32_text(args.target_temp_c));
    println!("stop_temp_c={}", optional_f32_text(args.stop_temp_c));
    println!("memory_pressure_mb={}", args.memory_pressure_mb);
    println!(
        "vlm_temperature_gate_c={}",
        optional_f32_text(args.vlm_temperature_gate_c)
    );
    println!("elapsed_ms={elapsed_ms}");
    println!(
        "admitted={} deferred={} rejected={} executed={}",
        stats.admitted, stats.deferred, stats.rejected, stats.executed
    );
    println!(
        "telemetry_valid_samples={} telemetry_invalid_samples={}",
        stats.telemetry_valid_samples, stats.telemetry_invalid_samples
    );
    println!(
        "high_load_samples={} throttle_samples={} degraded_samples={}",
        stats.high_load_samples, stats.throttle_samples, stats.degraded_samples
    );
    println!(
        "vlm_admitted={} vlm_deferred={} vlm_rejected={} vlm_thermal_pressure_deferrals={} vlm_memory_pressure_decisions={}",
        stats.vlm_admitted,
        stats.vlm_deferred,
        stats.vlm_rejected,
        stats.vlm_thermal_pressure_deferrals,
        stats.vlm_memory_pressure_decisions
    );
    println!(
        "execution_time_ms_p50={} execution_time_ms_p95={}",
        percentile_u64(&stats.execution_times_ms, 50),
        percentile_u64(&stats.execution_times_ms, 95)
    );
    println!(
        "runtime_latency_ms_p50={} runtime_latency_ms_p95={}",
        percentile_u64(&stats.runtime_latencies_ms, 50),
        percentile_u64(&stats.runtime_latencies_ms, 95)
    );
    println!("throughput_tps={throughput:.3}");
    println!(
        "peak_memory_percent={:.3} peak_temperature_c={:.3} peak_gpu_utilization={:.3} max_queue_pressure={}",
        stats.peak_memory_percent,
        stats.peak_temperature_c,
        stats.peak_gpu_utilization,
        stats.max_queue_pressure
    );
}

fn elapsed_execution_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis() as u64;
    millis.max(1)
}

fn optional_f32_text(value: Option<f32>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "none".to_string())
}

fn spawn_cpu_burners(count: usize) -> CpuBurners {
    let stop = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(count);

    for worker_id in 0..count {
        let stop = Arc::clone(&stop);
        handles.push(thread::spawn(move || burn_cpu(stop, worker_id)));
    }

    CpuBurners { stop, handles }
}

fn allocate_memory_pressure(total_mb: usize) -> Vec<Vec<u8>> {
    const CHUNK_MB: usize = 64;
    const BYTES_PER_MB: usize = 1024 * 1024;

    let mut remaining_mb = total_mb;
    let mut chunks = Vec::new();
    println!("memory_pressure_allocating_mb={total_mb}");

    while remaining_mb > 0 {
        let chunk_mb = remaining_mb.min(CHUNK_MB);
        let mut chunk = vec![0u8; chunk_mb * BYTES_PER_MB];
        for offset in (0..chunk.len()).step_by(4096) {
            chunk[offset] = (offset as u8).wrapping_add(chunks.len() as u8);
        }
        chunks.push(chunk);
        remaining_mb -= chunk_mb;
        println!(
            "memory_pressure_allocated_mb={}",
            total_mb.saturating_sub(remaining_mb)
        );
    }

    chunks
}

fn burn_cpu(stop: Arc<AtomicBool>, worker_id: usize) {
    let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ worker_id as u64;
    let mut accumulator = 0xBF58_476D_1CE4_E5B9u64;

    while !stop.load(Ordering::Relaxed) {
        for _ in 0..50_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            accumulator ^= state.rotate_left((state & 31) as u32);
            accumulator = accumulator.wrapping_mul(0x94D0_49BB_1331_11EB);
        }
        std::hint::black_box(accumulator);
    }
}
