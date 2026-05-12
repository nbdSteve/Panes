import { describe, it, expect } from "vitest";
import { deriveConfig } from "./deriveConfig";
import type { ConfigPrefs } from "../types";

const FALLBACK: ConfigPrefs = { adapter: "claude-code", agent: "", model: "sonnet" };

describe("deriveConfig", () => {
  it("returns the in-session wsPick when one exists", () => {
    const wsPick: ConfigPrefs = { adapter: "kiro-cli", agent: "builder", model: "opus" };
    expect(deriveConfig(wsPick, "claude-code", FALLBACK)).toEqual(wsPick);
  });

  it("uses the persisted workspace default adapter when no wsPick is set", () => {
    // Bug fix: changing the Settings-panel default adapter must surface in
    // the ThreadView picker without requiring a dropdown click.
    expect(deriveConfig(undefined, "kiro-cli", FALLBACK)).toEqual({
      adapter: "kiro-cli",
      agent: "",
      model: "",
    });
  });

  it("does NOT carry over agent/model when the persisted adapter differs from the global fallback", () => {
    // Bug fix: agent + model are adapter-specific. A claude 'planner' agent
    // or 'sonnet' model is not valid under kiro-cli. When the workspace
    // default forces a particular adapter, reset agent+model so ThreadView
    // picks the adapter's own default instead of starting a thread with an
    // invalid combo.
    const globalWithBakedClaude: ConfigPrefs = {
      adapter: "claude-code",
      agent: "planner",
      model: "opus",
    };
    const out = deriveConfig(undefined, "kiro-cli", globalWithBakedClaude);
    expect(out.adapter).toBe("kiro-cli");
    expect(out.agent).toBe("");
    expect(out.model).toBe("");
  });

  it("falls back to the global default when neither wsPick nor persisted default is set", () => {
    expect(deriveConfig(undefined, undefined, FALLBACK)).toEqual(FALLBACK);
  });

  it("prefers wsPick over the persisted workspace default", () => {
    // Reason: the user actively interacted with the dropdown during this
    // session, so that choice beats the persisted default.
    const wsPick: ConfigPrefs = { adapter: "kiro-cli", agent: "", model: "sonnet" };
    expect(deriveConfig(wsPick, "claude-code", FALLBACK).adapter).toBe("kiro-cli");
  });

  it("carries over agent/model when persisted adapter MATCHES global fallback's adapter", () => {
    // Bug fix: new workspaces are stamped with default_agent="claude-code"
    // at creation, matching the global fallback adapter. Resetting agent+
    // model in that case would throw away the user's most-recent picks.
    // When the adapters match, the global's agent/model are valid for the
    // new workspace and should carry through — otherwise adding a new
    // workspace after picking karen/Opus silently reverts to Default/
    // Sonnet.
    const global: ConfigPrefs = {
      adapter: "claude-code",
      agent: "karen",
      model: "opus",
    };
    expect(deriveConfig(undefined, "claude-code", global)).toEqual(global);
  });
});
