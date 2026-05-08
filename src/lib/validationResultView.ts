import type {
  AgentEvent,
  ValidationResultEvent,
  ValidatorTypeInfo,
} from "../types";

export type ValidationResultMode =
  | { kind: "pass" }
  | { kind: "failResolved" }
  | { kind: "failSteered" }
  | { kind: "failGate"; correctable: boolean };

/**
 * Decide how a ValidationResult event should render, given what came after
 * it in the thread and the catalog of known validator types.
 *
 * - Pass → always a pass card.
 * - Fail with a later follow_up → steered resolution card.
 * - Fail with a later complete/error (but no follow_up) → fail resolution card.
 * - Fail with nothing later → live gate card; correctable drives whether
 *   Auto-fix is offered.
 */
export function classifyValidationResult(
  event: ValidationResultEvent,
  laterEvents: AgentEvent[],
  validatorTypes: ValidatorTypeInfo[],
): ValidationResultMode {
  if (event.outcome === "pass") {
    return { kind: "pass" };
  }
  const hasFollowUp = laterEvents.some((e) => e.event_type === "follow_up");
  if (hasFollowUp) {
    return { kind: "failSteered" };
  }
  const hasTerminal = laterEvents.some(
    (e) => e.event_type === "complete" || e.event_type === "error",
  );
  if (hasTerminal) {
    return { kind: "failResolved" };
  }
  const info = validatorTypes.find((t) => t.typeId === event.validator);
  return { kind: "failGate", correctable: info?.correctable ?? false };
}
