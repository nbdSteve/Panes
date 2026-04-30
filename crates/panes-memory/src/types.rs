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

#[cfg(test)]
mod tests {
    use super::*;

    // --- MemoryType Display tests ---

    #[test]
    fn test_memory_type_display_decision() {
        assert_eq!(format!("{}", MemoryType::Decision), "decision");
    }

    #[test]
    fn test_memory_type_display_preference() {
        assert_eq!(format!("{}", MemoryType::Preference), "preference");
    }

    #[test]
    fn test_memory_type_display_constraint() {
        assert_eq!(format!("{}", MemoryType::Constraint), "constraint");
    }

    #[test]
    fn test_memory_type_display_pattern() {
        assert_eq!(format!("{}", MemoryType::Pattern), "pattern");
    }

    // --- MemoryType serde roundtrip ---

    #[test]
    fn test_memory_type_serde_roundtrip() {
        let variants = [
            MemoryType::Decision,
            MemoryType::Preference,
            MemoryType::Constraint,
            MemoryType::Pattern,
        ];
        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let deserialized: MemoryType = serde_json::from_str(&json).unwrap();
            assert_eq!(*variant, deserialized);
        }
    }

    // --- InjectedContext default ---

    #[test]
    fn test_injected_context_default() {
        let ctx = InjectedContext::default();
        assert_eq!(ctx.briefing, None);
        assert!(ctx.memories.is_empty());
        assert_eq!(ctx.token_estimate, 0);
    }
}
