import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import FeedView from "./FeedView";
import type { WorkspaceInfo } from "../types";

vi.mock("../lib/api", () => {
  const now = Date.now();
  return {
    api: {
      listAllThreads: vi.fn().mockResolvedValue([
        {
          id: "t1", workspaceId: "ws1", prompt: "fix the bug",
          status: "completed", summary: "Fixed", costUsd: 0.50,
          durationMs: 5000, createdAt: new Date(now - 86400000).toISOString(), events: [],
        },
        {
          id: "t2", workspaceId: "ws2", prompt: "add feature",
          status: "completed", summary: "Added", costUsd: 0.10,
          durationMs: 3000, createdAt: new Date(now - 2 * 86400000).toISOString(), events: [],
        },
        {
          id: "t3", workspaceId: "ws1", prompt: "old thread",
          status: "completed", summary: "Done", costUsd: 0.30,
          durationMs: 2000, createdAt: new Date(now - 60 * 86400000).toISOString(), events: [],
        },
      ]),
      getAggregateCost: vi.fn().mockResolvedValue(0.90),
      getCostTimeline: vi.fn().mockResolvedValue([
        { day: "2026-05-04", totalUsd: 0.50 },
        { day: "2026-05-05", totalUsd: 0.10 },
      ]),
      getWorkspaceCostBreakdown: vi.fn().mockResolvedValue([
        { workspaceId: "ws1", workspaceName: "Backend", totalUsd: 0.80, threadCount: 2 },
        { workspaceId: "ws2", workspaceName: "Frontend", totalUsd: 0.10, threadCount: 1 },
      ]),
    },
  };
});

const workspaces: WorkspaceInfo[] = [
  { id: "ws1", path: "/tmp/ws1", name: "Backend" },
  { id: "ws2", path: "/tmp/ws2", name: "Frontend" },
];

const baseProps = {
  workspaces,
  showCost: true,
  onNavigateToThread: vi.fn(),
};

describe("FeedView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("sort by cost orders threads descending", async () => {
    const user = userEvent.setup();
    render(<FeedView {...baseProps} />);
    await waitFor(() => expect(screen.getByText("fix the bug")).toBeInTheDocument());

    await user.click(screen.getByText("Cost"));

    const items = screen.getAllByRole("generic").filter(el => el.classList.contains("feed-item"));
    const prompts = items.map(el => el.querySelector(".feed-item-prompt")?.textContent);
    // Highest cost first: t1=$0.50, t3=$0.30, t2=$0.10
    expect(prompts[0]).toContain("fix the bug");
    expect(prompts[1]).toContain("old thread");
    expect(prompts[2]).toContain("add feature");
  });

  it("sort by workspace groups threads", async () => {
    const user = userEvent.setup();
    render(<FeedView {...baseProps} />);
    await waitFor(() => expect(screen.getByText("fix the bug")).toBeInTheDocument());

    await user.click(screen.getByText("Workspace"));

    const items = screen.getAllByRole("generic").filter(el => el.classList.contains("feed-item"));
    const wsNames = items.map(el => el.querySelector(".feed-item-workspace")?.textContent);
    // Backend items together, then Frontend
    expect(wsNames[0]).toBe("Backend");
    expect(wsNames[1]).toBe("Backend");
    expect(wsNames[2]).toBe("Frontend");
  });

  it("date range filter 7d hides older threads", async () => {
    const user = userEvent.setup();
    render(<FeedView {...baseProps} />);
    await waitFor(() => expect(screen.getByText("fix the bug")).toBeInTheDocument());

    await user.click(screen.getByText("7d"));

    // t3 is 60 days old, should be hidden
    expect(screen.queryByText("old thread")).not.toBeInTheDocument();
    // t1 and t2 are within 7 days
    expect(screen.getByText("fix the bug")).toBeInTheDocument();
    expect(screen.getByText("add feature")).toBeInTheDocument();
  });

  it("date range filter All shows everything", async () => {
    const user = userEvent.setup();
    render(<FeedView {...baseProps} />);
    await waitFor(() => expect(screen.getByText("fix the bug")).toBeInTheDocument());

    // First filter to 7d
    await user.click(screen.getByText("7d"));
    expect(screen.queryByText("old thread")).not.toBeInTheDocument();

    // Then select All
    await user.click(screen.getByText("All"));
    expect(screen.getByText("old thread")).toBeInTheDocument();
  });

  it("analytics section is collapsible", async () => {
    const user = userEvent.setup();
    render(<FeedView {...baseProps} />);
    await waitFor(() => expect(screen.getByText("Hide Analytics")).toBeInTheDocument());

    // Sparkline should be visible
    const { container } = render(<FeedView {...baseProps} />);
    await waitFor(() => expect(container.querySelector(".cost-sparkline")).not.toBeNull());

    // Click to hide
    await user.click(screen.getAllByText("Hide Analytics")[0]);
    expect(screen.getByText("Show Analytics")).toBeInTheDocument();
  });
});
