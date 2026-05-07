import ValidationFindings, { ValidationFinding } from "./ValidationFindings";

interface Props {
  validator: string;
  outcome: "pass" | "fail";
  findings: ValidationFinding[];
  durationMs: number;
}

export default function ValidationResultCard({
  validator,
  outcome,
  findings,
  durationMs,
}: Props) {
  const pass = outcome === "pass";
  return (
    <div
      className={`card validation-result-card ${
        pass ? "validation-pass" : "validation-fail"
      }`}
    >
      <div className="validation-header">
        <span className={`validation-outcome-badge ${outcome}`}>
          {pass ? "Passed" : "Failed"}
        </span>
        <span className="validation-name">{validator}</span>
        <span className="validation-duration">{durationMs}ms</span>
      </div>
      {!pass && findings.length > 0 && (
        <ValidationFindings findings={findings} />
      )}
    </div>
  );
}
