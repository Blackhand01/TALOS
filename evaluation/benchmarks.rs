use std::path::PathBuf;
use std::time::{Duration, Instant};

use talos::executor::dispatch_to_cpp;
use talos::{
    default_csv_path, default_jsonl_path, AdmissionController, ChangeDetector, DecisionStatus,
    FeatureEmbedding, GpuLeaseManager, ObservabilityLogger, ObservationStage, SchedulerState,
    StateMachine, SystemTelemetry, TaskObservation, TaskPriority, TaskRequest, TaskScheduler,
    TaskType, TelemetrySource,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StressMode {
    CvFlood,
    VlmBurst,
    ThermalSpike,
    MixedContention,
    ChangeDetection,
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
    execution_times_ms: Vec<u64>,
    runtime_latencies_ms: Vec<u64>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run_benchmark(args).await
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut mode = StressMode::CvFlood;
    let mut tasks = 100usize;
    let mut log_jsonl = default_jsonl_path();
    let mut log_csv = Some(default_csv_path());
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                let value = args.next().ok_or("--mode requires a value")?;
                mode = parse_mode(&value)?;
                log_jsonl = PathBuf::from(format!("logs/bench-{}.jsonl", mode_name(mode)));
                log_csv = Some(PathBuf::from(format!("logs/bench-{}.csv", mode_name(mode))));
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
                println!(
                    "Usage: talos_bench [--mode cv-flood|vlm-burst|thermal-spike|mixed-contention|change-detection] [--tasks N] [--log-jsonl PATH] [--log-csv PATH] [--no-csv]"
                );
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
        let state = state_machine.evaluate(&telemetry, queue_pressure);
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
                    })?;
                } else {
                    stats.deferred += 1;
                    record_decision(
                        &mut logger,
                        &task,
                        DecisionStatus::DEFER,
                        queue_pressure,
                        telemetry,
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
                    state,
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
                    telemetry,
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
    })
}

fn synthetic_task(mode: StressMode, index: usize) -> TaskRequest {
    let (task_type, priority, memory_estimate_mb) = match mode {
        StressMode::CvFlood => (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32),
        StressMode::VlmBurst => {
            if index % 5 == 0 {
                (TaskType::VLM_QUERY, TaskPriority::HIGH, 1_024)
            } else {
                (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32)
            }
        }
        StressMode::ThermalSpike => {
            if index % 3 == 0 {
                (TaskType::VLM_QUERY, TaskPriority::LOW, 1_024)
            } else {
                (TaskType::CV_FEATURES, TaskPriority::MEDIUM, 32)
            }
        }
        StressMode::MixedContention => match deterministic_bucket(index) {
            0 => (TaskType::CV_FEATURES, TaskPriority::HIGH, 32),
            1 => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
            2 => (TaskType::VLM_QUERY, TaskPriority::LOW, 1_024),
            _ => (TaskType::CV_FEATURES, TaskPriority::LOW, 32),
        },
        StressMode::ChangeDetection => (TaskType::CHANGE_DETECTION, TaskPriority::MEDIUM, 64),
    };

    let frame = match mode {
        StressMode::ChangeDetection => synthetic_change_frame(index),
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
    }
}

fn parse_mode(value: &str) -> Result<StressMode, Box<dyn std::error::Error>> {
    match value {
        "cv-flood" => Ok(StressMode::CvFlood),
        "vlm-burst" => Ok(StressMode::VlmBurst),
        "thermal-spike" => Ok(StressMode::ThermalSpike),
        "mixed-contention" => Ok(StressMode::MixedContention),
        "change-detection" => Ok(StressMode::ChangeDetection),
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
    }
}

fn print_summary(mode: StressMode, tasks: usize, elapsed: Duration, stats: &BenchStats) {
    println!("mode={}", mode_name(mode));
    println!("tasks={tasks}");
    println!("elapsed_ms={}", elapsed.as_millis());
    println!(
        "admitted={} deferred={} rejected={} executed={} changes_detected={}",
        stats.admitted, stats.deferred, stats.rejected, stats.executed, stats.changes_detected
    );
    println!(
        "execution_time_ms_p50={} execution_time_ms_p95={}",
        percentile(&stats.execution_times_ms, 50),
        percentile(&stats.execution_times_ms, 95)
    );
    println!(
        "runtime_latency_ms_p50={} runtime_latency_ms_p95={}",
        percentile(&stats.runtime_latencies_ms, 50),
        percentile(&stats.runtime_latencies_ms, 95)
    );
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted[index]
}

fn elapsed_execution_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis() as u64;
    millis.max(1)
}
