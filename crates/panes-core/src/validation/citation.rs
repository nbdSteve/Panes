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
        if !is_path_like(path_part) {
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

/// Common source/config/document extensions treated as path evidence.
/// Extensions must be 1-6 chars, all ASCII lowercase letters or digits,
/// and contain at least one letter — pure-digit "extensions" like `.3`
/// are version-number fragments, not file extensions.
fn looks_like_extension(ext: &str) -> bool {
    if ext.is_empty() || ext.len() > 6 {
        return false;
    }
    let mut has_letter = false;
    for b in ext.bytes() {
        if b.is_ascii_lowercase() {
            has_letter = true;
        } else if !b.is_ascii_digit() {
            return false;
        }
    }
    has_letter
}

/// True if the token is specific enough to plausibly be a filesystem path,
/// not prose. The validator's false-positive rate was unmanageable with the
/// old rule ("contains / or ."), which flagged things like "and/or",
/// "stacks/constructs", "e.g", version strings, and slash-separated
/// alternatives. We now require one of:
/// - an absolute or workspace-relative path prefix: `/`, `./`, `../`, `~/`
/// - a known file-extension suffix (lowercase, 1-6 chars)
/// - at least two path segments where the last one has an extension
fn is_path_like(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Filter out dotted English abbreviations and small numeric strings.
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "e.g" | "i.e" | "etc" | "e.g." | "i.e." | "etc." | "vs" | "vs." | "a.m" | "p.m"
    ) {
        return false;
    }

    // Any path-prefix sigil is strong evidence.
    if s.starts_with('/')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with("~/")
    {
        return true;
    }

    // Last segment with a plausible extension.
    let last_seg = s.rsplit('/').next().unwrap_or(s);
    if let Some((stem, ext)) = last_seg.rsplit_once('.') {
        // Stem must be non-empty and the extension must be extension-shaped.
        // Also reject "2.0"-style version numbers: if the stem is all digits
        // AND the ext is all digits, treat it as a number, not a path.
        let stem_all_digits = !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit());
        let ext_all_digits = !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_digit());
        if stem_all_digits && ext_all_digits {
            return false;
        }
        if !stem.is_empty() && looks_like_extension(ext) {
            return true;
        }
    }

    // A multi-segment path where NO segment has an extension is almost never
    // an actual file reference in free text — that's prose like "stacks/
    // constructs" or "fleet/partition". Require extension evidence.
    false
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

    // ─── prose false-positive regressions ────────────────────────────────
    //
    // Every case below was observed as a real false-positive against kiro-cli
    // output in production. The "validator too sensitive" complaint boiled
    // down to these three categories, so pin each one down as a test.

    #[tokio::test]
    async fn slash_separated_prose_alternatives_are_not_citations() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete(
            "Are the bakery steps a new fleet/partition, or new stacks/constructs \
             within the existing fleet? We could use and/or logic here.",
        );
        let report = v.validate(&event, &ctx).await;
        assert_eq!(
            report.outcome,
            panes_events::ValidationOutcome::Pass,
            "slash-separated prose alternatives should not be flagged as paths; got: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn english_abbreviations_are_not_citations() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete(
            "We should, e.g., prefer shared constructs (i.e., reusable patterns) \
             and avoid duplication. Etc.",
        );
        let report = v.validate(&event, &ctx).await;
        assert_eq!(
            report.outcome,
            panes_events::ValidationOutcome::Pass,
            "e.g / i.e / etc must not be treated as paths; got: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn version_numbers_are_not_citations() {
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("Upgrade requires 2.0 or 3.14.1 at minimum.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(
            report.outcome,
            panes_events::ValidationOutcome::Pass,
            "bare version strings should not be flagged; got: {:?}",
            report.findings
        );
    }

    #[tokio::test]
    async fn bare_word_with_slash_but_no_extension_is_not_flagged() {
        // The old heuristic accepted any token containing `/` as a path
        // candidate. That was wrong: English prose uses slashes as
        // alternation separators. Without a file extension or path-prefix
        // sigil we can't tell a file path from prose, so we err on the side
        // of passing.
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("The input/output boundary determines the read/write policy.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Pass);
    }

    #[tokio::test]
    async fn explicit_relative_path_prefix_still_flags_missing_files() {
        // Counter-check: the tightened heuristic must not silently drop real
        // citations. If the agent writes "./src/foo.rs" or "src/foo.rs",
        // missing files still need to be reported.
        let tmp = TempDir::new().unwrap();
        let v = CitationValidator::default();
        let ctx = ctx_for(&tmp, json!({}));
        let event = complete("Check ./src/foo.rs for the bug.");
        let report = v.validate(&event, &ctx).await;
        assert_eq!(report.outcome, panes_events::ValidationOutcome::Fail);
        assert!(report.findings[0].message.contains("does not exist"));
    }

    // ─── is_path_like unit tests ─────────────────────────────────────────

    #[test]
    fn is_path_like_accepts_explicit_prefixes() {
        assert!(is_path_like("/etc/hosts"));
        assert!(is_path_like("./src/foo.rs"));
        assert!(is_path_like("../common/util.ts"));
        assert!(is_path_like("~/projects/x"));
    }

    #[test]
    fn is_path_like_accepts_paths_with_file_extensions() {
        assert!(is_path_like("src/main.rs"));
        assert!(is_path_like("README.md"));
        assert!(is_path_like("package.json"));
        assert!(is_path_like("foo.tsx"));
    }

    #[test]
    fn is_path_like_rejects_prose_slashes() {
        assert!(!is_path_like("and/or"));
        assert!(!is_path_like("read/write"));
        assert!(!is_path_like("stacks/constructs"));
        assert!(!is_path_like("fleet/partition"));
    }

    #[test]
    fn is_path_like_rejects_abbreviations() {
        assert!(!is_path_like("e.g"));
        assert!(!is_path_like("i.e"));
        assert!(!is_path_like("etc"));
        assert!(!is_path_like("vs"));
    }

    #[test]
    fn is_path_like_rejects_version_numbers() {
        assert!(!is_path_like("2.0"));
        assert!(!is_path_like("3.14"));
        assert!(!is_path_like("1.2.3"));
    }

    #[test]
    fn is_path_like_rejects_uppercase_extensions() {
        // Source file extensions are conventionally lowercase. Accepting
        // uppercase would re-enable false positives from acronyms.
        assert!(!is_path_like("e.G"));
    }
}
