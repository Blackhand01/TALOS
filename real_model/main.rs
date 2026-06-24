use std::path::PathBuf;
use std::time::{Duration, Instant};

use talos::{
    run_real_model, AdmissionController, DecisionStatus, GpuLeaseManager, ObservabilityLogger,
    ObservationStage, RealModelBackend, RealModelConfig, SchedulerState, StateMachine,
    SystemTelemetry, TaskObservation, TaskPriority, TaskRequest, TaskType, TelemetryMonitor,
    TelemetrySource,
};

#[derive(Debug)]
struct Args {
    backend: RealModelBackend,
    model: String,
    image_path: Option<PathBuf>,
    prompt: Option<String>,
    tasks: usize,
    telemetry_source: TelemetrySource,
    sample_period_ms: u64,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
    max_new_tokens: u32,
    extra_args: Vec<String>,
}

#[derive(Debug, Default)]
struct Stats {
    admitted: usize,
    deferred: usize,
    rejected: usize,
    executed: usize,
    failed: usize,
    latencies_ms: Vec<u64>,
    peak_memory_percent: f32,
    peak_temperature_c: f32,
    peak_gpu_utilization: f32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run(args).await
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let admission = AdmissionController::default();
    let state_machine = StateMachine::default();
    let lease_manager = GpuLeaseManager::new();
    let mut telemetry_monitor = TelemetryMonitor::new(
        Duration::from_millis(args.sample_period_ms),
        args.telemetry_source,
    );
    let mut logger = ObservabilityLogger::new(&args.log_jsonl, args.log_csv.as_ref())?;
    let mut stats = Stats::default();
    let started = Instant::now();

    println!(
        "profile=hitl backend={} model={} telemetry={}",
        args.backend.name(),
        args.model,
        args.telemetry_source.name()
    );

    for index in 0..args.tasks {
        let sample = telemetry_monitor.tick_sample().await;
        let telemetry = sample.telemetry;
        stats.observe(telemetry);
        let task = real_model_task(index, args.backend);
        let state = state_machine.evaluate(&telemetry, 0);
        let decision = admission.decide(&task, &telemetry, state, lease_manager.is_active(), false);

        if decision.status != DecisionStatus::ADMIT {
            match decision.status {
                DecisionStatus::DEFER => stats.deferred += 1,
                DecisionStatus::REJECT => stats.rejected += 1,
                DecisionStatus::ADMIT => {}
            }
            logger.record(&decision_observation(
                &task,
                decision.status,
                state,
                sample.source,
                sample.valid,
                telemetry,
                args.backend,
            ))?;
            continue;
        }

        let Some(lease) = lease_manager.try_acquire() else {
            stats.deferred += 1;
            logger.record(&decision_observation(
                &task,
                DecisionStatus::DEFER,
                state,
                sample.source,
                sample.valid,
                telemetry,
                args.backend,
            ))?;
            continue;
        };

        stats.admitted += 1;
        let lease_id = lease.id.to_string();
        let config = RealModelConfig {
            backend: args.backend,
            model: args.model.clone(),
            image_path: args.image_path.clone(),
            prompt: args.prompt.clone(),
            max_new_tokens: args.max_new_tokens,
            extra_args: args.extra_args.clone(),
        };
        let execution_started = Instant::now();
        let model_run = {
            let _lease = lease;
            tokio::task::spawn_blocking(move || run_real_model(&config)).await?
        };
        let execution_time_ms = elapsed_ms(execution_started);

        match model_run {
            Ok(model_run) => {
                stats.executed += 1;
                if !model_run.ok {
                    stats.failed += 1;
                }
                stats.latencies_ms.push(model_run.latency_ms);
                logger.record(&TaskObservation {
                    stage: ObservationStage::Execution,
                    task_id: task.task_id,
                    task_type: task.task_type,
                    decision: DecisionStatus::ADMIT,
                    queue_pressure: 0,
                    scheduler_state: state,
                    telemetry_source: sample.source,
                    telemetry_valid: sample.valid,
                    memory_usage_percent: telemetry.memory_usage_percent,
                    temperature_c: telemetry.temperature_c,
                    gpu_utilization: telemetry.gpu_utilization,
                    lease_id: Some(lease_id),
                    pool_slot_id: task.pool_slot_id,
                    latency_ms: Some(model_run.latency_ms),
                    execution_time_ms,
                    runtime_ok: Some(model_run.ok),
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
                    vlm_model: if model_run.backend == RealModelBackend::SmolVlmCuda {
                        Some(model_run.model.clone())
                    } else {
                        None
                    },
                    vlm_quantization_bits: None,
                    vlm_gate_reason: Some(model_run.backend.name().to_string()),
                    vlm_output_tokens: model_run.output_tokens,
                    vlm_confidence: None,
                    vlm_answer_code: None,
                    real_model_backend: Some(model_run.backend.name().to_string()),
                    real_model_name: Some(model_run.model),
                    real_model_exit_code: model_run.exit_code,
                    real_model_peak_cuda_mb: model_run.peak_cuda_allocated_mb,
                })?;
            }
            Err(error) => {
                stats.failed += 1;
                logger.record(&failed_execution_observation(
                    &task,
                    state,
                    sample.source,
                    sample.valid,
                    telemetry,
                    execution_time_ms,
                    &error,
                    args.backend,
                ))?;
            }
        }
    }

