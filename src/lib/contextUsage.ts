import type { AgentEvent } from "../types";

const MODEL_CONTEXT_LIMITS: Record<string, number> = {
  "claude-opus-4-6": 200_000,
  "claude-sonnet-4-6": 200_000,
  "claude-haiku-4-5": 200_000,
  "claude-sonnet-4-5": 200_000,
  "claude-opus-4-0": 200_000,
  "claude-sonnet-3-5": 200_000,
};

const DEFAULT_CONTEXT_LIMIT = 200_000;

function getContextLimit(model?: string): number {
  if (!model) return DEFAULT_CONTEXT_LIMIT;
  for (const [key, limit] of Object.entries(MODEL_CONTEXT_LIMITS)) {
    if (model.includes(key)) return limit;
  }
  return DEFAULT_CONTEXT_LIMIT;
}

export interface ContextUsage {
  inputTokens: number;
  percentage: number;
  level: "ok" | "warning" | "danger";
}

export function calculateContextUsage(events: AgentEvent[]): ContextUsage | null {
  // Prefer an explicit ContextUsage event if the adapter emitted one —
  // ACP/kiro-cli sends _kiro.dev/metadata { contextUsagePercentage } which
  // the Rust translator maps to ContextUsageEvent. The backend knows its
  // own window size better than we can infer from token counts.
  let latestExplicit: number | null = null;
  let latestTotal = 0;
  let model: string | undefined;

  for (const e of events) {
    if (e.event_type === "context_usage") {
      latestExplicit = e.percentage;
    } else if (e.event_type === "cost_update") {
      const total =
        (e.input_tokens ?? 0) +
        (e.cache_read_tokens ?? 0) +
        (e.cache_creation_tokens ?? 0);
      if (total > 0) {
        latestTotal = total;
      }
      if (e.model) {
        model = e.model;
      }
    }
  }

  if (latestExplicit !== null) {
    const level =
      latestExplicit >= 80 ? "danger" : latestExplicit >= 40 ? "warning" : "ok";
    // No token count available in this path — display 0 so the tooltip
    // doesn't show a misleading number.
    return { inputTokens: 0, percentage: latestExplicit, level };
  }

  if (latestTotal === 0) return null;

  const limit = getContextLimit(model);
  const percentage = (latestTotal / limit) * 100;
  const level = percentage >= 80 ? "danger" : percentage >= 40 ? "warning" : "ok";

  return { inputTokens: latestTotal, percentage, level };
}
