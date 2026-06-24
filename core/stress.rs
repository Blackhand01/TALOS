use crate::types::SystemTelemetry;

#[derive(Clone, Copy, Debug)]
pub struct ThermalStressSimulator {
    sample: SystemTelemetry,
    heating_per_active_tick_c: f32,
    cooling_per_idle_tick_c: f32,
    memory_growth_per_tick_percent: f32,
}

impl Default for ThermalStressSimulator {
    fn default() -> Self {
        Self {
            sample: SystemTelemetry {
                memory_usage_percent: 48.0,
                temperature_c: 58.0,
                gpu_utilization: 0.0,
            },
            heating_per_active_tick_c: 2.4,
            cooling_per_idle_tick_c: 0.6,
            memory_growth_per_tick_percent: 3.2,
        }
    }
}

impl ThermalStressSimulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(&self) -> SystemTelemetry {
        self.sample
    }

    pub fn tick(&mut self, gpu_active: bool, memory_pressure: bool) -> SystemTelemetry {
        if gpu_active {
            self.sample.temperature_c =
                (self.sample.temperature_c + self.heating_per_active_tick_c).min(92.0);
            self.sample.gpu_utilization = 95.0;
        } else {
            self.sample.temperature_c =
                (self.sample.temperature_c - self.cooling_per_idle_tick_c).max(42.0);
            self.sample.gpu_utilization = 15.0;
        }

        if memory_pressure {
            self.sample.memory_usage_percent =
                (self.sample.memory_usage_percent + self.memory_growth_per_tick_percent).min(90.0);
        } else {
            self.sample.memory_usage_percent = (self.sample.memory_usage_percent - 0.4).max(40.0);
        }

        self.sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SchedulerState, StateMachine};

    #[test]
    fn thermal_stress_reaches_throttle_then_degraded() {
        let mut simulator = ThermalStressSimulator::new();
        let machine = StateMachine::default();
        let mut state = SchedulerState::NORMAL;

        for _ in 0..20 {
            let telemetry = simulator.tick(true, true);
            state = machine.evaluate(&telemetry, 0);
        }

        assert_eq!(state, SchedulerState::DEGRADED);
        assert!(simulator.sample().temperature_c > 80.0);
        assert!(simulator.sample().memory_usage_percent > 85.0);
    }
}
