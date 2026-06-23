use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{DecisionStatus, SchedulerState, TaskType};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationStage {
    Decision,
    Execution,
}

#[derive(Clone, Debug)]
pub struct TaskObservation {
    pub stage: ObservationStage,
    pub task_id: u64,
    pub task_type: TaskType,
    pub decision: DecisionStatus,
    pub queue_pressure: u32,
    pub scheduler_state: SchedulerState,
    pub lease_id: Option<String>,
    pub pool_slot_id: usize,
    pub latency_ms: Option<u64>,
    pub execution_time_ms: u64,
}

#[derive(Debug)]
pub struct ObservabilityLogger {
    jsonl: BufWriter<File>,
    csv: Option<BufWriter<File>>,
}

impl ObservabilityLogger {
    pub fn new(
        jsonl_path: impl AsRef<Path>,
        csv_path: Option<impl AsRef<Path>>,
    ) -> io::Result<Self> {
        let jsonl_path = jsonl_path.as_ref();
        ensure_parent_dir(jsonl_path)?;
        let jsonl = BufWriter::new(open_append(jsonl_path)?);

        let csv = match csv_path {
            Some(path) => {
                let path = path.as_ref();
                ensure_parent_dir(path)?;
                let is_empty = !path.exists() || path.metadata()?.len() == 0;
                let mut writer = BufWriter::new(open_append(path)?);
                if is_empty {
                    writeln!(
                        writer,
                        "timestamp_ms,trace_id,stage,task_id,task_type,decision,queue_pressure,scheduler_state,lease_id,pool_slot_id,latency_ms,execution_time_ms"
                    )?;
                }
                Some(writer)
            }
            None => None,
        };

        Ok(Self { jsonl, csv })
    }

    pub fn record(&mut self, observation: &TaskObservation) -> io::Result<()> {
        let timestamp_ms = unix_timestamp_ms();
        writeln!(self.jsonl, "{}", observation.to_json_line(timestamp_ms))?;
        self.jsonl.flush()?;

        if let Some(csv) = &mut self.csv {
            writeln!(csv, "{}", observation.to_csv_line(timestamp_ms))?;
            csv.flush()?;
        }

        Ok(())
    }
}

impl TaskObservation {
    pub fn trace_id(&self) -> String {
        format!("task-{}", self.task_id)
    }

    fn to_json_line(&self, timestamp_ms: u128) -> String {
        format!(
            "{{\"timestamp_ms\":{},\"trace_id\":\"{}\",\"stage\":\"{}\",\"task_id\":{},\"task_type\":\"{}\",\"decision\":\"{}\",\"queue_pressure\":{},\"scheduler_state\":\"{}\",\"lease_id\":{},\"pool_slot_id\":{},\"latency_ms\":{},\"execution_time_ms\":{}}}",
            timestamp_ms,
            self.trace_id(),
            stage_name(&self.stage),
            self.task_id,
            task_type_name(self.task_type),
            decision_name(self.decision),
            self.queue_pressure,
            scheduler_state_name(self.scheduler_state),
            optional_json_string(self.lease_id.as_deref()),
            self.pool_slot_id,
            optional_json_u64(self.latency_ms),
            self.execution_time_ms
        )
    }

    fn to_csv_line(&self, timestamp_ms: u128) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            timestamp_ms,
            self.trace_id(),
            stage_name(&self.stage),
            self.task_id,
            task_type_name(self.task_type),
            decision_name(self.decision),
            self.queue_pressure,
            scheduler_state_name(self.scheduler_state),
            self.lease_id.as_deref().unwrap_or(""),
            self.pool_slot_id,
            self.latency_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            self.execution_time_ms
        )
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    Ok(())
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after unix epoch")
        .as_millis()
}

fn optional_json_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", escape_json(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn optional_json_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub const fn stage_name(stage: &ObservationStage) -> &'static str {
    match stage {
        ObservationStage::Decision => "decision",
        ObservationStage::Execution => "execution",
    }
}

pub const fn task_type_name(task_type: TaskType) -> &'static str {
    match task_type {
        TaskType::CV_FEATURES => "CV_FEATURES",
        TaskType::CHANGE_DETECTION => "CHANGE_DETECTION",
        TaskType::VLM_QUERY => "VLM_QUERY",
    }
}

pub const fn decision_name(decision: DecisionStatus) -> &'static str {
    match decision {
        DecisionStatus::ADMIT => "ADMIT",
        DecisionStatus::DEFER => "DEFER",
        DecisionStatus::REJECT => "REJECT",
    }
}

pub const fn scheduler_state_name(state: SchedulerState) -> &'static str {
    match state {
        SchedulerState::NORMAL => "NORMAL",
        SchedulerState::HIGH_LOAD => "HIGH_LOAD",
        SchedulerState::THROTTLE => "THROTTLE",
        SchedulerState::DEGRADED => "DEGRADED",
    }
}

pub fn default_jsonl_path() -> PathBuf {
    PathBuf::from("logs/talos_tasks.jsonl")
}

pub fn default_csv_path() -> PathBuf {
    PathBuf::from("logs/talos_tasks.csv")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn writes_jsonl_and_csv_observation() {
        let root =
            std::env::temp_dir().join(format!("talos-observability-{}", unix_timestamp_ms()));
        let jsonl = root.join("events.jsonl");
        let csv = root.join("events.csv");
        let mut logger = ObservabilityLogger::new(&jsonl, Some(&csv)).expect("logger should open");

        logger
            .record(&TaskObservation {
                stage: ObservationStage::Execution,
                task_id: 42,
                task_type: TaskType::CV_FEATURES,
                decision: DecisionStatus::ADMIT,
                queue_pressure: 5,
                scheduler_state: SchedulerState::NORMAL,
                lease_id: Some("0001".to_string()),
                pool_slot_id: 2,
                latency_ms: Some(3),
                execution_time_ms: 4,
            })
            .expect("observation should write");

        let json = fs::read_to_string(&jsonl).expect("jsonl should be readable");
        assert!(json.contains("\"task_id\":42"));
        assert!(json.contains("\"task_type\":\"CV_FEATURES\""));
        assert!(json.contains("\"lease_id\":\"0001\""));
        assert!(json.contains("\"pool_slot_id\":2"));

        let csv_content = fs::read_to_string(&csv).expect("csv should be readable");
        assert!(csv_content.contains("timestamp_ms,trace_id,stage"));
        assert!(csv_content.contains("task-42,execution,42,CV_FEATURES,ADMIT"));
    }
}
