pub mod admission;
pub mod executor;
pub mod ingestion;
pub mod leases;
pub mod scheduler;
pub mod state_machine;
pub mod telemetry;
pub mod types;

#[path = "../ipc/cxx_bridge.rs"]
pub mod cxx_bridge;

pub use admission::AdmissionController;
pub use ingestion::MockFrameIngestor;
pub use leases::{GpuLease, GpuLeaseManager};
pub use scheduler::TaskScheduler;
pub use state_machine::StateMachine;
pub use telemetry::SyntheticTelemetryMonitor;
pub use types::{
    Decision, DecisionStatus, SchedulerState, SystemTelemetry, TaskPriority, TaskRequest, TaskType,
};