    print_summary(&args, started.elapsed(), &stats);
    Ok(())
}

impl Stats {
    fn observe(&mut self, telemetry: SystemTelemetry) {
        self.peak_memory_percent = self.peak_memory_percent.max(telemetry.memory_usage_percent);
        self.peak_temperature_c = self.peak_temperature_c.max(telemetry.temperature_c);
        self.peak_gpu_utilization = self.peak_gpu_utilization.max(telemetry.gpu_utilization);
    }
}

fn real_model_task(index: usize, backend: RealModelBackend) -> TaskRequest {
    let task_type = if backend == RealModelBackend::SmolVlmCuda {
        TaskType::VLM_QUERY
    } else {
        TaskType::CV_FEATURES
    };

    TaskRequest {
        task_id: index as u64 + 1,
        task_type,
        priority: if task_type == TaskType::VLM_QUERY {
            TaskPriority::LOW
        } else {
            TaskPriority::HIGH
        },
        memory_estimate_mb: if task_type == TaskType::VLM_QUERY {
            2_304
        } else {
            512
        },
        deadline_ms: if task_type == TaskType::VLM_QUERY {
            5_000
        } else {
            1_000
        },
        pool_slot_id: index % 5,
        frame: vec![1; 1024],
    }
}

fn decision_observation(
    task: &TaskRequest,
    decision: DecisionStatus,
    state: SchedulerState,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    telemetry: SystemTelemetry,
    backend: RealModelBackend,
) -> TaskObservation {
    TaskObservation {
        stage: ObservationStage::Decision,
        task_id: task.task_id,
        task_type: task.task_type,
        decision,
        queue_pressure: 0,
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
        vlm_model: None,
        vlm_quantization_bits: None,
        vlm_gate_reason: Some("admission_decision".to_string()),
        vlm_output_tokens: None,
        vlm_confidence: None,
        vlm_answer_code: None,
        real_model_backend: Some(backend.name().to_string()),
        real_model_name: None,
        real_model_exit_code: None,
        real_model_peak_cuda_mb: None,
    }
}

