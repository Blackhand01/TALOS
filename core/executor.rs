use tokio::task;

use crate::cxx_bridge::ffi::{run_cv_features, RuntimeResult};
use crate::types::{TaskRequest, TaskType};

pub async fn dispatch_to_cpp(task: TaskRequest) -> Result<RuntimeResult, task::JoinError> {
    if task.task_type != TaskType::CV_FEATURES {
        return Ok(unsupported_runtime_result(task.frame.len() as u64));
    }

    task::spawn_blocking(move || run_cv_features(&task.frame)).await
}

const fn unsupported_runtime_result(input_bytes: u64) -> RuntimeResult {
    RuntimeResult {
        ok: false,
        latency_ms: 0,
        feature_dim: 0,
        input_bytes,
        mean: 0.0,
        variance: 0.0,
        min_value: 0.0,
        max_value: 0.0,
        edge_density: 0.0,
        entropy: 0.0,
        checksum: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskPriority, TaskType};

    #[tokio::test]
    async fn ffi_accepts_rust_buffer_and_returns_result() {
        let task = TaskRequest {
            task_id: 1,
            task_type: TaskType::CV_FEATURES,
            priority: TaskPriority::MEDIUM,
            memory_estimate_mb: 1,
            deadline_ms: 100,
            pool_slot_id: 0,
            frame: vec![1, 2, 3, 4],
        };
        let result = dispatch_to_cpp(task)
            .await
            .expect("spawn_blocking should complete");
        assert!(result.ok);
        assert_eq!(result.latency_ms, 1);
        assert_eq!(result.feature_dim, 7);
        assert_eq!(result.input_bytes, 4);
        assert!(result.mean > 0.0);
        assert!(result.entropy > 0.0);
        assert_ne!(result.checksum, 0);
    }

    #[tokio::test]
    async fn non_cv_workloads_do_not_enter_cv_runtime() {
        let task = TaskRequest {
            task_id: 2,
            task_type: TaskType::VLM_QUERY,
            priority: TaskPriority::LOW,
            memory_estimate_mb: 1,
            deadline_ms: 100,
            pool_slot_id: 0,
            frame: vec![1, 2, 3, 4],
        };

        let result = dispatch_to_cpp(task)
            .await
            .expect("unsupported workload should return synchronously");
        assert!(!result.ok);
        assert_eq!(result.latency_ms, 0);
        assert_eq!(result.feature_dim, 0);
        assert_eq!(result.input_bytes, 4);
    }
}
