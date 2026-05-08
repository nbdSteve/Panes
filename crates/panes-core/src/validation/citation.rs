use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use panes_events::{AgentEvent, FindingSeverity, ValidationFinding};
use serde::Deserialize;
use serde_json::json;

use super::{OutputValidator, ValidationContext, ValidationReport, ValidatorTypeInfo};

#[derive(Default)]
pub struct CitationValidator;

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    allow_outside_workspace: bool,
    #[serde(default = "default_true")]
    check_line_refs: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allow_outside_workspace: false,
            check_line_refs: true,
        }
    }
}

impl CitationValidator {
    pub fn type_info() -> ValidatorTypeInfo {
        ValidatorTypeInfo {
            type_id: "citation",
            label: "Citation Check",
            description: "Verifies that file paths referenced in agent output resolve inside the workspace.",
            default_config: json!({
                "allow_outside_workspace": false,
                "check_line_refs": true,
            }),
            correctable: true,
        }
    }
}

#[async_trait]
impl OutputValidator for CitationValidator {
    fn type_id(&self) -> &'static str {
        "citation"
    }

    fn wants(&self, event: &AgentEvent) -> bool {
        matches!(
            event,
            AgentEvent::Complete { .. } | AgentEvent::Text { .. }
        )
    }

    async fn validate(
        &self,
        event: &AgentEvent,
        ctx: &ValidationContext,
    ) -> ValidationReport {
        let started = Instant::now();
        let config: Config =
            serde_json::from_value(ctx.config.clone()).unwrap_or_default();

        let text = match event {
            AgentEvent::Complete { summary, .. } => summary.clone(),
            AgentEvent::Text { text } => text.clone(),
            _ => return ValidationReport::pass(),
        };

        let mut findings = Vec::new();
        let candidates = extract_path_candidates(&text);

        for candidate in candidates {
            let resolved = resolve_candidate(&candidate, &ctx.workspace_path, &config);
            if let Some(finding) = resolved.into_finding(&candidate, &config) {
                findings.push(finding);
            }
        }

        let _ = started; // durations are tracked by caller
        if findings.is_empty() {
            ValidationReport::pass()
        } else {
            ValidationReport::fail(findings)
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct PathCandidate {
    raw: String,
    path_part: String,
    line_ref: Option<u32>,
}

fn extract_path_candidates(text: &str) -> Vec<PathCandidate> {
    let mut out = Vec::new();
    collect_markdown_links(text, &mut out);
    collect_bare_paths(text, &mut out);
    // Deduplicate on the raw citation string
    out.sort_by(|a, b| a.raw.cmp(&b.raw));
    out.dedup_by(|a, b| a.raw == b.raw);
    out
}

fn collect_markdown_links(text: &str, out: &mut Vec<PathCandidate>) {
    // naive [text](target) parser
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = find_byte(bytes, i + 1, b']') {
                if close + 1 < bytes.len() && bytes[close + 1] == b'(' {
                    if let Some(end) = find_byte(bytes, close + 2, b')') {
                        let target = &text[close + 2..end];
                        if looks_like_local_path(target) {
                            let (path_part, line_ref) = split_line_ref(target);
                            out.push(PathCandidate {
                                raw: target.to_string(),
                                path_part: path_part.to_string(),
                                line_ref,
                            });
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
}

fn collect_bare_paths(text: &str, out: &mut Vec<PathCandidate>) {
    for tok in text.split(|c: char| {
        c.is_whitespace()
            || matches!(c, ',' | ';' | '"' | '\'' | '`' | '(' | ')' | '<' | '>' | '[' | ']')
    }) {
        let trimmed = tok.trim_end_matches(|c: char| matches!(c, '.' | ':' | '!' | '?'));
        if trimmed.is_empty() {
            continue;
        }
        if !looks_like_local_path(trimmed) {
            continue;
        }
        let (path_part, line_ref) = split_line_ref(trimmed);
        // Require a file extension or at least one path separator to avoid
        // matching plain words with trailing punctuation.
        if !path_part.contains('/') && !path_part.contains('.') {
            continue;
        }
        out.push(PathCandidate {
            raw: trimmed.to_string(),
            path_part: path_part.to_string(),
            line_ref,
        });
    }
}

fn looks_like_local_path(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("mailto:")
        || s.starts_with("ftp://")
    {
        return false;
    }
    // Strip a line ref before the URL-like check so src/foo.rs:12 still qualifies.
    let (core, _) = split_line_ref(s);
    // Require either a slash or a dot-extension; filter noise like "e.g." via the
    // caller (extension check there), but that's belt-and-suspenders.
    core.chars().any(|c| c == '/' || c == '.')
}

fn split_line_ref(s: &str) -> (&str, Option<u32>) {
    if let Some(colon) = s.rfind(':') {
        let (head, tail) = s.split_at(colon);
        let tail = &tail[1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = tail.parse::<u32>() {
                return (head, Some(n));
            }
        }
    }
    (s, None)
}

fn find_byte(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    bytes[start..].iter().position(|b| *b == target).map(|p| start + p)
}

enum Resolution {
    Ok,
    Missing,
    Outside,
    LineOutOfRange(u32, u32),
}

impl Resolution {
    fn into_finding(
        self,
        candidate: &PathCandidate,
        config: &Config,
    ) -> Option<ValidationFinding> {
        match self {
            Resolution::Ok => None,
            Resolution::Missing => Some(ValidationFinding {
                severity: FindingSeverity::Error,
                message: format!("referenced path does not exist: {}", candidate.path_part),
                span: Some(candidate.raw.clone()),
                source_hint: Some("workspace".to_string()),
            }),
            Resolution::Outside => {
                if config.allow_outside_workspace {
                    None
                } else {
                    Some(ValidationFinding {
                        severity: FindingSeverity::Warning,
                        message: format!(
                            "referenced path escapes workspace: {}",
                            candidate.path_part
                        ),
                        span: Some(candidate.raw.clone()),
                        source_hint: Some("workspace".to_string()),
                    })
                }
            }
            Resolution::LineOutOfRange(asked, actual) => Some(ValidationFinding {
                severity: FindingSeverity::Error,
                message: format!(
                    "line {asked} exceeds file length ({actual} lines): {}",
                    candidate.path_part
                ),
                span: Some(candidate.raw.clone()),
                source_hint: Some("workspace".to_string()),
            }),
        }
    }
}

fn resolve_candidate(
    candidate: &PathCandidate,
    workspace: &Path,
    config: &Config,
) -> Resolution {
    let resolved: PathBuf = if Path::new(&candidate.path_part).is_absolute() {
        PathBuf::from(&candidate.path_part)
    } else {
        workspace.join(&candidate.path_part)
    };

    // Canonicalize to check escape. fall back to raw join if canonicalize fails
    // on a missing path — we want to report Missing, not Outside.
    let canonical_ws = workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf());
    let canonical = match resolved.canonicalize() {
        Ok(p) => p,
        Err(_) => return Resolution::Missing,
    };

    if !canonical.starts_with(&canonical_ws) && !Path::new(&candidate.path_part).is_absolute() {
        // Shouldn't happen for relative paths; guard anyway.
        return Resolution::Outside;
    }
    if Path::new(&candidate.path_part).is_absolute() && !canonical.starts_with(&canonical_ws) {
        return Resolution::Outside;
    }

    if config.check_line_refs {
        if let Some(asked) = candidate.line_ref {
            match std::fs::read_to_string(&canonical) {
                Ok(contents) => {
                    let count = contents.lines().count() as u32;
                    if asked == 0 || asked > count {
                        return Resolution::LineOutOfRange(asked, count);
                    }
                }
                Err(_) => {
                    // File exists (canonicalize succeeded) but isn't readable as text.
                    // Don't flag — line checks only apply to readable text files.
                }
            }
        }
    }

    Resolution::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_events::AgentEvent;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn ctx_for(tmp: &TempDir, config: serde_json::Value) -> ValidationContext {
        ValidationContext {
            thread_id: "t".into(),
            workspace_path: tmp.path().to_path_buf(),
            config,
            recent_text: vec![],
        }
    }

    fn complete(summary: &str) -> AgentEvent {
        AgentEvent::Complete {
            summary: summary.to_string(),
            total_cost_usd: 0.0,
            duration_ms: 0,
            turns: 1,
        }
    }

    #[tokio::test]
    async fn passes_on_existing_path() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/real.rs"), "line1\nline2\n").unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("See [it](src/real.rs) for details.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
        assert!(report.findings.is_empty());
    }

    #[tokio::test]
    async fn fails_on_missing_markdown_link() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("Check [gone](src/missing.rs) now.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].span.as_deref(), Some("src/missing.rs"));
    }

    #[tokio::test]
    async fn fails_on_bare_path_reference() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("The logic lives in src/nope.rs and is broken.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
    }

    #[tokio::test]
    async fn ignores_http_urls() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("See [docs](https://example.com/path) for more.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
    }

    #[tokio::test]
    async fn fails_when_line_out_of_range() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("short.txt"), "only\none\nline\n").unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("Bug at short.txt:999 is suspect.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
        assert!(report.findings[0].message.contains("exceeds file length"));
    }

    #[tokio::test]
    async fn line_ref_within_range_passes() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("x.txt"), "a\nb\nc\n").unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("See x.txt:2 for the bug.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
    }

    #[tokio::test]
    async fn check_line_refs_disabled_skips_range_check() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("x.txt"), "a\n").unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({ "check_line_refs": false }));
        let event = complete("See x.txt:999.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
    }

    #[tokio::test]
    async fn wants_only_text_and_complete() {
        let v = CitationValidator::default();
        assert!(v.wants(&AgentEvent::Text { text: "x".into() }));
        assert!(v.wants(&complete("s")));
        assert!(!v.wants(&AgentEvent::Thinking { text: "x".into() }));
    }

    #[test]
    fn extracts_mixed_citations() {
        let text = "see [a](src/a.rs) and b/c.txt plus [skip](https://x.y)";
        let cands = extract_path_candidates(text);
        let raws: Vec<_> = cands.iter().map(|c| c.raw.as_str()).collect();
        assert!(raws.contains(&"src/a.rs"));
        assert!(raws.contains(&"b/c.txt"));
        assert!(!raws.iter().any(|r| r.starts_with("http")));
    }
}
