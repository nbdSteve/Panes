import type { AgentInfo, ModelInfo } from "../types";
import { refreshAdapterLists } from "./adapterLists";

export interface AdapterListEntry {
  agents: AgentInfo[];
  models: ModelInfo[];
}

export type AdapterCacheMap = Map<string, AdapterListEntry>;

export interface AdapterCacheApi {
  listAgents: (adapter: string) => Promise<AgentInfo[]>;
  listModels: (adapter: string) => Promise<ModelInfo[]>;
}

export interface AdapterCacheSinks {
  /**
   * Update the adapter → lists map in React state. Receives a producer,
   * not a final value, so the caller can use the functional form of
   * setState and avoid stale-closure bugs when multiple adapters are
   * probed concurrently.
   */
  setLists: (updater: (prev: AdapterCacheMap) => AdapterCacheMap) => void;
  /** Called when an adapter's fetch starts. */
  onLoadingStart: (adapter: string) => void;
  /** Called when an adapter's fetch settles (success or error). */
  onLoadingEnd: (adapter: string) => void;
}

/**
 * Stateful per-adapter cache for agent/model lists.
 *
 * Two problems this solves:
 *
 * 1. Before, `agents` and `models` were single globals — switching
 *    workspaces or changing the Settings default left the previous
 *    adapter's lists visible until the user clicked the dropdown.
 *
 * 2. Without deduplication, the derivedAdapter effect could fire a second
 *    fetch before the first settled (e.g. rapid workspace-switching),
 *    producing wasted work and racey state writes.
 *
 * The class is framework-agnostic so we can unit-test caching and dedup
 * behavior without React. Pass it React setters via `sinks`.
 */
export class AdapterCache {
  private readonly cached = new Set<string>();
  private readonly inFlight = new Set<string>();

  constructor(
    private readonly api: AdapterCacheApi,
    private readonly sinks: AdapterCacheSinks,
    private readonly fallbackModels: ModelInfo[],
  ) {}

  /**
   * Fetch agent/model lists for `adapter` into the cache. Idempotent:
   * returns immediately if the adapter is already cached or a fetch is
   * already in flight. Swallows errors (the underlying refresh helper
   * never rejects).
   */
  async ensure(adapter: string): Promise<void> {
    if (!adapter) return;
    if (this.cached.has(adapter)) return;
    if (this.inFlight.has(adapter)) return;

    this.inFlight.add(adapter);
    this.sinks.onLoadingStart(adapter);
    try {
      const result = await refreshAdapterLists(adapter, this.fallbackModels, this.api);
      this.cached.add(adapter);
      this.sinks.setLists((prev) => {
        const next = new Map(prev);
        next.set(adapter, result);
        return next;
      });
    } finally {
      this.inFlight.delete(adapter);
      this.sinks.onLoadingEnd(adapter);
    }
  }

  /** Test hook: exposed for assertions, not for production use. */
  isCached(adapter: string): boolean {
    return this.cached.has(adapter);
  }

  /** Test hook: exposed for assertions, not for production use. */
  isInFlight(adapter: string): boolean {
    return this.inFlight.has(adapter);
  }
}
