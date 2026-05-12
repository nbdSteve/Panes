import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AgentEvent } from "../types";
import type { MemoryInfo } from "../lib/api";
import { formatCost } from "../lib/utils";
import MemoryChip from "./MemoryChip";

interface TranscriptViewProps {
  events: AgentEvent[];
  prompt: string;
  showCost?: boolean;
  /**
   * Current thread lifecycle state. The extracted-memory chip is gated on
   * this so it stays hidden during in-flight follow-up turns (status would
   * be "running" / "gate") even though prior `complete` events remain in
   * the stream. Defaults to treating the stream as quiescent when omitted
   * — legacy callers (tests) that don't pass status still work.
   */
  status?: "starting" | "running" | "gate" | "complete" | "error" | "interrupted";
  injectedMemories?: MemoryInfo[];
  injectedBriefing?: string | null;
  extractedMemories?: MemoryInfo[];
  extractedMemoriesError?: string;
  onViewMemories?: (memoryId?: string) => void;
}

export default function TranscriptView({
  events,
  prompt,
  showCost,
  status,
  injectedMemories,
  injectedBriefing,
  extractedMemories,
  extractedMemoriesError,
  onViewMemories,
}: TranscriptViewProps) {
  const hasCompleted = events.some((e) => e.event_type === "complete");
  // Hide the extracted chip while a follow-up is mid-flight. Its content
  // reflects the previous run's extraction and would mislead the user into
  // thinking memory has already been written for the in-flight turn.
  const isQuiescent = status === undefined || status === "complete" || status === "error" || status === "interrupted";
  const shouldShowExtracted = hasCompleted && isQuiescent && (extractedMemories !== undefined || extractedMemoriesError !== undefined);

  return (
    <div className="transcript-view">
      <div className="transcript-message transcript-user">
        <span className="transcript-role">You</span>
        <div className="transcript-body">{prompt}</div>
      </div>

      {(injectedMemories && injectedMemories.length > 0) || injectedBriefing ? (
        <MemoryChip
          variant="injected"
          memories={injectedMemories ?? []}
          briefing={injectedBriefing}
          onViewMemories={onViewMemories}
        />
      ) : null}

      {events.map((event, i) => {
        switch (event.event_type) {
          case "thinking":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-assistant transcript-thinking">
                <span className="transcript-role">Thinking</span>
                <div className="transcript-body">{event.text}</div>
              </div>
            );

          case "text":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-assistant">
                <span className="transcript-role">Assistant</span>
                <div className="transcript-body markdown-body">
                  <Markdown remarkPlugins={[remarkGfm]}>{event.text || ""}</Markdown>
                </div>
              </div>
            );

          case "tool_request":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-system">
                <span className="transcript-role">Tool call: {event.tool_name}</span>
                <div className="transcript-body">
                  <code>{event.description}</code>
                </div>
              </div>
            );

          case "tool_result":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-system">
                <span className="transcript-role">{event.success ? "Tool result" : "Tool error"}</span>
                {event.output && (
                  <pre className="transcript-code">{event.output}</pre>
                )}
              </div>
            );

          case "follow_up":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-user">
                <span className="transcript-role">You</span>
                <div className="transcript-body">{event.text}</div>
              </div>
            );

          case "sub_agent_spawned":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-system">
                <span className="transcript-role">Sub-agent spawned</span>
                <div className="transcript-body">{event.description}</div>
              </div>
            );

          case "sub_agent_complete":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-system">
                <span className="transcript-role">Sub-agent complete</span>
                <div className="transcript-body">
                  {event.summary}
                  {showCost !== false && event.cost_usd != null && (
                    <span className="transcript-cost"> ({formatCost(event.cost_usd)})</span>
                  )}
                </div>
              </div>
            );

          case "error":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-error">
                <span className="transcript-role">Error</span>
                <div className="transcript-body">{event.message}</div>
              </div>
            );

          case "complete":
            return (
              <div key={`${event.event_type}-${i}`} className="transcript-message transcript-system">
                <span className="transcript-role">Session complete</span>
                <div className="transcript-body">
                  {event.summary}
                  {showCost !== false && event.total_cost_usd != null && ` — ${formatCost(event.total_cost_usd)}`}
                  {event.turns != null && ` — ${event.turns} turns`}
                </div>
              </div>
            );

          default:
            return null;
        }
      })}

      {shouldShowExtracted && (
        <MemoryChip
          variant="extracted"
          memories={extractedMemories ?? []}
          error={extractedMemoriesError}
          onViewMemories={onViewMemories}
        />
      )}
    </div>
  );
}