fn failed_execution_observation(
    task: &TaskRequest,
    state: SchedulerState,
    telemetry_source: TelemetrySource,
    telemetry_valid: bool,
    telemetry: SystemTelemetry,
    execution_time_ms: u64,
    error: &std::io::Error,
    backend: RealModelBackend,
) -> TaskObservation {
    let code = error.raw_os_error().unwrap_or(1).unsigned_abs();
    TaskObservation {
        stage: ObservationStage::Execution,
        task_id: task.task_id,
        task_type: task.task_type,
        decision: DecisionStatus::ADMIT,
        queue_pressure: 0,
        scheduler_state: state,
        telemetry_source,
        telemetry_valid,
        memory_usage_percent: telemetry.memory_usage_percent,
        temperature_c: telemetry.temperature_c,
        gpu_utilization: telemetry.gpu_utilization,
        lease_id: None,
        pool_slot_id: task.pool_slot_id,
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
        vlm_gate_reason: Some(error.kind().to_string()),
        vlm_output_tokens: None,
        vlm_confidence: None,
        vlm_answer_code: None,
        real_model_backend: Some(backend.name().to_string()),
        real_model_name: None,
        real_model_exit_code: Some(code as i32),
        real_model_peak_cuda_mb: None,
    }
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut backend = RealModelBackend::TensorRtEngine;
    let mut model = String::from("models/vision.engine");
    let mut image_path = None;
    let mut prompt = None;
    let mut tasks = 1usize;
    let mut telemetry_source = TelemetrySource::Sysfs;
    let mut sample_period_ms = 100u64;
    let mut log_jsonl = PathBuf::from("logs/hitl-real-model.jsonl");
    let mut log_csv = None;
    let mut max_new_tokens = 32u32;
    let mut extra_args = Vec::new();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => {
                let value = args.next().ok_or("--backend requires a value")?;
                backend = RealModelBackend::parse(&value)
                    .ok_or_else(|| format!("unknown backend: {value}"))?;
            }
            "--model" | "--engine" | "--onnx" => {
                model = args.next().ok_or("--model requires a value")?;
            }
            "--image-path" => {
                image_path = Some(PathBuf::from(
                    args.next().ok_or("--image-path requires a value")?,
                ));
            }
            "--prompt" => {
                prompt = Some(args.next().ok_or("--prompt requires a value")?);
            }
            "--tasks" => {
                tasks = args.next().ok_or("--tasks requires a value")?.parse()?;
            }
            "--telemetry" => {
                let value = args.next().ok_or("--telemetry requires a value")?;
                telemetry_source = TelemetrySource::parse(&value)
                    .ok_or_else(|| format!("unknown telemetry source: {value}"))?;
            }
            "--sample-ms" => {
                sample_period_ms = args.next().ok_or("--sample-ms requires a value")?.parse()?;
            }
            "--log-jsonl" => {
                log_jsonl = PathBuf::from(args.next().ok_or("--log-jsonl requires a path")?);
            }
            "--log-csv" => {
                log_csv = Some(PathBuf::from(
                    args.next().ok_or("--log-csv requires a path")?,
                ));
            }
            "--no-csv" => {
                log_csv = None;
            }
            "--max-new-tokens" => {
                max_new_tokens = args
                    .next()
                    .ok_or("--max-new-tokens requires a value")?
                    .parse()?;
            }
            "--backend-arg" => {
                extra_args.push(args.next().ok_or("--backend-arg requires a value")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: talos_real_model [--backend tensorrt-engine|tensorrt-onnx|smolvlm-cuda] [--model PATH_OR_ID] [--image-path PATH] [--prompt TEXT] [--tasks N] [--telemetry sysfs|tegrastats|jtop] [--backend-arg ARG] [--log-jsonl PATH] [--no-csv]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        backend,
        model,
        image_path,
        prompt,
        tasks,
        telemetry_source,
        sample_period_ms,
        log_jsonl,
        log_csv,
        max_new_tokens,
        extra_args,
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().max(1) as u64
}

fn print_summary(args: &Args, elapsed: Duration, stats: &Stats) {
    println!("mode=real-model");
    println!("backend={}", args.backend.name());
    println!("model={}", args.model);
    println!("tasks={}", args.tasks);
    println!("elapsed_ms={}", elapsed.as_millis());
    println!(
        "admitted={} deferred={} rejected={} executed={} failed={}",
        stats.admitted, stats.deferred, stats.rejected, stats.executed, stats.failed
    );
    println!(
        "latency_ms_p50={} latency_ms_p95={}",
        percentile(&stats.latencies_ms, 50),
        percentile(&stats.latencies_ms, 95)
    );
    println!(
        "peak_memory_percent={:.3} peak_temperature_c={:.3} peak_gpu_utilization={:.3}",
        stats.peak_memory_percent, stats.peak_temperature_c, stats.peak_gpu_utilization
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
