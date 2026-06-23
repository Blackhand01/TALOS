use std::process::Command;
use std::time::Duration;

use tokio::task;
use tokio::time::{interval, Interval};

use crate::types::SystemTelemetry;

pub const TELEMETRY_PERIOD: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetrySource {
    Synthetic,
    Tegrastats,
    Jtop,
}

#[derive(Clone, Copy, Debug)]
pub struct TelemetrySample {
    pub telemetry: SystemTelemetry,
    pub source: TelemetrySource,
    pub valid: bool,
}

pub struct TelemetryMonitor {
    interval: Interval,
    period: Duration,
    source: TelemetrySource,
    last_good_sample: SystemTelemetry,
}

pub type SyntheticTelemetryMonitor = TelemetryMonitor;
pub type JetsonTelemetryMonitor = TelemetryMonitor;

impl TelemetrySource {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "synthetic" => Some(Self::Synthetic),
            "tegrastats" => Some(Self::Tegrastats),
            "jtop" => Some(Self::Jtop),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Synthetic => "synthetic",
            Self::Tegrastats => "tegrastats",
            Self::Jtop => "jtop",
        }
    }
}

impl TelemetryMonitor {
    pub fn new_10hz() -> Self {
        Self::new(TELEMETRY_PERIOD, TelemetrySource::Synthetic)
    }

