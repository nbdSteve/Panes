import { describe, it, expect } from "vitest";
import { classifyValidationResult } from "./validationResultView";
import type {
  ValidationResultEvent,
  AgentEvent,
  ValidatorTypeInfo,
} from "../types";

const passEvent: ValidationResultEvent = {
  event_type: "validation_result",
  validator: "citation",
  target_event_index: 1,
  outcome: "pass",
  findings: [],
  duration_ms: 1,
};

const failEvent: ValidationResultEvent = {
  ...passEvent,
  outcome: "fail",
  findings: [
    { severity: "error", message: "missing", span: null, source_hint: null },
  ],
};

const citation: ValidatorTypeInfo = {
  typeId: "citation",
  label: "Citation Check",
  description: "",
  defaultConfig: {},
  correctable: true,
};

const secret: ValidatorTypeInfo = {
  typeId: "secret_scan",
  label: "Secret Scan",
  description: "",
  defaultConfig: {},
  correctable: false,
};

describe("classifyValidationResult", () => {
  it("pass always returns pass", () => {
    expect(
      classifyValidationResult(passEvent, [], [citation]),
    ).toEqual({ kind: "pass" });
    // Even with subsequent events, pass stays pass.
    const later: AgentEvent[] = [
      { event_type: "complete", summary: "x", total_cost_usd: 0, duration_ms: 0, turns: 1 },
    ];
    expect(
      classifyValidationResult(passEvent, later, [citation]),
    ).toEqual({ kind: "pass" });
  });

  it("fail with nothing later → live gate (correctable from catalog)", () => {
    expect(
      classifyValidationResult(failEvent, [], [citation]),
    ).toEqual({ kind: "failGate", correctable: true });
  });

  it("fail with nothing later and unknown validator → not correctable", () => {
    expect(
      classifyValidationResult(failEvent, [], []),
    ).toEqual({ kind: "failGate", correctable: false });
  });

  it("fail with nothing later and non-correctable validator", () => {
    const secretFail: ValidationResultEvent = { ...failEvent, validator: "secret_scan" };
    expect(
      classifyValidationResult(secretFail, [], [citation, secret]),
    ).toEqual({ kind: "failGate", correctable: false });
  });

  it("fail followed by a terminal event → resolved fail card", () => {
    const later: AgentEvent[] = [
      { event_type: "complete", summary: "x", total_cost_usd: 0, duration_ms: 0, turns: 1 },
    ];
    expect(
      classifyValidationResult(failEvent, later, [citation]),
    ).toEqual({ kind: "failResolved" });
  });

  it("fail followed by an error → resolved fail card", () => {
    const later: AgentEvent[] = [
      { event_type: "error", message: "x" },
    ];
    expect(
      classifyValidationResult(failEvent, later, [citation]),
    ).toEqual({ kind: "failResolved" });
  });

  it("fail followed by a follow_up → steered card (preferred over terminal)", () => {
    const later: AgentEvent[] = [
      { event_type: "follow_up", text: "continue" },
      { event_type: "complete", summary: "x", total_cost_usd: 0, duration_ms: 0, turns: 1 },
    ];
    expect(
      classifyValidationResult(failEvent, later, [citation]),
    ).toEqual({ kind: "failSteered" });
  });

  it("fail followed only by a follow_up (no terminal yet) → steered", () => {
    const later: AgentEvent[] = [
      { event_type: "follow_up", text: "hi" },
    ];
    expect(
      classifyValidationResult(failEvent, later, [citation]),
    ).toEqual({ kind: "failSteered" });
  });
});
