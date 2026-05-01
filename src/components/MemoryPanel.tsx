import { useState, useEffect, useCallback } from "react";
import { api, type MemoryInfo, type BriefingInfo } from "../lib/api";

interface MemoryPanelProps {
  workspaceId: string;
}

export default function MemoryPanel({ workspaceId }: MemoryPanelProps) {
  const [memories, setMemories] = useState<MemoryInfo[]>([]);
  const [briefing, setBriefing] = useState<BriefingInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editingBriefing, setEditingBriefing] = useState(false);
  const [briefingDraft, setBriefingDraft] = useState("");
  const [editingMemory, setEditingMemory] = useState<string | null>(null);
  const [memoryDraft, setMemoryDraft] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const loadMemories = useCallback(async () => {
    try {
      const mems = await api.getMemories(workspaceId);
      setMemories(mems);
    } catch {
      setError("Failed to load memories");
    }
  }, [workspaceId]);

  const loadBriefing = useCallback(async () => {
    try {
      const b = await api.getBriefing(workspaceId);
      setBriefing(b);
    } catch {
      setError("Failed to load briefing");
    }
  }, [workspaceId]);

  useEffect(() => {
    setLoading(true);
    setError(null);
    Promise.all([loadMemories(), loadBriefing()]).finally(() => setLoading(false));
  }, [loadMemories, loadBriefing]);

  useEffect(() => {
    if (!confirmDeleteId) return;
    const timer = setTimeout(() => setConfirmDeleteId(null), 3000);
    return () => clearTimeout(timer);
  }, [confirmDeleteId]);

  const handleSaveBriefing = async () => {
    const trimmed = briefingDraft.trim();
    if (trimmed) {
      await api.setBriefing(workspaceId, trimmed);
    } else {
      await api.deleteBriefing(workspaceId);
    }
    setEditingBriefing(false);
    loadBriefing();
  };

  const handlePin = async (id: string, pinned: boolean) => {
    await api.pinMemory(id, pinned);
    loadMemories();
  };

  const handleDelete = (id: string) => {
    if (confirmDeleteId === id) {
      setConfirmDeleteId(null);
      api.deleteMemory(id).then(() => loadMemories());
    } else {
      setConfirmDeleteId(id);
    }
  };

  const handleSaveMemory = async (id: string) => {
    await api.updateMemory(id, memoryDraft);
    setEditingMemory(null);
    loadMemories();
  };

  const pinned = memories.filter((m) => m.pinned);
  const unpinned = memories.filter((m) => !m.pinned);

  if (loading) return <div className="panel-loading"><span className="spinner" /></div>;

  if (error) return <div className="inline-error"><span className="inline-error-icon">!</span>{error}</div>;

  return (
    <div className="memory-panel">
      <div className="memory-section">
        <div className="memory-section-header">
          <h3>Briefing</h3>
          {!editingBriefing && (
            <button
              className="btn btn-sm btn-secondary"
              onClick={() => {
                setBriefingDraft(briefing?.content ?? "");
                setEditingBriefing(true);
              }}
            >
              {briefing ? "Edit" : "Add"}
            </button>
          )}
        </div>

        {editingBriefing ? (
          <div className="briefing-editor">
            <textarea
              className="input briefing-textarea"
              value={briefingDraft}
              onChange={(e) => setBriefingDraft(e.target.value)}
              placeholder="Instructions for every thread in this workspace..."
              rows={4}
              autoFocus
            />
            <div className="briefing-actions">
              <button className="btn btn-sm btn-primary" onClick={handleSaveBriefing}>
                Save
              </button>
              <button
                className="btn btn-sm btn-secondary"
                onClick={() => setEditingBriefing(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : briefing ? (
          <div className="briefing-content">{briefing.content}</div>
        ) : (
          <div className="briefing-empty">No briefing set for this workspace.</div>
        )}
      </div>

      <div className="memory-section">
        <div className="memory-section-header">
          <h3>Memories</h3>
          <span className="memory-count">{memories.length}</span>
        </div>

        {memories.length === 0 && (
          <div className="memory-empty">
            No memories yet. Complete a thread to start building context.
          </div>
        )}

        {pinned.length > 0 && (
          <div className="memory-group">
            <div className="memory-group-label">Pinned</div>
            {pinned.map((m) => (
              <MemoryCard
                key={m.id}
                memory={m}
                editing={editingMemory === m.id}
                confirming={confirmDeleteId === m.id}
                draft={memoryDraft}
                onEdit={() => {
                  setEditingMemory(m.id);
                  setMemoryDraft(m.content);
                }}
                onSave={() => handleSaveMemory(m.id)}
                onCancel={() => setEditingMemory(null)}
                onDraftChange={setMemoryDraft}
                onPin={() => handlePin(m.id, false)}
                onDelete={() => handleDelete(m.id)}
              />
            ))}
          </div>
        )}

        {unpinned.length > 0 && (
          <div className="memory-group">
            {pinned.length > 0 && (
              <div className="memory-group-label">Other</div>
            )}
            {unpinned.map((m) => (
              <MemoryCard
                key={m.id}
                memory={m}
                editing={editingMemory === m.id}
                confirming={confirmDeleteId === m.id}
                draft={memoryDraft}
                onEdit={() => {
                  setEditingMemory(m.id);
                  setMemoryDraft(m.content);
                }}
                onSave={() => handleSaveMemory(m.id)}
                onCancel={() => setEditingMemory(null)}
                onDraftChange={setMemoryDraft}
                onPin={() => handlePin(m.id, true)}
                onDelete={() => handleDelete(m.id)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function MemoryCard({
  memory,
  editing,
  confirming,
  draft,
  onEdit,
  onSave,
  onCancel,
  onDraftChange,
  onPin,
  onDelete,
}: {
  memory: MemoryInfo;
  editing: boolean;
  confirming: boolean;
  draft: string;
  onEdit: () => void;
  onSave: () => void;
  onCancel: () => void;
  onDraftChange: (v: string) => void;
  onPin: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`memory-card ${memory.pinned ? "pinned" : ""}`}>
      {editing ? (
        <div className="memory-edit">
          <textarea
            className="input memory-textarea"
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            rows={2}
            autoFocus
          />
          <div className="memory-edit-actions">
            <button className="btn btn-sm btn-primary" onClick={onSave}>
              Save
            </button>
            <button className="btn btn-sm btn-secondary" onClick={onCancel}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="memory-content">{memory.content}</div>
          <div className="memory-meta">
            <span className={`memory-type type-${memory.memoryType}`}>
              {memory.memoryType}
            </span>
            <div className="memory-actions">
              <button className="btn-icon" onClick={onPin} title={memory.pinned ? "Unpin" : "Pin"}>
                {memory.pinned ? (
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                    <path d="M16 2l5 5-3.2 3.2 1.2 5.2-2 2L12 12.4 7 17.4V20H4v-3l5-5-5-5 2-2 5.2 1.2L16 2z" />
                  </svg>
                ) : (
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M16 2l5 5-3.2 3.2 1.2 5.2-2 2L12 12.4 7 17.4V20H4v-3l5-5-5-5 2-2 5.2 1.2L16 2z" />
                  </svg>
                )}
              </button>
              <button className="btn-icon" onClick={onEdit} title="Edit">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                  <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
                </svg>
              </button>
              {confirming ? (
                <button className="btn btn-sm btn-danger" onClick={onDelete}>Confirm?</button>
              ) : (
                <button className="btn-icon btn-danger" onClick={onDelete} title="Delete">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <polyline points="3 6 5 6 21 6" />
                    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                  </svg>
                </button>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
