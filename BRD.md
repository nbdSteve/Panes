# Panes — Business Requirements Document

## 1. Problem Statement

AI coding agents can now build entire features autonomously. But they also delete production databases in 9 seconds, wipe drives with a single misplaced command, and introduce security vulnerabilities that look professional. These incidents make the front page of Reddit with 30K+ upvotes every few weeks.

The people directing these agents — both developers and non-developers — face three compounding problems, validated by demand research across ~100K-subscriber communities:

1. **Destruction without recourse.** Agents take destructive actions that users approve without understanding. There is no undo button. When something breaks, the user is stuck — they either know git well enough to recover, or they pay someone $600+ to clean up the mess. *(Signal: Very Strong — the most emotionally charged topic in AI coding)*
2. **Amnesia.** Every new conversation starts from zero. By month 2, the agent "fights" the user by re-introducing rejected patterns. By month 3, the codebase is an incoherent mess that works but can't be maintained. *(Signal: Strong — universal complaint across all skill levels)*
3. **Cost blindness.** Users are blindsided by AI coding bills, confused by pricing changes, and have no aggregate view of spending. One user manually audited 926 sessions to understand their own token waste. *(Signal: Strong — pervasive and under-addressed)*

No existing tool solves all three. Every AI coding tool — IDEs, terminals, cloud dashboards — presents agent actions as diffs and shell commands, assumes the user can evaluate what they mean, and offers no safety net when things go wrong.

## 2. Product Vision

Panes is the safety layer for AI coding agents. It makes agent work legible, reversible, and cost-visible — so users can direct agents confidently without reading every line they write.

The core loop is: **dispatch work → monitor progress → decide at gates → review results → commit or revert.**

Panes is not an editor, not an agent, and not a workflow builder. It is a client that wraps any agent with safety (gates + rollback), memory (context persistence + Briefings), and cost visibility (per-thread tracking + budget caps).

## 3. Target Users

### Primary: Anyone directing AI coding agents who has been burned — or fears being burned

This includes technical founders, product managers, tech leads, designers, and developers. What unites them isn't their skill level — it's that they've experienced (or will experience) the core failure modes:

- Approved an agent action they didn't understand and it broke something
- Lost work because there was no undo
- Re-explained the same conventions to an AI for the third time
- Got blindsided by an unexpected AI coding bill
- Watched their codebase become an unmaintainable mess after months of AI sessions

### Secondary: Power Developers

Experienced developers managing multiple repos who want faster orchestration than juggling terminal sessions. They discover Panes for the multi-workspace dispatch and stay for the memory and cost tracking.

## 4. Success Criteria

### Beta (Phase 1)

| Metric | Target |
|--------|--------|
| Users feel safe directing agents (measured by: willingness to try risky tasks, rollback used < 20% of threads) | >70% report feeling "safe" in user testing |
| Gate engagement is meaningful (users read and make decisions, not rubber-stamp) | >50% of gates result in steer or reject (not just approve-all) |
| Memory reduces re-explaining on repeat tasks | User-reported reduction in context re-setting |
| Time from install to first completed thread | <5 minutes |
| Daily active users retain after 2 weeks | >30% |

### Post-Beta (Phase 2+)

| Metric | Target |
|--------|--------|
| Routine completion rate (scheduled tasks succeed without intervention) | >85% |
| Community-contributed agent adapters | ≥3 within 3 months of open-source launch |
| Users running ≥2 workspaces concurrently | >50% of active users |

## 5. Requirements by Phase

### Phase 1: Safety + Memory (Beta)

**Goal:** Prove that users feel safe directing AI agents through Panes and that memory compounds over time. The core loop is: dispatch → monitor → gate → review → commit or revert.

Ship: Safety layer (gates, rollback, risk classification), Memory (extraction, Briefings, memory panel), Cost visibility, Multi-workspace dashboard.

