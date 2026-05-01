import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import TranscriptView from "./TranscriptView";
import type { AgentEvent } from "../App";

describe("TranscriptView", () => {
  it("renders prompt as You message", () => {
    render(<TranscriptView events={[]} prompt="hello world" />);

    expect(screen.getByText("You")).toBeInTheDocument();
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });

  it("empty events shows only prompt", () => {
    const { container } = render(<TranscriptView events={[]} prompt="just me" />);

    const messages = container.querySelectorAll(".transcript-message");
    expect(messages).toHaveLength(1);
  });

  it("renders thinking event", () => {
    const events: AgentEvent[] = [
      { event_type: "thinking", text: "Let me consider..." },
    ];
    const { container } = render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Thinking")).toBeInTheDocument();
    expect(screen.getByText("Let me consider...")).toBeInTheDocument();
    expect(container.querySelector(".transcript-thinking")).toBeInTheDocument();
  });

  it("renders text event", () => {
    const events: AgentEvent[] = [
      { event_type: "text", text: "Here is my response" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Assistant")).toBeInTheDocument();
    expect(screen.getByText("Here is my response")).toBeInTheDocument();
  });

  it("renders tool_request event", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", tool_name: "Bash", description: "ls -la", id: "t1" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Tool call: Bash")).toBeInTheDocument();
    expect(screen.getByText("ls -la")).toBeInTheDocument();
  });

  it("renders tool_result success", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_result", success: true, output: "file.txt", id: "t1" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Tool result")).toBeInTheDocument();
    expect(screen.getByText("file.txt")).toBeInTheDocument();
  });

  it("renders tool_result failure", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_result", success: false, output: "command not found", id: "t1" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Tool error")).toBeInTheDocument();
  });

  it("renders follow_up event", () => {
    const events: AgentEvent[] = [
      { event_type: "follow_up", text: "what about this?" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    const youLabels = screen.getAllByText("You");
    expect(youLabels).toHaveLength(2); // prompt + follow_up
    expect(screen.getByText("what about this?")).toBeInTheDocument();
  });

  it("renders sub_agent_spawned", () => {
    const events: AgentEvent[] = [
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t1", description: "researching docs" },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Sub-agent spawned")).toBeInTheDocument();
    expect(screen.getByText("researching docs")).toBeInTheDocument();
  });

  it("renders sub_agent_complete with cost", () => {
    const events: AgentEvent[] = [
      { event_type: "sub_agent_complete", parent_tool_use_id: "t1", summary: "found the answer", cost_usd: 0.012 },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Sub-agent complete")).toBeInTheDocument();
    expect(screen.getByText(/found the answer/)).toBeInTheDocument();
    expect(screen.getByText(/\$0\.01/)).toBeInTheDocument();
  });

  it("renders error event", () => {
    const events: AgentEvent[] = [
      { event_type: "error", message: "something went wrong" },
    ];
    const { container } = render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Error")).toBeInTheDocument();
    expect(screen.getByText("something went wrong")).toBeInTheDocument();
    expect(container.querySelector(".transcript-error")).toBeInTheDocument();
  });

  it("renders complete event with cost and turns", () => {
    const events: AgentEvent[] = [
      { event_type: "complete", summary: "All done", total_cost_usd: 0.035, turns: 3 },
    ];
    render(<TranscriptView events={events} prompt="test" />);

    expect(screen.getByText("Session complete")).toBeInTheDocument();
    expect(screen.getByText(/All done/)).toBeInTheDocument();
    expect(screen.getByText(/\$0\.04/)).toBeInTheDocument();
    expect(screen.getByText(/3 turns/)).toBeInTheDocument();
  });
});
