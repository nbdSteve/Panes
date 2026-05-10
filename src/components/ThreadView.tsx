import { useState, useEffect, useRef, useCallback } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";
import type { WorkspaceInfo, ThreadInfo, AgentEvent, AgentInfo, ConfigPrefs, ModelInfo } from "../App";
import type { ValidatorTypeInfo } from "../types";
import type { RepoFileStatus, CommentThread } from "../types/diff";
import GateCard from "./GateCard";
import ValidationResultCard from "./ValidationResultCard";
import CompletionCard from "./CompletionCard";
import DiffView from "./DiffView";
import CostBadge from "./CostBadge";
import { threadDisplayCost } from "../lib/cost";
import { calculateContextUsage } from "../lib/contextUsage";
import { normalizeModelId } from "../lib/utils";
import { parseDiff } from "../lib/diffParser";
import { groupToolEvents, type ToolGroup } from "../lib/groupToolEvents";
import { collectTestResults, collectFilesChanged, extractFilePaths, FILE_WRITE_TOOLS } from "../lib/threadHelpers";
import { buildCorrectionPrompt } from "../lib/validationPrompt";
import { classifyValidationResult } from "../lib/validationResultView";
import TranscriptView from "./TranscriptView";
import RoutineBadge from "./RoutineBadge";

interface ThreadViewProps {
  workspace: WorkspaceInfo;
  thread: ThreadInfo | null;
  adapters: string[];
  agents: AgentInfo[];
  models: ModelInfo[];
  validatorTypes?: ValidatorTypeInfo[];
  defaultConfig: ConfigPrefs;
  onConfigChange: (config: ConfigPrefs) => void;
  onStartThread: (prompt: string, adapter?: string, agent?: string, model?: string) => void;
  onCompletionAction: (threadId: string, action: "committed" | "reverted" | "kept") => void;
  onCancel: (threadId: string) => void;
  onQueueFollowUp: (threadId: string, prompt: string) => void;
  onResumeThread: (threadId: string, prompt: string) => void;
  onAddDiffComment: (threadId: string, completionIdx: number, comment: CommentThread) => void;
  onDiffViewChange: (threadId: string, view: { completionIdx: number; activeFile?: string } | null) => void;
  onMarkFeedbackSent: (threadId: string, completionIdx: number, commentCount: number) => void;
  onSetBudgetCap: (workspaceId: string, budgetCap: number | null) => void;
  showCost?: boolean;
}

