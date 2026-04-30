interface CostBadgeProps {
  cost: number;
  label?: string;
  budgetCap?: number;
}

export default function CostBadge({ cost, label, budgetCap }: CostBadgeProps) {
  const formatted =
    cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;

  const capFormatted = budgetCap
    ? budgetCap < 0.01 ? `$${budgetCap.toFixed(4)}` : `$${budgetCap.toFixed(2)}`
    : null;

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
