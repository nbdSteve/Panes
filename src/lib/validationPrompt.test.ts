import { describe, it, expect } from "vitest";
import { buildCorrectionPrompt } from "./validationPrompt";
import type { ValidationFinding } from "../types";

describe("buildCorrectionPrompt", () => {
  it("returns an empty string when there are no findings", () => {
    expect(buildCorrectionPrompt([])).toBe("");
  });

  it("formats a single finding as a bullet", () => {
    const f: ValidationFinding = {
      severity: "error",
      message: "referenced path does not exist: src/missing.rs",
      span: "src/missing.rs",
      source_hint: "workspace",
    };
    const out = buildCorrectionPrompt([f]);
    expect(out).toContain("A validator flagged issues");
    expect(out).toContain("- referenced path does not exist: src/missing.rs");
    expect(out).toContain("Please revise your response");
  });

  it("includes all findings in output", () => {
    const findings: ValidationFinding[] = [
      { severity: "error", message: "one", span: null, source_hint: null },
      { severity: "error", message: "two", span: null, source_hint: null },
      { severity: "warning", message: "three", span: null, source_hint: null },
    ];
    const out = buildCorrectionPrompt(findings);
    expect(out).toContain("- one");
    expect(out).toContain("- two");
    expect(out).toContain("- three");
  });

  it("preserves special characters and does not escape markdown", () => {
    const findings: ValidationFinding[] = [
      {
        severity: "error",
        message: "path contains `backticks` and *stars*",
        span: null,
        source_hint: null,
      },
    ];
    expect(buildCorrectionPrompt(findings)).toContain("`backticks`");
    expect(buildCorrectionPrompt(findings)).toContain("*stars*");
  });

  it("does not mention the validator name (neutral copy)", () => {
    const findings: ValidationFinding[] = [
      { severity: "error", message: "missing path", span: null, source_hint: null },
    ];
    const out = buildCorrectionPrompt(findings);
    expect(out.toLowerCase()).not.toContain("citation");
    expect(out.toLowerCase()).not.toContain("secret_scan");
  });
});
