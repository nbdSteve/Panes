import type { AgentEvent } from "../types";

/**
 * Returns true if a newly-arrived `complete` event should trigger a
 * fresh memory extraction for a thread.
 *
 * The dedupe key is the number of `complete` events observed for the
 * thread — each new complete (including follow-up runs) deserves its
 * own extraction whose result overwrites the previous one. Repeats of
 * an already-counted complete are rejected.
 *
 * Persisted baseline: when a thread is loaded from SQLite it already
 * carries completes in `events` AND may carry an `extractedMemories`
 * row from a prior extraction. We treat those as already-extracted so
 * re-opening a completed thread does not trigger a redundant call.
 *
 * @param priorEvents  Events already on the thread (excluding the new one).
 * @param priorExtractedCount  How many completes *this session* have been
 *   routed to extraction for this thread (from a session-scoped ref map,
 *   0 if the thread is new/never-extracted this session).
 * @param hasPersistedExtraction  True when the thread row carries a
 *   non-null `extractedMemories` array from the backend.
 */
export function shouldExtractOnComplete(
  priorEvents: AgentEvent[],
  priorExtractedCount: number,
  hasPersistedExtraction: boolean,
): { extract: boolean; newExtractedCount: number } {
  const completesInStream =
    priorEvents.filter((e) => e.event_type === "complete").length + 1;
  const persistedBaseline =
    priorExtractedCount === 0 && hasPersistedExtraction
      ? completesInStream - 1
      : 0;
  const effective = Math.max(priorExtractedCount, persistedBaseline);
  if (completesInStream > effective) {
    return { extract: true, newExtractedCount: completesInStream };
  }
  return { extract: false, newExtractedCount: priorExtractedCount };
}
