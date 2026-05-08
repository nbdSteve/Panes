use async_trait::async_trait;
use panes_events::{AgentEvent, FindingSeverity, ValidationFinding};
use regex::Regex;
use serde::Deserialize;
use serde_json::json;

use super::{OutputValidator, ValidationContext, ValidationReport, ValidatorTypeInfo};

pub struct SecretScanValidator {
    builtins: Vec<(String, Regex)>,
}

impl Default for SecretScanValidator {
    fn default() -> Self {
        let builtins = vec![
            (
                "AWS access key id".to_string(),
                Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
            ),
            (
                "AWS secret access key".to_string(),
                Regex::new(r#"(?i)aws(.{0,20})?(secret|private)?(.{0,20})?['"][0-9a-zA-Z/+]{40}['"]"#).unwrap(),
            ),
            (
                "GitHub personal access token".to_string(),
                Regex::new(r"\bghp_[A-Za-z0-9]{36}\b").unwrap(),
            ),
            (
                "GitHub fine-grained token".to_string(),
                Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{22,}\b").unwrap(),
            ),
            (
                "GitHub OAuth token".to_string(),
                Regex::new(r"\bgho_[A-Za-z0-9]{36}\b").unwrap(),
            ),
            (
                "Slack bot token".to_string(),
                Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap(),
            ),
            (
                "Private key block".to_string(),
                Regex::new(r"-----BEGIN (RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY( BLOCK)?-----").unwrap(),
            ),
        ];
        Self { builtins }
    }
}

#[derive(Debug, Deserialize, Default)]
struct Config {
    #[serde(default)]
    custom_patterns: Vec<String>,
}

impl SecretScanValidator {
    pub fn type_info() -> ValidatorTypeInfo {
        ValidatorTypeInfo {
            type_id: "secret_scan",
            label: "Secret Scan",
            description: "Flags agent output containing well-known secret patterns (AWS keys, GitHub tokens, private keys, Slack tokens). Supports additional regex patterns via config.",
            default_config: json!({ "custom_patterns": [] }),
            // Asking the LLM to "please don't leak that secret" is nonsense —
            // the secret came from somewhere. Hard-abort is safer.
            correctable: false,
        }
    }
}

#[async_trait]
impl OutputValidator for SecretScanValidator {
    fn type_id(&self) -> &'static str {
        "secret_scan"
    }

    fn wants(&self, event: &AgentEvent) -> bool {
        matches!(
            event,
            AgentEvent::Text { .. } | AgentEvent::Complete { .. }
        )
    }

    async fn validate(
        &self,
        event: &AgentEvent,
        ctx: &ValidationContext,
    ) -> ValidationReport {
        let text = match event {
            AgentEvent::Text { text } => text.clone(),
            AgentEvent::Complete { summary, .. } => summary.clone(),
            _ => return ValidationReport::pass(),
        };
        let config: Config =
            serde_json::from_value(ctx.config.clone()).unwrap_or_default();

        let mut findings = Vec::new();

        for (label, pattern) in &self.builtins {
            if let Some(m) = pattern.find(&text) {
                findings.push(ValidationFinding {
                    severity: FindingSeverity::Error,
                    message: format!("possible {label} in output"),
                    span: Some(redact(m.as_str())),
                    source_hint: Some("built-in pattern".to_string()),
                });
            }
        }

        for raw in &config.custom_patterns {
            match Regex::new(raw) {
                Ok(re) => {
                    if let Some(m) = re.find(&text) {
                        findings.push(ValidationFinding {
                            severity: FindingSeverity::Error,
                            message: format!("custom pattern matched: {raw}"),
                            span: Some(redact(m.as_str())),
                            source_hint: Some("custom pattern".to_string()),
                        });
                    }
                }
                Err(err) => {
                    findings.push(ValidationFinding {
                        severity: FindingSeverity::Warning,
                        message: format!("invalid custom regex: {err}"),
                        span: Some(raw.clone()),
                        source_hint: Some("config".to_string()),
                    });
                }
            }
        }

        if findings.is_empty() {
            ValidationReport::pass()
        } else {
            ValidationReport::fail(findings)
        }
    }
}

fn redact(s: &str) -> String {
    // Keep first 4 chars so the user can identify the match without persisting
    // the whole secret in the event log.
    if s.len() <= 8 {
        "[redacted]".to_string()
    } else {
        format!("{}…[redacted]", &s[..4])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_events::AgentEvent;
    use serde_json::json;
    use std::path::PathBuf;

    fn ctx(config: serde_json::Value) -> ValidationContext {
        ValidationContext {
            thread_id: "t".into(),
            workspace_path: PathBuf::from("/tmp"),
            config,
            recent_text: vec![],
        }
    }

    fn text(s: &str) -> AgentEvent {
        AgentEvent::Text { text: s.to_string() }
    }

    #[tokio::test]
    async fn flags_aws_access_key() {
        let v = SecretScanValidator::default();
        let report = v
            .validate(&text("key=AKIAIOSFODNN7EXAMPLE here"), &ctx(json!({})))
            .await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
        assert!(report.findings.iter().any(|f| f.message.contains("AWS")));
    }

    #[tokio::test]
    async fn flags_github_token() {
        let v = SecretScanValidator::default();
        let report = v
            .validate(
                &text("token=ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
                &ctx(json!({})),
            )
            .await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
    }

    #[tokio::test]
    async fn flags_private_key_block() {
        let v = SecretScanValidator::default();
        let report = v
            .validate(
                &text("-----BEGIN RSA PRIVATE KEY-----\nabcd\n-----END RSA PRIVATE KEY-----"),
                &ctx(json!({})),
            )
            .await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
    }

    #[tokio::test]
    async fn clean_output_passes() {
        let v = SecretScanValidator::default();
        let report = v
            .validate(&text("all good here"), &ctx(json!({})))
            .await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
    }

    #[tokio::test]
    async fn custom_pattern_flagged() {
        let v = SecretScanValidator::default();
        let cfg = json!({ "custom_patterns": ["INTERNAL-[A-Z]{6}-[0-9]{4}"] });
        let report = v
            .validate(&text("id=INTERNAL-ABCDEF-1234 x"), &ctx(cfg))
            .await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
    }

    #[tokio::test]
    async fn invalid_custom_regex_reported_as_warning_not_block() {
        let v = SecretScanValidator::default();
        let cfg = json!({ "custom_patterns": ["[unclosed"] });
        let report = v.validate(&text("hello"), &ctx(cfg)).await;
        // Invalid regex still produces a finding (warning severity) — per plan,
        // findings → Fail. The UI can render severity distinctly.
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
        assert!(report.findings[0].message.contains("invalid custom regex"));
    }

    #[test]
    fn redact_hides_full_secret() {
        assert_eq!(redact("short"), "[redacted]");
        let long = "AKIAIOSFODNN7EXAMPLE";
        let r = redact(long);
        assert!(r.starts_with("AKIA"));
        assert!(r.contains("redacted"));
        assert!(!r.contains("EXAMPLE"));
    }
}
