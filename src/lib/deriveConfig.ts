import type { ConfigPrefs } from "../types";

/**
 * Resolve the `ConfigPrefs` the ThreadView should render for a given
 * workspace, combining three sources in priority order:
 *
 * 1. `wsPick` — the user's in-session dropdown selection, held in memory.
 * 2. `workspaceDefaultAdapter` — the persisted per-workspace default adapter
 *    (edited via the Settings panel, or stamped at workspace creation).
 * 3. `globalFallback` — the most-recently-used config across the session.
 *
 * Exposed as a pure function so the adapter/model precedence can be
 * unit-tested without mounting the App component.
 *
 * Three bugs this guards against:
 *
 * - Pre-fix, a Settings-panel change to the default adapter did not surface
 *   in the ThreadView picker until the user clicked the dropdown, because
 *   the derivation only consulted `wsPick` and `globalFallback` and ignored
 *   the persisted workspace value.
 *
 * - When falling back to `workspaceDefaultAdapter` that differs from the
 *   global fallback's adapter we do NOT carry over `globalFallback.agent` /
 *   `.model`. Those are adapter-specific: a claude "planner" agent or a
 *   "sonnet" model is meaningless under kiro-cli. We leave them empty so
 *   ThreadView picks the adapter's own default (and the backend's own
 *   fallback for the model), instead of silently starting a thread with an
 *   invalid agent/model combo.
 *
 * - When `workspaceDefaultAdapter` MATCHES the global fallback's adapter
 *   (the common case: both "claude-code", which new workspaces are stamped
 *   with at creation), we DO carry the global fallback's agent/model
 *   through. Without this, adding a new workspace after picking
 *   karen/Opus resets the picker to Default/Sonnet even though both values
 *   are valid under the new workspace's adapter.
 */
export function deriveConfig(
  wsPick: ConfigPrefs | undefined,
  workspaceDefaultAdapter: string | undefined,
  globalFallback: ConfigPrefs,
): ConfigPrefs {
  if (wsPick) return wsPick;
  if (workspaceDefaultAdapter) {
    if (workspaceDefaultAdapter === globalFallback.adapter) {
      return { ...globalFallback };
    }
    return { adapter: workspaceDefaultAdapter, agent: "", model: "" };
  }
  return globalFallback;
}
