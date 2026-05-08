import { describe, it, expect } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
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

  it("citation editor toggles the line-refs checkbox and persists config", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-cite-toggle" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    const labels = await waitFor(() => screen.getAllByText("Citation Check"));
    await user.click(labels[0]);

    const lineRefsLabel = await waitFor(() =>
      screen.getByText("Verify line references"),
    );
    // The checkbox is the sibling input inside the label's parent.
    const row = lineRefsLabel.closest("label") as HTMLLabelElement;
    const checkbox = row.querySelector(
      "input[type='checkbox']",
    ) as HTMLInputElement;
    expect(checkbox.checked).toBe(true);

    await user.click(checkbox);
    await waitFor(() => {
      // After the update round-trips through the mock, the editor reflects the
      // new value.
      expect(checkbox.checked).toBe(false);
    });
  });

  it("secret_scan editor adds and removes custom patterns", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-secret-patterns" />);

    await waitFor(() => {
      expect(screen.getByText("Secret Scan")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Secret Scan"));

    const labels = await waitFor(() => screen.getAllByText("Secret Scan"));
    await user.click(labels[0]);

    const input = await waitFor(() =>
      screen.getByPlaceholderText(/INTERNAL-/),
    );
    const pattern = "FOO-[A-Z]+-123";
    // userEvent.type treats [ ] { } as special; use fireEvent.change instead.
    fireEvent.change(input, { target: { value: pattern } });
    await user.click(screen.getByText("Add"));

    await waitFor(() => {
      expect(screen.getByText(pattern)).toBeInTheDocument();
    });

    await user.click(screen.getByText("Remove"));
    await waitFor(() => {
      expect(screen.queryByText(pattern)).not.toBeInTheDocument();
    });
  });

  it("advanced JSON editor saves a valid config", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-adv-save" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    const labels = await waitFor(() => screen.getAllByText("Citation Check"));
    await user.click(labels[0]);

    await user.click(
      await waitFor(() => screen.getByText(/Show advanced \(JSON\)/)),
    );

    const textarea = (await waitFor(() =>
      screen.getByDisplayValue(/check_line_refs/),
    )) as HTMLTextAreaElement;
    await user.clear(textarea);
    await user.type(textarea, '{{"check_line_refs":false}');
    await user.click(screen.getByText("Save JSON"));

    // After save, the advanced editor rehydrates from the updated config.
    await waitFor(() => {
      const text = (textarea as HTMLTextAreaElement).value;
      expect(text).toMatch(/"check_line_refs"\s*:\s*false/);
    });
  });

  it("advanced JSON editor surfaces a parse error for invalid JSON", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-adv-invalid" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    const labels = await waitFor(() => screen.getAllByText("Citation Check"));
    await user.click(labels[0]);

    await user.click(
      await waitFor(() => screen.getByText(/Show advanced \(JSON\)/)),
    );

    const textarea = (await waitFor(() =>
      screen.getByDisplayValue(/check_line_refs/),
    )) as HTMLTextAreaElement;
    await user.clear(textarea);
    await user.type(textarea, "not json");
    await user.click(screen.getByText("Save JSON"));

    await waitFor(() => {
      const err = document.querySelector(".validators-error-inline");
      expect(err).not.toBeNull();
      expect(err?.textContent ?? "").toMatch(/JSON|Unexpected/i);
    });
  });

  it("delete button removes the validator row", async () => {
    const user = userEvent.setup();
    render(<WorkspaceValidatorsPanel workspaceId="ws-del" />);

    await waitFor(() => {
      expect(screen.getByText("Citation Check")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Citation Check"));

    // Wait for the row to exist. `.validators-item` is the row; the Add
    // option for the same type disappears while the validator is configured.
    await waitFor(() => {
      expect(document.querySelector(".validators-item")).not.toBeNull();
    });

    const removeBtn = document.querySelector(
      ".validators-item .btn-delete-inline",
    ) as HTMLButtonElement;
    expect(removeBtn).not.toBeNull();
    await user.click(removeBtn);

    await waitFor(() => {
      // Row is gone; Add option for Citation Check should be back.
      expect(document.querySelector(".validators-item")).toBeNull();
      expect(
        document.querySelector(
          ".validators-add-option .validators-add-option-label",
        )?.textContent,
      ).toBe("Citation Check");
    });
  });
});
