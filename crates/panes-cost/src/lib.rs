use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use chrono::Utc;
use panes_events::AgentEvent;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadCost {
    pub thread_id: String,
    pub workspace_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_usd: f64,
    pub model: String,
}

pub struct CostTracker {
    running: Mutex<HashMap<String, RunningCost>>,
}

struct RunningCost {
    workspace_id: String,
    input_tokens: u64,
    output_tokens: u64,
    total_usd: f64,
    model: String,
}

impl CostTracker {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(HashMap::new()),
        }
    }

    pub fn start_tracking(&self, thread_id: &str, workspace_id: &str) {
        let mut running = self.running.lock().unwrap();
        running.insert(
            thread_id.to_string(),
            RunningCost {
                workspace_id: workspace_id.to_string(),
                input_tokens: 0,
                output_tokens: 0,
                total_usd: 0.0,
                model: String::new(),
            },
        );
    }

    pub fn process_event(&self, thread_id: &str, event: &AgentEvent) {
        let mut running = self.running.lock().unwrap();
        if let Some(cost) = running.get_mut(thread_id) {
            match event {
                AgentEvent::CostUpdate {
                    input_tokens,
                    output_tokens,
                    total_usd,
                    model,
                    ..
                } => {
                    cost.input_tokens += input_tokens;
                    cost.output_tokens += output_tokens;
                    cost.total_usd += total_usd;
                    cost.model.clone_from(model);
                }
                AgentEvent::Complete { total_cost_usd, .. } => {
                    // Result event has the authoritative total — override our running estimate
                    cost.total_usd = *total_cost_usd;
                }
                _ => {}
            }
        }
    }

    pub fn get_running_cost(&self, thread_id: &str) -> Option<f64> {
        let running = self.running.lock().unwrap();
        running.get(thread_id).map(|c| c.total_usd)
    }

    pub fn finalize(&self, thread_id: &str) -> Option<ThreadCost> {
        let mut running = self.running.lock().unwrap();
        running.remove(thread_id).map(|c| ThreadCost {
            thread_id: thread_id.to_string(),
            workspace_id: c.workspace_id,
            input_tokens: c.input_tokens,
            output_tokens: c.output_tokens,
            total_usd: c.total_usd,
            model: c.model,
        })
    }

    pub fn check_budget(&self, thread_id: &str, budget_cap: f64) -> bool {
        let running = self.running.lock().unwrap();
        if let Some(cost) = running.get(thread_id) {
            cost.total_usd >= budget_cap
        } else {
            false
        }
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn save_cost(conn: &Connection, cost: &ThreadCost) -> Result<()> {
    conn.execute(
        "INSERT INTO costs (thread_id, workspace_id, input_tokens, output_tokens, total_usd, model, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            cost.thread_id,
            cost.workspace_id,
            cost.input_tokens,
            cost.output_tokens,
            cost.total_usd,
            cost.model,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_workspace_cost(conn: &Connection, workspace_id: &str) -> Result<f64> {
    let total: f64 = conn.query_row(
        "SELECT COALESCE(SUM(total_usd), 0) FROM costs WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
        |row| row.get(0),
    )?;
    Ok(total)
}

pub fn get_total_cost(conn: &Connection) -> Result<f64> {
    let total: f64 = conn.query_row(
        "SELECT COALESCE(SUM(total_usd), 0) FROM costs",
        [],
        |row| row.get(0),
    )?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cost_event(input: u64, output: u64, usd: f64) -> AgentEvent {
        AgentEvent::CostUpdate {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total_usd: usd,
            model: "test-model".to_string(),
        }
    }

    fn make_complete_event(cost: f64) -> AgentEvent {
        AgentEvent::Complete {
            summary: "done".to_string(),
            total_cost_usd: cost,
            duration_ms: 1000,
            turns: 1,
        }
    }

    #[test]
    fn test_lifecycle_start_process_finalize() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(100, 50, 0.01));
        assert!((tracker.get_running_cost("t1").unwrap() - 0.01).abs() < f64::EPSILON);

        tracker.process_event("t1", &make_cost_event(200, 100, 0.02));
        assert!((tracker.get_running_cost("t1").unwrap() - 0.03).abs() < f64::EPSILON);

        let finalized = tracker.finalize("t1").unwrap();
        assert_eq!(finalized.input_tokens, 300);
        assert_eq!(finalized.output_tokens, 150);
        assert!((finalized.total_usd - 0.03).abs() < f64::EPSILON);
        assert_eq!(finalized.workspace_id, "ws1");
    }

    #[test]
    fn test_complete_overrides_running_total() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(100, 50, 0.01));
        tracker.process_event("t1", &make_cost_event(200, 100, 0.02));
        // Running estimate: 0.03
        // Complete event has authoritative total:
        tracker.process_event("t1", &make_complete_event(0.057));

        let finalized = tracker.finalize("t1").unwrap();
        assert!((finalized.total_usd - 0.057).abs() < f64::EPSILON);
        // Tokens are still accumulated, not overridden
        assert_eq!(finalized.input_tokens, 300);
    }

    #[test]
    fn test_budget_check() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(100, 50, 0.04));
        assert!(!tracker.check_budget("t1", 0.05));

        tracker.process_event("t1", &make_cost_event(100, 50, 0.01));
        assert!(tracker.check_budget("t1", 0.05)); // 0.05 >= 0.05

        tracker.process_event("t1", &make_cost_event(100, 50, 0.01));
        assert!(tracker.check_budget("t1", 0.05)); // 0.06 >= 0.05
    }

    #[test]
    fn test_budget_check_unknown_thread() {
        let tracker = CostTracker::new();
        assert!(!tracker.check_budget("nonexistent", 1.0));
    }

    #[test]
    fn test_finalize_unknown_thread() {
        let tracker = CostTracker::new();
        assert!(tracker.finalize("nonexistent").is_none());
    }

    #[test]
    fn test_running_cost_unknown_thread() {
        let tracker = CostTracker::new();
        assert!(tracker.get_running_cost("nonexistent").is_none());
    }

    #[test]
    fn test_process_event_on_unknown_thread_is_noop() {
        let tracker = CostTracker::new();
        // Should not panic
        tracker.process_event("nonexistent", &make_cost_event(100, 50, 0.01));
    }

    #[test]
    fn test_multiple_threads_independent() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");
        tracker.start_tracking("t2", "ws2");

        tracker.process_event("t1", &make_cost_event(100, 50, 0.01));
        tracker.process_event("t2", &make_cost_event(500, 200, 0.10));

        assert!((tracker.get_running_cost("t1").unwrap() - 0.01).abs() < f64::EPSILON);
        assert!((tracker.get_running_cost("t2").unwrap() - 0.10).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sqlite_save_and_query() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE costs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                total_usd REAL DEFAULT 0,
                model TEXT,
                timestamp TEXT NOT NULL
            )"
        ).unwrap();

        let cost1 = ThreadCost {
            thread_id: "t1".into(),
            workspace_id: "ws1".into(),
            input_tokens: 100,
            output_tokens: 50,
            total_usd: 0.05,
            model: "test".into(),
        };
        let cost2 = ThreadCost {
            thread_id: "t2".into(),
            workspace_id: "ws1".into(),
            input_tokens: 200,
            output_tokens: 100,
            total_usd: 0.10,
            model: "test".into(),
        };
        let cost3 = ThreadCost {
            thread_id: "t3".into(),
            workspace_id: "ws2".into(),
            input_tokens: 300,
            output_tokens: 150,
            total_usd: 0.20,
            model: "test".into(),
        };

        save_cost(&conn, &cost1).unwrap();
        save_cost(&conn, &cost2).unwrap();
        save_cost(&conn, &cost3).unwrap();

        let ws1_cost = get_workspace_cost(&conn, "ws1").unwrap();
        assert!((ws1_cost - 0.15).abs() < f64::EPSILON);

        let ws2_cost = get_workspace_cost(&conn, "ws2").unwrap();
        assert!((ws2_cost - 0.20).abs() < f64::EPSILON);

        let total = get_total_cost(&conn).unwrap();
        assert!((total - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn test_empty_workspace_cost() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE costs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                total_usd REAL DEFAULT 0,
                model TEXT,
                timestamp TEXT NOT NULL
            )"
        ).unwrap();

        assert!((get_workspace_cost(&conn, "nonexistent").unwrap()).abs() < f64::EPSILON);
        assert!((get_total_cost(&conn).unwrap()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_negative_cost_reduces_total() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(100, 50, 0.05));
        tracker.process_event("t1", &make_cost_event(0, 0, -0.03));
        assert!((tracker.get_running_cost("t1").unwrap() - 0.02).abs() < f64::EPSILON);
        assert!(!tracker.check_budget("t1", 0.05));
    }

    #[test]
    fn test_nan_cost_poisons_total() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(100, 50, f64::NAN));
        assert!(tracker.get_running_cost("t1").unwrap().is_nan());
        // IEEE 754: NaN >= budget_cap is always false — budget never triggers
        assert!(!tracker.check_budget("t1", 0.01));
    }

    #[test]
    fn test_nan_then_normal_stays_nan() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");

        tracker.process_event("t1", &make_cost_event(0, 0, f64::NAN));
        tracker.process_event("t1", &make_cost_event(100, 50, 0.05));
        // NaN + anything = NaN — tracker is permanently poisoned
        assert!(tracker.get_running_cost("t1").unwrap().is_nan());
    }

    #[test]
    fn test_start_tracking_overwrites() {
        let tracker = CostTracker::new();
        tracker.start_tracking("t1", "ws1");
        tracker.process_event("t1", &make_cost_event(100, 50, 0.05));

        tracker.start_tracking("t1", "ws2");
        assert!((tracker.get_running_cost("t1").unwrap()).abs() < f64::EPSILON);

        let finalized = tracker.finalize("t1").unwrap();
        assert_eq!(finalized.workspace_id, "ws2");
        assert_eq!(finalized.input_tokens, 0);
    }
}
