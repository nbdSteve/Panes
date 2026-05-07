import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import DashboardView from "./DashboardView";
import type { WorkspaceInfo, ThreadInfo } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    getAggregateCost: vi.fn().mockResolvedValue(1.25),
  },
}));

const workspaces: WorkspaceInfo[] = [
  { id: "ws1", path: "/tmp/ws1", name: "Backend" },
  { id: "ws2", path: "/tmp/ws2", name: "Frontend", budgetCap: 5.0 },
];

function makeThread(overrides: Partial<ThreadInfo> = {}): ThreadInfo {
  return {
    id: "t1",
    workspaceId: "ws1",
    prompt: "fix the login bug",
    status: "running",
    events: [],
    createdAt: Date.now() - 60000,
    ...overrides,
  };
}

const baseProps = {
  workspaces,
  threads: [] as ThreadInfo[],
  showCost: true,
  onNavigateToWorkspace: vi.fn(),
  onApproveGate: vi.fn(),
  onRejectGate: vi.fn(),
};

describe("DashboardView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders workspace cards for all workspaces", () => {
    render(<DashboardView {...baseProps} />);
    expect(screen.getByText("Backend")).toBeInTheDocument();
    expect(screen.getByText("Frontend")).toBeInTheDocument();
  });

  it("shows idle status when no threads running", () => {
    render(<DashboardView {...baseProps} />);
    const idleElements = screen.getAllByText("Idle");
    expect(idleElements.length).toBe(2);
  });

  it("shows working status when thread is running", () => {
    const threads = [makeThread({ status: "running" })];
    render(<DashboardView {...baseProps} threads={threads} />);
    expect(screen.getByText("working")).toBeInTheDocument();
    expect(screen.getByText(/fix the login/)).toBeInTheDocument();
  });

  it("shows gate status with approve/reject buttons", () => {
    const threads = [makeThread({
      status: "gate",
      events: [
        { event_type: "tool_request", id: "gate_0", tool_name: "Bash", needs_approval: true, risk_level: "critical" } as any,
      ],
    })];
    render(<DashboardView {...baseProps} threads={threads} />);
    expect(screen.getByText("gate")).toBeInTheDocument();
    expect(screen.getByText("Continue")).toBeInTheDocument();
    expect(screen.getByText("Abort")).toBeInTheDocument();
  });

  it("approve button calls onApproveGate with correct args", async () => {
    const user = userEvent.setup();
    const threads = [makeThread({
      id: "t-gate",
      status: "gate",
      events: [
        { event_type: "tool_request", id: "gate_42", tool_name: "Bash", needs_approval: true, risk_level: "high" } as any,
      ],
    })];
    render(<DashboardView {...baseProps} threads={threads} />);
    await user.click(screen.getByText("Continue"));
    expect(baseProps.onApproveGate).toHaveBeenCalledWith("t-gate", "gate_42");
  });

  it("reject button calls onRejectGate with correct args", async () => {
    const user = userEvent.setup();
    const threads = [makeThread({
      id: "t-gate",
      status: "gate",
      events: [
        { event_type: "tool_request", id: "gate_42", tool_name: "Bash", needs_approval: true, risk_level: "high" } as any,
      ],
    })];
    render(<DashboardView {...baseProps} threads={threads} />);
    await user.click(screen.getByText("Abort"));
    expect(baseProps.onRejectGate).toHaveBeenCalledWith("t-gate", "gate_42");
  });

  it("clicking card calls onNavigateToWorkspace", async () => {
    const user = userEvent.setup();
    render(<DashboardView {...baseProps} />);
    await user.click(screen.getByText("Backend").closest(".dashboard-card")!);
    expect(baseProps.onNavigateToWorkspace).toHaveBeenCalledWith("ws1");
  });

  it("shows running cost for active thread", () => {
    const threads = [makeThread({
      status: "running",
      events: [{ event_type: "cost_update", total_usd: 0.05 } as any],
    })];
    render(<DashboardView {...baseProps} threads={threads} />);
    expect(screen.getByText("$0.05")).toBeInTheDocument();
  });

  it("shows summary row with counts", () => {
    const threads = [
      makeThread({ id: "t1", status: "running" }),
      makeThread({ id: "t2", workspaceId: "ws2", status: "gate", events: [{ event_type: "tool_request", id: "g1", needs_approval: true } as any] }),
    ];
    render(<DashboardView {...baseProps} threads={threads} />);
    expect(screen.getByText("2")).toBeInTheDocument(); // active count
    expect(screen.getByText("1")).toBeInTheDocument(); // needs attention
  });

  it("hides cost when showCost is false", () => {
    const threads = [makeThread({
      status: "running",
      events: [{ event_type: "cost_update", total_usd: 0.05 } as any],
    })];
    render(<DashboardView {...baseProps} threads={threads} showCost={false} />);
    expect(screen.queryByText("$0.05")).not.toBeInTheDocument();
  });
});
