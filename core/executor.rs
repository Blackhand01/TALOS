use tokio::task;

use crate::cxx_bridge::ffi::{run_cv_features, run_vlm_query, RuntimeResult};
use crate::types::{TaskRequest, TaskType};

pub async fn dispatch_to_cpp(task: TaskRequest) -> Result<RuntimeResult, task::JoinError> {
    match task.task_type {
        TaskType::CV_FEATURES | TaskType::CHANGE_DETECTION => {
            task::spawn_blocking(move || run_cv_features(&task.frame)).await
        }
        TaskType::VLM_QUERY => task::spawn_blocking(move || run_vlm_query(&task.frame)).await,
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
    async fn change_detection_reuses_cv_embedding_runtime() {
        let task = TaskRequest {
            task_id: 2,
            task_type: TaskType::CHANGE_DETECTION,
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
        assert_eq!(result.feature_dim, 7);
        assert_ne!(result.checksum, 0);
    }

    #[tokio::test]
    async fn vlm_workloads_enter_quantized_vlm_runtime() {
        let task = TaskRequest {
            task_id: 3,
            task_type: TaskType::VLM_QUERY,
            priority: TaskPriority::LOW,
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
        assert_eq!(result.feature_dim, 0);
        assert_eq!(result.input_bytes, 4);
        assert!(result.vlm_output_tokens > 0);
        assert!(result.vlm_confidence > 0.0);
        assert_ne!(result.vlm_answer_code, 0);
    }
}
