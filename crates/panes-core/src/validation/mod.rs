use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use panes_events::{AgentEvent, ValidationFinding, ValidationOutcome};

pub mod citation;
pub mod secret_scan;

#[async_trait]
pub trait OutputValidator: Send + Sync {
    fn type_id(&self) -> &'static str;

    fn wants(&self, event: &AgentEvent) -> bool;

    async fn validate(
        &self,
        event: &AgentEvent,
        ctx: &ValidationContext,
    ) -> ValidationReport;
}

#[derive(Debug, Clone)]
pub struct ValidationContext {
    pub thread_id: String,
    pub workspace_path: PathBuf,
    pub config: serde_json::Value,
    pub recent_text: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub outcome: ValidationOutcome,
    pub findings: Vec<ValidationFinding>,
}

impl ValidationReport {
    pub fn pass() -> Self {
        Self {
            outcome: ValidationOutcome::Pass,
            findings: Vec::new(),
        }
    }

    pub fn fail(findings: Vec<ValidationFinding>) -> Self {
        Self {
            outcome: ValidationOutcome::Fail,
            findings,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorTypeInfo {
    pub type_id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default_config: serde_json::Value,
    /// Whether findings from this validator are usable as a correction prompt
    /// to the LLM. True for content problems (e.g. bad citations). False for
    /// things like secret leaks, where re-prompting doesn't make sense.
    pub correctable: bool,
}

#[derive(Clone)]
pub struct ValidatorRegistry {
    by_type: HashMap<&'static str, Arc<dyn OutputValidator>>,
    catalog: Vec<ValidatorTypeInfo>,
}

impl ValidatorRegistry {
    pub fn with_builtins() -> Self {
        let mut by_type: HashMap<&'static str, Arc<dyn OutputValidator>> = HashMap::new();
        let mut catalog = Vec::new();

        let citation = Arc::new(citation::CitationValidator::default());
        by_type.insert(citation.type_id(), citation);
        catalog.push(citation::CitationValidator::type_info());

        let secret = Arc::new(secret_scan::SecretScanValidator::default());
        by_type.insert(secret.type_id(), secret);
        catalog.push(secret_scan::SecretScanValidator::type_info());

        Self { by_type, catalog }
    }

    pub fn get(&self, type_id: &str) -> Option<Arc<dyn OutputValidator>> {
        self.by_type.get(type_id).cloned()
    }

    pub fn catalog(&self) -> &[ValidatorTypeInfo] {
        &self.catalog
    }
}

impl Default for ValidatorRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_contains_builtins() {
        let r = ValidatorRegistry::with_builtins();
        assert!(r.get("citation").is_some());
        assert!(r.get("secret_scan").is_some());
        assert!(r.get("nonexistent").is_none());
    }

    #[test]
    fn test_catalog_lists_builtins() {
        let r = ValidatorRegistry::with_builtins();
        let ids: Vec<_> = r.catalog().iter().map(|c| c.type_id).collect();
        assert!(ids.contains(&"citation"));
        assert!(ids.contains(&"secret_scan"));
    }

    #[test]
    fn test_report_helpers() {
        let p = ValidationReport::pass();
        assert_eq!(p.outcome, ValidationOutcome::Pass);
        assert!(p.findings.is_empty());

        let f = ValidationReport::fail(vec![ValidationFinding {
            severity: panes_events::FindingSeverity::Error,
            message: "x".into(),
            span: None,
            source_hint: None,
        }]);
        assert_eq!(f.outcome, ValidationOutcome::Fail);
        assert_eq!(f.findings.len(), 1);
    }
}