export default function ThreadView({ workspace, thread, adapters, agents, models, validatorTypes, defaultConfig, onConfigChange, onStartThread, onCompletionAction, onCancel, onQueueFollowUp, onResumeThread, onAddDiffComment, onDiffViewChange, onMarkFeedbackSent, onSetBudgetCap, showCost }: ThreadViewProps) {
  const [prompt, setPrompt] = useState("");
  const [selectedAdapter, setSelectedAdapter] = useState(defaultConfig.adapter || adapters[0] || "");
  const [selectedAgent, setSelectedAgent] = useState(defaultConfig.agent);
  const [selectedModel, setSelectedModel] = useState(defaultConfig.model || "sonnet");
  const [adapterOpen, setAdapterOpen] = useState(false);
  const [agentOpen, setAgentOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const adapterRef = useRef<HTMLDivElement>(null);
  const agentRef = useRef<HTMLDivElement>(null);
  const modelRef = useRef<HTMLDivElement>(null);
  const [editingBudget, setEditingBudget] = useState(false);
  const [budgetValue, setBudgetValue] = useState("");
  const [commitError, setCommitError] = useState<string | null>(null);
  const [suggestedMessage, setSuggestedMessage] = useState<string>("");
  const [generatingMessage, setGeneratingMessage] = useState(false);
  const [revertConfirm, setRevertConfirm] = useState<string | null>(null);
  const [showTranscript, setShowTranscript] = useState(false);
  const [cardFiles, setCardFiles] = useState<Record<number, RepoFileStatus[]>>({});
  const [diffView, setDiffViewLocal] = useState<{ completionIdx: number; initialFile?: string } | null>(
    thread?.activeDiffView ? { completionIdx: thread.activeDiffView.completionIdx, initialFile: thread.activeDiffView.activeFile } : null
  );
  const [diffRaw, setDiffRaw] = useState<string | null>(null);

  const setDiffView = useCallback((view: { completionIdx: number; initialFile?: string } | null) => {
    setDiffViewLocal(view);
    if (thread) {
      onDiffViewChange(thread.id, view ? { completionIdx: view.completionIdx, activeFile: view.initialFile } : null);
    }
  }, [thread, onDiffViewChange]);
  const cardComments = thread?.diffComments ?? {};
  const cardFilesFetched = useRef<Set<number>>(new Set());
  const contentRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const isRunning = thread?.status === "starting" || thread?.status === "running";
  const isActive = isRunning || thread?.status === "gate";
  const events = thread?.events ?? [];

  useEffect(() => {
    const completeEvents = events.filter((e) => e.event_type === "complete");
    if (completeEvents.length === 0) {
      setCardFiles({});
      cardFilesFetched.current = new Set();
      return;
    }

    let segStart = 0;
    completeEvents.forEach((_, idx) => {
      const segEnd = events.indexOf(completeEvents[idx]);

      if (cardFilesFetched.current.has(idx)) {
        segStart = segEnd + 1;
        return;
      }

      const segmentEvents = events.slice(segStart, segEnd);
      const paths = extractFilePaths(segmentEvents);
      segStart = segEnd + 1;

      cardFilesFetched.current.add(idx);

      if (paths.length === 0) return;

      // Resolve relative paths against the active workspace before asking the
      // backend for git status — get_files_git_status treats non-absolute paths
      // as relative to the backend CWD, which would point outside the workspace.
      const resolved = paths.map((p) =>
        p.startsWith("/") ? p : `${workspace.path.replace(/\/$/, "")}/${p}`,
      );
      api.getFilesGitStatus(resolved).then((status) => {
        setCardFiles((prev) => ({ ...prev, [idx]: status }));
      }).catch(() => {});
    });
  }, [events]);

  useEffect(() => {
    if (diffView && diffRaw === null) {
      const files = (cardFiles[diffView.completionIdx] ?? []).flatMap((r) => r.files.map((f) => f.absolutePath));
      api.getWorkspaceDiff(workspace.path, files.length > 0 ? files : undefined)
        .then(setDiffRaw)
        .catch(() => setDiffRaw(""));
    }
  }, [diffView, diffRaw, cardFiles, workspace.path]);

  useEffect(() => {
    onConfigChange({ adapter: selectedAdapter, agent: selectedAgent, model: selectedModel });
  }, [selectedAdapter, selectedAgent, selectedModel, onConfigChange]);

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

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (adapterRef.current && !adapterRef.current.contains(e.target as Node)) setAdapterOpen(false);
      if (agentRef.current && !agentRef.current.contains(e.target as Node)) setAgentOpen(false);
      if (modelRef.current && !modelRef.current.contains(e.target as Node)) setModelOpen(false);
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handleSend = () => {
    if (!prompt.trim()) return;
    if (isActive && thread) {
      onQueueFollowUp(thread.id, prompt.trim());
    } else if (
      thread &&
      (thread.status === "interrupted" || thread.status === "error")
    ) {
      // Continue an interrupted/errored thread instead of starting a new one.
      // Session is preserved on the backend, so resume picks up where we left off.
      onResumeThread(thread.id, prompt.trim());
    } else {
      onStartThread(prompt.trim(), selectedAdapter, selectedAgent, selectedModel);
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
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  };

  const runningCost = thread ? threadDisplayCost(thread) : 0;
  const contextUsage = calculateContextUsage(events);

  const visibleEvents = events.filter(
    (e) => e.event_type !== "cost_update"
  );

  return (
    <div className="thread-view">
      <div className="thread-header">
        <div className="thread-header-left">
          <span className="thread-header-title">{workspace.name}</span>
          {thread?.isRoutine && <RoutineBadge />}
          {isRunning && (
            <span className="thread-header-status">
              <span className="dot" />
              Working
            </span>
          )}
        </div>
        <div className="thread-header-right">
          {thread && events.length > 0 && (
            <button
              className={`btn-icon transcript-toggle ${showTranscript ? "active" : ""}`}
              onClick={() => setShowTranscript(!showTranscript)}
              title={showTranscript ? "Timeline view" : "Transcript view"}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <line x1="17" y1="10" x2="3" y2="10" /><line x1="21" y1="6" x2="3" y2="6" /><line x1="21" y1="14" x2="3" y2="14" /><line x1="17" y1="18" x2="3" y2="18" />
              </svg>
            </button>
          )}
          {contextUsage && (
            <span className={`context-usage context-usage-${contextUsage.level}`} title={`${contextUsage.inputTokens.toLocaleString()} tokens`}>
              {Math.round(contextUsage.percentage)}%
            </span>
          )}
          {showCost !== false && runningCost > 0 && <CostBadge cost={runningCost} label="Cost" budgetCap={workspace.budgetCap ?? undefined} />}
        </div>
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

        {thread && showTranscript && (
          <TranscriptView events={visibleEvents} prompt={thread.prompt} showCost={showCost} />
        )}

        {thread && !showTranscript && (
          <>
            <div className="thread-prompt-display">
              <span className="thread-prompt-label">You</span>
              <span className="thread-prompt-text">{thread.prompt}</span>
            </div>

            {(thread.memoryCount != null && thread.memoryCount > 0 || thread.hasBriefing) && (
              <div className="context-indicator">
                Using {thread.memoryCount || 0} {thread.memoryCount === 1 ? "memory" : "memories"}
                {thread.hasBriefing && " · 1 briefing"}
              </div>
            )}

            {isRunning && visibleEvents.length === 0 && (
              <div className="step-card">
                <span className="step-icon icon-thinking">
                  <span className="spinner" />
                </span>
                <span className="step-description thinking-text">Starting...</span>
              </div>
            )}

            {renderEvents(visibleEvents, runningCost, thread.id, thread.completionActions, thread.feedbackSent, cardFiles, cardComments, validatorTypes ?? [], {
              onInspect: (completionIdx: number) => {
                setDiffView({ completionIdx });
                if (diffRaw === null || diffView?.completionIdx !== completionIdx) {
                  setDiffRaw(null);
                  const files = (cardFiles[completionIdx] ?? []).flatMap((r) => r.files.map((f) => f.absolutePath));
                  api.getWorkspaceDiff(workspace.path, files.length > 0 ? files : undefined)
                    .then(setDiffRaw)
                    .catch(() => setDiffRaw(""));
                }
              },
              onRevert: () => setRevertConfirm(thread.id),
              onKeep: () => onCompletionAction(thread.id, "kept"),
              onSteer: (tid, toolUseId, text) => {
                api.rejectGate(tid, toolUseId, `Steer: ${text}`).catch(console.error);
                onQueueFollowUp(tid, text);
              },
              onViewDiff: (filePath: string, completionIdx: number) => {
                setDiffView({ completionIdx, initialFile: filePath });
                if (diffRaw === null || diffView?.completionIdx !== completionIdx) {
                  setDiffRaw(null);
                  const files = (cardFiles[completionIdx] ?? []).flatMap((r) => r.files.map((f) => f.absolutePath));
                  api.getWorkspaceDiff(workspace.path, files.length > 0 ? files : undefined)
                    .then(setDiffRaw)
                    .catch(() => setDiffRaw(""));
                }
              },
              onSendFeedback: (completionIdx: number) => {
                const comments = cardComments[completionIdx] ?? [];
                if (comments.length === 0) return;
                const feedbackPrompt = formatCommentsAsPrompt(comments);
                onResumeThread(thread.id, feedbackPrompt);
                onMarkFeedbackSent(thread.id, completionIdx, comments.length);
              },
            }, showCost)}

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
                <span className="step-description">
                  Cancelled
                  <span className="interrupted-hint">
                    Type a message to continue this thread.
                  </span>
                </span>
              </div>
            )}
          </>
        )}
      </div>

      {diffView && diffRaw !== null && thread && (
        <DiffView
          diff={parseDiff(diffRaw)}
          selectedFile={diffView.initialFile}
          comments={cardComments[diffView.completionIdx] ?? []}
          onAddComment={(filePath, side, startLine, endLine, body) => {
            const comment: CommentThread = {
              id: crypto.randomUUID(),
              filePath,
              side,
              startLine,
              endLine,
              body,
              createdAt: new Date().toISOString(),
            };
            onAddDiffComment(thread.id, diffView.completionIdx, comment);
          }}
          onActiveFileChange={(filePath) => {
            if (thread) {
              onDiffViewChange(thread.id, { completionIdx: diffView.completionIdx, activeFile: filePath });
            }
          }}
          onClose={() => setDiffView(null)}
          repoFiles={!thread.completionActions?.[diffView.completionIdx] ? cardFiles[diffView.completionIdx] : undefined}
          commitError={commitError}
          suggestedMessage={suggestedMessage}
          generatingMessage={generatingMessage}
          onGenerateMessage={() => {
            setGeneratingMessage(true);
            api.generateCommitMessage(workspace.path, diffRaw)
              .then((msg) => { setSuggestedMessage(msg); setGeneratingMessage(false); })
              .catch(() => setGeneratingMessage(false));
          }}
          onCommit={!thread.completionActions?.[diffView.completionIdx] ? async (commits) => {
            try {
              setCommitError(null);
              await api.commitRepos(commits);
              onCompletionAction(thread.id, "committed");
              setDiffView(null);
            } catch (e) {
              const msg = e instanceof Error ? e.message : typeof e === "string" ? e : (e as { message?: string })?.message ?? JSON.stringify(e);
              setCommitError(msg);
            }
          } : undefined}
          onSendFeedback={(cardComments[diffView.completionIdx] ?? []).length > 0 ? () => {
            const feedbackComments = cardComments[diffView.completionIdx] ?? [];
            if (feedbackComments.length === 0) return;
            const feedbackPrompt = formatCommentsAsPrompt(feedbackComments);
            onResumeThread(thread.id, feedbackPrompt);
            onMarkFeedbackSent(thread.id, diffView.completionIdx, feedbackComments.length);
            setDiffView(null);
          } : undefined}
        />
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
                  await api.revertChanges(workspace.path, revertConfirm);
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

      {(() => {
        const agentModel = agents.find(a => a.name === selectedAgent)?.model;
        const modelLocked = !!agentModel;
        const effectiveModel = agentModel ? normalizeModelId(agentModel) : selectedModel;
        return (
          <div className="config-bar">
            <div className="config-dropdowns">
              {adapters.length > 0 && (
                <div className="config-dropdown" ref={adapterRef}>
                  <button
                    className="config-dropdown-trigger"
                    onClick={() => { setAdapterOpen(!adapterOpen); setAgentOpen(false); setModelOpen(false); }}
                    disabled={isRunning}
                    title={isRunning ? "Cannot change while thread is running" : undefined}
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <rect x="2" y="3" width="20" height="14" rx="2" /><line x1="8" y1="21" x2="16" y2="21" /><line x1="12" y1="17" x2="12" y2="21" />
                    </svg>
                    <span className="config-dropdown-value">{selectedAdapter || "Adapter"}</span>
                    <svg className="config-dropdown-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><polyline points="6 9 12 15 18 9" /></svg>
                  </button>
                  {adapterOpen && (
                    <div className="config-dropdown-menu">
                      {adapters.map(a => (
                        <button
                          key={a}
                          className={`config-dropdown-item ${a === selectedAdapter ? "active" : ""}`}
                          onClick={() => { setSelectedAdapter(a); setAdapterOpen(false); }}
                        >
                          <span className="config-dropdown-item-label">{a}</span>
                          {a === selectedAdapter && (
                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="20 6 9 17 4 12" /></svg>
                          )}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              )}

              <div className="config-dropdown" ref={agentRef}>
                <button
                  className="config-dropdown-trigger"
                  onClick={() => { setAgentOpen(!agentOpen); setAdapterOpen(false); setModelOpen(false); }}
                  disabled={isRunning}
                  title={isRunning ? "Cannot change while thread is running" : undefined}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M12 2a4 4 0 0 1 4 4v2a4 4 0 0 1-8 0V6a4 4 0 0 1 4-4z" />
                    <path d="M16 14H8a4 4 0 0 0-4 4v2h16v-2a4 4 0 0 0-4-4z" />
                  </svg>
                  <span className="config-dropdown-value">{selectedAgent || "Default"}</span>
                  <svg className="config-dropdown-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><polyline points="6 9 12 15 18 9" /></svg>
                </button>
                {agentOpen && (
                  <div className="config-dropdown-menu">
                    <button
                      className={`config-dropdown-item ${selectedAgent === "" ? "active" : ""}`}
                      onClick={() => { setSelectedAgent(""); setAgentOpen(false); }}
                    >
                      <span className="config-dropdown-item-content">
                        <span className="config-dropdown-item-label">Default</span>
                        <span className="config-dropdown-item-desc">No agent override</span>
                      </span>
                      {selectedAgent === "" && (
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="20 6 9 17 4 12" /></svg>
                      )}
                    </button>
                    {agents.map(a => (
                      <button
                        key={a.name}
                        className={`config-dropdown-item ${a.name === selectedAgent ? "active" : ""}`}
                        onClick={() => {
                          setSelectedAgent(a.name);
                          if (a.model) setSelectedModel(normalizeModelId(a.model));
                          setAgentOpen(false);
                        }}
                      >
                        <span className="config-dropdown-item-content">
                          <span className="config-dropdown-item-label">{a.name}</span>
                          {a.description && <span className="config-dropdown-item-desc">{a.description}</span>}
                        </span>
                        {a.model && <span className="config-dropdown-item-badge">{normalizeModelId(a.model)}</span>}
                        {a.name === selectedAgent && (
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="20 6 9 17 4 12" /></svg>
                        )}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <div className="config-dropdown" ref={modelRef}>
                <button
                  className={`config-dropdown-trigger ${modelLocked ? "locked" : ""}`}
                  onClick={() => { if (!modelLocked) { setModelOpen(!modelOpen); setAdapterOpen(false); setAgentOpen(false); } }}
                  disabled={isRunning || modelLocked}
                  title={modelLocked ? `Set by ${selectedAgent} agent` : isRunning ? "Cannot change while thread is running" : undefined}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M12 2L2 7l10 5 10-5-10-5z" /><path d="M2 17l10 5 10-5" /><path d="M2 12l10 5 10-5" />
                  </svg>
                  <span className="config-dropdown-value">{models.find(m => m.id === effectiveModel)?.label ?? effectiveModel}</span>
                  {modelLocked ? (
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" /></svg>
                  ) : (
                    <svg className="config-dropdown-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"><polyline points="6 9 12 15 18 9" /></svg>
                  )}
                </button>
                {modelOpen && !modelLocked && (
                  <div className="config-dropdown-menu">
                    {models.map(m => (
                      <button
                        key={m.id}
                        className={`config-dropdown-item ${m.id === selectedModel ? "active" : ""}`}
                        onClick={() => { setSelectedModel(m.id); setModelOpen(false); }}
                      >
                        <span className="config-dropdown-item-content">
                          <span className="config-dropdown-item-label">{m.label}</span>
                          <span className="config-dropdown-item-desc">{m.description}</span>
                        </span>
                        {m.id === selectedModel && (
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="20 6 9 17 4 12" /></svg>
                        )}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>

            {showCost !== false && <div className="config-budget">
              {editingBudget ? (
                <input
                  className="config-budget-input"
                  type="number"
                  step="0.01"
                  min="0"
                  placeholder="No limit"
                  value={budgetValue}
                  onChange={(e) => setBudgetValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const val = budgetValue.trim() ? parseFloat(budgetValue) : null;
                      onSetBudgetCap(workspace.id, val && val > 0 ? val : null);
                      setEditingBudget(false);
                    }
                    if (e.key === "Escape") {
                      setEditingBudget(false);
                    }
                  }}
                  onBlur={() => {
                    const val = budgetValue.trim() ? parseFloat(budgetValue) : null;
                    onSetBudgetCap(workspace.id, val && val > 0 ? val : null);
                    setEditingBudget(false);
                  }}
                  autoFocus
                />
              ) : (
                <button
                  className="config-dropdown-trigger"
                  onClick={() => {
                    setBudgetValue(workspace.budgetCap ? String(workspace.budgetCap) : "");
                    setEditingBudget(true);
                  }}
                  disabled={isRunning}
                  title={isRunning ? "Cannot change while thread is running" : undefined}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <line x1="12" y1="1" x2="12" y2="23" /><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" />
                  </svg>
                  <span className="config-dropdown-value">
                    {workspace.budgetCap ? `$${workspace.budgetCap.toFixed(2)}` : "No limit"}
                  </span>
                </button>
              )}
            </div>}
          </div>
        );
      })()}

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
              title={!prompt.trim() ? "Enter a message to send" : isActive ? "Queue follow-up (Enter)" : "Send (Enter)"}
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

function ToolGroupCard({ group, showCost }: { group: ToolGroup; showCost?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const { request, result } = group;
  const inProgress = result === null;
  const duration = result?.duration_ms;

  return (
    <div className={`tool-group ${expanded ? "expanded" : ""}`}>
      <button className="tool-group-header" onClick={() => setExpanded(!expanded)}>
        <span className="step-icon icon-tool">
          {inProgress ? (
            <span className="spinner" />
          ) : result?.success ? (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          )}
        </span>
        <span className="tool-group-label">
          <span className="tool-name">{request.tool_name}</span>
          <span className="tool-group-desc">{request.description}</span>
        </span>
        {duration != null && duration > 0 && (
          <span className="step-elapsed">
            {duration < 1000 ? `${duration}ms` : `${(duration / 1000).toFixed(1)}s`}
          </span>
        )}
        <span className={`tool-group-chevron ${expanded ? "open" : ""}`}>
          <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
            <polyline points="9 6 15 12 9 18" />
          </svg>
        </span>
      </button>
      {expanded && (
        <div className="tool-group-body">
          {(() => {
            const raw = request.input?.command as string | undefined;
            return raw ? (
              <pre className="tool-group-command">{raw}</pre>
            ) : request.description ? (
              <pre className="tool-group-command">{request.description}</pre>
            ) : null;
          })()}
          {result?.output && (
            <pre className="tool-group-output">{result.output}</pre>
          )}
          {result && !result.success && !result.output && (
            <span className="tool-group-failed">Failed</span>
          )}
          {inProgress && !group.subAgentSpawned && (
            <span className="tool-group-pending">Running...</span>
          )}
          {group.subAgentSpawned && (
            <div className="sub-agent-nested">
              <div className="sub-agent-nested-header">
                Sub-agent: {group.subAgentSpawned.description}
              </div>
              {group.subAgentComplete ? (
                <div className="sub-agent-nested-result">
                  <span>{group.subAgentComplete.summary}</span>
                  {showCost !== false && group.subAgentComplete.cost_usd != null && (
                    <span className="sub-agent-cost">${group.subAgentComplete.cost_usd.toFixed(4)}</span>
                  )}
                </div>
              ) : (
                <span className="tool-group-pending">Sub-agent running...</span>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}


interface RenderCallbacks {
  onInspect: (completionIdx: number) => void;
  onRevert: () => void;
  onKeep: () => void;
  onSteer: (threadId: string, toolUseId: string, text: string) => void;
  onViewDiff: (filePath: string, completionIdx: number) => void;
  onSendFeedback: (completionIdx: number) => void;
}

function renderEvents(
  events: AgentEvent[],
  runningCost: number,
  threadId: string,
  completionActions: Record<number, "committed" | "reverted" | "kept"> | undefined,
  feedbackSent: Record<number, number> | undefined,
  cardFiles: Record<number, RepoFileStatus[]>,
  cardComments: Record<number, CommentThread[]>,
  validatorTypes: ValidatorTypeInfo[],
  callbacks: RenderCallbacks,
  showCost?: boolean,
) {
  let segmentHasWrites = false;
  let segmentEvents: AgentEvent[] = [];
  const items = groupToolEvents(events);

  // Track which completion card index each "complete" event is
  let completionCounter = 0;
  const itemToCompletionIdx = new Map<number, number>();
  items.forEach((item, i) => {
    if (item.type === "standalone" && item.event.event_type === "complete") {
      itemToCompletionIdx.set(i, completionCounter++);
    }
  });
  // Build a skip-text set: text events where the next non-cost event is complete with same content
  const skipTextIndices = new Set<number>();
  for (let i = 0; i < events.length; i++) {
    const ev = events[i];
    if (ev.event_type === "text") {
      const next = events[i + 1];
      if (next?.event_type === "complete" && next.summary === ev.text) {
        skipTextIndices.add(i);
      }
    }
  }

  // Track original event index for skip-text lookup
  let origIdx = 0;
  const eventToOrigIdx = new Map<AgentEvent, number>();
  for (const e of events) {
    eventToOrigIdx.set(e, origIdx++);
  }

  return items.map((item, i) => {
    if (item.type === "tool_group") {
      const { request } = item;
      if (request.tool_name && FILE_WRITE_TOOLS.has(request.tool_name)) {
        segmentHasWrites = true;
      }
      segmentEvents.push(request);
      if (item.result) segmentEvents.push(item.result);
      return <ToolGroupCard key={`tg-${i}`} group={item} showCost={showCost} />;
    }

    const event = item.event;

    if (event.event_type === "follow_up") {
      segmentHasWrites = false;
      segmentEvents = [];
    }

    segmentEvents.push(event);

    if (
      event.event_type === "tool_request" &&
      event.tool_name &&
      FILE_WRITE_TOOLS.has(event.tool_name)
    ) {
      segmentHasWrites = true;
    }

    const oi = eventToOrigIdx.get(event);
    if (event.event_type === "text" && oi !== undefined && skipTextIndices.has(oi)) {
      return null;
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
              <Markdown
                remarkPlugins={[remarkGfm]}
                components={{
                  table: ({ children }) => (
                    <div className="table-wrap"><table>{children}</table></div>
                  ),
                }}
              >{event.text || ""}</Markdown>
            </div>
          </div>
        );

      case "tool_request":
        if (event.needs_approval) {
          const gateId = event.id || "";
          const afterGate = events.filter((e) => (eventToOrigIdx.get(e) ?? 0) > (oi ?? 0));
          const hasResult = afterGate.some((e) => e.event_type === "tool_result" && e.id === gateId);
          const wasSteered = !hasResult && afterGate.some((e) => e.event_type === "follow_up");
          const wasRejected = !hasResult && !wasSteered && afterGate.some(
            (e) => e.event_type === "complete" || e.event_type === "error",
          );
          const resolved = hasResult ? "approved" as const
            : wasSteered ? "steered" as const
            : wasRejected ? "rejected" as const
            : undefined;
          return (
            <GateCard
              key={i}
              description={event.description || ""}
              riskLevel={event.risk_level || "medium"}
              toolUseId={gateId}
              toolName={event.tool_name || ""}
              runningCost={runningCost}
              showCost={showCost}
              resolved={resolved}
              onApprove={() => {
                api.approveGate(threadId, gateId).catch(console.error);
              }}
              onReject={() => {
                api.rejectGate(threadId, gateId, "User rejected").catch(console.error);
              }}
              onSteer={(text) => callbacks.onSteer(threadId, gateId, text)}
            />
          );
        }
        return null;

      case "tool_result":
        return null;

      case "complete": {
        const hadWrites = segmentHasWrites;
        const completionIdx = itemToCompletionIdx.get(i)!;
        const repoStatus = cardFiles[completionIdx];
        const heuristicFiles = collectFilesChanged(segmentEvents);
        const filesChanged = repoStatus && repoStatus.length > 0
          ? repoStatus.flatMap((r) => r.files.map((f) => ({
              path: f.relativePath,
              absolutePath: f.absolutePath,
              action: f.status === "??" ? "untracked" as const : f.status.includes("D") ? "deleted" as const : f.status.includes("A") ? "created" as const : "modified" as const,
            })))
          : heuristicFiles;
        const testResults = collectTestResults(segmentEvents);
        segmentHasWrites = false;
        segmentEvents = [];
        const cardAction = completionActions?.[completionIdx];
        const hasChanges = hadWrites || (repoStatus != null && repoStatus.length > 0);
        const commentCount = (cardComments[completionIdx] ?? []).length;
        const feedbackSentCount = feedbackSent?.[completionIdx];
        return (
          <CompletionCard
            key={i}
            summary={event.summary || ""}
            totalCost={event.total_cost_usd || 0}
            showCost={showCost}
            durationMs={event.duration_ms || 0}
            turns={event.turns || 0}
            hasFileChanges={hasChanges}
            filesChanged={filesChanged}
            testResults={testResults}
            completionAction={cardAction}
            commentCount={commentCount}
            feedbackSentCount={feedbackSentCount}
            onInspect={() => callbacks.onInspect(completionIdx)}
            onRevert={callbacks.onRevert}
            onKeep={callbacks.onKeep}
            onFileClick={(filePath) => callbacks.onViewDiff(filePath, completionIdx)}
            onSendFeedback={commentCount > 0 ? () => callbacks.onSendFeedback(completionIdx) : undefined}
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

      case "error": {
        const msg = (event.message || "").toLowerCase();
        const isAuth = msg.includes("auth") || msg.includes("token") || msg.includes("expired") || msg.includes("unauthorized") || msg.includes("forbidden");
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
            {isAuth && (
              <div className="auth-guidance">
                <span className="auth-guidance-text">Run this command in your terminal to re-authenticate:</span>
                <code className="auth-guidance-cmd" onClick={() => navigator.clipboard?.writeText("claude auth")} title="Click to copy">
                  claude auth
                </code>
              </div>
            )}
          </div>
        );
      }

      case "sub_agent_spawned":
        return (
          <div key={i} className="sub-agent-section">
            <div className="sub-agent-header">
              <span className="step-icon icon-tool">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                  <path d="M12 2a4 4 0 0 1 4 4v2a4 4 0 0 1-8 0V6a4 4 0 0 1 4-4z" />
                  <path d="M16 14H8a4 4 0 0 0-4 4v2h16v-2a4 4 0 0 0-4-4z" />
                </svg>
              </span>
              <span className="sub-agent-desc">
                <span className="sub-agent-label">Sub-agent</span>
                {event.description}
              </span>
              <span className="spinner" />
            </div>
          </div>
        );

      case "sub_agent_complete":
        return (
          <div key={i} className="sub-agent-section sub-agent-done">
            <div className="sub-agent-header">
              <span className="step-icon icon-success">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              </span>
              <span className="sub-agent-desc">
                <span className="sub-agent-label">Sub-agent</span>
                {event.summary}
              </span>
              {showCost !== false && event.cost_usd != null && (
                <span className="sub-agent-cost">${event.cost_usd.toFixed(4)}</span>
              )}
            </div>
          </div>
        );

      case "validation_result": {
        const afterIdx = eventToOrigIdx.get(event) ?? 0;
        const laterEvents = events.filter(
          (e) => (eventToOrigIdx.get(e) ?? 0) > afterIdx,
        );
        const mode = classifyValidationResult(event, laterEvents, validatorTypes);
        if (mode.kind === "pass") {
          return (
            <ValidationResultCard
              key={i}
              validator={event.validator}
              outcome="pass"
              findings={event.findings}
              durationMs={event.duration_ms}
            />
          );
        }
        if (mode.kind === "failResolved" || mode.kind === "failSteered") {
          return (
            <ValidationResultCard
              key={i}
              validator={event.validator}
              outcome="fail"
              findings={event.findings}
              durationMs={event.duration_ms}
              resolution={mode.kind === "failSteered" ? "steered" : undefined}
            />
          );
        }
        const correctionPrompt = buildCorrectionPrompt(event.findings);
        return (
          <GateCard
            key={i}
            variant="validator"
            validatorName={event.validator}
            validationFindings={event.findings}
            description=""
            riskLevel="high"
            toolUseId=""
            toolName=""
            runningCost={runningCost}
            showCost={false}
            onApprove={() => {
              api.approveGate(threadId, "").catch(console.error);
            }}
            onReject={() => {
              api
                .rejectGate(threadId, "", "Validator findings rejected")
                .catch(console.error);
            }}
            onSteer={(text) => callbacks.onSteer(threadId, "", text)}
            onAutoFix={
              mode.correctable
                ? () => callbacks.onSteer(threadId, "", correctionPrompt)
                : undefined
            }
            steerPlaceholder={correctionPrompt}
          />
        );
      }

      default:
        return null;
    }
  });
}

function formatCommentsAsPrompt(comments: CommentThread[]): string {
  const grouped: Record<string, CommentThread[]> = {};
  for (const c of comments) {
    (grouped[c.filePath] ??= []).push(c);
  }
  let prompt = "## Code Review Feedback\n\n";
  for (const [file, threads] of Object.entries(grouped)) {
    for (const t of threads) {
      const lineRef = t.startLine === t.endLine ? `line ${t.startLine}` : `lines ${t.startLine}-${t.endLine}`;
      prompt += `### ${file} (${lineRef}, ${t.side} side)\n`;
      prompt += `${t.body}\n\n`;
    }
  }
  return prompt;
}
