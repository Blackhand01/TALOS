use std::path::PathBuf;
use std::time::Duration;

use talos::executor::dispatch_to_cpp;
use talos::{
    AdmissionController, DecisionStatus, GpuLeaseManager, MockFrameIngestor, SchedulerState,
    StateMachine, SyntheticTelemetryMonitor, SystemTelemetry, TaskRequest, TaskScheduler,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Args {
    demo_dtu: PathBuf,
    max_tasks: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    run_dtu_demo(args.demo_dtu, args.max_tasks).await
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut demo_dtu = PathBuf::from("data/dtu_wind_turbine");
    let mut max_tasks = 3usize;
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
            "--help" | "-h" => {
                println!("Usage: edge_node [--demo-dtu PATH] [--max-tasks N]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Args {
        demo_dtu,
        max_tasks,
    })
}

async fn run_dtu_demo(
    dataset_path: PathBuf,
    max_tasks: usize,
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
) -> Option<JoinHandle<()>> {
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
                Some(tokio::spawn(async move {
                    let _lease = lease;
                    match dispatch_to_cpp(task).await {
                        Ok(result) => {
                            println!(
                                "lease={lease_id} runtime_ok={} latency_ms={}",
                                result.ok, result.latency_ms
                            );
                        }
                        Err(error) => {
                            eprintln!("lease={lease_id} execution join error: {error}");
                        }
                    }
                }))
            } else {
                scheduler.defer(task);
                None
            }
        }
        DecisionStatus::DEFER => {
            scheduler.defer(task);
            None
        }
        DecisionStatus::REJECT => {
            println!(
                "rejected task_type={:?} priority={:?} state={:?}",
                task.task_type, task.priority, state
            );
            None
        }
    }
}
