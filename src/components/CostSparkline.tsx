interface CostSparklineProps {
  data: { day: string; totalUsd: number }[];
  width?: number;
  height?: number;
}

export default function CostSparkline({ data, width = 300, height = 60 }: CostSparklineProps) {
  if (data.length === 0) {
    return <div className="sparkline-empty">No cost data yet</div>;
  }

  const maxVal = Math.max(...data.map(d => d.totalUsd), 0.001);
  const padding = 4;
  const chartW = width - padding * 2;
  const chartH = height - padding * 2;

  const points = data.map((d, i) => {
    const x = padding + (data.length === 1 ? chartW / 2 : (i / (data.length - 1)) * chartW);
    const y = padding + chartH - (d.totalUsd / maxVal) * chartH;
    return `${x},${y}`;
  }).join(" ");

  const areaPoints = `${padding},${padding + chartH} ${points} ${padding + (data.length === 1 ? chartW / 2 : chartW)},${padding + chartH}`;

  return (
    <svg className="cost-sparkline" width={width} height={height} viewBox={`0 0 ${width} ${height}`}>
      <polygon points={areaPoints} className="sparkline-area" />
      <polyline points={points} className="sparkline-line" fill="none" />
    </svg>
  );
}
