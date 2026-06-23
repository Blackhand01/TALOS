use tokio::task;

use crate::cxx_bridge::ffi::{run, RuntimeResult};
use crate::types::TaskRequest;

pub async fn dispatch_to_cpp(task: TaskRequest) -> Result<RuntimeResult, task::JoinError> {
    task::spawn_blocking(move || run(&task.frame)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskPriority, TaskType};

    #[tokio::test]
    async fn ffi_accepts_rust_buffer_and_returns_result() {
        let task = TaskRequest {
            task_type: TaskType::CV_FEATURES,
            priority: TaskPriority::MEDIUM,
            memory_estimate_mb: 1,
            deadline_ms: 100,
            frame: vec![1, 2, 3, 4],
        };
        let result = dispatch_to_cpp(task)
            .await
            .expect("spawn_blocking should complete");
        assert!(result.ok);
        assert_eq!(result.latency_ms, 1);
    }
}
