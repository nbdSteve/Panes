export type PanesErrorType =
  | "workspace_occupied"
  | "thread_not_found"
  | "adapter_not_found"
  | "no_gate_pending"
  | "git_error"
  | "database_error"
  | "spawn_failed"
  | "budget_exceeded"
  | "memory_error"
  | "validation_error"
  | "internal";

export interface PanesError {
  type: PanesErrorType;
  message: string;
  workspace_id?: string;
  thread_id?: string;
  adapter?: string;
}

export function parsePanesError(err: unknown): PanesError {
  if (typeof err === "string") {
    try {
      const parsed = JSON.parse(err);
      if (parsed && typeof parsed === "object" && "type" in parsed) {
        return parsed as PanesError;
      }
    } catch {
      // not JSON
    }
    return { type: "internal", message: err };
  }
  if (err && typeof err === "object" && "type" in err) {
    return err as PanesError;
  }
  return { type: "internal", message: String(err) };
}

export function isWorkspaceOccupied(err: PanesError): boolean {
  return err.type === "workspace_occupied";
}

export function isNoGatePending(err: PanesError): boolean {
  return err.type === "no_gate_pending";
}

export function isValidationError(err: PanesError): boolean {
  return err.type === "validation_error";
}
