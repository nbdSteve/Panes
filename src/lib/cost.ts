export interface ThreadEvent {
  event_type: string;
  total_usd?: number;
}

export function calculateRunningCost(events: ThreadEvent[]): number {
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
}