| ID | Requirement | Priority | Notes |
|----|------------|----------|-------|
| P1.1 | **Workspaces.** User can add workspaces by selecting a folder. Sidebar lists all workspaces with status (idle, working, gate, error). | Must | Core navigation surface. |
| P1.2 | **Thread creation.** User types a prompt, selects an agent, sends. A thread appears showing agent progress as step cards. | Must | The primary interaction. |
| P1.3 | **Agent selector.** Prompt bar includes agent picker. Default agent is remembered per workspace. Per-message override supported. | Must | |
| P1.4 | **ACP client.** Connect to any ACP-compatible agent via the `agent-client-protocol` crate. Session lifecycle: create, prompt, cancel. | Must | Primary agent integration path. |
| P1.5 | **Claude Code adapter.** Spawn `claude -p --output-format stream-json --verbose`, parse events into AgentEvent model. | Must | Most users will start with Claude Code. |
| P1.6 | **AgentEvent model.** Unified event enum (Thinking, Text, ToolRequest, ToolResult, CostUpdate, Error, SubAgentSpawned, SubAgentComplete, Complete). All adapters produce these. | Must | Foundation for all UI rendering. |
| P1.7 | **Step cards.** Render agent actions as collapsible step cards in the thread. Show tool name, human-readable description, elapsed time. | Must | Primary feedback surface. |
| P1.8 | **Gates.** When agent emits a tool request with `needs_approval: true`, pause and render a gate card: action summary, risk level, running cost. User can approve, reject, or steer. | Must | Core trust mechanism. |
| P1.9 | **Risk classification.** Classify tool requests as Low/Medium/High/Critical based on tool type and parameters. Surface in gate cards. | Must | Non-dev users need risk context. |
| P1.10 | **Thread completion.** On agent completion, show summary card: what changed, files affected, test results (if available), total cost, duration. | Must | |
| P1.11 | **Timeline view.** Expandable ordered list of every step with timing. One click deeper from the summary. | Must | Progressive disclosure layer 2. |
| P1.12 | **Transcript view.** Full raw conversation between user and agent, including reasoning. Two clicks deeper. | Should | Power users and debugging. |
| P1.13 | **Branch rendering.** When agent spawns sub-agents, render as collapsible nested sections (branches) in the timeline. | Should | Agents like Claude Code use sub-agents frequently. |
| P1.14 | **Memory extraction.** After thread completion, call Mem0 (or equivalent) to extract decisions, preferences, constraints, patterns. Store per-workspace and global. | Must | Core differentiator. |
| P1.15 | **Memory injection.** Before each new thread, retrieve relevant memories via FTS + recency ranking. Prepend to agent context within token budget. | Must | |
| P1.16 | **Memory panel.** UI to view, edit, delete, and pin memories. Organized by workspace and global scope. Show source thread for each memory. | Must | Trust through transparency. |
| P1.17 | **Briefings.** Per-workspace text field for persistent user instructions. Always injected into every thread's context. Editable from memory panel. | Must | Simpler than memory, immediately useful. |
| P1.18 | **Context indicator.** Thread shows "Using N memories · 1 briefing" with expandable detail of what was injected. | Should | Builds trust in memory system. |
| P1.19 | **Feed.** Aggregated activity stream showing completed threads across all workspaces. Each item: workspace name, summary, cost, timestamp. | Must | The morning review surface. |
| P1.20 | **Cost tracking.** Track cost per-thread, per-workspace, aggregate. Display running cost in active threads and totals in completion cards and Feed. | Must | Cost awareness is a product principle. |
| P1.21 | **Per-workspace budget caps.** Optional spending limit per workspace. Warn on approach, kill session on exceed. | Should | Safety rail. |
| P1.22 | **SQLite persistence.** All data (workspaces, threads, events, memories, briefings, costs) stored in local SQLite database. | Must | |
| P1.23 | **Tauri app shell.** Desktop app using Tauri 2. React frontend, Rust backend, Tauri IPC. macOS first. | Must | |
| P1.24 | **Multi-workspace concurrency.** Multiple threads can run simultaneously across different workspaces. Sidebar shows live status for all. | Must | Key differentiator over single-workspace tools. |
| P1.25 | **Workspace isolation.** Agent processes scoped to workspace directory. No cross-workspace file access. ACP fs scoping + adapter-level enforcement. | Must | Security boundary. |
| P1.26 | **Tags.** Apply tags to threads for cross-cutting organization and filtering. | Nice | Can be post-beta if needed. |
| P1.27 | **One active thread per workspace.** Prevent concurrent threads in the same workspace. Show clear message and option to queue. | Must | Avoids file conflicts without harness serialization. Lifted in Phase 3. |
| P1.28 | **Pre-thread git snapshot.** Before a thread starts, create a lightweight git snapshot (stash or temp branch) of the workspace state. | Must | Enables rollback. |
| P1.29 | **Revert changes.** Completion card includes "Revert all changes" button that restores the pre-thread snapshot. | Must | Core trust mechanism — users can undo bad agent work. |
| P1.30 | **Commit changes.** Completion card includes "Commit" button with auto-generated commit message from the thread summary. | Should | Prevents non-dev users from needing to open a terminal for git. |
| P1.31 | **Process group management.** Spawn agent processes in their own process group. On Panes shutdown, SIGTERM the group to prevent orphan processes. | Must | Prevents resource leaks and zombie processes. |
| P1.32 | **Event batching.** Batch agent events on the Rust side (50-100ms window) before emitting to frontend. | Must | Prevents UI lag during active agent streaming. |
| P1.33 | **Forward-compatible event parser.** Claude Code adapter must ignore unknown event types/subtypes without failing. | Must | Claude Code ships breaking changes weekly. Parser must be resilient. |
| P1.34 | **Auth failure detection.** Adapter detects auth-specific errors (expired token, missing API key) and surfaces as clear UI guidance, not raw stderr. | Must | First-time experience depends on this. |

