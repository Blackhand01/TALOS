use std::time::Duration;

use tokio::time::{interval, Interval};

use crate::types::SystemTelemetry;

pub const TELEMETRY_PERIOD: Duration = Duration::from_millis(100);

pub struct SyntheticTelemetryMonitor {
    interval: Interval,
    sample: SystemTelemetry,
}

impl SyntheticTelemetryMonitor {
    pub fn new_10hz() -> Self {
        Self {
            interval: interval(TELEMETRY_PERIOD),
            sample: SystemTelemetry::nominal(),
        }
    }

    pub fn period(&self) -> Duration {
        TELEMETRY_PERIOD
    }

    pub async fn tick(&mut self) -> SystemTelemetry {
        self.interval.tick().await;
        self.sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn telemetry_period_is_10hz() {
        let monitor = SyntheticTelemetryMonitor::new_10hz();
        assert_eq!(monitor.period(), Duration::from_millis(100));
    }
}
