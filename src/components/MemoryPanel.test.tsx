import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import MemoryPanel from "./MemoryPanel";

const mockInvoke = (window as any).__TAURI_INTERNALS__.invoke;

describe("MemoryPanel", () => {
  beforeEach(() => {
    // Reset mock state by clearing memories and briefings via the mock
    // The mock uses in-memory arrays, so we re-render fresh each test
  });

  it("renders empty state when no memories exist", async () => {
    render(<MemoryPanel workspaceId="test-ws" />);

    await waitFor(() => {
      expect(screen.getByText("No memories yet. Complete a thread to start building context.")).toBeInTheDocument();
    });
  });

  it("renders briefing section with Add button when no briefing", async () => {
    render(<MemoryPanel workspaceId="test-ws" />);

    await waitFor(() => {
      expect(screen.getByText("Briefing")).toBeInTheDocument();
      expect(screen.getByText("Add")).toBeInTheDocument();
      expect(screen.getByText("No briefing set for this workspace.")).toBeInTheDocument();
    });
  });

  it("can create a briefing", async () => {
    const user = userEvent.setup();
    render(<MemoryPanel workspaceId="test-ws-briefing" />);

    await waitFor(() => {
      expect(screen.getByText("Add")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Add"));

    const textarea = screen.getByPlaceholderText("Instructions for every thread in this workspace...");
    expect(textarea).toBeInTheDocument();

    await user.type(textarea, "Always use TypeScript");
    await user.click(screen.getByText("Save"));

    await waitFor(() => {
      expect(screen.getByText("Always use TypeScript")).toBeInTheDocument();
    });
  });

  it("can cancel briefing editing", async () => {
    const user = userEvent.setup();
    render(<MemoryPanel workspaceId="test-ws-cancel" />);

    await waitFor(() => {
      expect(screen.getByText("Add")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Add"));
    expect(screen.getByPlaceholderText("Instructions for every thread in this workspace...")).toBeInTheDocument();

    await user.click(screen.getByText("Cancel"));

    await waitFor(() => {
      expect(screen.getByText("No briefing set for this workspace.")).toBeInTheDocument();
    });
  });

  it("shows memory count badge", async () => {
    // First add a memory via the mock
    await mockInvoke("extract_memories", {
      workspaceId: "test-ws-count",
      threadId: "t1",
      transcript: "User: hello\nAssistant: hi",
    });

    render(<MemoryPanel workspaceId="test-ws-count" />);

    await waitFor(() => {
      expect(screen.getByText("1")).toBeInTheDocument();
    });
  });

  it("shows memory type badge", async () => {
    await mockInvoke("extract_memories", {
      workspaceId: "test-ws-type",
      threadId: "t1",
      transcript: "User: test\nAssistant: response",
    });

    render(<MemoryPanel workspaceId="test-ws-type" />);

    await waitFor(() => {
      expect(screen.getByText("pattern")).toBeInTheDocument();
    });
  });

  it("can delete a memory", async () => {
    await mockInvoke("extract_memories", {
      workspaceId: "test-ws-delete",
      threadId: "t1",
      transcript: "User: delete test\nAssistant: ok",
    });

    const user = userEvent.setup();
    render(<MemoryPanel workspaceId="test-ws-delete" />);

    await waitFor(() => {
      expect(screen.getByTitle("Delete")).toBeInTheDocument();
    });

    await user.click(screen.getByTitle("Delete"));

    await waitFor(() => {
      expect(screen.getByText("No memories yet. Complete a thread to start building context.")).toBeInTheDocument();
    });
  });

  it("can edit a memory", async () => {
    await mockInvoke("extract_memories", {
      workspaceId: "test-ws-edit",
      threadId: "t1",
      transcript: "User: edit test\nAssistant: response",
    });

    const user = userEvent.setup();
    render(<MemoryPanel workspaceId="test-ws-edit" />);

    await waitFor(() => {
      expect(screen.getByTitle("Edit")).toBeInTheDocument();
    });

    await user.click(screen.getByTitle("Edit"));

    const textarea = screen.getByRole("textbox");
    await user.clear(textarea);
    await user.type(textarea, "Updated memory content");
    await user.click(screen.getByText("Save"));

    await waitFor(() => {
      expect(screen.getByText("Updated memory content")).toBeInTheDocument();
    });
  });

  it("can pin and unpin a memory", async () => {
    await mockInvoke("extract_memories", {
      workspaceId: "test-ws-pin",
      threadId: "t1",
      transcript: "User: pin test\nAssistant: response",
    });

    const user = userEvent.setup();
    render(<MemoryPanel workspaceId="test-ws-pin" />);

    await waitFor(() => {
      expect(screen.getByTitle("Pin")).toBeInTheDocument();
    });

    await user.click(screen.getByTitle("Pin"));

    await waitFor(() => {
      expect(screen.getByText("Pinned")).toBeInTheDocument();
      expect(screen.getByTitle("Unpin")).toBeInTheDocument();
    });

    await user.click(screen.getByTitle("Unpin"));

    await waitFor(() => {
      expect(screen.queryByText("Pinned")).not.toBeInTheDocument();
    });
  });
});