### Phase 2: Automation

**Goal:** Panes works while you're not watching.

Ship: Routines, improved memory, adapter ecosystem.

| ID | Requirement | Priority | Notes |
|----|------------|----------|-------|
| P2.1 | **Routines.** User creates recurring prompts: workspace, prompt, cron schedule, budget cap, on-complete/on-failure actions. | Must | The "works while you sleep" feature. |
| P2.2 | **Routine execution.** Scheduler fires routines via Session Manager. Threads marked as `is_routine=true`. Results appear in Feed with routine badge. | Must | |
| P2.3 | **Routine gates.** If a routine thread hits a gate, it pauses and notifies the user. User can approve from Feed. | Must | Routines can't be fully autonomous without approval handling. |
| P2.4 | **Routine chaining.** On-complete/on-failure can trigger follow-up prompts in same or different workspace. | Should | |
| P2.5 | **Routine cost history.** Each routine tracks cost over time. Dashboard shows per-routine and aggregate spend. | Should | |
| P2.6 | **Per-routine budget caps.** Hard limit per execution. Kill thread on exceed, log to Feed. | Must | Safety for unattended execution. |
| P2.7 | **System notifications.** Notify user when routine completes, fails, or hits a gate. Native OS notifications. | Must | User isn't watching, needs alerts. |
| P2.8 | **Adapter contribution guide.** Document the AgentAdapter trait. Publish as open-source with example adapter. | Must | Community growth. |
| P2.9 | **Beads integration.** Expose Beads MCP server to agent sessions. Agents working in Panes can read/write structured tasks, track dependencies, and coordinate via Beads' task graph. Panes manages the Beads/Dolt instance lifecycle (start on launch, stop on quit). | Must | Beads validated locally. Provides structured task tracking and inter-agent coordination that complements Mem0's memory layer. |
| P2.10 | **Memory quality improvements.** Deduplication, conflict detection (contradictory memories), memory decay (reduce weight of old memories). | Should | Memory gets noisy with volume. |

### Phase 3: Orchestration

**Goal:** Coordinate work across workspaces and agents.

Ship: Flows, Harness, Playbooks.

