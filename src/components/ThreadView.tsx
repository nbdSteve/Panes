import { useState, useEffect, useRef } from "react";
import Markdown from "react-markdown";
import { invoke } from "@tauri-apps/api/core";
import type { WorkspaceInfo, ThreadInfo, AgentEvent } from "../App";
import GateCard from "./GateCard";
import CompletionCard from "./CompletionCard";
import CostBadge from "./CostBadge";

interface ThreadViewProps {
  workspace: WorkspaceInfo;
  thread: ThreadInfo | null;
  onStartThread: (prompt: string) => void;
  onCompletionAction: (threadId: string, action: "committed" | "reverted" | "kept") => void;
  onCancel: (threadId: string) => void;
  onQueueFollowUp: (threadId: string, prompt: string) => void;
}

export default function ThreadView({ workspace, thread, onStartThread, onCompletionAction, onCancel, onQueueFollowUp }: ThreadViewProps) {
  const [prompt, setPrompt] = useState("");
  const [commitDialog, setCommitDialog] = useState<{ threadId: string; summary: string } | null>(null);
  const [commitMessage, setCommitMessage] = useState("");
  const [revertConfirm, setRevertConfirm] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const isRunning = thread?.status === "starting" || thread?.status === "running";
  const isActive = isRunning || thread?.status === "gate";
  const events = thread?.events ?? [];

  useEffect(() => {
    if (contentRef.current) {
      contentRef.current.scrollTop = contentRef.current.scrollHeight;
    }
  }, [events.length]);

  useEffect(() => {
    if (!thread && textareaRef.current) {
      textareaRef.current.focus();
    }
  }, [thread]);

  const handleSend = () => {
    if (!prompt.trim()) return;
    if (isActive && thread) {
      onQueueFollowUp(thread.id, prompt.trim());
    } else {
      onStartThread(prompt.trim());
    }
    setPrompt("");
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
    if (e.key === "Escape" && isActive && thread) {
      e.preventDefault();
      onCancel(thread.id);
    }
  };

  const handleTextareaInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setPrompt(e.target.value);
    const el = e.target;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 120) + "px";
  };

  const runningCost = (() => {
    let total = 0;
    let latestCostInTurn = 0;
    for (const e of events) {
      if (e.event_type === "follow_up" || e.event_type === "complete") {
        total += latestCostInTurn;
        latestCostInTurn = 0;
      }
      if (e.event_type === "cost_update") {
        latestCostInTurn = e.total_usd || 0;
      }
    }
    return total + latestCostInTurn;
  })();

  const visibleEvents = events.filter(
    (e) => e.event_type !== "cost_update"
  );

  return (
    <div className="thread-view">
      <div className="thread-header">
        <div className="thread-header-left">
          <span className="thread-header-title">{workspace.name}</span>
          {isRunning && (
            <span className="thread-header-status">
              <span className="dot" />
              Working
            </span>
          )}
        </div>
        {runningCost > 0 && <CostBadge cost={runningCost} label="Cost" />}
      </div>

      <div className="thread-content" ref={contentRef}>
        {!thread && (
          <div className="thread-empty">
            <div className="thread-empty-icon">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
              </svg>
            </div>
            <p>Send a task to get started</p>
            <span className="thread-empty-hint">Enter to send, Shift+Enter for new line</span>
          </div>
        )}

        {thread && (
          <>
            <div className="thread-prompt-display">
              <span className="thread-prompt-label">You</span>
              <span className="thread-prompt-text">{thread.prompt}</span>
            </div>

            {isRunning && visibleEvents.length === 0 && (
              <div className="step-card">
                <span className="step-icon icon-thinking">
                  <span className="spinner" />
                </span>
                <span className="step-description thinking-text">Starting...</span>
              </div>
            )}

            {renderEvents(visibleEvents, runningCost, thread.id, thread.completionAction, {
              onCommit: (summary: string) => {
                setCommitDialog({ threadId: thread.id, summary });
                setCommitMessage(summary);
              },
              onRevert: () => setRevertConfirm(thread.id),
              onKeep: () => onCompletionAction(thread.id, "kept"),
            })}

            {isRunning && visibleEvents.length > 0 && (
              <div className="step-card">
                <span className="step-icon icon-thinking">
                  <span className="spinner" />
                </span>
                <span className="step-description thinking-text" />
              </div>
            )}

            {thread.status === "interrupted" && (
              <div className="step-card interrupted-card">
                <span className="step-icon icon-interrupted">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                    <rect x="6" y="6" width="12" height="12" rx="2" />
                  </svg>
                </span>
                <span className="step-description">Cancelled</span>
              </div>
            )}
          </>
        )}
      </div>

      {commitDialog && (
        <div className="commit-dialog">
          <div className="commit-dialog-title">Commit changes</div>
          <textarea
            value={commitMessage}
            onChange={(e) => setCommitMessage(e.target.value)}
            rows={3}
          />
          <div className="commit-dialog-actions">
            <button
              className="btn btn-success btn-sm"
              onClick={async () => {
                try {
                  await invoke("commit_changes", {
                    workspacePath: workspace.path,
                    message: commitMessage,
                  });
                  onCompletionAction(commitDialog.threadId, "committed");
                } catch (e) {
                  console.error("Commit failed:", e);
                }
                setCommitDialog(null);
              }}
            >
              Confirm
            </button>
            <button className="btn btn-secondary btn-sm" onClick={() => setCommitDialog(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {revertConfirm && (
        <div className="revert-confirm">
          <div className="revert-confirm-title">Undo all changes</div>
          <p>This will revert all file changes made by the agent.</p>
          <div className="revert-confirm-actions">
            <button
              className="btn btn-danger btn-sm"
              onClick={async () => {
                try {
                  await invoke("revert_changes", { workspacePath: workspace.path });
                  onCompletionAction(revertConfirm, "reverted");
                } catch (e) {
                  console.error("Revert failed:", e);
                }
                setRevertConfirm(null);
              }}
            >
              Revert
            </button>
            <button className="btn btn-secondary btn-sm" onClick={() => setRevertConfirm(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      <div className="prompt-bar">
        {thread?.queuedFollowUp && (
          <div className="queued-follow-up">
            <span className="queued-follow-up-label">Queued</span>
            <span className="queued-follow-up-text">{thread.queuedFollowUp}</span>
            <button
              className="queued-follow-up-cancel"
              onClick={() => onQueueFollowUp(thread.id, "")}
              title="Cancel queued message"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
                <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
        )}
        <div className="prompt-bar-inner">
          <textarea
            ref={textareaRef}
            placeholder={isActive ? "Queue a follow-up..." : thread ? "Follow up..." : `Send a task to ${workspace.name}...`}
            value={prompt}
            onChange={handleTextareaInput}
            onKeyDown={handleKeyDown}
            rows={1}
          />
          {isActive && !prompt.trim() ? (
            <button
              className="btn-stop"
              onClick={() => thread && onCancel(thread.id)}
              title="Stop (Esc)"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <rect x="6" y="6" width="12" height="12" rx="2" />
              </svg>
            </button>
          ) : (
            <button
              className="btn-send"
              onClick={handleSend}
              disabled={!prompt.trim()}
              title={isActive ? "Queue follow-up (Enter)" : "Send (Enter)"}
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

const FILE_WRITE_TOOLS = new Set(["Write", "Edit", "NotebookEdit"]);

interface CompletionCallbacks {
  onCommit: (summary: string) => void;
  onRevert: () => void;
  onKeep: () => void;
}

function renderEvents(
  events: AgentEvent[],
  runningCost: number,
  threadId: string,
  completionAction: "committed" | "reverted" | "kept" | undefined,
  callbacks: CompletionCallbacks,
) {
  let segmentHasWrites = false;

  return events.map((event, i) => {
    if (event.event_type === "follow_up") {
      segmentHasWrites = false;
    }

    if (
      event.event_type === "tool_request" &&
      event.tool_name &&
      FILE_WRITE_TOOLS.has(event.tool_name)
    ) {
      segmentHasWrites = true;
    }

    switch (event.event_type) {
      case "thinking":
        return (
          <div key={i} className="step-card">
            <span className="step-icon icon-thinking">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="12" r="1" />
              </svg>
            </span>
            <span className="step-description thinking-text">
              {event.text && event.text.length > 120
                ? event.text.substring(0, 120) + "..."
                : event.text || "Thinking..."}
            </span>
          </div>
        );

      case "text":
        return (
          <div key={i} className="step-card">
            <span className="step-icon icon-text">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
              </svg>
            </span>
            <div className="step-description text-content markdown-body">
              <Markdown>{event.text || ""}</Markdown>
            </div>
          </div>
        );

      case "tool_request":
        if (event.needs_approval) {
          return (
            <GateCard
              key={i}
              description={event.description || ""}
              riskLevel={event.risk_level || "medium"}
              toolUseId={event.id || ""}
              toolName={event.tool_name || ""}
              runningCost={runningCost}
              onApprove={() => {
                invoke("approve_gate", { threadId, toolUseId: event.id || "" }).catch(console.error);
              }}
              onReject={() => {
                invoke("reject_gate", { threadId, toolUseId: event.id || "", reason: "User rejected" }).catch(console.error);
              }}
              onSteer={() => {}}
            />
          );
        }
        return (
          <div key={i} className="step-card">
            <span className="step-icon icon-tool">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
              </svg>
            </span>
            <span className="step-description">
              <span className="tool-name">{event.tool_name}</span>
              <div className="tool-detail">{event.description}</div>
            </span>
          </div>
        );

      case "tool_result":
        return (
          <div key={i} className="step-card">
            <span className={`step-icon ${event.success ? "icon-success" : "icon-error"}`}>
              {event.success ? (
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
                  <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              )}
            </span>
            <span className="step-description">
              {event.success ? "Done" : "Failed"}
              {event.output && (
                <div className="result-preview">
                  {event.output.substring(0, 200)}
                </div>
              )}
            </span>
          </div>
        );

      case "complete": {
        const hadWrites = segmentHasWrites;
        segmentHasWrites = false;
        return (
          <CompletionCard
            key={i}
            summary={event.summary || ""}
            totalCost={event.total_cost_usd || 0}
            durationMs={event.duration_ms || 0}
            turns={event.turns || 0}
            hasFileChanges={hadWrites}
            completionAction={completionAction}
            onCommit={() => callbacks.onCommit(event.summary || "")}
            onRevert={callbacks.onRevert}
            onKeep={callbacks.onKeep}
          />
        );
      }

      case "follow_up":
        return (
          <div key={i} className="thread-prompt-display follow-up">
            <span className="thread-prompt-label">You</span>
            <span className="thread-prompt-text">{event.text}</span>
          </div>
        );

      case "error":
        return (
          <div key={i} className="card error-card">
            <div className="error-label">
              <span className="error-label-icon">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                  <circle cx="12" cy="12" r="10" /><line x1="15" y1="9" x2="9" y2="15" /><line x1="9" y1="9" x2="15" y2="15" />
                </svg>
              </span>
              <span className="error-label-text">Error</span>
            </div>
            <div className="error-message">{event.message}</div>
          </div>
        );

      default:
        return null;
    }
  });
}
