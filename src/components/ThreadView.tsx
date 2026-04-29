import { useState, useEffect, useRef } from "react";
import Markdown from "react-markdown";
import type { WorkspaceInfo, ThreadInfo, AgentEvent } from "../App";
import GateCard from "./GateCard";
import CompletionCard from "./CompletionCard";
import CostBadge from "./CostBadge";

interface ThreadViewProps {
  workspace: WorkspaceInfo;
  thread: ThreadInfo | null;
  onStartThread: (prompt: string) => void;
}

export default function ThreadView({ workspace, thread, onStartThread }: ThreadViewProps) {
  const [prompt, setPrompt] = useState("");
  const contentRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const isRunning = thread?.status === "starting" || thread?.status === "running";
  const canSend = !isRunning;
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
    if (!prompt.trim() || !canSend) return;
    onStartThread(prompt.trim());
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
  };

  const handleTextareaInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setPrompt(e.target.value);
    const el = e.target;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 120) + "px";
  };

  const runningCost = events
    .filter((e) => e.event_type === "cost_update")
    .reduce((_, e) => e.total_usd || 0, 0);

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

            {thread.status === "starting" && visibleEvents.length === 0 && (
              <div className="step-card">
                <span className="step-icon icon-thinking">
                  <span className="spinner" />
                </span>
                <span className="step-description thinking-text">Starting...</span>
              </div>
            )}

            {renderEvents(visibleEvents, runningCost)}

            {isRunning && visibleEvents.length > 0 && (
              <div className="step-card">
                <span className="step-icon icon-thinking">
                  <span className="spinner" />
                </span>
                <span className="step-description thinking-text" />
              </div>
            )}
          </>
        )}
      </div>

      <div className="prompt-bar">
        <div className="prompt-bar-inner">
          <textarea
            ref={textareaRef}
            placeholder={thread ? `Follow up...` : `Send a task to ${workspace.name}...`}
            value={prompt}
            onChange={handleTextareaInput}
            onKeyDown={handleKeyDown}
            rows={1}
            disabled={!canSend}
          />
          <button
            className="btn-send"
            onClick={handleSend}
            disabled={!canSend || !prompt.trim()}
            title="Send (Enter)"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}

const FILE_WRITE_TOOLS = new Set(["Write", "Edit", "NotebookEdit"]);

function renderEvents(events: AgentEvent[], runningCost: number) {
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
              onApprove={() => {}}
              onReject={() => {}}
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
            onCommit={() => {}}
            onRevert={() => {}}
            onKeep={() => {}}
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
