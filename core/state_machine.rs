use crate::types::{SchedulerState, SystemTelemetry};

#[derive(Clone, Copy, Debug)]
pub struct StateMachine {
    pub queue_pressure_threshold: u32,
    pub thermal_throttle_c: f32,
    pub degraded_memory_percent: f32,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self {
            queue_pressure_threshold: 20,
            thermal_throttle_c: 80.0,
            degraded_memory_percent: 85.0,
        }
    }
}

impl StateMachine {
    pub fn evaluate(&self, telemetry: &SystemTelemetry, queue_pressure: u32) -> SchedulerState {
        if telemetry.memory_usage_percent > self.degraded_memory_percent {
            SchedulerState::DEGRADED
        } else if telemetry.temperature_c > self.thermal_throttle_c {
            SchedulerState::THROTTLE
        } else if queue_pressure > self.queue_pressure_threshold {
            SchedulerState::HIGH_LOAD
        } else {
            SchedulerState::NORMAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_pressure_triggers_high_load() {
        let machine = StateMachine::default();
        assert_eq!(
            machine.evaluate(&SystemTelemetry::nominal(), 21),
            SchedulerState::HIGH_LOAD
        );
    }

    #[test]
    fn thermal_and_memory_transitions_are_more_severe_than_queue_pressure() {
        let machine = StateMachine::default();
        assert_eq!(
            machine.evaluate(
                &SystemTelemetry {
                    memory_usage_percent: 40.0,
                    temperature_c: 81.0,
                    gpu_utilization: 0.0,
                },
                21,
            ),
            SchedulerState::THROTTLE
        );
        assert_eq!(
            machine.evaluate(
                &SystemTelemetry {
                    memory_usage_percent: 86.0,
                    temperature_c: 81.0,
                    gpu_utilization: 0.0,
                },
                21,
            ),
            SchedulerState::DEGRADED
        );
    }
}
