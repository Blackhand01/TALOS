use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::types::{TaskPriority, TaskRequest, TaskType};

pub const DEFAULT_PHASE1_DEADLINE_MS: u64 = 250;
pub const DEFAULT_POOL_SLOT_COUNT: usize = 5;

#[derive(Debug)]
pub struct MockFrameIngestor {
    files: Vec<PathBuf>,
    next_index: usize,
    pool_slot_count: usize,
}

impl MockFrameIngestor {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::new_with_pool_slots(root, DEFAULT_POOL_SLOT_COUNT)
    }

    pub fn new_with_pool_slots(root: impl AsRef<Path>, pool_slot_count: usize) -> io::Result<Self> {
        assert!(pool_slot_count > 0, "pool_slot_count must be positive");
        let mut files = Vec::new();
        discover_jpg_files(root.as_ref(), &mut files)?;
        files.sort();
        Ok(Self {
            files,
            next_index: 0,
            pool_slot_count,
        })
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn read_next_task(&mut self) -> io::Result<Option<TaskRequest>> {
        if self.files.is_empty() {
            return Ok(None);
        }

        let path = &self.files[self.next_index % self.files.len()];
        let task_id = self.next_index as u64 + 1;
        let pool_slot_id = self.next_index % self.pool_slot_count;
        self.next_index += 1;
        let frame = fs::read(path)?;
        let memory_estimate_mb = bytes_to_estimated_mb(frame.len());

        Ok(Some(TaskRequest {
            task_id,
            task_type: TaskType::CV_FEATURES,
            priority: TaskPriority::MEDIUM,
            memory_estimate_mb,
            deadline_ms: DEFAULT_PHASE1_DEADLINE_MS,
            pool_slot_id,
            frame,
        }))
    }
}

fn discover_jpg_files(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            discover_jpg_files(&path, files)?;
        } else if is_jpg(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_jpg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("jpg"))
        .unwrap_or(false)
}

fn bytes_to_estimated_mb(bytes: usize) -> u64 {
    let mb = 1024 * 1024;
    bytes.div_ceil(mb).max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_and_loads_dtu_frames_when_dataset_is_present() {
        let dataset = Path::new("data/dtu_wind_turbine");
        if !dataset.exists() {
            return;
        }

        let mut ingestor = MockFrameIngestor::new(dataset).expect("ingestor should initialize");
        assert!(ingestor.len() > 0);
        let task = ingestor
            .read_next_task()
            .expect("frame should load")
            .expect("dataset should contain at least one JPG");

        assert_eq!(task.task_type, TaskType::CV_FEATURES);
        assert_eq!(task.priority, TaskPriority::MEDIUM);
        assert_eq!(task.task_id, 1);
        assert_eq!(task.pool_slot_id, 0);
        assert!(task.memory_estimate_mb >= 1);
        assert_eq!(task.deadline_ms, DEFAULT_PHASE1_DEADLINE_MS);
        assert!(!task.frame.is_empty());
    }
}
