import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import WorkspaceCostBars from "./WorkspaceCostBars";

describe("WorkspaceCostBars", () => {
  it("renders one bar per workspace", () => {
    const data = [
      { workspaceName: "Backend", totalUsd: 0.50 },
      { workspaceName: "Frontend", totalUsd: 0.30 },
      { workspaceName: "Infra", totalUsd: 0.10 },
    ];
    const { container } = render(<WorkspaceCostBars data={data} />);
    const rows = container.querySelectorAll(".cost-bar-row");
    expect(rows.length).toBe(3);
  });

  it("bar widths proportional to cost", () => {
    const data = [
      { workspaceName: "High", totalUsd: 1.0 },
      { workspaceName: "Low", totalUsd: 0.5 },
    ];
    const { container } = render(<WorkspaceCostBars data={data} />);
    const fills = container.querySelectorAll(".cost-bar-fill") as NodeListOf<HTMLElement>;
    expect(fills[0].style.width).toBe("100%");
    expect(fills[1].style.width).toBe("50%");
  });

  it("shows workspace name and formatted cost", () => {
    const data = [{ workspaceName: "MyProject", totalUsd: 0.75 }];
    render(<WorkspaceCostBars data={data} />);
    expect(screen.getByText("MyProject")).toBeInTheDocument();
    expect(screen.getByText("$0.75")).toBeInTheDocument();
  });

  it("handles empty data", () => {
    const { container } = render(<WorkspaceCostBars data={[]} />);
    expect(container.textContent).toContain("No workspace data");
  });

  it("handles all-zero costs", () => {
    const data = [
      { workspaceName: "A", totalUsd: 0 },
      { workspaceName: "B", totalUsd: 0 },
    ];
    const { container } = render(<WorkspaceCostBars data={data} />);
    const fills = container.querySelectorAll(".cost-bar-fill") as NodeListOf<HTMLElement>;
    // Should not crash from division by zero; bars render at 0%
    expect(fills.length).toBe(2);
    expect(fills[0].style.width).toBe("0%");
  });
});
