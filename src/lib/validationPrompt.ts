import type { ValidationFinding } from "../types";

/**
 * Build a neutral correction prompt from validator findings. The output is
 * plain text intended to be sent back to the LLM as the next user turn.
 *
 * Returns an empty string when there are no findings; callers should guard
 * and avoid sending an empty prompt.
 */
export function buildCorrectionPrompt(findings: ValidationFinding[]): string {
  if (findings.length === 0) return "";
  const bullets = findings
    .map((f) => `- ${f.message}`)
    .join("\n");
  return `A validator flagged issues with your last response:\n\n${bullets}\n\nPlease revise your response to address these.`;
}
