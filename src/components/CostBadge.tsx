interface CostBadgeProps {
  cost: number;
  label?: string;
}

export default function CostBadge({ cost, label }: CostBadgeProps) {
  const formatted =
    cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;

  return (
    <span className="cost-badge">
      {label && <span className="cost-label">{label}</span>}
      {formatted}
    </span>
  );
}
