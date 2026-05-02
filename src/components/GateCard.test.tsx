import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import GateCard from "./GateCard";

const baseProps = {
  description: "rm -rf /important",
  riskLevel: "critical",
  toolUseId: "gate_0",
  toolName: "Bash",
  runningCost: 0.005,
  onApprove: vi.fn(),
  onReject: vi.fn(),
};

describe("GateCard", () => {
  it("pending state renders action buttons", () => {
    const onSteer = vi.fn();
    render(<GateCard {...baseProps} onSteer={onSteer} />);

    expect(screen.getByText("Continue")).toBeInTheDocument();
    expect(screen.getByText("Steer")).toBeInTheDocument();
    expect(screen.getByText("Abort")).toBeInTheDocument();
    expect(screen.getByText("Approval needed")).toBeInTheDocument();
  });

  it("steer button absent without onSteer prop", () => {
    render(<GateCard {...baseProps} />);

    expect(screen.getByText("Continue")).toBeInTheDocument();
    expect(screen.getByText("Abort")).toBeInTheDocument();
    expect(screen.queryByText("Steer")).not.toBeInTheDocument();
  });

  it("click steer opens textarea", async () => {
    const user = userEvent.setup();
    render(<GateCard {...baseProps} onSteer={vi.fn()} />);

    await user.click(screen.getByText("Steer"));
    expect(screen.getByPlaceholderText("Redirect the agent...")).toBeInTheDocument();
  });

  it("enter submits steer text", async () => {
    const user = userEvent.setup();
    const onSteer = vi.fn();
    render(<GateCard {...baseProps} onSteer={onSteer} />);

    await user.click(screen.getByText("Steer"));
    const textarea = screen.getByPlaceholderText("Redirect the agent...");
    await user.type(textarea, "try a different approach");
    await user.keyboard("{Enter}");

    expect(onSteer).toHaveBeenCalledWith("try a different approach");
  });

  it("escape dismisses steer mode", async () => {
    const user = userEvent.setup();
    render(<GateCard {...baseProps} onSteer={vi.fn()} />);

    await user.click(screen.getByText("Steer"));
    expect(screen.getByPlaceholderText("Redirect the agent...")).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(screen.queryByPlaceholderText("Redirect the agent...")).not.toBeInTheDocument();
  });

  it("send button disabled when empty", async () => {
    const user = userEvent.setup();
    render(<GateCard {...baseProps} onSteer={vi.fn()} />);

    await user.click(screen.getByText("Steer"));
    expect(screen.getByText("Send")).toBeDisabled();
  });

  it("send button has tooltip when disabled", async () => {
    const user = userEvent.setup();
    render(<GateCard {...baseProps} onSteer={vi.fn()} />);

    await user.click(screen.getByText("Steer"));
    const sendBtn = screen.getByText("Send");
    expect(sendBtn).toBeDisabled();
    expect(sendBtn).toHaveAttribute("title", "Enter a message to steer");
  });

  it("send button click calls onSteer", async () => {
    const user = userEvent.setup();
    const onSteer = vi.fn();
    render(<GateCard {...baseProps} onSteer={onSteer} />);

    await user.click(screen.getByText("Steer"));
    await user.type(screen.getByPlaceholderText("Redirect the agent..."), "new direction");
    await user.click(screen.getByText("Send"));

    expect(onSteer).toHaveBeenCalledWith("new direction");
  });

  it("resolved approved shows Continued", () => {
    render(<GateCard {...baseProps} resolved="approved" />);

    expect(screen.getByText("Continued")).toBeInTheDocument();
    expect(screen.queryByText("Continue")).not.toBeInTheDocument();
  });

  it("resolved rejected shows Aborted", () => {
    render(<GateCard {...baseProps} resolved="rejected" />);

    expect(screen.getByText("Aborted")).toBeInTheDocument();
    expect(screen.queryByText("Abort")).not.toBeInTheDocument();
  });

  it("resolved steered shows Steered", () => {
    render(<GateCard {...baseProps} resolved="steered" />);

    expect(screen.getByText("Steered")).toBeInTheDocument();
    expect(screen.queryByText("Steer")).not.toBeInTheDocument();
  });

  it("renders risk badge", () => {
    render(<GateCard {...baseProps} />);

    const badge = screen.getByText("critical");
    expect(badge).toBeInTheDocument();
    expect(badge.className).toContain("risk-badge");
    expect(badge.className).toContain("critical");
  });

  it("hides cost badge when showCost is false", () => {
    render(<GateCard {...baseProps} showCost={false} />);

    expect(screen.queryByText(/So far/)).not.toBeInTheDocument();
  });

  it("shows cost badge by default", () => {
    render(<GateCard {...baseProps} />);

    expect(screen.getByText(/So far/)).toBeInTheDocument();
  });
});
