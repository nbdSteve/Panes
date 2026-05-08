import type { ThreadInfo, WorkspaceInfo } from "../types";

export interface PendingResume {
  threadId: string;
  prompt: string;
}

export interface PendingResumeTarget {
  workspace: WorkspaceInfo;
  threadId: string;
  prompt: string;
}

/**
 * Decide whether a queued follow-up is ready to be resumed. Returns the
 * workspace + prompt when the thread is in a terminal state (we should not
 * fire into a still-running thread, even if the ref happens to be populated),
 * and the workspace is known. Returns null in all other cases.
 */
export function shouldFirePendingResume(
  pending: PendingResume | null,
  threads: ThreadInfo[],
  workspaces: WorkspaceInfo[],
): PendingResumeTarget | null {
  if (!pending) return null;
  const thread = threads.find((t) => t.id === pending.threadId);
  if (!thread) return null;
  if (
    thread.status !== "error" &&
    thread.status !== "interrupted" &&
    thread.status !== "complete"
  ) {
    return null;
  }
  const workspace = workspaces.find((w) => w.id === thread.workspaceId);
  if (!workspace) return null;
  return { workspace, threadId: pending.threadId, prompt: pending.prompt };
}
