import type { AgentInfo, ModelInfo } from "../types";

/**
 * Refresh the agent + model lists for an adapter. Exposed as a standalone
 * helper so the adapter-switch logic in `App.tsx` can be unit-tested
 * without React. The contract:
 * - `listAgents` rejects → return `[]` for agents (empty picker is better
 *   than a stale list from the previous adapter).
 * - `listModels` rejects or returns empty → return `fallbackModels` so the
 *   UI always has something selectable.
 * - Neither failure should propagate to the caller. This is the reason the
 *   helper exists; previously the inline logic could drop errors on the
 *   floor inside a React effect, making them invisible to tests.
 */
export async function refreshAdapterLists(
  adapter: string,
  fallbackModels: ModelInfo[],
  api: {
    listAgents: (adapter: string) => Promise<AgentInfo[]>;
    listModels: (adapter: string) => Promise<ModelInfo[]>;
  },
): Promise<{ agents: AgentInfo[]; models: ModelInfo[] }> {
  const [agentsResult, modelsResult] = await Promise.allSettled([
    api.listAgents(adapter),
    api.listModels(adapter),
  ]);

  const agents = agentsResult.status === "fulfilled" ? agentsResult.value : [];
  let models = fallbackModels;
  if (modelsResult.status === "fulfilled" && modelsResult.value.length > 0) {
    models = modelsResult.value;
  }
  return { agents, models };
}
