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
