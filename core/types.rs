#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskType {
    CV_FEATURES,
    CHANGE_DETECTION,
    VLM_QUERY,
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskPriority {
    HIGH,
    MEDIUM,
    LOW,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRequest {
    pub task_id: u64,
    pub task_type: TaskType,
    pub priority: TaskPriority,
    pub memory_estimate_mb: u64,
    pub deadline_ms: u64,
    pub pool_slot_id: usize,
    pub frame: Vec<u8>,
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionStatus {
    ADMIT,
    DEFER,
    REJECT,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    pub status: DecisionStatus,
    pub lease_duration_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemTelemetry {
    pub memory_usage_percent: f32,
    pub temperature_c: f32,
    pub gpu_utilization: f32,
}

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerState {
    NORMAL,
    HIGH_LOAD,
    THROTTLE,
    DEGRADED,
}

impl SystemTelemetry {
    pub const fn nominal() -> Self {
        Self {
            memory_usage_percent: 40.0,
            temperature_c: 45.0,
            gpu_utilization: 0.0,
        }
    }
}
