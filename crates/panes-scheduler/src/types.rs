use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub trait Notifier: Send + Sync + 'static {
    fn send(&self, title: &str, body: &str);
}

pub struct LogNotifier;

impl Notifier for LogNotifier {
    fn send(&self, title: &str, body: &str) {
        tracing::info!(title = %title, body = %body, "notification (no OS backend)");
    }
}

pub type NotifierRef = Arc<dyn Notifier>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ScheduleAction {
    Notify,
    RetryOnce,
    Chain {
        prompt: String,
        workspace_id: Option<String>,
    },
}

impl Default for ScheduleAction {
    fn default() -> Self {
        Self::Notify
    }
}

#[derive(Debug, Clone)]
pub struct Routine {
    pub id: String,
    pub workspace_id: String,
    pub prompt: String,
    pub cron_expr: String,
    pub budget_cap: Option<f64>,
    pub on_complete: ScheduleAction,
    pub on_failure: ScheduleAction,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    BudgetExceeded,
    SkippedWorkspaceBusy,
    Retrying,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::BudgetExceeded => write!(f, "budget_exceeded"),
            Self::SkippedWorkspaceBusy => write!(f, "skipped_workspace_busy"),
            Self::Retrying => write!(f, "retrying"),
        }
    }
}

impl std::str::FromStr for ExecutionStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "budget_exceeded" => Ok(Self::BudgetExceeded),
            "skipped_workspace_busy" => Ok(Self::SkippedWorkspaceBusy),
            "retrying" => Ok(Self::Retrying),
            other => anyhow::bail!("unknown execution status: {other}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutineExecution {
    pub id: String,
    pub routine_id: String,
    pub thread_id: Option<String>,
    pub status: ExecutionStatus,
    pub cost_usd: f64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_action_serde_notify() {
        let action = ScheduleAction::Notify;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"action":"notify"}"#);
        let de: ScheduleAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(de, ScheduleAction::Notify));
    }

    #[test]
    fn test_schedule_action_serde_retry() {
        let action = ScheduleAction::RetryOnce;
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"action":"retry_once"}"#);
        let de: ScheduleAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(de, ScheduleAction::RetryOnce));
    }

    #[test]
    fn test_schedule_action_serde_chain() {
        let action = ScheduleAction::Chain {
            prompt: "follow up".to_string(),
            workspace_id: Some("ws-2".to_string()),
        };
        let json = serde_json::to_string(&action).unwrap();
        let de: ScheduleAction = serde_json::from_str(&json).unwrap();
        match de {
            ScheduleAction::Chain {
                prompt,
                workspace_id,
            } => {
                assert_eq!(prompt, "follow up");
                assert_eq!(workspace_id, Some("ws-2".to_string()));
            }
            other => panic!("expected Chain, got {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_default_is_notify() {
        assert!(matches!(ScheduleAction::default(), ScheduleAction::Notify));
    }

    #[test]
    fn test_execution_status_display_roundtrip() {
        let statuses = [
            ExecutionStatus::Running,
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::BudgetExceeded,
            ExecutionStatus::SkippedWorkspaceBusy,
            ExecutionStatus::Retrying,
        ];
        for status in statuses {
            let s = status.to_string();
            let parsed: ExecutionStatus = s.parse().unwrap();
            assert_eq!(parsed, status);
        }
    }
}
