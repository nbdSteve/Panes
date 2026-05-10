import { describe, it, expect, vi } from "vitest";
import { refreshAdapterLists } from "./adapterLists";
import type { AgentInfo, ModelInfo } from "../types";

const FALLBACK: ModelInfo[] = [
  { id: "fallback-1", label: "Fallback 1", description: "fallback model" },
];

describe("refreshAdapterLists", () => {
  it("returns both lists when the backend succeeds", async () => {
    const agents: AgentInfo[] = [{ name: "harold", model: null, description: "" }];
    const models: ModelInfo[] = [{ id: "sonnet", label: "Sonnet", description: "" }];
    const api = {
      listAgents: vi.fn().mockResolvedValue(agents),
      listModels: vi.fn().mockResolvedValue(models),
    };
    const out = await refreshAdapterLists("kiro-cli", FALLBACK, api);
    expect(out.agents).toEqual(agents);
    expect(out.models).toEqual(models);
  });

  it("returns empty agents when listAgents rejects", async () => {
    const api = {
      listAgents: vi.fn().mockRejectedValue(new Error("backend down")),
      listModels: vi.fn().mockResolvedValue([{ id: "x", label: "X", description: "" }]),
    };
    const out = await refreshAdapterLists("broken", FALLBACK, api);
    expect(out.agents).toEqual([]);
    expect(out.models).toEqual([{ id: "x", label: "X", description: "" }]);
  });

  it("returns fallback models when listModels rejects", async () => {
    const api = {
      listAgents: vi.fn().mockResolvedValue([]),
      listModels: vi.fn().mockRejectedValue(new Error("no models")),
    };
    const out = await refreshAdapterLists("kiro-cli", FALLBACK, api);
    expect(out.models).toEqual(FALLBACK);
  });

  it("returns fallback models when listModels returns empty", async () => {
    const api = {
      listAgents: vi.fn().mockResolvedValue([]),
      listModels: vi.fn().mockResolvedValue([]),
    };
    const out = await refreshAdapterLists("kiro-cli", FALLBACK, api);
    expect(out.models).toEqual(FALLBACK);
  });

  it("never rejects even when both backends fail", async () => {
    const api = {
      listAgents: vi.fn().mockRejectedValue(new Error("boom")),
      listModels: vi.fn().mockRejectedValue(new Error("boom")),
    };
    // The entire point of the helper: the UI switches adapter and the worst
    // case is an empty picker + fallback models, never an unhandled promise.
    await expect(refreshAdapterLists("broken", FALLBACK, api)).resolves.toEqual({
      agents: [],
      models: FALLBACK,
    });
  });

  it("queries the adapter id it was given", async () => {
    const api = {
      listAgents: vi.fn().mockResolvedValue([]),
      listModels: vi.fn().mockResolvedValue([]),
    };
    await refreshAdapterLists("kiro-cli", FALLBACK, api);
    expect(api.listAgents).toHaveBeenCalledWith("kiro-cli");
    expect(api.listModels).toHaveBeenCalledWith("kiro-cli");
  });
});
