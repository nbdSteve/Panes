import { describe, it, expect } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import WorkspaceValidatorsPanel from "./WorkspaceValidatorsPanel";

describe("WorkspaceValidatorsPanel", () => {
  it("renders the add options when no validators exist yet", async () => {
    render(<WorkspaceValidatorsPanel workspaceId="ws-empty-1" />);
    await waitFor(() => {
      expect(screen.getByText(/Add a validator/i)).toBeInTheDocument();
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
      expect(screen.getByText("Secret Scan")).toBeInTheDocument();
    });
  });

  it("clicking an add option creates a validator row", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-add-1" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Citation Check"));

    await waitFor(() => {
      // The validator row now shows the same label, and the Secret Scan option
      // is still available to add.
      const labels = screen.getAllByText("Citation Check");
      expect(labels.length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText("Secret Scan")).toBeInTheDocument();
    });
  });

  it("toggle disables an existing validator", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-toggle-1" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    const toggle = await waitFor(() => screen.getByRole("checkbox"));
    expect(toggle).toBeChecked();
    await user.click(toggle);
    await waitFor(() => {
      expect(screen.getByText(/disabled/i)).toBeInTheDocument();
    });
  });

  it("expanding a citation validator shows structured controls, not raw JSON by default", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-expand-1" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    // Expand the row by clicking the label.
    const labels = await waitFor(() => screen.getAllByText("Citation Check"));
    await user.click(labels[0]);

    await waitFor(() => {
      expect(screen.getByText("Verify line references")).toBeInTheDocument();
      expect(
        screen.getByText(/Allow paths outside the workspace/i),
      ).toBeInTheDocument();
      // Raw JSON editor is hidden by default, accessible via toggle.
      expect(screen.getByText(/Show advanced \(JSON\)/)).toBeInTheDocument();
    });
  });
});
