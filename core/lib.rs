pub mod admission;
pub mod change_detection;
pub mod deployment;
pub mod executor;
pub mod ingestion;
pub mod leases;
pub mod observability;
pub mod optimization;
pub mod profiles;
pub mod scheduler;
pub mod state_machine;
pub mod stress;
pub mod telemetry;
pub mod types;
pub mod vlm;

#[path = "../ipc/cxx_bridge.rs"]
pub mod cxx_bridge;

pub use admission::AdmissionController;
pub use change_detection::{
    ChangeDetectionResult, ChangeDetector, ChangeDetectorConfig, FeatureEmbedding,
};
pub use deployment::{
    CommandOutcome, DeploymentCommand, JetsonHardeningConfig, JetsonHardeningPlan,
};
pub use ingestion::MockFrameIngestor;
pub use leases::{GpuLease, GpuLeaseManager};
pub use observability::{
    default_csv_path, default_jsonl_path, ObservabilityLogger, ObservationStage, TaskObservation,
};
pub use optimization::{
    percentile_u64, OptimizationMetrics, OptimizationProfile, OptimizationRecommendation,
};
pub use profiles::ExecutionProfile;
pub use scheduler::TaskScheduler;
pub use state_machine::StateMachine;
pub use stress::ThermalStressSimulator;
pub use telemetry::{
    JetsonTelemetryMonitor, SyntheticTelemetryMonitor, TelemetryMonitor, TelemetrySample,
    TelemetrySource,
};
pub use types::{
    Decision, DecisionStatus, SchedulerState, SystemTelemetry, TaskPriority, TaskRequest, TaskType,
};
pub use vlm::{QuantizedVlmProfile, VlmGateDecision, VlmGateReason, VlmRuntimeMetadata};
