use std::collections::VecDeque;

use crate::types::{TaskPriority, TaskRequest, TaskType};

pub const HIGH_PRIORITY_WEIGHT: u32 = 10;
pub const MEDIUM_PRIORITY_WEIGHT: u32 = 5;
pub const LOW_PRIORITY_WEIGHT: u32 = 1;

#[derive(Debug, Default)]
pub struct TaskScheduler {
    queue: VecDeque<TaskRequest>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn defer(&mut self, task: TaskRequest) {
        self.queue.push_back(task);
    }

    pub fn pop_next(&mut self) -> Option<TaskRequest> {
        self.queue.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn queue_pressure(&self) -> u32 {
        self.queue
            .iter()
            .map(|task| priority_weight(task.priority))
            .sum()
    }

    pub fn cv_burst_active(&self) -> bool {
        self.queue
            .iter()
            .filter(|task| task.task_type == TaskType::CV_FEATURES)
            .count()
            >= 2
    }
}

pub const fn priority_weight(priority: TaskPriority) -> u32 {
    match priority {
        TaskPriority::HIGH => HIGH_PRIORITY_WEIGHT,
        TaskPriority::MEDIUM => MEDIUM_PRIORITY_WEIGHT,
        TaskPriority::LOW => LOW_PRIORITY_WEIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(priority: TaskPriority) -> TaskRequest {
        TaskRequest {
            task_id: 1,
            task_type: TaskType::CV_FEATURES,
            priority,
            memory_estimate_mb: 1,
            deadline_ms: 100,
            pool_slot_id: 0,
            frame: vec![1],
        }
    }

    #[test]
    fn queue_pressure_is_derived_from_queued_tasks() {
        let mut scheduler = TaskScheduler::new();
        scheduler.defer(task(TaskPriority::HIGH));
        scheduler.defer(task(TaskPriority::MEDIUM));
        scheduler.defer(task(TaskPriority::LOW));
        assert_eq!(scheduler.queue_pressure(), 16);
    }
}
