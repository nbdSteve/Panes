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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // --- RiskLevel Display tests ---

    #[test]
    fn test_risk_level_display_low() {
        assert_eq!(format!("{}", RiskLevel::Low), "Low");
    }

    #[test]
    fn test_risk_level_display_medium() {
        assert_eq!(format!("{}", RiskLevel::Medium), "Medium");
    }

    #[test]
    fn test_risk_level_display_high() {
        assert_eq!(format!("{}", RiskLevel::High), "High");
    }

    #[test]
    fn test_risk_level_display_critical() {
        assert_eq!(format!("{}", RiskLevel::Critical), "Critical");
    }

    #[test]
    fn test_risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);
        assert!(RiskLevel::Low < RiskLevel::Critical);
    }

    // --- AgentEvent serde roundtrip tests ---

    #[test]
    fn test_agent_event_serde_text() {
        let event = AgentEvent::Text {
            text: "hello world".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::Text { text } => assert_eq!(text, "hello world"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_thinking() {
        let event = AgentEvent::Thinking {
            text: "pondering...".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::Thinking { text } => assert_eq!(text, "pondering..."),
            other => panic!("expected Thinking, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_tool_request() {
        let event = AgentEvent::ToolRequest {
            id: "tr-1".to_string(),
            tool_name: "bash".to_string(),
            description: "run a command".to_string(),
            input: serde_json::json!({"cmd": "ls"}),
            needs_approval: true,
            risk_level: RiskLevel::High,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::ToolRequest {
                id,
                tool_name,
                description,
                input,
                needs_approval,
                risk_level,
            } => {
                assert_eq!(id, "tr-1");
                assert_eq!(tool_name, "bash");
                assert_eq!(description, "run a command");
                assert_eq!(input, serde_json::json!({"cmd": "ls"}));
                assert!(needs_approval);
                assert_eq!(risk_level, RiskLevel::High);
            }
            other => panic!("expected ToolRequest, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_tool_result() {
        let event = AgentEvent::ToolResult {
            id: "tr-1".to_string(),
            tool_name: "bash".to_string(),
            success: true,
            output: "file.txt".to_string(),
            raw_output: Some("file.txt\n".to_string()),
            duration_ms: 42,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::ToolResult {
                id,
                tool_name,
                success,
                output,
                raw_output,
                duration_ms,
            } => {
                assert_eq!(id, "tr-1");
                assert_eq!(tool_name, "bash");
                assert!(success);
                assert_eq!(output, "file.txt");
                assert_eq!(raw_output, Some("file.txt\n".to_string()));
                assert_eq!(duration_ms, 42);
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_cost_update() {
        let event = AgentEvent::CostUpdate {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 50,
            cache_creation_tokens: 10,
            total_usd: 0.003,
            model: "claude-sonnet".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::CostUpdate {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_usd,
                model,
            } => {
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 200);
                assert_eq!(cache_read_tokens, 50);
                assert_eq!(cache_creation_tokens, 10);
                assert!((total_usd - 0.003).abs() < f64::EPSILON);
                assert_eq!(model, "claude-sonnet");
            }
            other => panic!("expected CostUpdate, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_error() {
        let event = AgentEvent::Error {
            message: "something broke".to_string(),
            recoverable: false,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::Error {
                message,
                recoverable,
            } => {
                assert_eq!(message, "something broke");
                assert!(!recoverable);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_complete() {
        let event = AgentEvent::Complete {
            summary: "done".to_string(),
            total_cost_usd: 0.05,
            duration_ms: 12000,
            turns: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::Complete {
                summary,
                total_cost_usd,
                duration_ms,
                turns,
            } => {
                assert_eq!(summary, "done");
                assert!((total_cost_usd - 0.05).abs() < f64::EPSILON);
                assert_eq!(duration_ms, 12000);
                assert_eq!(turns, 5);
            }
            other => panic!("expected Complete, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_sub_agent_spawned() {
        let event = AgentEvent::SubAgentSpawned {
            parent_tool_use_id: "tu-99".to_string(),
            description: "researching docs".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::SubAgentSpawned {
                parent_tool_use_id,
                description,
            } => {
                assert_eq!(parent_tool_use_id, "tu-99");
                assert_eq!(description, "researching docs");
            }
            other => panic!("expected SubAgentSpawned, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_event_serde_sub_agent_complete() {
        let event = AgentEvent::SubAgentComplete {
            parent_tool_use_id: "tu-99".to_string(),
            summary: "found the answer".to_string(),
            cost_usd: 0.01,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            AgentEvent::SubAgentComplete {
                parent_tool_use_id,
                summary,
                cost_usd,
            } => {
                assert_eq!(parent_tool_use_id, "tu-99");
                assert_eq!(summary, "found the answer");
                assert!((cost_usd - 0.01).abs() < f64::EPSILON);
            }
            other => panic!("expected SubAgentComplete, got {:?}", other),
        }
    }

    // --- ThreadEvent serde roundtrip ---

    #[test]
    fn test_thread_event_serde() {
        let now = Utc::now();
        let te = ThreadEvent {
            thread_id: "t-1".to_string(),
            timestamp: now,
            event: AgentEvent::Text {
                text: "hi".to_string(),
            },
            parent_tool_use_id: Some("tu-0".to_string()),
        };
        let json = serde_json::to_string(&te).unwrap();
        let de: ThreadEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(de.thread_id, "t-1");
        assert_eq!(de.timestamp, now);
        assert_eq!(de.parent_tool_use_id, Some("tu-0".to_string()));
        match de.event {
            AgentEvent::Text { text } => assert_eq!(text, "hi"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    // --- SessionContext defaults / roundtrip ---

    #[test]
    fn test_session_context_defaults() {
        let ctx = SessionContext {
            briefing: None,
            memories: vec![],
            budget_cap: None,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let de: SessionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(de.briefing, None);
        assert!(de.memories.is_empty());
        assert_eq!(de.budget_cap, None);
    }
}
