export interface ThreadEvent {
  event_type: string;
  total_usd?: number;
  total_cost_usd?: number;
}

export interface CostThread {
  status: string;
  costUsd?: number;
  events: ThreadEvent[];
}

export function threadDisplayCost(thread: CostThread): number {
  const isActive = thread.status === "running" || thread.status === "starting" || thread.status === "gate";
  if (!isActive && thread.costUsd) {
    return thread.costUsd;
  }
  return calculateRunningCost(thread.events);
}

export function workspaceDisplayCost(threads: CostThread[]): number {
  return threads.reduce((sum, t) => sum + threadDisplayCost(t), 0);
}

export function calculateRunningCost(events: ThreadEvent[]): number {
  let total = 0;
  let turnEstimate = 0;
  for (const e of events) {
    if (e.event_type === "cost_update") {
      turnEstimate += e.total_usd || 0;
    }
    if (e.event_type === "complete") {
      // complete.total_cost_usd is authoritative for the turn
      total += e.total_cost_usd != null ? e.total_cost_usd : turnEstimate;
      turnEstimate = 0;
    }
    if (e.event_type === "follow_up") {
      total += turnEstimate;
      turnEstimate = 0;
    }
  }
  return total + turnEstimate;
}
