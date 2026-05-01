import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AgentEvent } from "../App";
import { formatCost } from "../lib/utils";

interface TranscriptViewProps {
  events: AgentEvent[];
  prompt: string;
}

export default function TranscriptView({ events, prompt }: TranscriptViewProps) {
  return (
    <div className="transcript-view">
      <div className="transcript-message transcript-user">
        <span className="transcript-role">You</span>
        <div className="transcript-body">{prompt}</div>
      </div>

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
                  {event.cost_usd != null && (
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
                  {event.total_cost_usd != null && ` — ${formatCost(event.total_cost_usd)}`}
                  {event.turns != null && ` — ${event.turns} turns`}
                </div>
              </div>
            );

          default:
            return null;
        }
      })}
    </div>
  );
}