| ID | Requirement | Priority | Notes |
|----|------------|----------|-------|
| P3.1 | **Flows.** User defines multi-step, cross-workspace DAGs: steps with workspace, agent, prompt template, dependencies, gate requirements, budget caps. | Must | Power-user feature for cross-repo coordination. |
| P3.2 | **Flow engine.** Topological sort, parallel execution of independent steps, context templating (`{{steps.<name>.summary}}`), failure handling (block downstream, user decides retry/skip/abort). | Must | |
| P3.3 | **Flow UI.** Mini DAG visualization in sidebar. Per-step status, cost, click-through to thread. Completion summary in Feed. | Must | |
| P3.4 | **Harness.** Plan → execute → verify → decide loop for autonomous task execution within a single workspace. Complexity classifier skips harness for simple prompts. | Should | Adds structured execution for complex tasks. |
| P3.5 | **Three step types.** Agent (default), Shell (direct command), Gate (approval pause). | Should | |
| P3.6 | **Pluggable verifiers.** ShellVerifier, LlmVerifier, ScreenshotVerifier, ContractVerifier. Verifier trait for custom implementations. | Should | |
| P3.7 | **Playbooks.** Per-workspace markdown + YAML front matter with domain knowledge and default steps. Planner uses playbook context for dynamic plan generation. | Nice | Advanced configuration for power users. |
| P3.8 | **Escalate-first failure.** Default: stop and surface to user. Auto-replan opt-in for Routines and explicit user request. | Must | Matches the "user decides" philosophy. |
| P3.9 | **Schedulable Flows.** Routines can reference a Flow instead of a single prompt. | Should | Enables recurring cross-workspace automation. |
| P3.10 | **Beads-backed Flows.** Use Beads' dependency-aware task graph as the coordination layer for Flow steps. Flow steps map to Beads tasks with dependency edges. Agents within a Flow read task context from Beads rather than only relying on prompt templating. | Should | Replaces custom DAG tracking with a battle-tested multi-agent-safe system. |

### Phase 4: Scale & Teams

**Goal:** Multi-user, hosted features, premium tier.

| ID | Requirement | Priority | Notes |
|----|------------|----------|-------|
| P4.1 | **Hosted Routines.** Routines run on server infrastructure, no desktop required. | Must | Premium feature, solves "laptop must be open" problem. |
| P4.2 | **Team workspaces.** Shared workspaces with shared memory and Briefings. Multiple team members see the same Feed. | Must | |
| P4.3 | **Role-based gates.** Certain gate types routed to specific team members based on risk level or workspace. | Should | |
| P4.4 | **Advanced memory.** Dedicated extraction models (beyond LLM prompting). Cross-workspace memory intelligence. | Should | This is where the moat develops. |
| P4.5 | **Windows and Linux.** Cross-platform Tauri builds. | Must | |
| P4.6 | **Audit log.** Full history of agent actions, approvals, and costs for compliance. | Should | Enterprise requirement. |

## 6. Architecture Constraints

These constraints ensure that Phase 1 code is extensible for later phases without premature engineering.

| Constraint | Rationale | Phase 1 Impact |
|-----------|-----------|----------------|
| **AgentEvent is the universal event model.** All agent output flows through this enum. | Phases 2-3 add Routines and Flows. Both consume the same events. No agent-specific rendering paths. | Define the full enum in Phase 1 even if some variants (SubAgentSpawned) are rarely used. |
| **AgentAdapter trait is the primary integration boundary.** All agents — including ACP agents — go through this trait. ACP is one adapter implementation, not a privileged path. | Claude Code (the most important agent) does not speak ACP. The ACP crate is pre-1.0 with breaking changes. Adapters are primary, ACP is secondary. | Design the trait for the Claude Code adapter (stream-json). ACP adapter wraps the `agent-client-protocol` crate behind the same trait. |
| **Memory is a pluggable backend.** The memory engine has a trait boundary between extraction/injection logic and storage. | Phase 1 uses Mem0. Phase 4 may use custom models. Storage may move from SQLite to a hosted service for teams. | Code to the trait, not the implementation. |
| **Threads table supports Routine and Flow metadata.** `is_routine`, `flow_id`, `flow_step` columns exist from Phase 1. | Avoid schema migrations when adding Routines and Flows. | Columns are nullable, unused in Phase 1 but present. |
| **Crate boundaries match phase boundaries.** `panes-scheduler`, `panes-orchestrator`, `panes-harness` are separate crates. | Phases 2-3 add crates without restructuring existing code. | Phase 1 ships `panes-app`, `panes-core`, `panes-acp`, `panes-adapters`, `panes-memory`, `panes-events`, `panes-cost`. Other crates exist as stubs or are unimplemented. |
| **Frontend components are feature-flagged, not absent.** RoutinesManager, FlowBuilder exist as placeholder components. | Prevents large UI restructuring in later phases. | Placeholder components with "Coming soon" or hidden behind feature flags. |

## 7. External Dependencies

