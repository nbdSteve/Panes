import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import SettingsPanel from "./SettingsPanel";
import type { WorkspaceInfo } from "../App";
import type { FeatureInfo } from "../types";

const workspaces: WorkspaceInfo[] = [
  { id: "ws-1", path: "/tmp/a", name: "Alpha", defaultAdapter: "claude-code" },
  { id: "ws-2", path: "/tmp/b", name: "Beta", defaultAdapter: "kiro-cli" },
];

const features: FeatureInfo[] = [];

describe("SettingsPanel — workspace default adapter", () => {
  it("renders a dropdown for each workspace reflecting its stored default", async () => {
    render(
      <SettingsPanel
        workspaces={workspaces}
        features={features}
        onToggleFeature={() => {}}
        onSetDefaultAdapter={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Workspace Defaults" })).toBeInTheDocument();
    });

    const alpha = screen.getByLabelText("Default adapter for Alpha") as HTMLSelectElement;
    const beta = screen.getByLabelText("Default adapter for Beta") as HTMLSelectElement;
    expect(alpha.value).toBe("claude-code");
    expect(beta.value).toBe("kiro-cli");
  });

  it("invokes the handler with the selected adapter when changed", async () => {
    const user = userEvent.setup();
    const onSetDefaultAdapter = vi.fn();

    render(
      <SettingsPanel
        workspaces={workspaces}
        features={features}
        onToggleFeature={() => {}}
        onSetDefaultAdapter={onSetDefaultAdapter}
      />,
    );

    await waitFor(() => {
      expect(screen.getByLabelText("Default adapter for Alpha")).toBeInTheDocument();
    });

    await user.selectOptions(screen.getByLabelText("Default adapter for Alpha"), "kiro-cli");

    expect(onSetDefaultAdapter).toHaveBeenCalledWith("ws-1", "kiro-cli");
  });

  it("includes the stored value as an option even when it's not in the adapter list", async () => {
    // Scenario: a workspace stores `unknown-backend` (kiro-cli binary went
    // missing, adapter was unregistered, etc). The UI should still show
    // what's persisted rather than silently snapping it to claude-code.
    const workspaces: WorkspaceInfo[] = [
      { id: "ws-x", path: "/tmp/x", name: "Exo", defaultAdapter: "unknown-backend" },
    ];

    render(
      <SettingsPanel
        workspaces={workspaces}
        features={features}
        onToggleFeature={() => {}}
        onSetDefaultAdapter={() => {}}
      />,
    );

    await waitFor(() => {
      const select = screen.getByLabelText("Default adapter for Exo") as HTMLSelectElement;
      expect(select.value).toBe("unknown-backend");
      const option = Array.from(select.options).find((o) => o.value === "unknown-backend");
      expect(option).toBeDefined();
    });
  });

  it("does not render the workspace defaults section when no workspaces exist", async () => {
    render(
      <SettingsPanel
        workspaces={[]}
        features={features}
        onToggleFeature={() => {}}
        onSetDefaultAdapter={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.queryByRole("heading", { name: "Workspace Defaults" })).not.toBeInTheDocument();
    });
  });
});
