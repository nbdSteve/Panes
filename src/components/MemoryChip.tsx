import { useState } from "react";
import type { MemoryInfo } from "../lib/api";

interface MemoryChipProps {
  variant: "injected" | "extracted";
  memories: MemoryInfo[];
  briefing?: string | null;
  error?: string;
  /**
   * Navigate to the Memory panel. If a memoryId is passed, the panel will
   * scroll to and briefly highlight that memory; otherwise it opens at the
   * top.
   */
  onViewMemories?: (memoryId?: string) => void;
}

export default function MemoryChip({ variant, memories, briefing, error, onViewMemories }: MemoryChipProps) {
  const [expanded, setExpanded] = useState(false);

  const hasBriefing = variant === "injected" && !!briefing;
  const memCount = memories.length;
  const isError = variant === "extracted" && !!error;

  if (variant === "injected" && memCount === 0 && !hasBriefing) {
    return null;
  }

  const label = isError
    ? "memory extraction failed"
    : variant === "injected"
      ? `${memCount} ${memCount === 1 ? "memory" : "memories"} injected${hasBriefing ? " · briefing loaded" : ""}`
      : `${memCount} ${memCount === 1 ? "memory" : "memories"} written`;

  const toggle = () => setExpanded((v) => !v);

  return (
    <div className={`memory-chip memory-chip-${variant}${expanded ? " memory-chip-open" : ""}${isError ? " memory-chip-error" : ""}`}>
      <button
        type="button"
        className="memory-chip-summary"
        onClick={toggle}
        aria-expanded={expanded}
      >
        <svg
          className="memory-chip-caret"
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          style={{ transform: expanded ? "rotate(90deg)" : "rotate(0deg)" }}
        >
          <polyline points="9 18 15 12 9 6" />
        </svg>
        <svg
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          {variant === "injected" ? (
            <>
              <path d="M12 2v20" />
              <path d="M5 9l7-7 7 7" />
            </>
          ) : (
            <>
              <path d="M12 22V2" />
              <path d="M5 15l7 7 7-7" />
            </>
          )}
        </svg>
        <span className="memory-chip-label">{label}</span>
      </button>

      {expanded && (
        <div className="memory-chip-body">
          {isError && (
            <div className="memory-chip-empty memory-chip-error-message">
              Couldn't reach the memory store. {error}
            </div>
          )}
          {hasBriefing && (
            <div className="memory-chip-briefing">
              <span className="memory-chip-item-type">briefing</span>
              <div className="memory-chip-item-content">{briefing}</div>
            </div>
          )}
          {!isError && memCount === 0 && variant === "extracted" && (
            <div className="memory-chip-empty">Nothing new to remember from this thread.</div>
          )}
          {memories.map((m) => (
            <div key={m.id} className="memory-chip-item">
              <span className="memory-chip-item-type">{m.memoryType}</span>
              <div className="memory-chip-item-content">{m.content}</div>
              {onViewMemories && (
                <button
                  type="button"
                  className="memory-chip-item-link"
                  onClick={() => onViewMemories(m.id)}
                  aria-label={`Open this memory in the Memories panel`}
                >
                  Open in Memories →
                </button>
              )}
            </div>
          ))}
          {onViewMemories && memCount > 0 && (
            <button
              type="button"
              className="memory-chip-link"
              onClick={() => onViewMemories()}
            >
              Manage all memories →
            </button>
          )}
        </div>
      )}
    </div>
  );
}