| Dependency | What it provides | Risk | Mitigation |
|-----------|-----------------|------|------------|
| **ACP (Agent Client Protocol)** | Standard agent communication protocol. Rust crate. | Pre-1.0 (v0.12), could change. | Adapter layer means non-ACP agents still work. Abstract ACP usage behind internal trait. |
| **Mem0** | Memory extraction, storage, and retrieval. | Python library — needs sidecar process or API. | Evaluate Rust-native alternatives. Worst case: embed Python via PyO3 or run as local HTTP service. If Mem0 doesn't work, fall back to LLM prompt extraction + SQLite FTS (which is already in the architecture). |
| **Beads** | Structured task graph for multi-agent coordination. MCP wrapper available. | Young project, adds Dolt to the runtime. | Validated locally — Dolt runs fine on desktop. Panes manages Dolt lifecycle. MCP wrapper means agents opt into Beads without deep coupling. |
| **Tauri 2** | Desktop app shell. | Active, well-maintained. Low risk. | Standard choice for Rust desktop apps. |
| **Claude Code CLI** | Primary agent backend for many users. | Anthropic controls the CLI interface. stream-json format could change. | Adapter isolates Panes from CLI changes. Pin to tested CLI versions. |

## 8. Mem0 Integration Design

Mem0 is the planned memory backend for Phase 1. Key integration points:

**What Mem0 provides:**
- Automatic extraction from conversation transcripts
- Hybrid search (vector + graph) for retrieval
- Deduplication and conflict resolution
- Multi-level scoping (user, session, agent — maps to global, workspace)
- Python SDK and REST API

**Integration approach:**
- Bundle Mem0 as a PyInstaller sidecar binary (platform-specific target-triple naming per Tauri convention: `mem0-sidecar-aarch64-apple-darwin`, etc.)
- Configure Mem0 with **`fastembed` for local CPU embeddings** (~200MB model download on first run). This avoids requiring an OpenAI API key for embeddings and keeps conversation data on the user's machine.
- Mem0's LLM extraction step routes through the user's existing API provider (Anthropic, OpenAI, etc.) — the only step that calls an external API.
- Qdrant runs in **embedded local mode** (no separate server). Writes to `~/Library/Application Support/dev.panes/qdrant/`.
- Panes-memory crate calls Mem0's REST API for extraction (post-thread) and retrieval (pre-thread)
- Briefings are NOT stored in Mem0 — they're user-authored, stored in SQLite, always injected verbatim
- Memory panel reads from Mem0 for display; edits/deletes are forwarded to Mem0 API
- If Mem0 sidecar is unavailable or user opts out, fall back to simple LLM extraction + SQLite FTS

**Why not build from scratch:**
- Memory extraction quality is hard. Mem0 has solved deduplication, conflict detection, and relevance ranking.
- Mem0's hybrid search (combining semantic similarity with graph relationships) outperforms pure FTS for memory retrieval.
- Building our own would delay Phase 1 by weeks with no differentiation — the UX on top of memory is the differentiator, not the extraction engine.

**Why not use Mem0 for everything:**
- Briefings are deterministic (always inject, user-controlled) — Mem0's relevance scoring would be wrong here.
- Cost data and thread metadata belong in SQLite for fast querying and aggregation.
- If Mem0 ever becomes unavailable or changes pricing, the fallback path must work.

