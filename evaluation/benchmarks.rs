use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use talos::executor::dispatch_to_cpp;
use talos::{
    percentile_u64, AdmissionController, ChangeDetectionResult, ChangeDetector, DecisionStatus,
    FeatureEmbedding, GpuLeaseManager, ObservabilityLogger, ObservationStage, OptimizationMetrics,
    OptimizationProfile, SchedulerState, StateMachine, SystemTelemetry, TaskObservation,
    TaskPriority, TaskRequest, TaskScheduler, TaskType, TelemetrySource, ThermalStressSimulator,
    VlmGateDecision, VlmRuntimeMetadata,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StressMode {
    CvFlood,
    VlmBurst,
    ThermalSpike,
    MixedContention,
    ChangeDetection,
    VlmQuery,
    Phase6Contention,
    Phase8Optimization,
}

#[derive(Debug)]
struct Args {
    mode: StressMode,
    tasks: usize,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct BenchStats {
    admitted: usize,
    deferred: usize,
    rejected: usize,
    executed: usize,
    changes_detected: usize,
    vlm_admitted: usize,
    vlm_rejected: usize,
    high_load_samples: usize,
    throttle_samples: usize,
    degraded_samples: usize,
    peak_memory_percent: f32,
    peak_temperature_c: f32,
    max_queue_pressure: u32,
    execution_times_ms: Vec<u64>,
    runtime_latencies_ms: Vec<u64>,
}

impl BenchStats {
    fn observe_pressure(&mut self, telemetry: SystemTelemetry, queue_pressure: u32) {
        self.peak_memory_percent = self.peak_memory_percent.max(telemetry.memory_usage_percent);
        self.peak_temperature_c = self.peak_temperature_c.max(telemetry.temperature_c);
        self.max_queue_pressure = self.max_queue_pressure.max(queue_pressure);
    }

    fn optimization_metrics(&self, tasks: usize, elapsed: Duration) -> OptimizationMetrics {
        OptimizationMetrics {
            tasks,
            elapsed_ms: elapsed.as_millis(),
            admitted: self.admitted,
            deferred: self.deferred,
            rejected: self.rejected,
            executed: self.executed,
            execution_p50_ms: percentile_u64(&self.execution_times_ms, 50),
            execution_p95_ms: percentile_u64(&self.execution_times_ms, 95),
            runtime_p95_ms: percentile_u64(&self.runtime_latencies_ms, 95),
            peak_memory_percent: self.peak_memory_percent,
            peak_temperature_c: self.peak_temperature_c,
            max_queue_pressure: self.max_queue_pressure,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    if args.mode == StressMode::Phase6Contention {
        run_phase6_contention(args).await
    } else {
        run_benchmark(args).await
    }
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut mode = StressMode::CvFlood;
    let mut tasks = 100usize;
    let mut log_jsonl = sitl_jsonl_path(mode);
    let mut log_csv = Some(sitl_csv_path(mode));
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                let value = args.next().ok_or("--mode requires a value")?;
                mode = parse_mode(&value)?;
                log_jsonl = sitl_jsonl_path(mode);
                log_csv = Some(sitl_csv_path(mode));
            }
            "--tasks" => {
                let value = args.next().ok_or("--tasks requires a value")?;
                tasks = value.parse()?;
            }
            "--log-jsonl" => {
                let value = args.next().ok_or("--log-jsonl requires a path")?;
                log_jsonl = PathBuf::from(value);
            }
            "--log-csv" => {
                let value = args.next().ok_or("--log-csv requires a path")?;
                log_csv = Some(PathBuf::from(value));
            }
            "--no-csv" => {
                log_csv = None;
            }
            "--help" | "-h" => {
                println!("Usage: talos_bench [--mode cv-flood|vlm-burst|thermal-spike|mixed-contention|change-detection|vlm-query|phase6-contention|phase8-optimization] [--tasks N] [--log-jsonl PATH] [--log-csv PATH] [--no-csv]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        mode,
        tasks,
        log_jsonl,
        log_csv,
    })
}

async fn run_benchmark(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let admission = AdmissionController::default();
    let state_machine = StateMachine::default();
    let lease_manager = GpuLeaseManager::new();
    let mut change_detector = ChangeDetector::default();
    let mut scheduler = TaskScheduler::new();
    let mut logger = ObservabilityLogger::new(&args.log_jsonl, args.log_csv.as_ref())?;
    let mut stats = BenchStats::default();
    let started = Instant::now();

    for index in 0..args.tasks {
        let task = synthetic_task(args.mode, index);
        let telemetry = synthetic_telemetry(args.mode, index, args.tasks);
        let queue_pressure = scheduler.queue_pressure();
        stats.observe_pressure(telemetry, queue_pressure);
        let state = state_machine.evaluate(&telemetry, queue_pressure);
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
                    let change_result = if task_type == TaskType::CHANGE_DETECTION && result.ok {
                        Some(change_detector.evaluate(FeatureEmbedding::from(&result)))
                    } else {
                        None
                    };
                    if change_result.map(|result| result.changed).unwrap_or(false) {
                        stats.changes_detected += 1;
                    }
                    let vlm_runtime = if task_type == TaskType::VLM_QUERY && result.ok {
                        stats.vlm_admitted += 1;
                        Some((
                            vlm_metadata,
                            result.vlm_output_tokens,
                            result.vlm_confidence,
                            result.vlm_answer_code,
                        ))
                    } else {
                        None
                    };
                    let execution_time_ms = elapsed_execution_ms(execution_started);
                    stats.executed += 1;
                    stats.execution_times_ms.push(execution_time_ms);
                    stats.runtime_latencies_ms.push(result.latency_ms);
                    logger.record(&TaskObservation {
                        stage: ObservationStage::Execution,
                        task_id,
                        task_type,
                        decision: DecisionStatus::ADMIT,
                        queue_pressure,
                        scheduler_state: state,
                        telemetry_source: TelemetrySource::Synthetic,
                        telemetry_valid: true,
                        memory_usage_percent: telemetry.memory_usage_percent,
                        temperature_c: telemetry.temperature_c,
                        gpu_utilization: telemetry.gpu_utilization,
                        lease_id: Some(lease_id),
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
                        change_baseline_ready: change_result.map(|result| result.baseline_ready),
                        change_score: change_result.map(|result| result.score),
                        change_detected: change_result.map(|result| result.changed),
                        vlm_model: vlm_runtime
                            .map(|(metadata, _, _, _)| metadata.model_name.to_string()),
                        vlm_quantization_bits: vlm_runtime
                            .map(|(metadata, _, _, _)| metadata.quantization_bits),
                        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
                        vlm_output_tokens: vlm_runtime.map(|(_, tokens, _, _)| tokens),
                        vlm_confidence: vlm_runtime.map(|(_, _, confidence, _)| confidence),
                        vlm_answer_code: vlm_runtime.map(|(_, _, _, answer_code)| answer_code),
                    })?;
                } else {
                    stats.deferred += 1;
                    record_decision(
                        &mut logger,
                        &task,
                        DecisionStatus::DEFER,
                        queue_pressure,
                        telemetry,
                        vlm_gate,
                        vlm_metadata,
                        state,
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
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                    state,
                )?;
                scheduler.defer(task);
            }
            DecisionStatus::REJECT => {
                stats.rejected += 1;
                if task.task_type == TaskType::VLM_QUERY {
                    stats.vlm_rejected += 1;
                }
                record_decision(
                    &mut logger,
                    &task,
                    DecisionStatus::REJECT,
                    queue_pressure,
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                    state,
                )?;
            }
        }

        if let Some(delay) = mode_delay(args.mode) {
            tokio::time::sleep(delay).await;
        }
    }

    print_summary(args.mode, args.tasks, started.elapsed(), &stats);
    Ok(())
}

fn record_decision(
    logger: &mut ObservabilityLogger,
    task: &TaskRequest,
    decision: DecisionStatus,
    queue_pressure: u32,
    telemetry: SystemTelemetry,
    vlm_gate: Option<VlmGateDecision>,
    vlm_metadata: VlmRuntimeMetadata,
    state: SchedulerState,
) -> std::io::Result<()> {
    logger.record(&TaskObservation {
        stage: ObservationStage::Decision,
        task_id: task.task_id,
        task_type: task.task_type,
        decision,
        queue_pressure,
        scheduler_state: state,
        telemetry_source: TelemetrySource::Synthetic,
        telemetry_valid: true,
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
        change_baseline_ready: None,
        change_score: None,
        change_detected: None,
        vlm_model: vlm_gate.map(|_| vlm_metadata.model_name.to_string()),
        vlm_quantization_bits: vlm_gate.map(|_| vlm_metadata.quantization_bits),
        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
        vlm_output_tokens: None,
        vlm_confidence: None,
        vlm_answer_code: None,
    })
}

async fn run_phase6_contention(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let admission = AdmissionController::default();
    let state_machine = StateMachine::default();
    let lease_manager = GpuLeaseManager::new();
    let change_detector = Arc::new(Mutex::new(ChangeDetector::default()));
    let logger = Arc::new(Mutex::new(ObservabilityLogger::new(
        &args.log_jsonl,
        args.log_csv.as_ref(),
    )?));
    let stats = Arc::new(Mutex::new(BenchStats::default()));
    let mut scheduler = TaskScheduler::new();
    let mut simulator = ThermalStressSimulator::new();
    let started = Instant::now();
    let mut handles = Vec::new();

    for index in 0..args.tasks {
        let task = phase6_task(index);
        let telemetry = simulator.tick(lease_manager.is_active(), index % 2 == 0);
        let queue_pressure = scheduler.queue_pressure();
        let state = state_machine.evaluate(&telemetry, queue_pressure);
        stats
            .lock()
            .expect("stats mutex poisoned")
            .observe_pressure(telemetry, queue_pressure);
        record_state_sample(&stats, state);

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

        match decision.status {
            DecisionStatus::ADMIT => {
                stats.lock().expect("stats mutex poisoned").admitted += 1;
                if let Some(lease) = lease_manager.try_acquire() {
                    let logger = Arc::clone(&logger);
                    let stats = Arc::clone(&stats);
                    let change_detector = Arc::clone(&change_detector);
                    let lease_id = lease.id.to_string();
                    let task_type = task.task_type;
                    let task_id = task.task_id;
                    let pool_slot_id = task.pool_slot_id;
                    handles.push(tokio::spawn(async move {
                        let _lease = lease;
                        let execution_started = Instant::now();
                        let result = dispatch_to_cpp(task).await;
                        tokio::time::sleep(Duration::from_millis(75)).await;
                        let execution_time_ms = elapsed_execution_ms(execution_started);

                        if let Ok(result) = result {
                            let change_result =
                                if task_type == TaskType::CHANGE_DETECTION && result.ok {
                                    Some(
                                        change_detector
                                            .lock()
                                            .expect("change detector mutex poisoned")
                                            .evaluate(FeatureEmbedding::from(&result)),
                                    )
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

                            {
                                let mut stats = stats.lock().expect("stats mutex poisoned");
                                stats.executed += 1;
                                stats.execution_times_ms.push(execution_time_ms);
                                stats.runtime_latencies_ms.push(result.latency_ms);
                                if change_result.map(|result| result.changed).unwrap_or(false) {
                                    stats.changes_detected += 1;
                                }
                                if task_type == TaskType::VLM_QUERY && result.ok {
                                    stats.vlm_admitted += 1;
                                }
                            }

                            let observation = execution_observation(
                                task_id,
                                task_type,
                                DecisionStatus::ADMIT,
                                queue_pressure,
                                state,
                                telemetry,
                                Some(lease_id),
                                pool_slot_id,
                                execution_time_ms,
                                &result,
                                change_result,
                                vlm_gate,
                                vlm_runtime,
                            );
                            let _ = logger
                                .lock()
                                .expect("logger mutex poisoned")
                                .record(&observation);
                        }
                    }));
                } else {
                    stats.lock().expect("stats mutex poisoned").deferred += 1;
                    record_decision_shared(
                        &logger,
                        &task,
                        DecisionStatus::DEFER,
                        queue_pressure,
                        telemetry,
                        vlm_gate,
                        vlm_metadata,
                        state,
                    )?;
                    scheduler.defer(task);
                }
            }
            DecisionStatus::DEFER => {
                stats.lock().expect("stats mutex poisoned").deferred += 1;
                record_decision_shared(
                    &logger,
                    &task,
                    DecisionStatus::DEFER,
                    queue_pressure,
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                    state,
                )?;
                scheduler.defer(task);
            }
            DecisionStatus::REJECT => {
                {
                    let mut stats = stats.lock().expect("stats mutex poisoned");
                    stats.rejected += 1;
                    if task.task_type == TaskType::VLM_QUERY {
                        stats.vlm_rejected += 1;
                    }
                }
                record_decision_shared(
                    &logger,
                    &task,
                    DecisionStatus::REJECT,
                    queue_pressure,
                    telemetry,
                    vlm_gate,
                    vlm_metadata,
                    state,
                )?;
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    for handle in handles {
        let _ = handle.await;
    }

    let stats = stats.lock().expect("stats mutex poisoned");
    print_summary(args.mode, args.tasks, started.elapsed(), &stats);
    Ok(())
}

fn record_state_sample(stats: &Arc<Mutex<BenchStats>>, state: SchedulerState) {
    let mut stats = stats.lock().expect("stats mutex poisoned");
    match state {
        SchedulerState::HIGH_LOAD => stats.high_load_samples += 1,
        SchedulerState::THROTTLE => stats.throttle_samples += 1,
        SchedulerState::DEGRADED => stats.degraded_samples += 1,
        SchedulerState::NORMAL => {}
    }
}

fn record_decision_shared(
    logger: &Arc<Mutex<ObservabilityLogger>>,
    task: &TaskRequest,
    decision: DecisionStatus,
    queue_pressure: u32,
    telemetry: SystemTelemetry,
    vlm_gate: Option<VlmGateDecision>,
    vlm_metadata: VlmRuntimeMetadata,
    state: SchedulerState,
) -> std::io::Result<()> {
    logger
        .lock()
        .expect("logger mutex poisoned")
        .record(&decision_observation(
            task,
            decision,
            queue_pressure,
            telemetry,
            vlm_gate,
            vlm_metadata,
            state,
        ))
}

fn decision_observation(
    task: &TaskRequest,
    decision: DecisionStatus,
    queue_pressure: u32,
    telemetry: SystemTelemetry,
    vlm_gate: Option<VlmGateDecision>,
    vlm_metadata: VlmRuntimeMetadata,
    state: SchedulerState,
) -> TaskObservation {
    TaskObservation {
        stage: ObservationStage::Decision,
        task_id: task.task_id,
        task_type: task.task_type,
        decision,
        queue_pressure,
        scheduler_state: state,
        telemetry_source: TelemetrySource::Synthetic,
        telemetry_valid: true,
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
        change_baseline_ready: None,
        change_score: None,
        change_detected: None,
        vlm_model: vlm_gate.map(|_| vlm_metadata.model_name.to_string()),
        vlm_quantization_bits: vlm_gate.map(|_| vlm_metadata.quantization_bits),
        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
        vlm_output_tokens: None,
        vlm_confidence: None,
        vlm_answer_code: None,
    }
}

fn execution_observation(
    task_id: u64,
    task_type: TaskType,
    decision: DecisionStatus,
    queue_pressure: u32,
    state: SchedulerState,
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
        decision,
        queue_pressure,
        scheduler_state: state,
        telemetry_source: TelemetrySource::Synthetic,
        telemetry_valid: true,
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
        change_baseline_ready: change_result.map(|result| result.baseline_ready),
        change_score: change_result.map(|result| result.score),
        change_detected: change_result.map(|result| result.changed),
        vlm_model: vlm_runtime.map(|(metadata, _, _, _)| metadata.model_name.to_string()),
        vlm_quantization_bits: vlm_runtime.map(|(metadata, _, _, _)| metadata.quantization_bits),
        vlm_gate_reason: vlm_gate.map(|gate| gate.reason.as_str().to_string()),
        vlm_output_tokens: vlm_runtime.map(|(_, tokens, _, _)| tokens),
        vlm_confidence: vlm_runtime.map(|(_, _, confidence, _)| confidence),
        vlm_answer_code: vlm_runtime.map(|(_, _, _, answer_code)| answer_code),
    }
}

fn synthetic_task(mode: StressMode, index: usize) -> TaskRequest {
    let (task_type, priority, memory_estimate_mb) = match mode {
        StressMode::CvFlood => (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32),
        StressMode::VlmBurst => {
            if index % 5 == 0 {
                (TaskType::VLM_QUERY, TaskPriority::HIGH, 2_304)
            } else {
                (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32)
            }
        }
        StressMode::ThermalSpike => {
            if index % 3 == 0 {
                (TaskType::VLM_QUERY, TaskPriority::LOW, 2_304)
            } else {
                (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32)
            }
        }
        StressMode::MixedContention => match deterministic_bucket(index) {
            0 => (TaskType::CV_FEATURES, TaskPriority::HIGH, 32),
            1 => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
            2 => (TaskType::VLM_QUERY, TaskPriority::LOW, 2_304),
            _ => (TaskType::CV_FEATURES, TaskPriority::LOW, 32),
        },
        StressMode::ChangeDetection => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
        StressMode::VlmQuery => (TaskType::VLM_QUERY, TaskPriority::LOW, 2_304),
        StressMode::Phase6Contention => unreachable!("phase6-contention uses phase6_task"),
        StressMode::Phase8Optimization => match deterministic_bucket(index) {
            0 => (TaskType::CV_FEATURES, TaskPriority::HIGH, 32),
            1 => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
            2 => (TaskType::CV_FEATURES, TaskPriority::LOW, 32),
            _ => (TaskType::VLM_QUERY, TaskPriority::LOW, 2_304),
        },
    };

    let frame = match mode {
        StressMode::ChangeDetection => synthetic_change_frame(index),
        StressMode::VlmQuery => vec![32 + (index % 64) as u8; 1024],
        StressMode::Phase6Contention => unreachable!("phase6-contention uses phase6_task"),
        StressMode::Phase8Optimization => match task_type {
            TaskType::CHANGE_DETECTION => synthetic_change_frame(index),
            TaskType::VLM_QUERY => vec![64 + (index % 32) as u8; 4096],
            TaskType::CV_FEATURES => vec![1 + (index % 16) as u8; 4096],
        },
        _ => vec![1; 1024],
    };

    TaskRequest {
        task_id: index as u64 + 1,
        task_type,
        priority,
        memory_estimate_mb,
        deadline_ms: 250,
        pool_slot_id: index % 5,
        frame,
    }
}

fn synthetic_telemetry(mode: StressMode, index: usize, tasks: usize) -> SystemTelemetry {
    match mode {
        StressMode::CvFlood => SystemTelemetry::nominal(),
        StressMode::VlmBurst => {
            if index % 5 == 0 {
                SystemTelemetry {
                    memory_usage_percent: 86.0,
                    temperature_c: 60.0,
                    gpu_utilization: 25.0,
                }
            } else {
                SystemTelemetry::nominal()
            }
        }
        StressMode::ThermalSpike => {
            if index >= tasks / 2 {
                SystemTelemetry {
                    memory_usage_percent: 55.0,
                    temperature_c: 85.0,
                    gpu_utilization: 90.0,
                }
            } else {
                SystemTelemetry::nominal()
            }
        }
        StressMode::MixedContention => {
            let memory_usage_percent = if index % 11 == 0 { 86.0 } else { 62.0 };
            let temperature_c = if index % 7 == 0 { 82.0 } else { 58.0 };
            SystemTelemetry {
                memory_usage_percent,
                temperature_c,
                gpu_utilization: 70.0,
            }
        }
        StressMode::ChangeDetection => SystemTelemetry::nominal(),
        StressMode::VlmQuery => SystemTelemetry::nominal(),
        StressMode::Phase6Contention => SystemTelemetry::nominal(),
        StressMode::Phase8Optimization => {
            let memory_usage_percent = if index % 13 == 0 {
                83.0
            } else {
                56.0 + (index % 10) as f32
            };
            let temperature_c = if index > tasks / 2 {
                70.0 + (index % 12) as f32
            } else {
                58.0 + (index % 8) as f32
            };
            SystemTelemetry {
                memory_usage_percent,
                temperature_c,
                gpu_utilization: 72.0,
            }
        }
    }
}

fn phase6_task(index: usize) -> TaskRequest {
    let (task_type, priority, memory_estimate_mb) = match index % 6 {
        0 => (TaskType::VLM_QUERY, TaskPriority::LOW, 2_304),
        1 | 2 => (TaskType::CV_FEATURES, TaskPriority::HIGH, 32),
        3 => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
        _ => (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32),
    };

    let frame = match task_type {
        TaskType::CHANGE_DETECTION => synthetic_change_frame(index),
        TaskType::VLM_QUERY => vec![48 + (index % 64) as u8; 2048],
        TaskType::CV_FEATURES => vec![1 + (index % 8) as u8; 2048],
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

fn synthetic_change_frame(index: usize) -> Vec<u8> {
    let value = if index < 2 {
        16
    } else if index % 3 == 0 {
        220
    } else {
        32
    };
    vec![value; 1024]
}

fn deterministic_bucket(index: usize) -> usize {
    ((index.wrapping_mul(1_103_515_245).wrapping_add(12_345)) / 65_536) % 4
}

fn mode_delay(mode: StressMode) -> Option<Duration> {
    match mode {
        StressMode::CvFlood => Some(Duration::from_millis(20)),
        StressMode::VlmBurst => Some(Duration::from_millis(100)),
        StressMode::ThermalSpike => Some(Duration::from_millis(50)),
        StressMode::MixedContention => Some(Duration::from_millis(25)),
        StressMode::ChangeDetection => Some(Duration::from_millis(40)),
        StressMode::VlmQuery => Some(Duration::from_millis(100)),
        StressMode::Phase6Contention => Some(Duration::from_millis(10)),
        StressMode::Phase8Optimization => Some(Duration::from_millis(15)),
    }
}

fn parse_mode(value: &str) -> Result<StressMode, Box<dyn std::error::Error>> {
    match value {
        "cv-flood" => Ok(StressMode::CvFlood),
        "vlm-burst" => Ok(StressMode::VlmBurst),
        "thermal-spike" => Ok(StressMode::ThermalSpike),
        "mixed-contention" => Ok(StressMode::MixedContention),
        "change-detection" => Ok(StressMode::ChangeDetection),
        "vlm-query" => Ok(StressMode::VlmQuery),
        "phase6-contention" => Ok(StressMode::Phase6Contention),
        "phase8-optimization" => Ok(StressMode::Phase8Optimization),
        other => Err(format!("unknown mode: {other}").into()),
    }
}

fn mode_name(mode: StressMode) -> &'static str {
    match mode {
        StressMode::CvFlood => "cv-flood",
        StressMode::VlmBurst => "vlm-burst",
        StressMode::ThermalSpike => "thermal-spike",
        StressMode::MixedContention => "mixed-contention",
        StressMode::ChangeDetection => "change-detection",
        StressMode::VlmQuery => "vlm-query",
        StressMode::Phase6Contention => "phase6-contention",
        StressMode::Phase8Optimization => "phase8-optimization",
    }
}

fn sitl_jsonl_path(mode: StressMode) -> PathBuf {
    PathBuf::from(format!("logs/sitl-{}.jsonl", mode_name(mode)))
}

fn sitl_csv_path(mode: StressMode) -> PathBuf {
    PathBuf::from(format!("logs/sitl-{}.csv", mode_name(mode)))
}

fn print_summary(mode: StressMode, tasks: usize, elapsed: Duration, stats: &BenchStats) {
    let metrics = stats.optimization_metrics(tasks, elapsed);
    let profile = OptimizationProfile::default();
    let recommendations = metrics.recommend(profile);

    println!("mode={}", mode_name(mode));
    println!("tasks={tasks}");
    println!("elapsed_ms={}", elapsed.as_millis());
    println!(
        "admitted={} deferred={} rejected={} executed={} changes_detected={} vlm_admitted={} vlm_rejected={} high_load_samples={} throttle_samples={} degraded_samples={}",
        stats.admitted,
        stats.deferred,
        stats.rejected,
        stats.executed,
        stats.changes_detected,
        stats.vlm_admitted,
        stats.vlm_rejected,
        stats.high_load_samples,
        stats.throttle_samples,
        stats.degraded_samples
    );
    println!(
        "execution_time_ms_p50={} execution_time_ms_p95={}",
        metrics.execution_p50_ms, metrics.execution_p95_ms
    );
    println!(
        "runtime_latency_ms_p50={} runtime_latency_ms_p95={}",
        percentile_u64(&stats.runtime_latencies_ms, 50),
        metrics.runtime_p95_ms
    );
    println!(
        "throughput_tps={:.3} admission_rate={:.3} defer_rate={:.3} reject_rate={:.3}",
        metrics.throughput_tps(),
        metrics.admission_rate(),
        metrics.defer_rate(),
        metrics.reject_rate()
    );
    println!(
        "peak_memory_percent={:.3} peak_temperature_c={:.3} max_queue_pressure={}",
        metrics.peak_memory_percent, metrics.peak_temperature_c, metrics.max_queue_pressure
    );
    println!(
        "optimization_recommendations={}",
        recommendations
            .iter()
            .map(|recommendation| recommendation.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
}

fn elapsed_execution_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis() as u64;
    millis.max(1)
}
