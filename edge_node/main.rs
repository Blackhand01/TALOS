use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use talos::executor::dispatch_to_cpp;
use talos::{
    default_csv_path, default_jsonl_path, AdmissionController, ChangeDetector, DecisionStatus,
    ExecutionProfile, FeatureEmbedding, GpuLeaseManager, MockFrameIngestor, ObservabilityLogger,
    ObservationStage, SchedulerState, StateMachine, SystemTelemetry, TaskObservation, TaskPriority,
    TaskRequest, TaskScheduler, TaskType, TelemetryMonitor, TelemetrySource, VlmGateDecision,
    VlmRuntimeMetadata,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Args {
    demo_dtu: PathBuf,
    max_tasks: usize,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
    profile: ExecutionProfile,
    telemetry_source: TelemetrySource,
    workload: WorkloadMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkloadMode {
    Cv,
    ChangeDetection,
    Vlm,
    Alternating,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run_dtu_demo(
        args.demo_dtu,
        args.max_tasks,
        args.log_jsonl,
        args.log_csv,
        args.profile,
        args.telemetry_source,
        args.workload,
    )
    .await
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut demo_dtu = PathBuf::from("data/dtu_wind_turbine");
    let mut max_tasks = 3usize;
    let mut log_jsonl = default_jsonl_path();
    let mut log_csv = Some(default_csv_path());
    let mut profile = ExecutionProfile::Sitl;
    let mut telemetry_source = TelemetrySource::Synthetic;
    let mut telemetry_overridden = false;
    let mut workload = WorkloadMode::Cv;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo-dtu" => {
                let value = args.next().ok_or("--demo-dtu requires a path")?;
                demo_dtu = PathBuf::from(value);
            }
            "--max-tasks" => {
                let value = args.next().ok_or("--max-tasks requires a value")?;
                max_tasks = value.parse()?;
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
            "--profile" => {
                let value = args.next().ok_or("--profile requires a profile")?;
                profile = ExecutionProfile::parse(&value).ok_or("profile must be sitl or hitl")?;
            }
            "--telemetry" => {
                let value = args.next().ok_or("--telemetry requires a source")?;
                telemetry_source = TelemetrySource::parse(&value)
                    .ok_or("telemetry source must be synthetic, sysfs, tegrastats, or jtop")?;
                telemetry_overridden = true;
            }
            "--workload" => {
                let value = args.next().ok_or("--workload requires a mode")?;
                workload = parse_workload_mode(&value)
                    .ok_or("workload must be cv, change-detection, vlm, or alternating")?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: edge_node [--demo-dtu PATH] [--max-tasks N] [--log-jsonl PATH] [--log-csv PATH] [--no-csv] [--profile sitl|hitl] [--telemetry synthetic|sysfs|tegrastats|jtop] [--workload cv|change-detection|vlm|alternating]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    if profile == ExecutionProfile::Hitl && telemetry_source == TelemetrySource::Synthetic {
        if telemetry_overridden {
            return Err("hitl profile cannot use synthetic telemetry".into());
        }
        telemetry_source = TelemetrySource::Sysfs;
    }

    Ok(Args {
        demo_dtu,
        max_tasks,
        log_jsonl,
        log_csv,
        profile,
        telemetry_source,
        workload,
    })
}

async fn run_dtu_demo(
    dataset_path: PathBuf,
    max_tasks: usize,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
    profile: ExecutionProfile,
    telemetry_source: TelemetrySource,
    workload: WorkloadMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ingestor = MockFrameIngestor::new(&dataset_path)?;
    if ingestor.is_empty() {
        return Err(format!("no DTU .JPG frames found under {}", dataset_path.display()).into());
    }

    let (task_tx, mut task_rx) = mpsc::channel::<TaskRequest>(16);
    tokio::spawn(async move {
        for _ in 0..max_tasks {
            match ingestor.read_next_task() {
                Ok(Some(mut task)) => {
                    apply_workload(&mut task, workload);
                    if task_tx.send(task).await.is_err() {
                        return;
                    }
                }
                Ok(None) => return,
                Err(error) => {
                    eprintln!("ingestion error: {error}");
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    let admission = AdmissionController::default();
    let state_machine = StateMachine::default();
    let lease_manager = GpuLeaseManager::new();
    let change_detector = Arc::new(Mutex::new(ChangeDetector::default()));
    let logger = Arc::new(Mutex::new(ObservabilityLogger::new(
        &log_jsonl,
        log_csv.as_ref(),
    )?));
    let mut scheduler = TaskScheduler::new();
    let mut telemetry_monitor = TelemetryMonitor::new(Duration::from_millis(100), telemetry_source);
    let mut current_telemetry = SystemTelemetry::nominal();
    let mut current_telemetry_source = telemetry_source;
    let mut current_telemetry_valid = telemetry_source == TelemetrySource::Synthetic;
    let mut current_state = SchedulerState::NORMAL;
    let mut producer_done = false;
    let mut handles: Vec<JoinHandle<()>> = Vec::new();

    println!(
        "profile={} telemetry={}",
        profile.name(),
        telemetry_source.name()
    );

    loop {
        tokio::select! {
            maybe_task = task_rx.recv(), if !producer_done => {
                match maybe_task {
                    Some(task) => {
                        if let Some(handle) = evaluate_and_dispatch(
                            task,
                            &admission,
                            &lease_manager,
                            &mut scheduler,
                            current_telemetry,
                            current_telemetry_source,
                            current_telemetry_valid,
                            current_state,
                            Arc::clone(&change_detector),
                            Arc::clone(&logger),
                        ) {
                            handles.push(handle);
                        }
                    }
                    None => producer_done = true,
                }
            }
            telemetry_sample = telemetry_monitor.tick_sample() => {
                current_telemetry = telemetry_sample.telemetry;
                current_telemetry_source = telemetry_sample.source;
                current_telemetry_valid = telemetry_sample.valid;
                if !telemetry_sample.valid {
                    eprintln!(
                        "telemetry source={} unavailable; using last known sample",
                        telemetry_sample.source.name()
                    );
                }
                current_state = state_machine.evaluate(&current_telemetry, scheduler.queue_pressure());

                if !lease_manager.is_active() {
                    if let Some(task) = scheduler.pop_next() {
                        if let Some(handle) = evaluate_and_dispatch(
                            task,
                            &admission,
                            &lease_manager,
                            &mut scheduler,
                            current_telemetry,
                            current_telemetry_source,
                            current_telemetry_valid,
                            current_state,
                            Arc::clone(&change_detector),
                            Arc::clone(&logger),
                        ) {
                            handles.push(handle);
                        }
                    }
                }

                if producer_done && scheduler.is_empty() && !lease_manager.is_active() {
                    break;
                }
            }
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

fn evaluate_and_dispatch(
    task: TaskRequest,
    admission: &AdmissionController,
    lease_manager: &GpuLeaseManager,
    scheduler: &mut TaskScheduler,
    telemetry: SystemTelemetry,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    state: SchedulerState,
    change_detector: Arc<Mutex<ChangeDetector>>,
    logger: Arc<Mutex<ObservabilityLogger>>,
) -> Option<JoinHandle<()>> {
    let queue_pressure = scheduler.queue_pressure();
    let gpu_lease_active = lease_manager.is_active();
    let cv_burst_active = scheduler.cv_burst_active();
    let vlm_gate = if task.task_type == TaskType::VLM_QUERY {
        Some(admission.vlm_gate_decision(
            &task,
            &telemetry,
            state,
            gpu_lease_active,
            cv_burst_active,
        ))
    } else {
        None
    };
    let vlm_metadata = admission.vlm_profile.runtime_metadata();
    let decision = admission.decide(&task, &telemetry, state, gpu_lease_active, cv_burst_active);

    match decision.status {
        DecisionStatus::ADMIT => {
            if let Some(lease) = lease_manager.try_acquire() {
                let lease_id = lease.id;
                let task_type = task.task_type;
                let task_id = task.task_id;
                let pool_slot_id = task.pool_slot_id;
                Some(tokio::spawn(async move {
                    let _lease = lease;
                    let started = Instant::now();
                    match dispatch_to_cpp(task).await {
                        Ok(result) => {
                            let execution_time_ms = elapsed_execution_ms(started);
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
                            record_observation(
                                &logger,
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
                                    lease_id: Some(lease_id.to_string()),
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
                                    change_baseline_ready: change_result
                                        .map(|result| result.baseline_ready),
                                    change_score: change_result.map(|result| result.score),
                                    change_detected: change_result.map(|result| result.changed),
                                    vlm_model: vlm_runtime
                                        .map(|(metadata, _, _, _)| metadata.model_name.to_string()),
                                    vlm_quantization_bits: vlm_runtime
                                        .map(|(metadata, _, _, _)| metadata.quantization_bits),
                                    vlm_gate_reason: vlm_gate
                                        .map(|gate| gate.reason.as_str().to_string()),
                                    vlm_output_tokens: vlm_runtime.map(|(_, tokens, _, _)| tokens),
                                    vlm_confidence: vlm_runtime
                                        .map(|(_, _, confidence, _)| confidence),
                                    vlm_answer_code: vlm_runtime
                                        .map(|(_, _, _, answer_code)| answer_code),
                                    real_model_backend: None,
                                    real_model_name: None,
                                    real_model_exit_code: None,
                                    real_model_peak_cuda_mb: None,
                                },
                            );
                            println!(
                                "lease={lease_id} runtime_ok={} latency_ms={} feature_dim={} checksum={} change_score={} vlm_tokens={}",
                                result.ok,
                                result.latency_ms,
                                result.feature_dim,
                                result.checksum,
                                change_result
                                    .map(|result| format!("{:.4}", result.score))
                                    .unwrap_or_else(|| "n/a".to_string()),
                                vlm_runtime
                                    .map(|(_, tokens, _, _)| tokens.to_string())
                                    .unwrap_or_else(|| "n/a".to_string())
                            );
                        }
                        Err(error) => {
                            let execution_time_ms = elapsed_execution_ms(started);
                            record_observation(
                                &logger,
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
                                    lease_id: Some(lease_id.to_string()),
                                    pool_slot_id,
                                    latency_ms: None,
                                    execution_time_ms,
                                    runtime_ok: Some(false),
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
                                    vlm_model: None,
                                    vlm_quantization_bits: None,
                                    vlm_gate_reason: vlm_gate
                                        .map(|gate| gate.reason.as_str().to_string()),
                                    vlm_output_tokens: None,
                                    vlm_confidence: None,
                                    vlm_answer_code: None,
                                    real_model_backend: None,
                                    real_model_name: None,
                                    real_model_exit_code: None,
                                    real_model_peak_cuda_mb: None,
                                },
                            );
                            eprintln!("lease={lease_id} execution join error: {error}");
                        }
                    }
                }))
            } else {
                record_decision_observation(
                    &logger,
                    &task,
                    DecisionStatus::DEFER,
                    queue_pressure,
                    telemetry,
                    telemetry_source,
                    telemetry_valid,
                    vlm_gate,
                    vlm_metadata,
                    state,
                );
                scheduler.defer(task);
                None
            }
        }
        DecisionStatus::DEFER => {
            record_decision_observation(
                &logger,
                &task,
                DecisionStatus::DEFER,
                queue_pressure,
                telemetry,
                telemetry_source,
                telemetry_valid,
                vlm_gate,
                vlm_metadata,
                state,
            );
            scheduler.defer(task);
            None
        }
        DecisionStatus::REJECT => {
            record_decision_observation(
                &logger,
                &task,
                DecisionStatus::REJECT,
                queue_pressure,
                telemetry,
                telemetry_source,
                telemetry_valid,
                vlm_gate,
                vlm_metadata,
                state,
            );
            println!(
                "rejected task_type={:?} priority={:?} state={:?}",
                task.task_type, task.priority, state
            );
            None
        }
    }
}

fn record_decision_observation(
    logger: &Arc<Mutex<ObservabilityLogger>>,
    task: &TaskRequest,
    decision: DecisionStatus,
    queue_pressure: u32,
    telemetry: SystemTelemetry,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    vlm_gate: Option<VlmGateDecision>,
    vlm_metadata: VlmRuntimeMetadata,
    state: SchedulerState,
) {
    record_observation(
        logger,
        TaskObservation {
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
        },
    );
}

fn record_observation(logger: &Arc<Mutex<ObservabilityLogger>>, observation: TaskObservation) {
    if let Err(error) = logger
        .lock()
        .expect("observability logger mutex poisoned")
        .record(&observation)
    {
        eprintln!("observability write error: {error}");
    }
}

fn elapsed_execution_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis() as u64;
    millis.max(1)
}

fn parse_workload_mode(value: &str) -> Option<WorkloadMode> {
    match value {
        "cv" => Some(WorkloadMode::Cv),
        "change-detection" => Some(WorkloadMode::ChangeDetection),
        "vlm" => Some(WorkloadMode::Vlm),
        "alternating" => Some(WorkloadMode::Alternating),
        _ => None,
    }
}

fn apply_workload(task: &mut TaskRequest, workload: WorkloadMode) {
    task.task_type = match workload {
        WorkloadMode::Cv => TaskType::CV_FEATURES,
        WorkloadMode::ChangeDetection => TaskType::CHANGE_DETECTION,
        WorkloadMode::Vlm => TaskType::VLM_QUERY,
        WorkloadMode::Alternating => {
            if task.task_id % 3 == 0 {
                TaskType::VLM_QUERY
            } else if task.task_id % 2 == 0 {
                TaskType::CHANGE_DETECTION
            } else {
                TaskType::CV_FEATURES
            }
        }
    };

    if task.task_type == TaskType::VLM_QUERY {
        task.priority = TaskPriority::LOW;
        task.memory_estimate_mb = 2_304;
        task.deadline_ms = 1_000;
    }
}
