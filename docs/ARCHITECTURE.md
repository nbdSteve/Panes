# Panes — Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        Panes Desktop App                        │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                    Frontend (React)                        │  │
│  │                                                           │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐  │  │
│  │  │Workspace │ │  Feed    │ │ Memory   │ │  Routines   │  │  │
│  │  │  View    │ │          │ │  Panel   │ │  Manager    │  │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └─────────────┘  │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐  │  │
│  │  │  Gate    │ │ Thread   │ │ Cost     │ │  Agent      │  │  │
│  │  │  Cards   │ │ Timeline │ │ Tracker  │ │  Selector   │  │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └─────────────┘  │  │
│  └───────────────────────┬───────────────────────────────────┘  │
│                          │ Tauri IPC                             │
│  ┌───────────────────────┴───────────────────────────────────┐  │
│  │                  Rust Backend (Tauri)                      │  │
│  │                                                           │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │              Session Manager                        │  │  │
│  │  │                                                     │  │  │
│  │  │  Owns all active agent sessions. Routes frontend    │  │  │
│  │  │  actions (send prompt, approve, reject, steer,      │  │  │
│  │  │  cancel) to the correct session. Emits AgentEvents  │  │  │
│  │  │  to frontend via Tauri event system.                │  │  │
│  │  └──────────┬──────────────────────┬───────────────────┘  │  │
│  │             │                      │                      │  │
│  │  ┌──────────▼──────────┐ ┌────────▼────────────────────┐ │  │
│  │  │   ACP Client        │ │   Adapter Layer             │ │  │
│  │  │                     │ │                             │ │  │
│  │  │  agent-client-      │ │  trait AgentAdapter {       │ │  │
│  │  │  protocol crate     │ │    fn spawn() -> Session    │ │  │
│  │  │                     │ │    fn events() -> Stream    │ │  │
│  │  │  JSON-RPC 2.0       │ │    fn approve()             │ │  │
│  │  │  over stdio         │ │    fn reject()              │ │  │
│  │  │                     │ │    fn steer()               │ │  │
│  │  │  Handles:           │ │  }                          │ │  │
│  │  │  - initialize       │ │                             │ │  │
│  │  │  - session/new      │ │  ┌───────────────────────┐  │ │  │
│  │  │  - session/prompt   │ │  │ Claude Code Adapter   │  │ │  │
│  │  │  - session/cancel   │ │  │                       │  │ │  │
│  │  │  - fs.* / terminal  │ │  │ Spawns claude CLI     │  │ │  │
│  │  │  - approval flow    │ │  │ --output-format       │  │ │  │
│  │  │                     │ │  │   stream-json         │  │ │  │
│  │  └──────────┬──────────┘ │  │ --input-format        │  │ │  │
│  │             │            │  │   stream-json         │  │ │  │
│  │             │            │  │                       │  │ │  │
│  │             │            │  │ Translates to/from    │  │ │  │
│  │             │            │  │ AgentEvent model      │  │ │  │
│  │             │            │  └───────────────────────┘  │ │  │
│  │             │            │  ┌───────────────────────┐  │ │  │
│  │             │            │  │ Community Adapters    │  │ │  │
│  │             │            │  │ (Aider, Goose, Q...) │  │ │  │
│  │             │            │  └───────────────────────┘  │ │  │
│  │             │            └─────────────┬───────────────┘ │  │
│  │             │                          │                  │  │
│  │  ┌──────────▼──────────────────────────▼───────────────┐  │  │
│  │  │              Agent Process Pool                     │  │  │
│  │  │                                                     │  │  │
│  │  │  Manages child processes (one per active session).  │  │  │
│  │  │  Handles stdin/stdout piping, process lifecycle,    │  │  │
│  │  │  crash recovery, and graceful shutdown.             │  │  │
│  │  │                                                     │  │  │
│  │  │  ┌──────────┐ ┌──────────┐ ┌──────────┐           │  │  │
│  │  │  │kiro-cli  │ │claude    │ │goose     │  ...       │  │  │
│  │  │  │  acp     │ │  -p      │ │          │           │  │  │
│  │  │  │(stdio)   │ │(stdio)   │ │(stdio)   │           │  │  │
│  │  │  └──────────┘ └──────────┘ └──────────┘           │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │                                                           │  │
│  │  ┌──────────────────┐ ┌────────────────┐ ┌────────────┐  │  │
│  │  │  Memory Engine   │ │  Scheduler     │ │  Cost      │  │  │
│  │  │                  │ │                │ │  Tracker   │  │  │
│  │  │  Backend: Mem0   │ │  Tokio-based   │ │            │  │  │
│  │  │  (local sidecar  │ │  cron runner   │ │  Per-thread│  │  │
│  │  │  REST API)       │ │                │ │  Per-ws    │  │  │
│  │  │                  │ │  Persists      │ │  Aggregate │  │  │
│  │  │  Extraction:     │ │  routines      │ │            │  │  │
│  │  │  Post-thread,    │ │  to SQLite     │ │  Budget    │  │  │
│  │  │  via Mem0 API    │ │                │ │  caps &    │  │  │
│  │  │                  │ │  Spawns agent  │ │  alerts    │  │  │
│  │  │  Injection:      │ │  sessions via  │ │            │  │  │
│  │  │  Mem0 hybrid     │ │  Session Mgr   │ │  Reads     │  │  │
│  │  │  search (vector  │ │  on schedule   │ │  cost from │  │  │
│  │  │  + graph) +      │ │                │ │  agent     │  │  │
│  │  │  briefing        │ │  Results →     │ │  events    │  │  │
│  │  │                  │ │  Feed          │ │            │  │  │
│  │  │  Briefings:      │ │                │ │            │  │  │
│  │  │  Per-workspace   │ │                │ │            │  │  │
│  │  │  user-authored   │ │                │ │            │  │  │
│  │  │  (SQLite, not    │ │                │ │            │  │  │
│  │  │  Mem0)           │ │                │ │            │  │  │
│  │  │                  │ │                │ │            │  │  │
│  │  │  Fallback:       │ │                │ │            │  │  │
│  │  │  LLM extraction  │ │                │ │            │  │  │
│  │  │  + SQLite FTS    │ │                │ │            │  │  │
│  │  └────────┬─────────┘ └───────┬────────┘ └─────┬──────┘  │  │
│  │           │                   │                 │          │  │
│  │  ┌────────▼───────────────────▼─────────────────▼───────┐ │  │
│  │  │                    SQLite Database                    │ │  │
│  │  │                                                      │ │  │
│  │  │  workspaces    │ threads      │ memories             │ │  │
│  │  │  ─────────     │ ────────     │ ────────             │ │  │
│  │  │  id            │ id           │ id                   │ │  │
│  │  │  path          │ workspace_id │ workspace_id (null=  │ │  │
│  │  │  name          │ agent_type   │   global)            │ │  │
│  │  │  default_agent │ status       │ type (decision /     │ │  │
│  │  │  created_at    │ prompt       │   preference /       │ │  │
│  │  │               │ started_at   │   constraint /       │ │  │
│  │  │               │ completed_at │   pattern)           │ │  │
│  │  │               │ cost_usd     │ content              │ │  │
│  │  │               │ is_routine   │ source_thread_id     │ │  │
│  │  │               │ flow_id      │ created_at           │ │  │
│  │  │               │ flow_step    │ edited_at            │ │  │
│  │  │               │ parent_id    │                      │ │  │
│  │  │               │ tags (json)  │                      │ │  │
│  │  │               │ transcript   │                      │ │  │
│  │  │               │              │                      │ │  │
│  │  │  briefings     │ routines     │ costs                │ │  │
│  │  │  ─────────     │ ────────     │ ─────                │ │  │
│  │  │  id            │ id           │ thread_id            │ │  │
│  │  │  workspace_id  │ workspace_id │ input_tokens         │ │  │
│  │  │  content       │ type         │ output_tokens        │ │  │
│  │  │  updated_at    │ prompt       │ total_usd            │ │  │
│  │  │               │ flow_id      │ model                │ │  │
│  │  │  events        │ cron_expr    │ provider             │ │  │
│  │  │  ──────        │ budget_cap   │                      │ │  │
│  │  │  id            │ on_complete  │                      │ │  │
│  │  │  thread_id     │ on_failure   │                      │ │  │
│  │  │  type          │ enabled      │                      │ │  │
│  │  │  timestamp     │ last_run_at  │                      │ │  │
│  │  │  data (json)   │              │                      │ │  │
│  │  │               │ flows        │ flow_steps           │ │  │
│  │  │               │ ─────        │ ──────────           │ │  │
│  │  │               │ id           │ id                   │ │  │
│  │  │               │ name         │ flow_id              │ │  │
│  │  │               │ edges (json) │ workspace_id         │ │  │
│  │  │               │ created_at   │ agent                │ │  │
│  │  │               │              │ prompt_tmpl          │ │  │
│  │  │               │              │ gate_required        │ │  │
│  │  │               │              │ budget_cap           │ │  │
│  │  └──────────────────────────────────────────────────────┘ │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                  Filesystem (per workspace)               │  │
│  │                                                           │  │
│  │  ~/projects/backend/          (user's project)            │  │
│  │  ~/projects/backend/.panes/   (panes workspace metadata)  │  │
│  │                                                           │  │
│  │  Agent processes are chroot-scoped to workspace path.     │  │
│  │  No cross-workspace file access.                          │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Flow: Prompt to Completion

```
User types prompt
       │
       ▼
┌──────────────┐    Tauri IPC     ┌──────────────────┐
│   Frontend   │ ────────────────▶│  Session Manager  │
└──────────────┘                  └────────┬─────────┘
                                           │
                              ┌────────────┴────────────┐
                              │ Is agent ACP-native?     │
                              └────┬───────────────┬────┘
                                   │               │
                                  Yes              No
                                   │               │
                                   ▼               ▼
                            ┌────────────┐  ┌──────────────┐
                            │ ACP Client │  │   Adapter    │
                            └─────┬──────┘  └──────┬───────┘
                                  │                │
                                  ▼                ▼
                           ┌───────────┐    ┌───────────┐
                           │ kiro-cli  │    │  claude    │
                           │   acp     │    │   -p ...   │
                           └─────┬─────┘    └─────┬─────┘
                                 │                │
                     ┌───────────┴────────────────┘
                     │
                     ▼  (both produce the same internal events)
              ┌──────────────┐
              │  AgentEvent  │
              │  stream      │
              └──────┬───────┘
                     │
          ┌──────────┼──────────┬──────────────┐
          │          │          │              │
          ▼          ▼          ▼              ▼
    ┌──────────┐ ┌────────┐ ┌────────┐  ┌──────────┐
    │ Frontend │ │  Cost  │ │ Event  │  │  Memory  │
    │ (render  │ │Tracker │ │  Log   │  │ (on      │
    │  cards,  │ │(accum.)│ │(SQLite)│  │  session │
    │  steps)  │ │        │ │        │  │  end)    │
    └──────────┘ └────────┘ └────────┘  └──────────┘
```

---

## Data Flow: Approval Request

```
Agent process emits tool request
       │
       ▼
┌─────────────────────┐
│  ACP Client or      │
│  Adapter Layer      │
│                     │
│  Detects approval-  │
│  required event     │
└──────────┬──────────┘
           │
           ▼ AgentEvent::ToolRequest { needs_approval: true }
┌─────────────────────┐
│  Session Manager    │
│                     │
│  Pauses session,    │
│  emits to frontend  │
└──────────┬──────────┘
           │ Tauri event
           ▼
┌─────────────────────┐
│  Frontend           │
│                     │
│  Renders gate       │
│  card with:         │
│  - Action summary   │
│  - Risk level       │
│  - Running cost     │
│                     │
│  User clicks:       │
│  [Approve] [Reject] │
│  [Steer]            │
└──────────┬──────────┘
           │ Tauri IPC
           ▼
┌─────────────────────┐
│  Session Manager    │
│                     │
│  Routes decision    │
│  back to agent:     │
│                     │
│  Approve → allow    │
│  Reject  → deny     │
│  Steer   → deny +   │
│    inject feedback   │
│    as next prompt    │
└──────────┬──────────┘
           │
           ▼
     Agent process
     continues or stops
```

---

## Data Flow: Routine Execution

```
┌──────────────────┐
│    Scheduler     │
│                  │
│  Tokio cron loop │
│  checks SQLite   │
│  every 60s       │
└────────┬─────────┘
         │ Routine fires
         ▼
┌──────────────────┐
│  Session Manager │
│                  │
│  1. Load workspace config               │
│  2. Retrieve relevant memories +         │
│     workspace briefing                   │
│  3. Spawn agent session with:            │
│     - Workspace path                     │
│     - Prompt from routine                │
│     - Memory + briefing prepended        │
│     - Budget cap from routine            │
│  4. Mark thread as is_routine=true       │
└────────┬─────────┘
         │
         ▼
   Agent executes normally
   (same flow as manual task)
         │
         ▼
┌──────────────────────────┐
│  On completion:          │
│                          │
│  1. Log to Feed          │
│  2. Extract memories     │
│  3. Record cost          │
│  4. Check on_complete:   │
│     - "notify" → system  │
│       notification       │
│     - "chain:prompt" →   │
│       spawn follow-up    │
│  5. Check on_failure:    │
│     - "retry" → re-run   │
│     - "notify" → alert   │
│                          │
│  If budget cap hit:      │
│     - Kill session       │
│     - Log as budget_     │
│       exceeded           │
│     - Notify user        │
└──────────────────────────┘
```

---

## Data Flow: Memory Extraction and Injection

```
Thread completes
       │
       ▼
┌──────────────────────────────┐
│  Memory Engine — Extraction  │
│  (via Mem0 sidecar API)      │
│                              │
│  1. Take full transcript     │
│  2. POST to Mem0 /add with   │
│     transcript + metadata:   │
│     - workspace_id           │
│     - thread_id              │
│     - agent_type             │
│                              │
│  3. Mem0 handles:            │
│     - Fact extraction        │
│     - Deduplication          │
│     - Conflict resolution    │
│     - Graph relationship     │
│       building               │
│                              │
│  4. Memories stored in Mem0  │
│     with workspace scoping   │
│                              │
│  Fallback (if Mem0 down):    │
│  LLM prompt extraction →     │
│  SQLite FTS storage          │
└──────────────────────────────┘

New thread starts
       │
       ▼
┌──────────────────────────────┐
│  Memory Engine — Injection   │
│                              │
│  1. Load workspace Briefing  │
│     from SQLite (always      │
│     injected, user-authored) │
│  2. GET Mem0 /search with    │
│     prompt text + workspace  │
│     scope filter             │
│  3. Mem0 returns ranked      │
│     memories (hybrid search: │
│     vector similarity +      │
│     graph relationships)     │
│  4. Also query global-scope  │
│     memories from Mem0       │
│  5. Select top N within      │
│     token budget             │
│  6. Format context block:    │
│     [Briefing] + [Memories]  │
│  7. Prepend to session       │
│     system prompt or         │
│     first user message       │
└──────────────────────────────┘
```

---

## Agent Orchestration

There are three distinct orchestration patterns Panes must support, each at a different layer:

### Level 1: Agent-Internal Sub-Agents (passthrough)

The agent itself spawns child agents within its own session. ACP already models this with parent/child session relationships. Claude Code does the same with its internal `Task` tool (subagents).

Panes does not orchestrate this — the agent does. Panes renders the sub-agent activity as **branches** — collapsible nested sections in the thread timeline:

```
Task: "Add authentication to the API"
│
├─ Step 1: Agent reads existing routes
├─ Step 2: Agent plans approach
├─ Step 3: Agent spawns sub-agent: "Write auth middleware"
│  │
│  ├─ Sub-step 1: Sub-agent reads express config
│  ├─ Sub-step 2: Sub-agent creates auth.ts
│  └─ Sub-step 3: Sub-agent writes tests
│
├─ Step 4: Agent integrates middleware into routes
└─ Step 5: Agent runs full test suite
```

ACP exposes sub-agent sessions via the `meta.sub_agent_session_id` field. The Claude Code adapter can detect sub-agent spawning from `Task` tool_use events. In both cases, Panes tracks the parent-child relationship for the timeline view but does not intervene in the orchestration.

**Frontend impact:** Branches are collapsible in the timeline. Sub-agent gates bubble up to the parent thread's gate card.

### Level 2: Parallel Independent Sessions (what we already have)

Multiple agent sessions running simultaneously across different workspaces. No coordination between them — they share nothing except the Panes UI.

This is the core multi-workspace feature already in the architecture. The Session Manager owns N concurrent sessions, each isolated to its workspace.

### Level 3: Panes-Orchestrated Flows

This is the new layer. Panes itself coordinates multiple agents working together on a single task. The user defines a Flow, and Panes executes it.

```
User creates a Flow:
  "Build a new feature: add user profiles"

  Step 1: [kiro-cli @ backend]  "Add /users/:id endpoint returning profile data"
  Step 2: [kiro-cli @ frontend] "Add a user profile page that calls /users/:id"
       ↑ depends on Step 1 (needs to know the API shape)
  Step 3: [claude @ backend]    "Review the new endpoint for security issues"
       ↑ depends on Step 1
  Step 4: [claude @ frontend]   "Run lighthouse and check the new page performance"
       ↑ depends on Step 2
```

#### Flow Architecture

```
┌──────────────────────────────────────────────────────┐
│                  Flow Engine                          │
│                  (panes-orchestrator crate)           │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │              Flow Definition                    │  │
│  │                                                │  │
│  │  steps: Vec<FlowStep>                          │  │
│  │  edges: Vec<(step_id, step_id)>  (DAG)         │  │
│  │                                                │  │
│  │  Each step has:                                │  │
│  │  - workspace                                   │  │
│  │  - agent                                       │  │
│  │  - prompt (can template outputs from prior     │  │
│  │    steps via {{step_1.summary}})               │  │
│  │  - gate_required: bool                          │  │
│  │  - budget_cap                                  │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │              Execution Engine                   │  │
│  │                                                │  │
│  │  1. Topological sort of the DAG               │  │
│  │  2. Execute steps with no dependencies first   │  │
│  │     (parallel via Session Manager)             │  │
│  │  3. When a step completes:                     │  │
│  │     - Extract summary from result              │  │
│  │     - Check if any dependent steps are now     │  │
│  │       unblocked                                │  │
│  │     - Template the dependent step's prompt     │  │
│  │       with outputs from completed steps        │  │
│  │     - Spawn newly unblocked steps              │  │
│  │  4. If a step fails:                           │  │
│  │     - Mark all downstream steps as blocked     │  │
│  │     - Notify user with option to retry,        │  │
│  │       skip, or abort flow                      │  │
│  │  5. Flow completes when all steps done         │  │
│  └────────────────────────────────────────────────┘  │
│                                                      │
│  ┌────────────────────────────────────────────────┐  │
│  │              Context Passing                    │  │
│  │                                                │  │
│  │  Steps can reference outputs of prior steps:   │  │
│  │                                                │  │
│  │  Step 2 prompt:                                │  │
│  │    "Add a profile page. The backend endpoint   │  │
│  │     is described here:                         │  │
│  │     {{steps.add_endpoint.summary}}"            │  │
│  │                                                │  │
│  │  Available template variables per step:        │  │
│  │  - {{steps.<name>.summary}}  (completion text) │  │
│  │  - {{steps.<name>.cost}}     (cost so far)     │  │
│  │  - {{steps.<name>.status}}   (success/failed)  │  │
│  │  - {{steps.<name>.files}}    (files changed)   │  │
│  │                                                │  │
│  │  Panes injects these into the prompt before    │  │
│  │  sending to the agent. The agent never knows   │  │
│  │  it's part of a pipeline — it just sees a      │  │
│  │  well-informed prompt.                         │  │
│  └────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────┘
```

#### Flow Data Flow

```
User defines Flow (UI or YAML)
       │
       ▼
┌──────────────────┐
│  Flow Engine     │
│                  │
│  Topo-sort DAG   │
│  Identify roots  │
│  (no deps)       │
└────────┬─────────┘
         │
         ▼ Spawn root steps in parallel
┌──────────────────┐     ┌──────────────────┐
│  Session Manager │     │  Session Manager  │
│  Step 1: backend │     │  Step 3: review   │
│  (kiro-cli)      │     │  (claude)         │
└────────┬─────────┘     │  BLOCKED on #1    │
         │               └──────────────────┘
         │ Step 1 completes
         │
         ▼
┌──────────────────┐
│  Flow Engine     │
│                  │
│  1. Record step  │
│     1 output     │
│  2. Template     │
│     step 2 & 3   │
│     prompts with │
│     step 1 data  │
│  3. Unblock &    │
│     spawn both   │
└──────┬─────┬─────┘
       │     │
       ▼     ▼
   Step 2   Step 3
  (parallel execution)
       │     │
       ▼     ▼
  Flow Engine
  checks for more
  unblocked steps...
       │
       ▼
  All steps done → Flow Complete
  Feed shows full Flow summary
  with per-step results
```

#### Frontend Flow View

```
┌──────────────────────────────────────────────────┐
│  Flow: Add User Profiles                          │
│  Status: In Progress (2/4 complete)               │
│  Cost: $1.23 total                                │
│                                                   │
│  ┌────────────┐     ┌────────────┐               │
│  │ 1. Backend │────▶│ 2. Frontend│               │
│  │  endpoint  │     │  page      │               │
│  │  ✓ Done    │     │  ● Working │               │
│  │  $0.45     │     │  $0.38     │               │
│  └────────────┘     └────────────┘               │
│        │                                          │
│        ▼                                          │
│  ┌────────────┐     ┌────────────┐               │
│  │ 3. Security│     │ 4. Perf    │               │
│  │  review    │     │  check     │               │
│  │  ✓ Done    │     │  ○ Blocked │               │
│  │  $0.40     │     │  on #2     │               │
│  └────────────┘     └────────────┘               │
└──────────────────────────────────────────────────┘
```

#### The Harness Crate: `panes-harness`

The Flow engine above handles multi-agent coordination across workspaces, but each individual agent session also needs structured execution management — planning, step-by-step execution, verification, and replanning on failure. This is what the harness provides.

`panes-harness` owns the autonomous execution lifecycle for a single task within a single workspace. It's inspired by the TsukiHarness plan → execute → verify → replan pattern, with key deviations: simpler step types, pluggable verification, escalate-to-user as the default failure mode, and automatic complexity detection so simple prompts skip the harness entirely.

```
┌──────────────────────────────────────────────────────────────┐
│                       panes-harness                          │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                    Core Loop                           │  │
│  │                                                        │  │
│  │  1. CLASSIFY — is this a simple prompt or a task?      │  │
│  │               Simple → skip harness, send to agent     │  │
│  │               Task   → continue to PLAN                │  │
│  │  2. PLAN     — resolve a step plan                     │  │
│  │  3. EXECUTE  — run each step via agent session         │  │
│  │  4. VERIFY   — validate step output (pluggable)        │  │
│  │  5. DECIDE   — on failure: escalate to user (default)  │  │
│  │               or auto-replan (opt-in)                  │  │
│  │  6. REPEAT   — until plan complete or budget exhausted │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────┐   │
│  │  Planner     │  │  Step Runner │  │  Verifiers      │   │
│  │              │  │              │  │  (pluggable)    │   │
│  │  Static plan │  │  Routes step │  │                 │   │
│  │  (user-      │  │  to agent    │  │  ShellVerifier  │   │
│  │  provided)   │  │  via Session │  │  (run cmd,      │   │
│  │       or     │  │  Manager     │  │  check exit)    │   │
│  │  Dynamic     │  │              │  │                 │   │
│  │  (LLM plans  │  │  Step types: │  │  LlmVerifier    │   │
│  │  from prompt │  │              │  │  ("does output  │   │
│  │  + playbook  │  │  agent:      │  │  match intent?" │   │
│  │  context)    │  │  send to     │  │  — for non-dev  │   │
│  │       or     │  │  agent, the  │  │  users)         │   │
│  │  Playbook    │  │  default     │  │                 │   │
│  │  fallback    │  │              │  │  ScreenshotDiff │   │
│  │  (default    │  │  shell:      │  │  (capture       │   │
│  │  steps)      │  │  run cmd     │  │  before/after   │   │
│  │              │  │  directly,   │  │  for frontend)  │   │
│  │              │  │  no agent    │  │                 │   │
│  │              │  │              │  │  Custom         │   │
│  │              │  │  gate:       │  │  (user-provided │   │
│  │              │  │  pause for   │  │  trait impl)    │   │
│  │              │  │  approval    │  │                 │   │
│  └──────────────┘  └──────────────┘  └─────────────────┘   │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────┐   │
│  │  Failure     │  │  Playbooks   │  │  Contracts      │   │
│  │  Handler     │  │              │  │                 │   │
│  │              │  │  Markdown +  │  │  Executable     │   │
│  │  Default:    │  │  YAML front  │  │  requirements   │   │
│  │  ESCALATE    │  │  matter      │  │  (shell cmds    │   │
│  │  to user     │  │              │  │  that verify    │   │
│  │  with        │  │  Domain      │  │  correctness)   │   │
│  │  failure     │  │  knowledge   │  │                 │   │
│  │  context     │  │  for the     │  │  e.g. "npm test │   │
│  │              │  │  planner     │  │  must pass"     │   │
│  │  Opt-in:     │  │              │  │                 │   │
│  │  AUTO-REPLAN │  │  Fallback    │  │  Run after step │   │
│  │  (for cron   │  │  static      │  │  or at end      │   │
│  │  tasks or    │  │  steps       │  │                 │   │
│  │  explicit    │  │              │  │                 │   │
│  │  user        │  │              │  │                 │   │
│  │  request)    │  │              │  │                 │   │
│  └──────────────┘  └──────────────┘  └─────────────────┘   │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                   Step DAG Scheduling                  │  │
│  │                                                        │  │
│  │  Steps declare depends_on: ["step-a", "step-b"]        │  │
│  │  Scheduler topo-sorts into parallel layers:            │  │
│  │                                                        │  │
│  │  Layer 0: [research]          (no deps)                │  │
│  │  Layer 1: [design, scaffold]  (depend on research)     │  │
│  │  Layer 2: [implement]         (depends on scaffold)    │  │
│  │  Layer 3: [test, review]      (depend on implement)    │  │
│  │                                                        │  │
│  │  Steps within a layer execute in parallel.             │  │
│  │  Steps with modifies_files=true in the same workspace  │  │
│  │  are serialized (safety constraint).                   │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │                   Callbacks / Hooks                    │  │
│  │                                                        │  │
│  │  on_step_complete(step_name, result)                   │  │
│  │  on_plan_resolved(steps)                               │  │
│  │  pre_step_hook(step_name) → continue | skip | abort    │  │
│  │  on_failure(step, error) → escalate | replan | retry   │  │
│  │  on_budget_exceeded(spent, cap)                        │  │
│  │  on_needs_approval(step_name, action) → approve|reject │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

##### Key Deviations from TsukiHarness

1. **Complexity classification.** Before engaging the harness, a lightweight classifier determines if the prompt is a simple question/small task (skip harness, send directly to agent) or substantive work (engage harness). This avoids the overhead of planning + verification for "what Node version are we on?"

2. **Three step types, not five.** `agent` (send to agent — the default, let the agent figure out how to accomplish the intent), `shell` (run a command directly, no agent), `gate` (pause for user approval or verification). TsukiHarness distinguishes `prompt`/`code_fix`/`research` — in Panes, the agent backend handles that distinction.

3. **Pluggable verifiers.** Instead of hardcoding shell-command contracts as the only verification:

```rust
trait Verifier: Send + Sync {
    async fn verify(&self, step: &StepSpec, result: &StepResult, ctx: &HarnessContext) -> VerifyResult;
}

pub enum VerifyResult {
    Pass,
    Fail { reason: String },
    NeedsHumanReview { summary: String },  // escalate to user
}
```

Four built-in verifiers, any combination per step:
  - `ShellVerifier` — run a command, check exit code (`npm test`, `cargo build`)
  - `LlmVerifier` — ask an LLM "does this output match the intent?" (for non-dev users who can't evaluate code)
  - `ScreenshotVerifier` — capture before/after for frontend changes (requires a running dev server)
  - `ContractVerifier` — run executable contracts (shell commands defined in playbook)

5. **Escalate-first failure handling.** TsukiHarness auto-replans up to N times. Panes defaults to escalating to the user on failure — surface the error, what was tried, and let them steer. Auto-replan is opt-in, enabled per-task or forced on for scheduled/cron tasks (where there's no user to escalate to).

##### Harness Data Model

```rust
pub struct StepSpec {
    pub name: String,
    pub intent: String,                // what to accomplish (human readable)
    pub instructions: String,          // detailed how-to for the agent
    pub step_type: StepType,
    pub depends_on: Vec<String>,
    pub verifiers: Vec<VerifierConfig>, // which verifiers to run after step
    pub checkpoint: bool,              // review plan validity after this step
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub modifies_files: bool,          // parallel safety: serialize if true
    pub model: Option<String>,         // per-step model override
    pub agent: Option<String>,         // per-step agent override
    pub when: Option<StepCondition>,   // conditional execution
    pub budget_cap: Option<f64>,       // per-step spending limit
}

pub enum StepType {
    Agent,  // send to agent backend (default — agent decides how to accomplish)
    Shell,  // run command directly, no agent
    Gate,   // pause for user approval or verification
}

pub enum StepCondition {
    Always,
    OnPriorPassed(String),   // run only if named step passed
    OnPriorFailed(String),   // run only if named step failed
}

pub enum VerifierConfig {
    Shell { command: String },
    Llm { criteria: String },           // "does output satisfy: {criteria}?"
    Screenshot { url: String },
    Contract { contracts: Vec<ExecutableContract> },
}

pub struct StepResult {
    pub name: String,
    pub output: Option<String>,
    pub passed: bool,
    pub attempts: u32,
    pub duration_ms: u64,
    pub summary: Option<String>,
    pub artifact_dir: Option<PathBuf>,
    pub cost_usd: f64,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub verification: Vec<VerifyResult>,
}

pub struct StepPlan {
    pub steps: Vec<StepSpec>,
    pub source: PlanSource,
    pub replan_count: u32,
    pub playbook_name: Option<String>,
}

pub enum PlanSource {
    Static,      // user-provided explicit plan
    Dynamic,     // LLM-generated from prompt + playbook
    Fallback,    // playbook default_steps
    Replan,      // replanned after failure (opt-in)
}

pub struct HarnessConfig {
    pub failure_mode: FailureMode,
    pub max_replans: u32,             // only used if failure_mode is AutoReplan
    pub max_parallel_steps: u32,
    pub budget_cap: Option<f64>,
}

pub enum FailureMode {
    Escalate,   // default: stop and surface to user
    AutoReplan, // opt-in: replan up to max_replans, then escalate
    Retry,      // retry same step up to max_retries, then escalate
}
```

##### Playbooks

Playbooks are markdown files with YAML front matter that provide domain knowledge to the planner. They live per-workspace in `.panes/playbooks/`:

```markdown
---
name: backend-api
default_steps:
  - name: research
    step_type: agent
    intent: "Understand the existing API structure"
    modifies_files: false
  - name: implement
    step_type: agent
    intent: "Implement the requested change"
    depends_on: [research]
    verifiers:
      - type: shell
        command: "npm test"
  - name: review
    step_type: gate
    intent: "User reviews the changes"
    depends_on: [implement]
---

# Backend API Playbook

## Stack
- Express.js with TypeScript
- PostgreSQL via Prisma ORM
- Jest for testing

## Conventions
- All endpoints follow REST naming: /api/v1/{resource}
- Input validation uses Zod schemas
- Error responses use the ApiError class in src/errors.ts

## Common Pitfalls
- Always run `prisma generate` after schema changes
- The auth middleware expects a Bearer token, not a session cookie
```

The planner reads the playbook body as context when generating a dynamic plan. The `default_steps` serve as a fallback if dynamic planning fails.

##### Task Layer: Beads (Sidecar)

Beads owns the task graph — what needs doing, what depends on what, what's done. Panes owns execution — how tasks get done (worktrees, gates, cost, verification). They communicate via `bd --json` CLI calls from Rust.

**Sidecar architecture:**
```
Panes.app/
├── Contents/MacOS/
│   ├── Panes                          (main binary)
│   ├── dolt-aarch64-apple-darwin      (Tauri sidecar — version-controlled DB)
│   └── bd-aarch64-apple-darwin        (Tauri sidecar — task graph CLI)
```

On app start, Panes spawns `dolt sql-server` as a managed child process. `bd` commands connect to this Dolt instance. On app quit, Tauri kills the sidecar.

**Beads provides:**
- Dependency graph with four link types (`blocks`, `related`, `parent-child`, `discovered-from`) — only `blocks` affects the ready queue
- `bd ready` → unblocked tasks ready for execution
- `bd update <id> --claim` → atomic task claiming (prevents two agents grabbing the same task)
- Hash-based IDs (`bd-a3f8`) → no merge collisions in multi-agent workflows
- Dolt cell-level merge → concurrent agents can update different tasks without conflicts
- Compaction → semantic summarization of closed tasks for context efficiency
- `discovered-from` links → agents create new tasks mid-execution and link them to the parent

**Panes-side execution config:**

Beads tasks don't carry execution-specific fields (budget caps, gate policies, verification). These live in Panes' SQLite, keyed by beads task ID:

```rust
pub struct ExecutionConfig {
    pub beads_task_id: String,          // bd-a3f8
    pub workspace_id: String,
    pub budget_cap: Option<f64>,
    pub gate_policy: GatePolicy,
    pub verification: Option<VerifierConfig>,
    pub on_complete: TaskAction,
    pub on_failure: TaskAction,
    pub worktree_path: Option<PathBuf>, // set when dispatched
    pub thread_id: Option<String>,      // set when thread starts
}
```

**Execution flow:**
1. Planner LLM creates beads task breakdown via `bd create` + `bd dep add`
2. User reviews/refines in Panes UI (maps to `bd update` calls)
3. User sets execution config per task (budget, gate policy, verification)
4. Scheduler polls `bd ready --json` for unblocked tasks
5. For each ready task: create worktree, spawn thread via `SessionManager`
6. On completion: `bd close <id> --reason "summary"`, check for newly unblocked tasks
7. Agents can create discovered tasks mid-execution via `bd create` + `bd dep add --type discovered-from`
8. On failure: execute `on_failure` action (default: escalate to user)

**Why beads over petgraph:** The dependency DAG with ready-queue queries is ~200 lines with petgraph. But persistence, agent-writable interface, concurrent multi-agent access, cell-level merge, compaction, and atomic claiming add up fast. Beads has all of this tested and shipped, with 23k stars and 89 releases. The Dolt foundation means concurrent agents writing to the task graph don't conflict at the database level — something SQLite can't do.

##### Where the Harness Sits

```
Frontend
    │
    ├── Simple prompt ───────────→ Session Manager → Agent
    │   (classified as simple,     (single turn, no harness)
    │    skip harness)
    │
    ├── Autonomous task ─────────→ Harness ─────→ Session Manager → Agent
    │   (classified as complex,    (plan, execute,  (per-step sessions)
    │    or user requested)        verify, decide)
    │
    └── Flow ────────────────────→ Flow Engine → Harness(es) → Session Manager
        (multi-workspace,           (DAG across   (per-step     → Agent(s)
         multi-agent)               workspaces)   within each
                                                  workspace)
```

Three execution tiers:
1. **Simple prompt** — user sends a prompt, agent responds. No planning, no verification. Harness classifier detects simple tasks and skips the machinery.
2. **Autonomous task** — harness plans, executes steps, verifies, and on failure escalates to user (or auto-replans if opted in). Single workspace, single or multiple agents.
3. **Flow** — orchestrates multiple autonomous tasks across workspaces. Flow Engine manages the cross-workspace DAG; each step runs through a Harness instance for its own plan-execute-verify loop.

The user picks the tier implicitly based on how they frame the task, or explicitly via a "run as task" toggle in the UI. A simple prompt like "what's the current Node version?" goes through tier 1. A task like "add rate limiting to the API" goes through tier 2. A Flow defined in the UI or YAML goes through tier 3. Routines default to tier 2 with `FailureMode::AutoReplan`.

#### Flows and Routines

Flows are schedulable as Routines. A Routine can reference a Flow instead of a single prompt:

```
routines table (extended):
  id
  workspace_id   (null for cross-workspace flows)
  type           ("prompt" | "flow" | "autonomous")
  prompt         (for type="prompt" or "autonomous")
  flow_id        (for type="flow")
  playbook       (for type="autonomous")
  cron_expr
  budget_cap     (applies to entire flow/task)
  ...
```

---

## Internal Event Model

All agent backends (ACP and adapters) produce the same event type:

```rust
pub enum AgentEvent {
    /// Agent is reasoning / planning
    Thinking {
        text: String,
    },

    /// Agent produced text output
    Text {
        text: String,
    },

    /// Agent wants to use a tool (may or may not need approval)
    ToolRequest {
        id: String,
        tool_name: String,
        description: String,      // human-readable summary
        input: serde_json::Value, // raw tool input for detail view
        needs_approval: bool,
        risk_level: RiskLevel,
    },

    /// Tool execution completed
    ToolResult {
        id: String,
        tool_name: String,
        success: bool,
        output: String,           // human-readable summary
        raw_output: Option<String>, // full output for transcript
        duration_ms: u64,
    },

    /// Cost update from the agent
    CostUpdate {
        input_tokens: u64,
        output_tokens: u64,
        total_usd: f64,
        model: String,
    },

    /// Agent encountered an error
    Error {
        message: String,
        recoverable: bool,
    },

    /// Sub-agent spawned within this session
    SubAgentSpawned {
        parent_session_id: String,
        child_session_id: String,
        description: String,
    },

    /// Sub-agent completed
    SubAgentComplete {
        child_session_id: String,
        summary: String,
        cost_usd: f64,
    },

    /// Agent session completed
    Complete {
        summary: String,
        total_cost_usd: f64,
        duration_ms: u64,
        turns: u32,
    },
}

pub enum RiskLevel {
    Low,      // read operations, running tests
    Medium,   // creating/modifying files
    High,     // deleting files, running destructive commands
    Critical, // operations outside workspace, network access
}

/// Flow — a DAG of steps across agents and workspaces
pub struct Flow {
    pub id: String,
    pub name: String,
    pub steps: Vec<FlowStep>,
    pub edges: Vec<(String, String)>, // (from_step_id, to_step_id)
}

pub struct FlowStep {
    pub id: String,
    pub workspace_id: String,
    pub agent: String,
    pub prompt_template: String,      // may contain {{steps.<name>.<field>}}
    pub gate_required: bool,          // pause flow for approval before this step
    pub budget_cap: Option<f64>,
}

pub enum FlowStepStatus {
    Blocked,                          // waiting on upstream steps
    Ready,                            // all deps met, queued for execution
    Running { thread_id: String },
    Complete { summary: String, cost: f64 },
    Failed { error: String },
    Skipped,                          // user chose to skip after upstream failure
}
```

---

## Crate Structure

```
panes/
├── Cargo.toml                    (workspace root)
├── crates/
│   ├── panes-app/                (Tauri app entry point, IPC handlers)
│   ├── panes-core/               (Session manager, workspace config)
│   ├── panes-acp/                (ACP client wrapper, event translation)
│   ├── panes-adapters/           (Non-ACP agent adapters)
│   │   ├── claude/               (Claude Code CLI adapter)
│   │   └── ...                   (community adapters)
│   ├── panes-memory/             (Extraction, injection, storage)
│   ├── panes-harness/             (Plan → execute → verify → replan loop)
│   │                              (Step scheduling, playbooks, contracts,
│   │                               replanning, quality gates)
│   ├── panes-orchestrator/        (Flow engine, cross-workspace DAG,
│   │                               context templating between steps)
│   ├── panes-scheduler/          (Routines — cron scheduler, task chaining)
│   ├── panes-cost/               (Cost tracking, budget enforcement)
│   └── panes-events/             (AgentEvent enum, shared types)
└── frontend/                     (React app)
    ├── src/
    │   ├── components/
    │   │   ├── WorkspaceView/
    │   │   ├── Feed/
    │   │   ├── GateCard/
    │   │   ├── CompletionCard/
    │   │   ├── ThreadTimeline/
    │   │   ├── BranchView/
    │   │   ├── MemoryPanel/
    │   │   ├── BriefingEditor/
    │   │   ├── RoutinesManager/
    │   │   ├── FlowBuilder/
    │   │   └── CostTracker/
    │   ├── hooks/
    │   │   ├── useThread.ts      (subscribe to agent events)
    │   │   ├── useWorkspaces.ts
    │   │   └── useCost.ts
    │   └── lib/
    │       └── tauri.ts          (IPC bindings)
    └── package.json
```

---

## Key Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| App shell | Tauri 2 | Cross-platform, Rust backend, lighter than Electron, native webview |
| Backend language | Rust | Process management, async (Tokio), ACP crate exists, performance |
| Frontend framework | React | Largest ecosystem, fast iteration, team familiarity |
| ACP client | `agent-client-protocol` crate | Official Rust SDK, maintained by ACP project |
| Database | SQLite (via `rusqlite`) | Embedded, zero-config, sufficient for single-user desktop app |
| Memory backend | Mem0 (local sidecar) | Hybrid search (vector + graph), deduplication, conflict resolution. Fallback: SQLite FTS5. |
| Full-text search | SQLite FTS5 | Fallback memory retrieval, Briefing storage, general queries |
| Cron scheduling | `tokio-cron-scheduler` | Lightweight, async, runs in-process |
| IPC | Tauri events + commands | Built-in, typed, bidirectional |
| Memory extraction | Mem0 (primary), LLM prompt (fallback) | Mem0 handles extraction quality; LLM fallback ensures graceful degradation |
| Task DAG | Beads (`bd` CLI, Dolt-backed) | Distributed graph issue tracker with dependency-aware ready queue, cell-level merge via Dolt, hash-based IDs (no merge conflicts), four link types (blocks, related, parent-child, discovered-from). Bundled as Tauri sidecar alongside Dolt. Panes shells out to `bd --json` from Rust. Phase 2+. |
| Git worktrees | `git2` (libgit2 bindings) | Enables concurrent agent threads in the same repo via isolated working trees. git2 over CLI because swarms need: concurrent worktree creation with typed error handling, structured merge conflict detection (`IndexConflict` entries, three-way merge), and in-process status queries across N worktrees without per-worktree subprocess overhead. Phase 2+. |

---

## Knowledge Stack

Panes has three distinct persistence layers for agent context, each serving a different purpose:

```
┌─────────────────────────────────────────────────────────────┐
│                     Agent Context                            │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Briefing (always injected, deterministic)              │ │
│  │  User-authored workspace instructions. SQLite.          │ │
│  │  "Always use Zod. Run tests before committing."         │ │
│  └─────────────────────────────────────────────────────────┘ │
│                         +                                    │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Memory (relevance-ranked, extracted)                   │ │
│  │  Decisions, preferences, patterns from past threads.    │ │
│  │  Mem0 (vector + graph search). Workspace + global.      │ │
│  │  "We chose PostgreSQL over DynamoDB because..."         │ │
│  └─────────────────────────────────────────────────────────┘ │
│                         +                                    │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │  Tasks (structured, dependency-aware)        [Phase 2+] │ │
│  │  What needs doing, what blocks what, what's done.       │ │
│  │  Beads (Dolt-backed graph tracker, bundled as sidecar). │ │
│  │  LLM planner creates beads breakdown; scheduler reads   │ │
│  │  `bd ready`, dispatches into worktree threads.          │ │
│  └─────────────────────────────────────────────────────────┘ │
│                                                              │
│  Each layer answers a different question:                    │
│  Briefing → "How should you work here?"                     │
│  Memory   → "What do we already know?"                      │
│  Tasks    → "What are we doing and what depends on what?"   │
└─────────────────────────────────────────────────────────────┘
```

---

## Security Boundaries

```
┌─────────────────────────────────────────────┐
│  Panes Process (Rust)                       │
│                                             │
│  ┌───────────────────────────────────────┐  │
│  │  Workspace: backend                   │  │
│  │  Path: ~/projects/backend             │  │
│  │                                       │  │
│  │  ┌─────────────────────────────────┐  │  │
│  │  │  Agent subprocess               │  │  │
│  │  │  cwd: ~/projects/backend        │  │  │
│  │  │  ACP fs scope: ~/projects/backend│  │  │
│  │  │                                 │  │  │
│  │  │  Cannot access:                 │  │  │
│  │  │  - ~/projects/frontend          │  │  │
│  │  │  - ~/projects/infra             │  │  │
│  │  │  - ~/.ssh, ~/.aws, etc          │  │  │
│  │  └─────────────────────────────────┘  │  │
│  └───────────────────────────────────────┘  │
│                                             │
│  ┌───────────────────────────────────────┐  │
│  │  Workspace: frontend                  │  │
│  │  Path: ~/projects/frontend            │  │
│  │                                       │  │
│  │  ┌─────────────────────────────────┐  │  │
│  │  │  Agent subprocess               │  │  │
│  │  │  cwd: ~/projects/frontend       │  │  │
│  │  │  ACP fs scope: ~/projects/frontend│ │  │
│  │  │                                 │  │  │
│  │  │  Isolated from backend and infra│  │  │
│  │  └─────────────────────────────────┘  │  │
│  └───────────────────────────────────────┘  │
│                                             │
│  Panes SQLite DB: ~/Library/Application     │
│  Support/dev.panes/panes.db                 │
│  (memories, schedules, sessions, costs)     │
└─────────────────────────────────────────────┘
```

Workspace isolation is enforced by:
1. Setting the agent subprocess `cwd` to the workspace path
2. Using ACP's `fs` capability scoping (ACP agents only serve file requests within their declared scope)
3. For non-ACP adapters (Claude Code): using `--add-dir` to restrict access
4. Panes never passes cross-workspace paths to any agent

---

## Git Integration & Rollback

Panes is not a git client, but it must be git-aware. Every agent modifies files on disk. Without rollback, the trust layer is incomplete.

### Phase 1: Minimal Git Awareness

```
Thread starts
       │
       ▼
┌──────────────────────────────┐
│  Snapshot Manager            │
│                              │
│  1. Detect if workspace is   │
│     a git repo               │
│  2. If yes: create snapshot  │
│     git stash push -m        │
│     "panes:thread:{id}:pre"  │
│     git stash pop            │
│     (records the stash ref)  │
│                              │
│     OR: lightweight approach │
│     git rev-parse HEAD →     │
│     store commit hash as     │
│     rollback point           │
│  3. If not a git repo:       │
│     skip snapshot, no        │
│     rollback available       │
└──────────────────────────────┘

Thread completes
       │
       ▼
┌──────────────────────────────┐
│  Completion Card Actions     │
│                              │
│  [Commit changes]            │
│    → Opens minimal commit    │
│    dialog: auto-generated    │
│    message from thread       │
│    summary. User can edit.   │
│    Runs: git add -A &&       │
│    git commit -m "..."       │
│                              │
│  [Revert all changes]        │
│    → Restores to snapshot:   │
│    git checkout . &&         │
│    git clean -fd             │
│    (back to pre-thread       │
│    commit state)             │
│                              │
│  [Keep uncommitted]          │
│    → Default. Changes stay   │
│    on disk, user handles     │
│    git themselves.           │
└──────────────────────────────┘
```

### Phase 2: Git Worktrees (Concurrent Threads)

Phase 1 enforces one thread per workspace — safe but limits throughput. Phase 2 lifts this by giving each concurrent thread its own git worktree: an isolated checkout of the same repo at a separate filesystem path.

```
Workspace: ~/projects/backend    (main working tree)

Thread A starts:
  git worktree add /tmp/panes-wt-{thread_a_id} -b panes/{thread_a_id}
  → Agent runs in /tmp/panes-wt-{thread_a_id}
  → Isolated: changes don't affect main tree or other threads

Thread B starts (concurrent, same repo):
  git worktree add /tmp/panes-wt-{thread_b_id} -b panes/{thread_b_id}
  → Agent runs in /tmp/panes-wt-{thread_b_id}

Thread A completes:
  → User reviews changes in worktree
  → [Merge to main] or [Discard]
  → git worktree remove /tmp/panes-wt-{thread_a_id}
```

**Worktree lifecycle (via git2):**
1. `create_worktree(workspace_path, thread_id)` → `Repository::worktree()` creates worktree + branch, returns worktree path
2. Thread runs in worktree path (agent cwd = worktree path)
3. On completion: user chooses merge strategy (merge, rebase, cherry-pick, discard)
4. `cleanup_worktree(worktree_path)` → removes worktree + optionally deletes branch

**Why git2 over CLI for worktrees:** Phase 1 uses CLI for sequential git operations (snapshot, revert, commit) — adequate when one thread at a time. Swarm execution makes git operations concurrent and conflict-prone: multiple worktree creates racing on lock files, merge conflict detection across N completed threads, changed-file overlap queries before merging. git2 provides structured error types, `repo.merge_analysis()`, `IndexConflict` entries, and in-process queries without per-worktree subprocess overhead. Phase 1 CLI calls can migrate to git2 later but don't need to.

**Merge conflicts:** When merging worktree results back to main, conflicts are possible if multiple threads touch overlapping files. git2's three-way merge primitives and `IndexConflict` entries let Panes detect and surface conflicts structurally. User is offered: resolve manually, keep one side, or discard the conflicting thread's changes.

### Swarm Execution: Beads + Worktrees Integrated Flow

Beads (task graph) and git worktrees (file isolation) are independent systems that Panes orchestrates together. Beads never touches git, git never touches Dolt. The separation is clean:

```
┌─────────────────────────────────────────────────────────────────┐
│                   Panes Swarm Architecture                       │
│                                                                  │
│  ┌────────────────────────────────┐                             │
│  │  Dolt Server (sidecar process) │  ← single instance,        │
│  │  ┌──────────────────────────┐  │    all agents share it     │
│  │  │  Beads task graph        │  │                             │
│  │  │                          │  │                             │
│  │  │  bd-a3f8: Add endpoint   │  │                             │
│  │  │  bd-b7c1: Add UI page   │──│── blocks ──→ bd-a3f8       │
│  │  │  bd-c2d9: Security scan │──│── blocks ──→ bd-a3f8       │
│  │  │  bd-d4e2: Perf test     │──│── blocks ──→ bd-b7c1       │
│  │  └──────────────────────────┘  │                             │
│  └──────────────┬─────────────────┘                             │
│                 │ bd ready --json / bd close / bd create         │
│                 │                                                │
│  ┌──────────────▼─────────────────┐                             │
│  │  Panes Scheduler               │                             │
│  │                                │                             │
│  │  1. Poll bd ready --json       │                             │
│  │  2. Claim: bd update --claim   │                             │
│  │  3. Look up ExecutionConfig    │                             │
│  │  4. Create worktree (git2)     │                             │
│  │  5. Spawn agent thread         │                             │
│  │  6. On complete: bd close,     │                             │
│  │     merge worktree, re-poll    │                             │
│  └──────┬────────┬────────┬───────┘                             │
│         │        │        │                                      │
│  ┌──────▼──┐ ┌──▼─────┐ ┌▼────────┐                            │
│  │Worktree │ │Worktree│ │Worktree │  ← isolated git checkouts  │
│  │   A     │ │   B    │ │   C     │                             │
│  │         │ │        │ │         │                             │
│  │Agent 1  │ │Agent 2 │ │Agent 3  │                             │
│  │bd-a3f8  │ │bd-c2d9 │ │(idle)   │                             │
│  │         │ │        │ │         │                             │
│  │cwd:     │ │cwd:    │ │cwd:     │                             │
│  │/tmp/    │ │/tmp/   │ │/tmp/    │                             │
│  │panes-wt-│ │panes-wt│ │panes-wt-│                             │
│  │{a_id}   │ │{b_id}  │ │{c_id}   │                             │
│  └─────────┘ └────────┘ └─────────┘                             │
│                                                                  │
│  Main working tree: ~/projects/backend (untouched during swarm) │
└─────────────────────────────────────────────────────────────────┘
```

**Full swarm cycle:**

```
Phase: PLAN
  1. Planner agent creates beads breakdown:
     bd create "Add /users/:id endpoint" -t task -p 1
     bd create "Add profile UI page" -t task -p 2
     bd dep add bd-b7c1 bd-a3f8 --type blocks
     ...
  2. User reviews tasks in Panes UI
  3. User sets ExecutionConfig per task (budget, gate policy, verification)
  4. User approves plan

Phase: EXECUTE (scheduler loop)
  5. Scheduler: bd ready --json → [bd-a3f8, bd-c2d9]  (no deps)
  6. For each ready task:
     a. bd update bd-a3f8 --claim           (atomic — prevents double-dispatch)
     b. git2: create worktree + branch      (isolated checkout)
     c. SessionManager: spawn agent thread  (cwd = worktree path)
     d. Panes gates, cost tracking, budget caps apply normally

Phase: COMPLETE (per task)
  7. Agent finishes in worktree A
  8. Panes runs verification (if configured)
  9. bd close bd-a3f8 --reason "Added GET /users/:id with Zod validation"
  10. Scheduler re-polls: bd ready --json → [bd-b7c1]  (newly unblocked)
  11. User reviews worktree A changes:
      [Merge to main] [Rebase] [Cherry-pick] [Discard]

Phase: DISCOVERY (mid-execution)
  12. Agent in worktree B discovers a missing migration while working:
      bd create "Add users table migration" -t bug -p 1
      bd dep add bd-e5f3 bd-c2d9 --type discovered-from
  13. Scheduler sees bd-e5f3 on next poll, dispatches worktree D

Phase: DONE
  14. bd ready --json → []  (no more unblocked tasks)
  15. All worktrees merged or discarded
  16. Swarm complete — Panes shows aggregate cost, per-task results
```

**Why this separation works:**
- Beads state lives in Dolt server (sidecar), not in `.beads/` on disk — worktrees don't copy or conflict on task data
- `--claim` is atomic at the Dolt level — no double-dispatch even with concurrent scheduler polls
- Dolt cell-level merge means two agents closing different tasks simultaneously don't conflict
- Git worktrees isolate file changes per agent — no filesystem conflicts
- Each layer is independently testable: beads with `bd` commands, worktrees with `git2`, execution with the existing `SessionManager` + `FakeAdapter`

### Constraints

- **One active thread per workspace** (Phase 1). Prevents concurrent agents from creating conflicting file changes. Phase 2 lifts this via git worktrees — each concurrent thread gets its own isolated checkout.
- **Rollback only works in git repos.** Non-git workspaces get a warning: "Changes cannot be reverted — this workspace is not a git repository."
- **Rollback is all-or-nothing.** No partial revert (that's a git UI, which Panes is not). Either keep all changes or revert all.
- **Worktree limit** (Phase 2). Max concurrent worktrees per workspace is configurable (default: 4). Prevents runaway swarms from exhausting disk space or git lock contention.

---

## Process Lifecycle

### Sidecar Management (Phase 2)

```
┌──────────────────────────────────────────────────────┐
│  Sidecar Lifecycle                                    │
│                                                       │
│  App Start:                                           │
│  1. Spawn dolt sql-server (Unix socket)               │
│     - Managed by Tauri sidecar API                    │
│     - Data dir: ~/Library/Application Support/         │
│       dev.panes/dolt/                                 │
│  2. Health-check: wait for socket ready               │
│  3. bd commands connect via BEADS_DOLT_HOST env var   │
│                                                       │
│  Runtime:                                              │
│  - Dolt server stays up for app lifetime              │
│  - bd commands are short-lived (spawn, run, exit)     │
│  - Panes wraps bd calls in a BeadsClient struct       │
│    that handles JSON parsing and error mapping        │
│                                                       │
│  App Quit:                                             │
│  1. Tauri kills sidecar process group                 │
│  2. Dolt WAL is flushed on clean shutdown             │
│                                                       │
│  Crash Recovery:                                       │
│  - Dolt uses WAL mode, recovers on next start         │
│  - Orphaned worktrees cleaned up via                  │
│    git worktree prune on app start                    │
└──────────────────────────────────────────────────────┘
```

### Agent Process Management

```
┌──────────────────────────────────────────────────────┐
│  Process Pool                                         │
│                                                       │
│  Spawn:                                               │
│  1. Create new process group (setsid on macOS/Linux,  │
│     Job Object on Windows)                            │
│  2. Set cwd to workspace path                         │
│  3. Pipe stdin/stdout for event streaming              │
│  4. Store PGID/Job handle for cleanup                  │
│                                                       │
│  Shutdown:                                             │
│  1. Send SIGTERM to process group (-PGID)             │
│  2. Wait 5s for graceful exit                         │
│  3. Send SIGKILL if still alive                       │
│  4. Cleans up child processes (npm, cargo, etc.)      │
│                                                       │
│  Crash recovery:                                       │
│  1. Detect unexpected process exit                    │
│  2. Mark thread as "interrupted"                      │
│  3. Surface in UI: "Agent process exited unexpectedly. │
│     [Retry] [View partial results]"                   │
│                                                       │
│  Suspend/Resume (laptop sleep):                        │
│  1. On wake, health-check active sessions             │
│  2. If process alive: continue normally               │
│  3. If process died: mark as interrupted              │
└──────────────────────────────────────────────────────┘
```

### Claude Code Adapter Specifics

```
Spawn command:
  claude -p \
    --output-format stream-json \
    --verbose \
    --input-format stream-json \
    --permission-mode acceptEdits

Permission handling:
  - acceptEdits: file creates/edits auto-approved
  - Bash commands: intercepted as ToolRequest events,
    presented as gates in the UI
  - User approval injected back via stdin stream-json

Parser requirements:
  - Forward-compatible: ignore unknown type/subtype fields
  - Sub-agent detection: parse parent_tool_use_id for
    branch rendering (best-effort, graceful degradation)
  - Auth detection: pattern-match stderr for auth errors,
    surface as first-class UI guidance
```
