import { formatCost } from "../lib/utils";

interface CostBadgeProps {
  cost: number;
  label?: string;
  budgetCap?: number;
  /**
   * Badge the cost as an estimate. Used when the adapter tokenized locally
   * rather than reading real token counts from the backend — ACP sessions
   * today. Renders an "est." prefix and a tooltip.
   */
  estimated?: boolean;
}

export default function CostBadge({ cost, label, budgetCap, estimated }: CostBadgeProps) {
  const formatted = formatCost(cost);

  const capFormatted = budgetCap ? formatCost(budgetCap) : null;

  const ratio = budgetCap ? cost / budgetCap : 0;
  const warningClass = budgetCap
    ? ratio > 0.95 ? "cost-danger" : ratio > 0.8 ? "cost-warning" : ""
    : "";

  return (
    <span
      className={`cost-badge ${warningClass} ${estimated ? "cost-estimated" : ""}`}
      title={estimated ? "Estimated — backend does not report real token counts" : undefined}
    >
      {label && <span className="cost-label">{label}</span>}
      {estimated && <span className="cost-est-prefix">est. </span>}
      {formatted}
      {capFormatted && <span className="cost-cap"> / {capFormatted}</span>}
    </span>
  );
}
