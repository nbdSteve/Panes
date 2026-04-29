use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum AgentEvent {
    Thinking {
        text: String,
    },
    Text {
        text: String,
    },
    ToolRequest {
        id: String,
        tool_name: String,
        description: String,
        input: serde_json::Value,
        needs_approval: bool,
        risk_level: RiskLevel,
    },
    ToolResult {
        id: String,
        tool_name: String,
        success: bool,
        output: String,
        raw_output: Option<String>,
        duration_ms: u64,
    },
    CostUpdate {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
        total_usd: f64,
        model: String,
    },
    Error {
        message: String,
        recoverable: bool,
    },
    SubAgentSpawned {
        parent_tool_use_id: String,
        description: String,
    },
    SubAgentComplete {
        parent_tool_use_id: String,
        summary: String,
        cost_usd: f64,
    },
    Complete {
        summary: String,
        total_cost_usd: f64,
        duration_ms: u64,
        turns: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "Low"),
            RiskLevel::Medium => write!(f, "Medium"),
            RiskLevel::High => write!(f, "High"),
            RiskLevel::Critical => write!(f, "Critical"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub thread_id: String,
    pub timestamp: DateTime<Utc>,
    pub event: AgentEvent,
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInit {
    pub session_id: String,
    pub model: String,
    pub cwd: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub briefing: Option<String>,
    pub memories: Vec<String>,
    pub budget_cap: Option<f64>,
}
