export type FindingSeverity = "info" | "warning" | "error";

export interface ValidationFinding {
  severity: FindingSeverity;
  message: string;
  span?: string | null;
  source_hint?: string | null;
}

interface Props {
  findings: ValidationFinding[];
}

export default function ValidationFindings({ findings }: Props) {
  if (findings.length === 0) return null;
  return (
    <ul className="validation-findings">
      {findings.map((f, i) => (
        <li key={i} className={`validation-finding severity-${f.severity}`}>
          <span className={`finding-severity-badge severity-${f.severity}`}>
            {f.severity}
          </span>
          <div className="finding-body">
            <div className="finding-message">{f.message}</div>
            {f.span && <code className="finding-span">{f.span}</code>}
            {f.source_hint && (
              <div className="finding-source">source: {f.source_hint}</div>
            )}
          </div>
        </li>
      ))}
    </ul>
  );
}
