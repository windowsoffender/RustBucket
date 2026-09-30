use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;
use crate::message::CommandOutput;

/// Status of a task
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl ToString for TaskStatus {
    fn to_string(&self) -> String {
        match self {
            TaskStatus::Pending => "Pending".to_string(),
            TaskStatus::InProgress => "In Progress".to_string(),
            TaskStatus::Completed => "Completed".to_string(),
            TaskStatus::Failed => "Failed".to_string(),
            TaskStatus::Cancelled => "Cancelled".to_string(),
        }
    }
}

/// A task to be executed by an agent/implant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique ID for this task
    pub id: Uuid,

    /// implant/Agent ID this task is assigned to
    pub implant_id: Uuid,

    /// Session ID for this task
    pub session_id: usize,

    /// Command to execute to do
    pub command: String,

    /// Arguments for the command
    pub args: Vec<String>,

    /// Optional binary payload (base64 on the wire), used by `upload`
    #[serde(default, with = "crate::message::base64_bytes_opt")]
    pub data: Option<Vec<u8>>,

    /// When the task was created
    pub created_at: SystemTime,

    /// Current status of the task
    pub status: TaskStatus,
}

/// Task result from an agent/implant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Task ID this result belongs to
    pub task_id: Uuid,

    /// implant/Agent ID that executed the task
    pub implant_id: Uuid,

    /// Session ID for this task
    pub session_id: usize,

    /// Output from the command
    pub output: CommandOutput,

    /// Error output, if any
    pub error: Option<String>,

    /// Status code returned by the command
    pub status_code: Option<i32>,

    /// Status of the task
    pub status: TaskStatus,

    /// When the result was created
    pub completed_at: SystemTime,
    // /// Execution time in milliseconds
    // pub execution_time_ms: u64,
}

/// Task request from the operator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    /// implant/Agent ID this task is for
    pub implant_id: Uuid,

    /// Command to execute
    pub command: String,

    /// Command arguments
    pub args: Vec<String>,

    /// Optional timeout in seconds
    pub timeout: Option<u64>,
}

/// Serializable task response for API endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    /// Task ID
    pub id: Uuid,

    /// implant ID
    pub implant_id: Uuid,

    /// Command
    pub command: String,

    /// Command arguments
    pub args: Vec<String>,

    /// Status
    pub status: String,

    /// Created timestamp (ISO format)
    pub created_at: String,
}

impl From<Task> for TaskResponse {
    fn from(task: Task) -> Self {
        // Convert SystemTime to ISO string (this is a simplification)
        let created_at = match task.created_at.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(n) => {
                let secs = n.as_secs();
                format!("{}", secs)
            }
            Err(_) => "Invalid time".to_string(),
        };

        Self {
            id: task.id,
            implant_id: task.implant_id,
            command: task.command,
            args: task.args,
            status: task.status.to_string(),
            created_at,
        }
    }
}

/// Serializable task result response for API endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultResponse {
    /// Task ID
    pub task_id: Uuid,

    /// Output
    pub output: CommandOutput,

    /// Error (if any)
    pub error: Option<String>,

    /// Status
    pub status: String,

    /// Status code
    pub status_code: Option<i32>,

    /// Completed timestamp (ISO format)
    pub completed_at: String,
    // /// Execution time in milliseconds
    // pub execution_time_ms: u64,
}

impl From<TaskResult> for TaskResultResponse {
    fn from(result: TaskResult) -> Self {
        // Convert SystemTime to ISO string (this is a simplification)
        let completed_at = match result.completed_at.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(n) => {
                let secs = n.as_secs();
                format!("{}", secs)
            }
            Err(_) => "Invalid time".to_string(),
        };

        Self {
            task_id: result.task_id,
            output: result.output,
            error: result.error,
            status: result.status.to_string(),
            status_code: result.status_code,
            completed_at,
            // execution_time_ms: result.execution_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_data_is_base64_on_the_wire() {
        let task = Task {
            id: Uuid::new_v4(),
            implant_id: Uuid::new_v4(),
            session_id: 1,
            command: "upload".to_string(),
            args: vec!["remote.bin".to_string()],
            data: Some(vec![0, 1, 2, 253, 254, 255]),
            created_at: SystemTime::now(),
            status: TaskStatus::Pending,
        };

        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("\"data\":\"AAEC/f7/\""), "{}", json);

        let decoded: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.data, Some(vec![0, 1, 2, 253, 254, 255]));
    }

    #[test]
    fn task_without_data_round_trips() {
        let task = Task {
            id: Uuid::new_v4(),
            implant_id: Uuid::new_v4(),
            session_id: 1,
            command: "pwd".to_string(),
            args: vec![],
            data: None,
            created_at: SystemTime::now(),
            status: TaskStatus::Pending,
        };

        let json = serde_json::to_string(&task).unwrap();
        let decoded: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.data, None);
    }
}
