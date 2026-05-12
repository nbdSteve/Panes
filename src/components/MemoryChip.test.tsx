import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import MemoryChip from "./MemoryChip";
import type { MemoryInfo } from "../lib/api";

const mem = (overrides: Partial<MemoryInfo> = {}): MemoryInfo => ({
  id: overrides.id ?? crypto.randomUUID(),
  workspaceId: "ws-1",
  memoryType: "pattern",
  content: "always run tests before committing",
  sourceThreadId: "t-0",
  pinned: false,
  createdAt: "2026-01-01T00:00:00Z",
  ...overrides,
});

describe("MemoryChip", () => {
  it("returns null for injected with no memories and no briefing", () => {
    const { container } = render(<MemoryChip variant="injected" memories={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders collapsed count for injected memories", () => {
    render(
      <MemoryChip
        variant="injected"
        memories={[mem({ id: "a", content: "first" }), mem({ id: "b", content: "second" })]}
      />,
    );
    expect(screen.getByText("2 memories injected")).toBeInTheDocument();
    // Expanded body not rendered yet
    expect(screen.queryByText("first")).not.toBeInTheDocument();
  });

  it("singularizes the label", () => {
    render(<MemoryChip variant="injected" memories={[mem({ content: "only one" })]} />);
    expect(screen.getByText("1 memory injected")).toBeInTheDocument();
  });

  it("appends briefing loaded suffix", () => {
    render(
      <MemoryChip
        variant="injected"
        memories={[mem({ content: "x" })]}
        briefing="use 4 space indent"
      />,
    );
    expect(screen.getByText("1 memory injected · briefing loaded")).toBeInTheDocument();
  });

  it("shows briefing-only when no memories are injected", () => {
    render(<MemoryChip variant="injected" memories={[]} briefing="be concise" />);
    expect(screen.getByText("0 memories injected · briefing loaded")).toBeInTheDocument();
  });

  it("expand reveals memory content and sets aria-expanded", () => {
    render(
      <MemoryChip
        variant="injected"
        memories={[mem({ id: "a", content: "first rule", memoryType: "decision" })]}
      />,
    );
    const btn = screen.getByRole("button");
    expect(btn).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(btn);
    expect(btn).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("first rule")).toBeInTheDocument();
    expect(screen.getByText("decision")).toBeInTheDocument();
  });

  it("extracted with zero memories still renders a muted chip", () => {
    render(<MemoryChip variant="extracted" memories={[]} />);
    expect(screen.getByText("0 memories written")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByText(/Nothing new to remember/)).toBeInTheDocument();
  });

  it("extracted error state replaces the label and shows the message", () => {
    render(<MemoryChip variant="extracted" memories={[]} error="mem0 unreachable" />);
    expect(screen.getByText("memory extraction failed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByText(/Couldn't reach the memory store/)).toBeInTheDocument();
    expect(screen.getByText(/mem0 unreachable/)).toBeInTheDocument();
  });

  it("Manage all memories link is only shown when memories > 0, and is called with no id", () => {
    const onView = vi.fn();
    const { rerender } = render(
      <MemoryChip variant="extracted" memories={[]} onViewMemories={onView} />,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(screen.queryByText(/Manage all memories/)).not.toBeInTheDocument();

    rerender(
      <MemoryChip
        variant="extracted"
        memories={[mem({ id: "m-a", content: "remember this" })]}
        onViewMemories={onView}
      />,
    );
    const btn = screen.getByRole("button", { name: /memory written/i });
    if (btn.getAttribute("aria-expanded") === "false") fireEvent.click(btn);
    const link = screen.getByText(/Manage all memories/);
    fireEvent.click(link);
    expect(onView).toHaveBeenCalledOnce();
    // Manage-all invokes with no id (or undefined). Assert via positional arg.
    expect(onView.mock.calls[0][0]).toBeUndefined();
  });

  it("per-memory 'Open in Memories' link invokes onViewMemories with that memory id", () => {
    const onView = vi.fn();
    render(
      <MemoryChip
        variant="extracted"
        memories={[
          mem({ id: "m-one", content: "first" }),
          mem({ id: "m-two", content: "second" }),
        ]}
        onViewMemories={onView}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /memories written/i }));
    const links = screen.getAllByText(/Open in Memories/);
    expect(links).toHaveLength(2);
    fireEvent.click(links[1]);
    expect(onView).toHaveBeenCalledWith("m-two");
  });

  it("does not render per-memory links when onViewMemories is absent", () => {
    render(
      <MemoryChip
        variant="extracted"
        memories={[mem({ id: "m-a", content: "hi" })]}
      />,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(screen.queryByText(/Open in Memories/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Manage all memories/)).not.toBeInTheDocument();
  });
});
