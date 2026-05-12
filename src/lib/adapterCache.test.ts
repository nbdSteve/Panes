import { describe, it, expect, vi } from "vitest";
import { AdapterCache, type AdapterCacheMap } from "./adapterCache";
import type { AgentInfo, ModelInfo } from "../types";

const FALLBACK: ModelInfo[] = [{ id: "fallback-1", label: "Fallback 1", description: "" }];

function makeHarness() {
  const deferred: { adapter: string; resolve: (v: { agents: AgentInfo[]; models: ModelInfo[] }) => void; reject: (e: unknown) => void }[] = [];
  const listAgents = vi.fn((adapter: string) =>
    new Promise<AgentInfo[]>((resolve, reject) => {
      deferred.push({
        adapter,
        resolve: (v) => resolve(v.agents),
        reject,
      });
    }),
  );
  const listModels = vi.fn((_adapter: string) => Promise.resolve<ModelInfo[]>([{ id: "m1", label: "M1", description: "" }]));

  let lists: AdapterCacheMap = new Map();
  const setLists = vi.fn((updater: (prev: AdapterCacheMap) => AdapterCacheMap) => {
    lists = updater(lists);
  });
  const onLoadingStart = vi.fn();
  const onLoadingEnd = vi.fn();

  const cache = new AdapterCache(
    { listAgents, listModels },
    { setLists, onLoadingStart, onLoadingEnd },
    FALLBACK,
  );

  return {
    cache,
    listAgents,
    listModels,
    setLists,
    onLoadingStart,
    onLoadingEnd,
    getLists: () => lists,
    resolveNext: (agents: AgentInfo[]) => {
      const d = deferred.shift();
      if (!d) throw new Error("no pending fetch to resolve");
      d.resolve({ agents, models: [] });
    },
  };
}

describe("AdapterCache", () => {
  it("fetches lists once and stores them in the map", async () => {
    const h = makeHarness();
    const p = h.cache.ensure("claude-code");
    expect(h.onLoadingStart).toHaveBeenCalledWith("claude-code");
    h.resolveNext([{ name: "planner", model: null, description: "" }]);
    await p;

    expect(h.listAgents).toHaveBeenCalledTimes(1);
    expect(h.setLists).toHaveBeenCalledTimes(1);
    expect(h.getLists().get("claude-code")?.agents).toEqual([
      { name: "planner", model: null, description: "" },
    ]);
    expect(h.onLoadingEnd).toHaveBeenCalledWith("claude-code");
  });

  it("does not re-fetch when the adapter is already cached", async () => {
    const h = makeHarness();
    const p1 = h.cache.ensure("claude-code");
    h.resolveNext([]);
    await p1;

    await h.cache.ensure("claude-code");
    expect(h.listAgents).toHaveBeenCalledTimes(1);
  });

  it("dedups concurrent fetches for the same adapter", async () => {
    // Scenario: derivedAdapter effect fires, then rapid workspace-switching
    // fires it again for the same adapter before the first settles. We
    // must not start a second fetch.
    const h = makeHarness();
    const p1 = h.cache.ensure("kiro-cli");
    const p2 = h.cache.ensure("kiro-cli");
    expect(h.listAgents).toHaveBeenCalledTimes(1);
    expect(h.cache.isInFlight("kiro-cli")).toBe(true);

    h.resolveNext([]);
    await Promise.all([p1, p2]);

    expect(h.listAgents).toHaveBeenCalledTimes(1);
    expect(h.cache.isInFlight("kiro-cli")).toBe(false);
    expect(h.cache.isCached("kiro-cli")).toBe(true);
  });

  it("fetches each adapter independently", async () => {
    const h = makeHarness();
    const p1 = h.cache.ensure("claude-code");
    const p2 = h.cache.ensure("kiro-cli");
    expect(h.listAgents).toHaveBeenCalledTimes(2);

    h.resolveNext([]); // claude-code
    h.resolveNext([{ name: "builder", model: null, description: "" }]); // kiro-cli
    await Promise.all([p1, p2]);

    expect(h.getLists().get("claude-code")?.agents).toEqual([]);
    expect(h.getLists().get("kiro-cli")?.agents).toEqual([
      { name: "builder", model: null, description: "" },
    ]);
  });

  it("ignores empty adapter names", async () => {
    const h = makeHarness();
    await h.cache.ensure("");
    expect(h.listAgents).not.toHaveBeenCalled();
    expect(h.onLoadingStart).not.toHaveBeenCalled();
  });

  it("calls onLoadingEnd even if refreshAdapterLists resolves with empty lists", async () => {
    // refreshAdapterLists never rejects (that's its whole point), so the
    // cache never sees a thrown error — but we still need the loading
    // indicator to clear. This guards the finally block.
    const h = makeHarness();
    const p = h.cache.ensure("claude-code");
    h.resolveNext([]);
    await p;
    expect(h.onLoadingEnd).toHaveBeenCalledWith("claude-code");
  });
});
