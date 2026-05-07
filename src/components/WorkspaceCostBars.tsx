import { formatCost } from "../lib/utils";

interface WorkspaceCostBarsProps {
  data: { workspaceName: string; totalUsd: number }[];
}

export default function WorkspaceCostBars({ data }: WorkspaceCostBarsProps) {
  if (data.length === 0) {
    return <div className="cost-bars-empty">No workspace data</div>;
  }

  const maxCost = Math.max(...data.map(d => d.totalUsd), 0.001);

  return (
    <div className="cost-bars">
      {data.map((item) => (
        <div key={item.workspaceName} className="cost-bar-row">
          <span className="cost-bar-label">{item.workspaceName}</span>
          <div className="cost-bar-track">
            <div
              className="cost-bar-fill"
              style={{ width: `${(item.totalUsd / maxCost) * 100}%` }}
            />
          </div>
          <span className="cost-bar-value">{formatCost(item.totalUsd)}</span>
        </div>
      ))}
    </div>
  );
}
