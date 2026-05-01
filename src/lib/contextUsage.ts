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
  let latestTotal = 0;
  let model: string | undefined;

  for (const e of events) {
    if (e.event_type === "cost_update") {
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

  if (latestTotal === 0) return null;

  const limit = getContextLimit(model);
  const percentage = (latestTotal / limit) * 100;
  const level = percentage >= 80 ? "danger" : percentage >= 40 ? "warning" : "ok";

  return { inputTokens: latestTotal, percentage, level };
}
