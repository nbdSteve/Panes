import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import CostSparkline from "./CostSparkline";

describe("CostSparkline", () => {
  it("renders SVG with polyline", () => {
    const data = [
      { day: "2026-05-01", totalUsd: 0.10 },
      { day: "2026-05-02", totalUsd: 0.20 },
      { day: "2026-05-03", totalUsd: 0.15 },
    ];
    const { container } = render(<CostSparkline data={data} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    const polyline = container.querySelector("polyline");
    expect(polyline).not.toBeNull();
    expect(polyline!.getAttribute("points")).toBeTruthy();
  });

  it("handles empty data gracefully", () => {
    const { container } = render(<CostSparkline data={[]} />);
    expect(container.querySelector("svg")).toBeNull();
    expect(container.textContent).toContain("No cost data");
  });

  it("single data point renders without error", () => {
    const data = [{ day: "2026-05-01", totalUsd: 0.10 }];
    const { container } = render(<CostSparkline data={data} />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    const polyline = container.querySelector("polyline");
    expect(polyline!.getAttribute("points")).toBeTruthy();
  });

  it("scales Y-axis to data range", () => {
    const data = [
      { day: "2026-05-01", totalUsd: 0.0 },
      { day: "2026-05-02", totalUsd: 1.0 },
    ];
    const { container } = render(<CostSparkline data={data} width={100} height={50} />);
    const polyline = container.querySelector("polyline")!;
    const points = polyline.getAttribute("points")!;
    const coords = points.split(" ").map(p => {
      const [x, y] = p.split(",").map(Number);
      return { x, y };
    });
    // Largest value (1.0) should map to top (lowest y), smallest to bottom (highest y)
    expect(coords[1].y).toBeLessThan(coords[0].y);
  });
});
