use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanesError {
    #[error("{message}")]
    WorkspaceOccupied {
        workspace_id: String,
        message: String,
    },
    #[error("{message}")]
    ThreadNotFound {
        thread_id: String,
        message: String,
    },
    #[error("{message}")]
    AdapterNotFound { adapter: String, message: String },
    #[error("{message}")]
    NoGatePending {
        thread_id: String,
        message: String,
    },
    #[error("{message}")]
    GitError { message: String },
    #[error("{message}")]
    DatabaseError { message: String },
    #[error("{message}")]
    SpawnFailed { message: String },
    #[error("{message}")]
    BudgetExceeded { message: String },
    #[error("{message}")]
    MemoryError { message: String },
    #[error("{message}")]
    ValidationError { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl From<String> for PanesError {
    fn from(s: String) -> Self {
        PanesError::Internal { message: s }
    }
}

impl From<anyhow::Error> for PanesError {
    fn from(e: anyhow::Error) -> Self {
        let msg = e.to_string();
        if msg.contains("already running in this workspace") {
            PanesError::WorkspaceOccupied {
                workspace_id: String::new(),
                message: msg,
            }
        } else if msg.contains("thread not found") {
            PanesError::ThreadNotFound {
                thread_id: String::new(),
                message: msg,
            }
        } else if msg.contains("unknown agent") {
            PanesError::AdapterNotFound {
                adapter: String::new(),
                message: msg,
            }
        } else if msg.contains("no gate pending") {
            PanesError::NoGatePending {
                thread_id: String::new(),
                message: msg,
            }
        } else if msg.contains("failed to spawn") {
            PanesError::SpawnFailed { message: msg }
        } else if msg.contains("Budget cap") {
            PanesError::BudgetExceeded { message: msg }
        } else {
            PanesError::Internal { message: msg }
        }
    }
}
