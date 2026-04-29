use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub workspace_id: Option<String>,
    pub memory_type: MemoryType,
    pub content: String,
    pub source_thread_id: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Decision,
    Preference,
    Constraint,
    Pattern,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryType::Decision => write!(f, "decision"),
            MemoryType::Preference => write!(f, "preference"),
            MemoryType::Constraint => write!(f, "constraint"),
            MemoryType::Pattern => write!(f, "pattern"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Briefing {
    pub id: String,
    pub workspace_id: String,
    pub content: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InjectedContext {
    pub briefing: Option<String>,
    pub memories: Vec<Memory>,
    pub token_estimate: usize,
}
