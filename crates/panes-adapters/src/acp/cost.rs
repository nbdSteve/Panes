//! Local cost estimation for ACP adapters whose backend doesn't report
//! real token counts.
//!
//! The honest caveats:
//! - Token counts are approximated as `chars / CHARS_PER_TOKEN`. This is
//!   accurate to ~15% for English + code on Claude models, way off for
//!   non-Latin scripts, and doesn't account for special tokens or
//!   tool-schema overhead.
//! - Cache-read / cache-creation columns are always zero because we can't
//!   see the backend's cache state. Users on heavy-cache workloads will
//!   see numbers 20-80% higher than reality.
//! - The rate table is hardcoded per model family. Bedrock-priced agents
//!   like kiro-cli will match Anthropic list prices closely but may be
//!   discounted via AWS contracts.
//!
//! Every `CostUpdate` this module produces sets `estimated: true` so the
//! UI can badge it as "est." and users know not to treat the numbers as
//! billing-grade. Real numbers (Claude stream-json) take precedence when
//! an adapter can report them.

use panes_events::AgentEvent;

/// Rough conversion used by every LLM rough-estimate doc you'll read.
/// Slightly conservative so we err on the "too many tokens" side.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Rate in USD per million tokens, `(input, output)`. The defaults are
/// Anthropic's public list prices for the Claude 3.5/4 family; unknown
/// models fall back to the Sonnet rate — deliberately mid-range.
///
/// Match is case-insensitive substring: `"claude-3-5-sonnet-20240620"`
/// and `"sonnet"` both hit the "sonnet" row.
fn rate_usd_per_million_tokens(model: &str) -> (f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        (15.0, 75.0)
    } else if m.contains("haiku") {
        (0.80, 4.00)
    } else if m.contains("sonnet") {
        (3.0, 15.0)
    } else {
        // Conservative default — prefer overshoot to undershoot so users
        // don't think the agent is cheaper than it actually is.
        (3.0, 15.0)
    }
}

/// Running estimate that accumulates input/output characters and converts
/// to a `CostUpdate` on demand. One instance per session.
#[derive(Debug, Default, Clone)]
pub(crate) struct CostEstimator {
    /// Total characters the user/adapter has sent INTO the model this session.
    /// Includes the prompt, briefing, memories, and every tool-result we've
    /// handed back to the agent.
    input_chars: usize,
    /// Total characters the model has streamed OUT — agent_message_chunk text
    /// plus tool_call titles/rawInput when they're text-ish.
    output_chars: usize,
    /// Model id reported by the backend (or empty if unknown).
    model: String,
}

impl CostEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    /// Record text headed INTO the model (prompt body, tool output the
    /// agent will consume on the next turn, etc.). Each turn's input
    /// includes prior output that's now in the context — but we track
    /// that as part of accumulated input_chars which grows monotonically,
    /// mirroring how cached-input billing would work.
    pub fn record_input(&mut self, text: &str) {
        self.input_chars = self.input_chars.saturating_add(text.len());
    }

    /// Record text streamed OUT by the model.
    pub fn record_output(&mut self, text: &str) {
        self.output_chars = self.output_chars.saturating_add(text.len());
    }

    /// Build a CostUpdate snapshot from the current counts. Marks
    /// `estimated: true` so the UI can differentiate from real counts.
    pub fn snapshot(&self) -> AgentEvent {
        let input_tokens = (self.input_chars as f64 / CHARS_PER_TOKEN) as u64;
        let output_tokens = (self.output_chars as f64 / CHARS_PER_TOKEN) as u64;
        let (in_rate, out_rate) = rate_usd_per_million_tokens(&self.model);
        let total_usd = (input_tokens as f64 / 1_000_000.0) * in_rate
            + (output_tokens as f64 / 1_000_000.0) * out_rate;
        AgentEvent::CostUpdate {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total_usd,
            model: self.model.clone(),
            estimated: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_lookup_matches_model_families() {
        assert_eq!(rate_usd_per_million_tokens("claude-3-5-sonnet"), (3.0, 15.0));
        assert_eq!(rate_usd_per_million_tokens("opus"), (15.0, 75.0));
        assert_eq!(rate_usd_per_million_tokens("haiku-4-5"), (0.80, 4.00));
    }

    #[test]
    fn rate_lookup_falls_back_to_sonnet_for_unknown() {
        assert_eq!(rate_usd_per_million_tokens("kiro-custom-model"), (3.0, 15.0));
        assert_eq!(rate_usd_per_million_tokens(""), (3.0, 15.0));
    }

    #[test]
    fn snapshot_returns_cost_update_with_estimated_flag() {
        let mut est = CostEstimator::new();
        est.set_model("sonnet");
        est.record_input(&"a".repeat(4000));
        est.record_output(&"b".repeat(400));
        match est.snapshot() {
            AgentEvent::CostUpdate {
                input_tokens,
                output_tokens,
                total_usd,
                model,
                estimated,
                cache_read_tokens,
                cache_creation_tokens,
            } => {
                assert_eq!(input_tokens, 1000); // 4000 / 4
                assert_eq!(output_tokens, 100); // 400 / 4
                assert_eq!(cache_read_tokens, 0);
                assert_eq!(cache_creation_tokens, 0);
                assert_eq!(model, "sonnet");
                assert!(estimated, "ACP estimator must mark updates as estimated");
                let expected = (1000.0 / 1_000_000.0) * 3.0 + (100.0 / 1_000_000.0) * 15.0;
                assert!((total_usd - expected).abs() < f64::EPSILON);
            }
            other => panic!("expected CostUpdate, got {other:?}"),
        }
    }

    #[test]
    fn record_input_accumulates_across_calls() {
        let mut est = CostEstimator::new();
        est.record_input("hello ");
        est.record_input("world");
        // 11 chars / 4 = 2 tokens
        match est.snapshot() {
            AgentEvent::CostUpdate { input_tokens, .. } => assert_eq!(input_tokens, 2),
            _ => unreachable!(),
        }
    }

    #[test]
    fn snapshot_with_no_activity_yields_zero_cost() {
        let est = CostEstimator::new();
        match est.snapshot() {
            AgentEvent::CostUpdate {
                input_tokens,
                output_tokens,
                total_usd,
                estimated,
                ..
            } => {
                assert_eq!(input_tokens, 0);
                assert_eq!(output_tokens, 0);
                assert_eq!(total_usd, 0.0);
                assert!(estimated);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn output_rate_is_higher_than_input_rate_for_sonnet() {
        // Sanity: output tokens cost ~5x input on Anthropic's pricing.
        // If this stops holding, we misread a new price sheet.
        let (input, output) = rate_usd_per_million_tokens("sonnet");
        assert!(output > input, "output should cost more than input");
        assert!(output / input > 4.0 && output / input < 6.0);
    }
}
