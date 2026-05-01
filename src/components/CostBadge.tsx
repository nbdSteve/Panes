import { formatCost } from "../lib/utils";

interface CostBadgeProps {
  cost: number;
  label?: string;
  budgetCap?: number;
}

export default function CostBadge({ cost, label, budgetCap }: CostBadgeProps) {
  const formatted = formatCost(cost);

  const capFormatted = budgetCap ? formatCost(budgetCap) : null;

  const ratio = budgetCap ? cost / budgetCap : 0;
  const warningClass = budgetCap
    ? ratio > 0.95 ? "cost-danger" : ratio > 0.8 ? "cost-warning" : ""
    : "";

  return (
    <span className={`cost-badge ${warningClass}`}>
      {label && <span className="cost-label">{label}</span>}
      {formatted}
      {capFormatted && <span className="cost-cap"> / {capFormatted}</span>}
    </span>
  );
}
