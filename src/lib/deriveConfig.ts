import type { ConfigPrefs } from "../types";

/**
 * Resolve the `ConfigPrefs` the ThreadView should render for a given
 * workspace, combining three sources in priority order:
 *
 * 1. `wsPick` — the user's in-session dropdown selection, held in memory.
 * 2. `workspaceDefaultAdapter` — the persisted per-workspace default adapter
 *    (edited via the Settings panel).
 * 3. `globalFallback` — the app-wide fallback (currently claude-code / sonnet).
 *
 * Exposed as a pure function so the adapter/model precedence can be
 * unit-tested without mounting the App component.
 *
 * Two bugs this guards against:
 *
 * - Pre-fix, a Settings-panel change to the default adapter did not surface
 *   in the ThreadView picker until the user clicked the dropdown, because
 *   the derivation only consulted `wsPick` and `globalFallback` and ignored
 *   the persisted workspace value.
 *
 * - When falling back to `workspaceDefaultAdapter` we do NOT carry over
 *   `globalFallback.agent` / `.model`. Those are adapter-specific: a claude
 *   "planner" agent or a "sonnet" model is meaningless under kiro-cli. We
 *   leave them empty so ThreadView picks the adapter's own default (and the
 *   backend's own fallback for the model), instead of silently starting a
 *   thread with an invalid agent/model combo.
 */
export function deriveConfig(
  wsPick: ConfigPrefs | undefined,
  workspaceDefaultAdapter: string | undefined,
  globalFallback: ConfigPrefs,
): ConfigPrefs {
  if (wsPick) return wsPick;
  if (workspaceDefaultAdapter) {
    return { adapter: workspaceDefaultAdapter, agent: "", model: "" };
  }
  return globalFallback;
}