    pub fn new(period: Duration, source: TelemetrySource) -> Self {
        Self {
            interval: interval(period),
            period,
            source,
            last_good_sample: SystemTelemetry::nominal(),
        }
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    pub fn source(&self) -> TelemetrySource {
        self.source
    }

    pub async fn tick(&mut self) -> SystemTelemetry {
        self.tick_sample().await.telemetry
    }

    pub async fn tick_sample(&mut self) -> TelemetrySample {
        self.interval.tick().await;
        let sample = sample_source(self.source).await;

        if sample.valid {
            self.last_good_sample = sample.telemetry;
            sample
        } else {
            TelemetrySample {
                telemetry: self.last_good_sample,
                source: sample.source,
                valid: false,
            }
        }
    }
}

async fn sample_source(source: TelemetrySource) -> TelemetrySample {
    match source {
        TelemetrySource::Synthetic => TelemetrySample {
            telemetry: SystemTelemetry::nominal(),
            source,
            valid: true,
        },
        TelemetrySource::Tegrastats => command_sample(source, sample_tegrastats).await,
        TelemetrySource::Jtop => command_sample(source, sample_jtop).await,
    }
}

async fn command_sample(
    source: TelemetrySource,
    sampler: fn() -> Option<SystemTelemetry>,
) -> TelemetrySample {
    match task::spawn_blocking(sampler).await {
        Ok(Some(telemetry)) => TelemetrySample {
            telemetry,
            source,
            valid: true,
        },
        _ => TelemetrySample {
            telemetry: SystemTelemetry::nominal(),
            source,
            valid: false,
        },
    }
}

fn sample_tegrastats() -> Option<SystemTelemetry> {
    let output = Command::new("tegrastats")
        .args(["--interval", "100", "--count", "1"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_tegrastats_line(stdout.lines().next().unwrap_or_default())
}

fn sample_jtop() -> Option<SystemTelemetry> {
    let script = r#"
import json
try:
    from jtop import jtop
    with jtop() as jetson:
        stats = jetson.stats
        ram = stats.get("RAM", {})
        if isinstance(ram, dict):
            ram_used = ram.get("use", ram.get("used", 0))
            ram_total = ram.get("tot", ram.get("total", 0))
            memory = (float(ram_used) / float(ram_total) * 100.0) if ram_total else 0.0
        else:
            memory = float(stats.get("RAM", 0))
        gpu = float(stats.get("GPU", stats.get("GR3D", 0)))
        temp = float(stats.get("Temp GPU", stats.get("GPU Temp", stats.get("temperature", 0))))
        print(json.dumps({"memory_usage_percent": memory, "gpu_utilization": gpu, "temperature_c": temp}))
except Exception:
    raise SystemExit(1)
"#;

    let output = Command::new("python3").args(["-c", script]).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_jtop_json(stdout.trim())
}

pub fn parse_tegrastats_line(line: &str) -> Option<SystemTelemetry> {
    let memory_usage_percent = parse_ram_percent(line)?;
    let gpu_utilization = parse_percent_after(line, "GR3D_FREQ").unwrap_or(0.0);
    let temperature_c = parse_gpu_temperature(line)
        .or_else(|| parse_max_temperature(line))
        .unwrap_or(SystemTelemetry::nominal().temperature_c);

    Some(SystemTelemetry {
        memory_usage_percent,
        temperature_c,
        gpu_utilization,
    })
}

pub fn parse_jtop_json(json: &str) -> Option<SystemTelemetry> {
    Some(SystemTelemetry {
        memory_usage_percent: parse_json_number(json, "memory_usage_percent")?,
        temperature_c: parse_json_number(json, "temperature_c")?,
        gpu_utilization: parse_json_number(json, "gpu_utilization")?,
    })
}

fn parse_ram_percent(line: &str) -> Option<f32> {
    let (_, after_ram) = line.split_once("RAM ")?;
    let token = after_ram.split_whitespace().next()?;
    let (used, total) = token.split_once('/')?;
    let total = total.strip_suffix("MB").unwrap_or(total);
    let used: f32 = used.parse().ok()?;
    let total: f32 = total.parse().ok()?;

    if total <= 0.0 {
        return None;
    }

    Some((used / total) * 100.0)
}

fn parse_percent_after(line: &str, label: &str) -> Option<f32> {
    let (_, after_label) = line.split_once(label)?;
    after_label
        .split_whitespace()
        .find_map(|token| token.strip_suffix('%'))
        .and_then(|value| value.parse().ok())
}

fn parse_gpu_temperature(line: &str) -> Option<f32> {
    parse_temperature_after(line, "GPU@")
}

fn parse_max_temperature(line: &str) -> Option<f32> {
    let mut max_temp = None;
    for token in line.split_whitespace() {
        if let Some((_, value)) = token.split_once('@') {
            if let Some(value) = value.strip_suffix('C') {
                if let Ok(parsed) = value.parse::<f32>() {
                    max_temp = Some(max_temp.map_or(parsed, |current: f32| current.max(parsed)));
                }
            }
        }
    }
    max_temp
}

fn parse_temperature_after(line: &str, label: &str) -> Option<f32> {
    let (_, after_label) = line.split_once(label)?;
    after_label
        .split_whitespace()
        .next()?
        .strip_suffix('C')?
        .parse()
        .ok()
}

fn parse_json_number(json: &str, key: &str) -> Option<f32> {
    let key_pattern = format!("\"{key}\"");
    let (_, after_key) = json.split_once(&key_pattern)?;
    let (_, after_colon) = after_key.split_once(':')?;
    let value = after_colon
        .trim_start()
        .trim_start_matches('"')
        .split(|character: char| {
            !(character.is_ascii_digit() || character == '.' || character == '-')
        })
        .next()?;
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn telemetry_period_is_10hz() {
        let monitor = SyntheticTelemetryMonitor::new_10hz();
        assert_eq!(monitor.period(), Duration::from_millis(100));
        assert_eq!(monitor.source(), TelemetrySource::Synthetic);
    }

    #[test]
    fn parses_tegrastats_line() {
        let line = "RAM 2048/4096MB (lfb 12x4MB) SWAP 0/2048MB CPU [1%@102,off] EMC_FREQ 12% GR3D_FREQ 37% GPU@54.5C CPU@49.0C";
        let telemetry = parse_tegrastats_line(line).expect("tegrastats line should parse");

        assert_eq!(telemetry.memory_usage_percent, 50.0);
        assert_eq!(telemetry.gpu_utilization, 37.0);
        assert_eq!(telemetry.temperature_c, 54.5);
    }

    #[test]
    fn parses_tegrastats_without_gpu_temperature() {
        let line = "RAM 1024/4096MB CPU [1%@102] GR3D_FREQ 4% CPU@45.0C thermal@47.5C";
        let telemetry = parse_tegrastats_line(line).expect("tegrastats line should parse");

        assert_eq!(telemetry.memory_usage_percent, 25.0);
        assert_eq!(telemetry.gpu_utilization, 4.0);
        assert_eq!(telemetry.temperature_c, 47.5);
    }

    #[test]
    fn parses_jtop_json() {
        let telemetry = parse_jtop_json(
            r#"{"memory_usage_percent": 41.5, "temperature_c": 52.0, "gpu_utilization": 13.0}"#,
        )
        .expect("jtop json should parse");

        assert_eq!(telemetry.memory_usage_percent, 41.5);
        assert_eq!(telemetry.temperature_c, 52.0);
        assert_eq!(telemetry.gpu_utilization, 13.0);
    }
}
