use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use talos::executor::dispatch_to_cpp;
use talos::{
    default_csv_path, default_jsonl_path, AdmissionController, DecisionStatus, GpuLeaseManager,
    MockFrameIngestor, ObservabilityLogger, ObservationStage, SchedulerState, StateMachine,
    SyntheticTelemetryMonitor, SystemTelemetry, TaskObservation, TaskRequest, TaskScheduler,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Args {
    demo_dtu: PathBuf,
    max_tasks: usize,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run_dtu_demo(args.demo_dtu, args.max_tasks, args.log_jsonl, args.log_csv).await
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut demo_dtu = PathBuf::from("data/dtu_wind_turbine");
    let mut max_tasks = 3usize;
    let mut log_jsonl = default_jsonl_path();
    let mut log_csv = Some(default_csv_path());
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
            "--help" | "-h" => {
                println!(
                    "Usage: edge_node [--demo-dtu PATH] [--max-tasks N] [--log-jsonl PATH] [--log-csv PATH] [--no-csv]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        demo_dtu,
        max_tasks,
        log_jsonl,
        log_csv,
    })
}

async fn run_dtu_demo(
    dataset_path: PathBuf,
    max_tasks: usize,
    log_jsonl: PathBuf,
    log_csv: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ingestor = MockFrameIngestor::new(&dataset_path)?;
    if ingestor.is_empty() {
        return Err(format!("no DTU .JPG frames found under {}", dataset_path.display()).into());
    }

    let (task_tx, mut task_rx) = mpsc::channel::<TaskRequest>(16);
    tokio::spawn(async move {
        for _ in 0..max_tasks {
            match ingestor.read_next_task() {
                Ok(Some(task)) => {
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
    let logger = Arc::new(Mutex::new(ObservabilityLogger::new(
        &log_jsonl,
        log_csv.as_ref(),
    )?));
    let mut scheduler = TaskScheduler::new();
    let mut telemetry_monitor = SyntheticTelemetryMonitor::new_10hz();
    let mut current_telemetry = SystemTelemetry::nominal();
    let mut current_state = SchedulerState::NORMAL;
    let mut producer_done = false;
    let mut handles: Vec<JoinHandle<()>> = Vec::new();

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
                            current_state,
                            Arc::clone(&logger),
                        ) {
                            handles.push(handle);
                        }
                    }
                    None => producer_done = true,
                }
            }
            telemetry_update = telemetry_monitor.tick() => {
                current_telemetry = telemetry_update;
                current_state = state_machine.evaluate(&current_telemetry, scheduler.queue_pressure());

                if !lease_manager.is_active() {
                    if let Some(task) = scheduler.pop_next() {
                        if let Some(handle) = evaluate_and_dispatch(
                            task,
                            &admission,
                            &lease_manager,
                            &mut scheduler,
                            current_telemetry,
                            current_state,
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
    state: SchedulerState,
    logger: Arc<Mutex<ObservabilityLogger>>,
) -> Option<JoinHandle<()>> {
    let queue_pressure = scheduler.queue_pressure();
    let decision = admission.decide(
        &task,
        &telemetry,
        state,
        lease_manager.is_active(),
        scheduler.cv_burst_active(),
    );

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
                            record_observation(
                                &logger,
                                TaskObservation {
                                    stage: ObservationStage::Execution,
                                    task_id,
                                    task_type,
                                    decision: DecisionStatus::ADMIT,
                                    queue_pressure,
                                    scheduler_state: state,
                                    lease_id: Some(lease_id.to_string()),
                                    pool_slot_id,
                                    latency_ms: Some(result.latency_ms),
                                    execution_time_ms,
                                },
                            );
                            println!(
                                "lease={lease_id} runtime_ok={} latency_ms={}",
                                result.ok, result.latency_ms
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
                                    lease_id: Some(lease_id.to_string()),
                                    pool_slot_id,
                                    latency_ms: None,
                                    execution_time_ms,
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
            lease_id: None,
            pool_slot_id: task.pool_slot_id,
            latency_ms: None,
            execution_time_ms: 0,
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