**Data residency note:** With fastembed, embeddings are computed locally. Conversation content only leaves the machine during the LLM extraction step (which uses the user's own API key). This should be disclosed clearly in the privacy documentation. Phase 4 can offer fully local extraction via a small local model for enterprise users.

## 9. Implementation Risks & Gotchas

### Tier 1: Must resolve before Phase 1 ships

| # | Risk | Impact | Mitigation |
|---|------|--------|------------|
| 1 | **Claude Code does not speak ACP.** Its native protocol is `stream-json` over stdin/stdout. The ACP crate is pre-1.0 with breaking changes every few months (0.9→0.11 in 3 months). | The adapter layer is the primary integration path, not the fallback. Architecture docs assumed ACP-first. | Invert the architecture: adapters are primary for Phase 1. ACP is secondary, used only for agents that natively support it. Do not depend on ACP crate stability. |
| 2 | **stream-json format is undocumented and ships breaking changes weekly.** Claude Code releases every 1-2 days. New event types appear without notice. | Adapter breaks silently on any Claude Code update. | Parser must be forward-compatible: ignore unknown `type`/`subtype` fields. Pin to tested Claude Code versions in CI. |
| 3 | **Mem0 requires OpenAI API key for embeddings by default.** Uses `text-embedding-3-small`. User with only an Anthropic key can't use memory. | Breaks "bring your own key" story. Adds friction to onboarding. | Configure Mem0 with `fastembed` (local CPU embeddings, ~200MB model download). Only use LLM API for extraction step, routed through user's existing provider. |
| 4 | **Mem0 is Python. Panes is Rust/Tauri.** Cannot assume Python installed. Bundling Python adds ~100MB+ to app size. | First-time experience breaks if Python isn't installed. Packaging is complex. | Bundle Mem0 as a PyInstaller sidecar binary with platform-specific target-triple naming per Tauri's sidecar convention. Qdrant runs embedded (local mode, no separate server). |
| 5 | **No git awareness.** Thread completes, user sees "Files changed: 2" — then what? Non-dev users don't know git. | Breaks the "never leave Panes" promise. Users context-switch to terminal for every commit. | Add minimal git UX to completion cards: "Commit changes" button, pre-thread snapshot for "Revert changes." Not a full git panel — just commit and rollback. |
| 6 | **No rollback/undo.** Approve at gate, agent finishes, result is wrong. No recovery path. | Trust layer is incomplete. Users can approve but can't recover from bad approvals. | Before each thread starts, create a git stash or lightweight branch snapshot. Completion card gets "Revert all changes" button that restores the snapshot. |
| 7 | **Concurrent threads in same workspace.** Two threads writing to the same files = conflicts. Phase 1 has no harness serialization. | Data corruption, confused agent state. | Phase 1 constraint: one active thread per workspace. Show clear message: "This workspace has an active thread. Queue this or wait." Lift in Phase 3 with harness serialization. |
| 8 | **Orphan processes on exit.** Agent spawns child processes (npm install, cargo build). Killing the agent doesn't kill its children. | Resource leaks, zombie processes after Panes quits. | Spawn each agent via `setsid` (macOS/Linux) for its own process group. On shutdown, `kill(-pgid, SIGTERM)`. On Windows, use Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. |
| 9 | **Claude Code permission loop in non-interactive mode.** With `--permission-mode default`, every file write triggers a permission denial that the agent retries infinitely. | Infinite retry loops, wasted cost, hung threads. | Use `--permission-mode acceptEdits` as baseline. Intercept `tool_use` events for bash commands and present as gates. Inject approval via `--input-format stream-json` on stdin. |

### Tier 2: Must resolve before GA

| # | Risk | Impact | Mitigation |
|---|------|--------|------------|
| 10 | **Cost tracking is agent-dependent.** Claude Code emits cost. Other agents may not. | "Cost always visible" principle has holes. | Estimate cost from token counts when agent doesn't report. Show "Estimated" badge vs "Reported" to maintain trust. |
| 11 | **Privacy / data residency for memory.** Mem0 extraction step calls an LLM API. Conversation content leaves the machine. | Trust violation for "data stays local" users. Enterprise blocker. | Disclose data flow clearly. Phase 4: offer fully local extraction via small local model. |
| 12 | **"Approve for this thread" scoping is ambiguous.** What does "similar actions" mean? | Users surprised by auto-approvals they didn't intend. | Scope to: same tool type + same or lower risk level. "Approve file creates" doesn't auto-approve file deletes. Document scoping rules in gate UI. |
| 13 | **Agent auth failures surface as cryptic errors.** Token expiry, missing API key, etc. | First-time experience is a mysterious failure. | Adapter layer must detect auth-specific stderr patterns and surface as first-class UI: "Claude Code session expired. Run `claude auth` to re-authenticate." |
| 14 | **macOS notarization.** Requires $99/yr Apple Developer account, Developer ID Application certificate, notarization via App Store Connect API. First-time setup takes a full day. | Distribution blocked without it. Gatekeeper warnings scare users. | Budget time in Phase 1 timeline. Cannot use free signing for distributed builds. |
| 15 | **Webview rendering under high-frequency streaming.** Claude Code emits partial chunks rapidly. Pushing every event through Tauri IPC saturates the webview. | Laggy UI during active agent work. | Batch events on Rust side (50-100ms window). Use `requestAnimationFrame` on React side to coalesce renders. |
| 16 | **Sub-agent (Branch) visibility depends on undocumented `parent_tool_use_id` field.** Claude Code's Task tool events aren't formally documented. | Branch rendering breaks silently on Claude Code updates. | Treat branch rendering as best-effort. Graceful degradation: if sub-agent events can't be parsed, show them as flat steps instead of nested branches. |

### Tier 3: Worth tracking

| # | Risk | Impact | Mitigation |
|---|------|--------|------------|
| 17 | **Agents get too good.** If agents become reliable enough to run fully autonomously, the gate system becomes friction. | Product thesis weakens. | Per-workspace trust slider: "Approve everything" → "Notify on completion only." Complexity classifier already helps. Make gates tunable, not mandatory. |
| 18 | **Agent prerequisite management.** User installs Panes, has no agents installed. | Onboarding hits a wall. | Guided install flows: detect missing agents, show install instructions. Don't try to manage installation — just detect and guide. |
| 19 | **Long-running operations + laptop sleep.** Agent process gets SIGSTOP'd on sleep. Session state unclear on wake. | Hung threads, lost progress. | Detect suspend/resume. On wake, health-check active sessions. If agent process died, mark thread as interrupted with option to resume. |
| 20 | **Offline mode.** No internet = no agents. | Feed, memory, briefings should still work. | SQLite is local. UI works offline for read operations. Clearly indicate online/offline state. Queue prompt submissions for when connectivity returns. |

## 10. Open Questions

| # | Question | Needs answer by | Owner |
|---|---------|----------------|-------|
| 1 | Can Mem0 run as a lightweight local sidecar without GPU? What's the memory/CPU footprint? | Phase 1 prototype | Engineering |
| 2 | What's the actual quality of LLM-based memory extraction vs. Mem0's extraction? Is the Mem0 dependency worth the operational complexity? | Phase 1 prototype (A/B test both approaches) | Engineering |
| 3 | How do we handle the trust-layer translation for agent tools we've never seen? Can we build a generic "intent summarizer" or do we need per-tool mappings? | Phase 1 design | Product + Engineering |
| 4 | Should Phase 1 target macOS-only or include a web-based fallback for initial testing? | Before beta launch | Product |
| 5 | What is the minimum set of ACP agent features Panes requires? (Some ACP agents may implement minimal subsets.) | Phase 1 development | Engineering |
| 6 | What's the right lifecycle management for Beads/Dolt? Start on Panes launch + stop on quit, or lazy-start when first needed? | Phase 2 development | Engineering |
| 7 | For the non-developer persona: what approval UX actually works? Need user research. | Before GA | Product + Design |

## 10. Glossary

| Term | Definition |
|------|-----------|
| **Workspace** | A folder on disk with a project. The top-level organizational unit. |
| **Thread** | A conversation within a workspace. One prompt, one agent, one result. |
| **Branch** | A sub-agent's work within a thread, rendered as a collapsible nested section. |
| **Gate** | A pause point where work stops for a human decision (approve, reject, steer). |
| **Feed** | The activity stream showing results from Routines, Flows, and completed background work. |
| **Memory** | Structured learnings extracted from threads, scoped to a workspace or global. |
| **Briefing** | Persistent user instructions for a workspace, injected into every thread's context. |
| **Routine** | A scheduled, recurring prompt that runs on a cadence with a budget cap. |
| **Flow** | A coordinated sequence of steps across multiple workspaces and agents (DAG). |
| **Playbook** | Domain knowledge and default steps for the harness, stored per-workspace. |
| **Tag** | A label applied to threads for cross-cutting organization and filtering. |
| **ACP** | Agent Client Protocol. Open standard for editor/client-to-agent communication. |
| **Adapter** | A Rust module that translates a non-ACP agent's CLI output into the AgentEvent model. |
| **Harness** | The plan → execute → verify → decide loop for autonomous task execution. |
| **Mem0** | Open-source memory layer for AI agents. Used for extraction and retrieval. |
| **Beads** | Distributed graph issue tracker for AI agents. Under evaluation for Flow coordination. |
