use crate::types::{
    Decision, DecisionStatus, SchedulerState, SystemTelemetry, TaskPriority, TaskRequest, TaskType,
};

#[derive(Clone, Copy, Debug)]
pub struct AdmissionController {
    pub default_lease_duration_ms: u64,
    pub memory_reject_percent: f32,
    pub vlm_memory_gate_percent: f32,
    pub vlm_temperature_gate_c: f32,
}

impl Default for AdmissionController {
    fn default() -> Self {
        Self {
            default_lease_duration_ms: 1_000,
            memory_reject_percent: 85.0,
            vlm_memory_gate_percent: 75.0,
            vlm_temperature_gate_c: 75.0,
        }
    }
}

impl AdmissionController {
    pub fn decide(
        &self,
        task: &TaskRequest,
        telemetry: &SystemTelemetry,
        state: SchedulerState,
        gpu_lease_active: bool,
        cv_burst_active: bool,
    ) -> Decision {
        if gpu_lease_active {
            return self.decision(DecisionStatus::DEFER);
        }

        if task.task_type == TaskType::VLM_QUERY
            && telemetry.memory_usage_percent > self.memory_reject_percent
        {
            return self.decision(DecisionStatus::REJECT);
        }

        match state {
            SchedulerState::NORMAL => self.normal_policy(task, telemetry, cv_burst_active),
            SchedulerState::HIGH_LOAD => {
                if task.task_type == TaskType::VLM_QUERY {
                    self.decision(DecisionStatus::DEFER)
                } else {
                    self.decision(DecisionStatus::ADMIT)
                }
            }
            SchedulerState::THROTTLE => {
                if task.priority == TaskPriority::LOW || task.task_type == TaskType::VLM_QUERY {
                    self.decision(DecisionStatus::REJECT)
                } else {
                    self.decision(DecisionStatus::ADMIT)
                }
            }
            SchedulerState::DEGRADED => {
                if task.priority == TaskPriority::HIGH && task.task_type == TaskType::CV_FEATURES {
                    self.decision(DecisionStatus::ADMIT)
                } else {
                    self.decision(DecisionStatus::REJECT)
                }
            }
        }
    }

    fn normal_policy(
        &self,
        task: &TaskRequest,
        telemetry: &SystemTelemetry,
        cv_burst_active: bool,
    ) -> Decision {
        if task.task_type != TaskType::VLM_QUERY {
            return self.decision(DecisionStatus::ADMIT);
        }

        if telemetry.memory_usage_percent < self.vlm_memory_gate_percent
            && telemetry.temperature_c < self.vlm_temperature_gate_c
            && !cv_burst_active
        {
            self.decision(DecisionStatus::ADMIT)
        } else {
            self.decision(DecisionStatus::DEFER)
        }
    }

    const fn decision(&self, status: DecisionStatus) -> Decision {
        Decision {
            status,
            lease_duration_ms: self.default_lease_duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(task_type: TaskType, priority: TaskPriority) -> TaskRequest {
        TaskRequest {
            task_type,
            priority,
            memory_estimate_mb: 1,
            deadline_ms: 100,
            frame: vec![1, 2, 3],
        }
    }

    #[test]
    fn active_lease_defers_everything() {
        let controller = AdmissionController::default();
        let decision = controller.decide(
            &task(TaskType::CV_FEATURES, TaskPriority::HIGH),
            &SystemTelemetry::nominal(),
            SchedulerState::NORMAL,
            true,
            false,
        );
        assert_eq!(decision.status, DecisionStatus::DEFER);
    }

    #[test]
    fn memory_pressure_rejects_vlm() {
        let controller = AdmissionController::default();
        let telemetry = SystemTelemetry {
            memory_usage_percent: 86.0,
            temperature_c: 60.0,
            gpu_utilization: 0.0,
        };
        let decision = controller.decide(
            &task(TaskType::VLM_QUERY, TaskPriority::HIGH),
            &telemetry,
            SchedulerState::NORMAL,
            false,
            false,
        );
        assert_eq!(decision.status, DecisionStatus::REJECT);
    }

    #[test]
    fn vlm_gate_uses_memory_temperature_and_cv_burst() {
        let controller = AdmissionController::default();
        let request = task(TaskType::VLM_QUERY, TaskPriority::MEDIUM);
        assert_eq!(
            controller
                .decide(
                    &request,
                    &SystemTelemetry::nominal(),
                    SchedulerState::NORMAL,
                    false,
                    false
                )
                .status,
            DecisionStatus::ADMIT
        );
        assert_eq!(
            controller
                .decide(
                    &request,
                    &SystemTelemetry::nominal(),
                    SchedulerState::NORMAL,
                    false,
                    true
                )
                .status,
            DecisionStatus::DEFER
        );
    }

    #[test]
    fn throttle_rejects_low_priority_and_vlm() {
        let controller = AdmissionController::default();
        assert_eq!(
            controller
                .decide(
                    &task(TaskType::CV_FEATURES, TaskPriority::LOW),
                    &SystemTelemetry::nominal(),
                    SchedulerState::THROTTLE,
                    false,
                    false
                )
                .status,
            DecisionStatus::REJECT
        );
        assert_eq!(
            controller
                .decide(
                    &task(TaskType::VLM_QUERY, TaskPriority::HIGH),
                    &SystemTelemetry::nominal(),
                    SchedulerState::THROTTLE,
                    false,
                    false
                )
                .status,
            DecisionStatus::REJECT
        );
    }

    #[test]
    fn degraded_allows_only_high_priority_cv() {
        let controller = AdmissionController::default();
        assert_eq!(
            controller
                .decide(
                    &task(TaskType::CV_FEATURES, TaskPriority::HIGH),
                    &SystemTelemetry::nominal(),
                    SchedulerState::DEGRADED,
                    false,
                    false
                )
                .status,
            DecisionStatus::ADMIT
        );
        assert_eq!(
            controller
                .decide(
                    &task(TaskType::CHANGE_DETECTION, TaskPriority::HIGH),
                    &SystemTelemetry::nominal(),
                    SchedulerState::DEGRADED,
                    false,
                    false
                )
                .status,
            DecisionStatus::REJECT
        );
    }
}
