# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Panes is a Tauri 2 desktop app (Rust backend, React frontend) that wraps AI coding agents with safety gates, one-click git rollback, persistent memory, and cost visibility. macOS only. It is not an IDE or an agent — it's a safety/orchestration layer that spawns agent CLIs as child processes.

## Build & Run

```bash
npm install
npx tauri dev                     # Run with real Claude CLI
PANES_TEST_MODE=1 npx tauri dev   # Run with fake adapter (no Claude CLI needed)
```

## Tests

```bash
# Rust unit + integration (289 tests across 6 crates)
cargo test --workspace

# Run a single Rust test
cargo test -p panes-core test_gate_approve_completes_thread

# Frontend unit tests (vitest, 105 tests)
npx vitest run
npx vitest run src/lib/cost.test.ts           # single file

# E2E tests (Playwright, webkit — requires vite dev server)
npm run test:e2e                               # frontend-only E2E (mock Tauri)
npm run test:e2e:headed                        # same, visible browser
npm run test:e2e:fullstack                     # fullstack E2E (real Tauri app + test bridge)

# Everything
npm run test:all
```

## Architecture

### Two-process model

The Rust backend spawns agent CLIs (e.g. `claude -p --output-format stream-json`) as child processes, parses their stdout event streams into a unified `AgentEvent` enum, and forwards them to the React frontend via Tauri events. The frontend never talks to agents directly.

### Crate dependency graph

```
panes-events          ← Shared types: AgentEvent, RiskLevel, ThreadEvent, SessionContext
    ↑
panes-adapters        ← AgentAdapter trait + implementations (Claude CLI, Fake)
panes-cost            ← CostTracker (in-memory accumulator) + SQLite persistence
panes-memory          ← MemoryManager (dual-backend: SQLite FTS5 + Mem0 sidecar), briefings
    ↑
panes-core            ← SessionManager (owns active threads, gate pausing, git snapshots)
    ↑
panes-app             ← Tauri entry point, IPC command handlers, test bridge
```

### Key data flow: prompt → completion

1. Frontend calls `start_thread` via Tauri IPC
2. `SessionManager` takes a pre-thread git snapshot, spawns an adapter session
3. Adapter produces a `Stream<Item = AgentEvent>`, consumed by `consume_events` in a tokio task
4. Events are: persisted to SQLite, sent through `CostTracker`, forwarded to frontend via `panes://thread-events` Tauri event (batched at 50ms intervals)
5. `ToolRequest { needs_approval: true }` pauses the stream on a `oneshot` channel — frontend renders a gate card — user approves/rejects — oneshot resolves, stream resumes
6. On `Complete`, cost is finalized and frontend triggers memory extraction

### Agent adapter abstraction

`AgentAdapter` (trait in `panes-adapters/src/lib.rs`) defines `spawn()` → `Box<dyn AgentSession>`. `AgentSession` provides `events()` → `Stream<AgentEvent>`, plus `approve()`, `reject()`, `cancel()`. Two implementations exist:
- `ClaudeAdapter` — spawns `claude` CLI, parses stream-json via `parser.rs`, classifies risk via `risk.rs`
- `FakeAdapter` — configurable scenarios (TextOnly, GatedAction, FileEdit, MultiStep, Error) for tests

### Gate mechanism

Gates are implemented entirely in `SessionManager::consume_events`. When a `ToolRequest { needs_approval: true }` arrives, a `oneshot::channel` is created and stored in the `ActiveThread`. The event loop awaits the receiver. `approve()` / `reject()` on SessionManager find the sender and resolve it. The stream is truly paused — no events are consumed while gated.

### Test bridge (E2E architecture)

In `PANES_TEST_MODE`, the app starts a WebSocket server on `ws://127.0.0.1:3001/ws` (`test_bridge.rs`) alongside the Tauri app. E2E tests use `tauriBridge.ts` to install a fake `__TAURI_INTERNALS__` that routes `invoke()` calls over this WebSocket and receives events back. This lets Playwright tests exercise the real frontend against the real Rust backend without native Tauri webview automation.

Fullstack E2E tests (`e2e-fullstack/`) use a different config: they build the real Tauri app, start it with `PANES_TEST_MODE=1`, and connect via the WebSocket bridge.

### Prompt routing in test mode

`PromptRoutedFakeAdapter` in `panes-app/src/lib.rs` routes prompts to fake scenarios by keyword: "error"/"fail" → Error, "gate"/"dangerous" → GatedAction, "edit"/"write" → FileEdit, "multi"/"complex" → MultiStep, "slow" → slow MultiStep (for cancel testing), anything else → TextOnly.

### Memory system

`MemoryManager` wraps two backends with automatic failover: Mem0 (vector + graph search via Python sidecar) and SQLite FTS5 (always-available fallback). A health monitor checks Mem0 every 30s and restarts the sidecar on failure. Briefings (user-authored workspace instructions) always go through SQLite regardless of active backend. Memory extraction happens post-thread via `extract_memories` IPC command called from the frontend.

### One-thread-per-workspace invariant (Phase 1)

`SessionManager` enforces that only one thread can be active per workspace at a time. This prevents concurrent agents from creating conflicting file changes in the same directory. The guard checks happen in both `start_thread` and `resume_thread`.

Phase 2 lifts this constraint via git worktrees — each concurrent thread gets an isolated checkout. See `docs/ARCHITECTURE.md` "Phase 2: Git Worktrees" section.

### Task DAG / Swarm execution (Phase 2, not yet implemented)

Multi-agent swarms use `petgraph` for dependency-aware task scheduling (not beads_rust — see GAPS.md for rationale). The planner LLM produces an `ExecutionPlan` DAG, user refines it, `panes-scheduler` dispatches unblocked tasks into worktree-backed threads. Each task carries its own prompt, budget cap, gate policy, and verification config.

## Conventions

- Rust types serialized for the frontend use `#[serde(rename_all = "camelCase")]`
- `AgentEvent` uses `#[serde(tag = "event_type", rename_all = "snake_case")]` — internally tagged enum
- IPC commands are in `commands.rs`, registered in `lib.rs` via `tauri::generate_handler![]`
- Frontend state is managed in `App.tsx` — threads/workspaces are React state, events arrive via `listen("panes://thread-events")`
- SQLite is the single persistence layer. Schema lives in `panes-core/src/db.rs` with `run_migrations()`
- The `DbState` type is `Arc<std::sync::Mutex<Connection>>` (not tokio Mutex — rusqlite is synchronous)

## Environment Variables

| Var | Effect |
|-----|--------|
| `PANES_TEST_MODE` | Use fake adapters, start WebSocket test bridge on :3001 |
| `PANES_CLAUDE_PATH` | Path to Claude CLI binary (default: `claude`) |
| `PANES_DATA_DIR` | Override data directory (default: `~/Library/Application Support/dev.panes/`) |
| `CLAUDE_CODE_USE_BEDROCK` | Passed through to Claude CLI |
| `AWS_PROFILE` | Passed through to Claude CLI |
| `PANES_MEM0_PYTHON` | Python binary for Mem0 sidecar |
