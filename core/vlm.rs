use crate::types::{DecisionStatus, SchedulerState, SystemTelemetry, TaskRequest, TaskType};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuantizedVlmProfile {
    pub model_name: &'static str,
    pub quantization_bits: u32,
    pub estimated_memory_mb: u64,
    pub max_input_bytes: u64,
    pub max_output_tokens: u32,
    pub soft_memory_gate_percent: f32,
    pub hard_memory_gate_percent: f32,
    pub temperature_gate_c: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VlmGateReason {
    NotVlm,
    GpuLeaseActive,
    InputTooLarge,
    MemoryPressure,
    ThermalPressure,
    HighLoad,
    Throttle,
    Degraded,
    CvBurstActive,
    AdmitQuantized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VlmGateDecision {
    pub status: DecisionStatus,
    pub reason: VlmGateReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VlmRuntimeMetadata {
    pub model_name: &'static str,
    pub quantization_bits: u32,
    pub max_output_tokens: u32,
}

impl Default for QuantizedVlmProfile {
    fn default() -> Self {
        Self {
            model_name: "qwen2.5-vl-int4",
            quantization_bits: 4,
            estimated_memory_mb: 2_304,
            max_input_bytes: 8 * 1024 * 1024,
            max_output_tokens: 128,
            soft_memory_gate_percent: 75.0,
            hard_memory_gate_percent: 85.0,
            temperature_gate_c: 75.0,
        }
    }
}

impl QuantizedVlmProfile {
    pub fn runtime_metadata(self) -> VlmRuntimeMetadata {
        VlmRuntimeMetadata {
            model_name: self.model_name,
            quantization_bits: self.quantization_bits,
            max_output_tokens: self.max_output_tokens,
        }
    }

    pub fn evaluate(
        self,
        task: &TaskRequest,
        telemetry: &SystemTelemetry,
        state: SchedulerState,
        gpu_lease_active: bool,
        cv_burst_active: bool,
    ) -> VlmGateDecision {
        if task.task_type != TaskType::VLM_QUERY {
            return VlmGateDecision {
                status: DecisionStatus::ADMIT,
                reason: VlmGateReason::NotVlm,
            };
        }

        if gpu_lease_active {
            return self.decision(DecisionStatus::DEFER, VlmGateReason::GpuLeaseActive);
        }

        if task.frame.len() as u64 > self.max_input_bytes {
            return self.decision(DecisionStatus::REJECT, VlmGateReason::InputTooLarge);
        }

        if telemetry.memory_usage_percent > self.hard_memory_gate_percent {
            return self.decision(DecisionStatus::REJECT, VlmGateReason::MemoryPressure);
        }

        match state {
            SchedulerState::DEGRADED => {
                return self.decision(DecisionStatus::REJECT, VlmGateReason::Degraded)
            }
            SchedulerState::THROTTLE => {
                return self.decision(DecisionStatus::REJECT, VlmGateReason::Throttle)
            }
            SchedulerState::HIGH_LOAD => {
                return self.decision(DecisionStatus::DEFER, VlmGateReason::HighLoad)
            }
            SchedulerState::NORMAL => {}
        }

        if telemetry.temperature_c >= self.temperature_gate_c {
            return self.decision(DecisionStatus::DEFER, VlmGateReason::ThermalPressure);
        }

        if telemetry.memory_usage_percent >= self.soft_memory_gate_percent {
            return self.decision(DecisionStatus::DEFER, VlmGateReason::MemoryPressure);
        }

        if cv_burst_active {
            return self.decision(DecisionStatus::DEFER, VlmGateReason::CvBurstActive);
        }

        self.decision(DecisionStatus::ADMIT, VlmGateReason::AdmitQuantized)
    }

    const fn decision(self, status: DecisionStatus, reason: VlmGateReason) -> VlmGateDecision {
        VlmGateDecision { status, reason }
    }
}

impl VlmGateReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotVlm => "not_vlm",
            Self::GpuLeaseActive => "gpu_lease_active",
            Self::InputTooLarge => "input_too_large",
            Self::MemoryPressure => "memory_pressure",
            Self::ThermalPressure => "thermal_pressure",
            Self::HighLoad => "high_load",
            Self::Throttle => "throttle",
            Self::Degraded => "degraded",
            Self::CvBurstActive => "cv_burst_active",
            Self::AdmitQuantized => "admit_quantized",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskPriority, TaskRequest};

    fn vlm_task(frame_len: usize) -> TaskRequest {
        TaskRequest {
            task_id: 1,
            task_type: TaskType::VLM_QUERY,
            priority: TaskPriority::LOW,
            memory_estimate_mb: 2_304,
            deadline_ms: 1_000,
            pool_slot_id: 0,
            frame: vec![1; frame_len],
        }
    }

    #[test]
    fn admits_quantized_vlm_under_nominal_conditions() {
        let gate = QuantizedVlmProfile::default().evaluate(
            &vlm_task(1024),
            &SystemTelemetry::nominal(),
            SchedulerState::NORMAL,
            false,
            false,
        );

        assert_eq!(gate.status, DecisionStatus::ADMIT);
        assert_eq!(gate.reason, VlmGateReason::AdmitQuantized);
    }

    #[test]
    fn rejects_vlm_under_hard_memory_pressure() {
        let gate = QuantizedVlmProfile::default().evaluate(
            &vlm_task(1024),
            &SystemTelemetry {
                memory_usage_percent: 86.0,
                temperature_c: 60.0,
                gpu_utilization: 0.0,
            },
            SchedulerState::NORMAL,
            false,
            false,
        );

        assert_eq!(gate.status, DecisionStatus::REJECT);
        assert_eq!(gate.reason, VlmGateReason::MemoryPressure);
    }

    #[test]
    fn rejects_inputs_beyond_quantized_context_budget() {
        let profile = QuantizedVlmProfile::default();
        let gate = profile.evaluate(
            &vlm_task(profile.max_input_bytes as usize + 1),
            &SystemTelemetry::nominal(),
            SchedulerState::NORMAL,
            false,
            false,
        );

        assert_eq!(gate.status, DecisionStatus::REJECT);
        assert_eq!(gate.reason, VlmGateReason::InputTooLarge);
    }
}
