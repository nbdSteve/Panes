import ValidationFindings, { ValidationFinding } from "./ValidationFindings";

interface Props {
  validator: string;
  outcome: "pass" | "fail";
  findings: ValidationFinding[];
  durationMs: number;
  resolution?: "steered";
}

export default function ValidationResultCard({
  validator,
  outcome,
  findings,
  durationMs,
  resolution,
}: Props) {
  const pass = outcome === "pass";
  const label = pass
    ? "Passed"
    : resolution === "steered"
      ? "Steered"
      : "Failed";
  return (
    <div
      className={`card validation-result-card ${
        pass ? "validation-pass" : "validation-fail"
      }${resolution ? ` validation-${resolution}` : ""}`}
    >
      <div className="validation-header">
        <span className={`validation-outcome-badge ${outcome}`}>{label}</span>
        <span className="validation-name">{validator}</span>
        <span className="validation-duration">{durationMs}ms</span>
      </div>
      {!pass && findings.length > 0 && (
        <ValidationFindings findings={findings} />
      )}
    </div>
  );
}
