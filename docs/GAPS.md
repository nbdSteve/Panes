# Panes — Market Gap Analysis

*Last updated: April 29, 2026*
*Informed by Reddit demand research (see REDDIT-RESEARCH.md)*

## Competitive Landscape

The AI coding tool space has matured rapidly. Several products now offer pieces of what Panes targets. This document identifies what's genuinely differentiated versus what's table-stakes, grounded in validated user pain points.

### Current State of Competitors

| Product | Rollback | Legible Approvals | Memory | Cost Visibility | Scheduling | Notes |
|---|:---:|:---:|:---:|:---:|:---:|---|
| **Claude Code** | No (user manages git) | Partial (visual diffs, but dev-oriented) | Yes (CLAUDE.md + auto-memory) | Partial (per-session, not aggregated) | Yes (cloud Routines) | Most direct overlap. No rollback UX, no risk classification, no budget caps. |
| **Devin** | No | Partial (Slack approval, but dev-centric) | Partial (learns in-project) | Partial | Yes (Automations) | $20-$200/mo. No one-click revert. |
| **Cursor** | No (manual git) | No (inline diffs, requires code literacy) | Partial (.cursorrules) | No | No | Agent deleted a user's database — front-page news. No safety layer. |
| **Zed** | No | No (editor-first) | No | No | No | Best ACP support, but it's an editor. |
| **GitHub Copilot Agent** | Partial (PR-based, can close PR) | No (requires GitHub fluency) | No | No | No (event-triggered only) | Narrow: one task → one PR. |
| **OpenAI Codex** | Partial (cloud sandbox) | Partial (ChatGPT-accessible) | No | No | No | Sandboxed, but no persistent workspace or memory. |
| **Replit Agent** | No | No (fully autonomous) | Not confirmed | No | No | Designed for non-devs but no approval workflow. Agent acts freely. |
| **Bolt / Lovable / v0** | No | No | No | No | No | Single-project app builders. Different category. |

### What's Table-Stakes vs. Differentiated

**Table-stakes (competitors already have this):**
- Multi-workspace / multi-session support
- Persistent memory across conversations
- Scheduled / recurring task execution

These are no longer differentiators. Panes must have them to compete, but they won't win the sale.

**Genuinely differentiated (validated by demand research):**
- **Safety layer with legible approvals and one-click rollback.** No product offers all three: plain-english risk-classified gates, pre-thread snapshots, and one-click revert. This addresses the #1 pain point in AI coding — agent destruction — which generates 30K+ upvote Reddit threads weekly.
- **Visible, editable, persistent memory.** Claude Code has auto-memory but it's a hidden file. Cursor has .cursorrules but it's manual. Nobody exposes memory as a user-inspectable knowledge base with workspace Briefings. This addresses the #2 pain point — the "month 3 wall" where agents forget everything.
- **Cost visibility everywhere.** No tool shows running cost in active threads, budget caps per-workspace, and aggregate spend across all activity. Users are blindsided by bills and building their own tracking spreadsheets.

**Differentiated but not yet validated by demand (conviction-based):**
- **Agent-agnostic orchestration.** Everything is still provider-locked. One viral Reddit thread (2.2K pts) validates the pain for power users who switch agents, but most users are locked to one tool.
- **Structured multi-agent workflows (Flows).** Nobody has user-defined cross-workspace DAGs with verification. Zero organic demand signal yet — users need to trust single-agent use before they'll want multi-agent orchestration.

---

## The Three Validated Gaps (Phase 1 Focus)

### Gap 1: No Safety Layer — Agents Destroy Things and There's No Undo

#### The problem
AI coding agents delete production databases in 9 seconds, wipe entire drives with misplaced commands, introduce security vulnerabilities that look professional, and lie about what they did. This isn't hypothetical — these incidents make the front page of Reddit with 30K+ upvotes every few weeks.

Every tool presents agent actions as diffs and shell commands. When the agent says "I want to run `bash: rm -rf ./data`", a developer knows the risk. A product manager does not. And even developers approve things they shouldn't — because the tools don't classify risk or offer easy rollback.

No existing tool provides all three requirements of a complete safety layer: legible approvals, risk classification, and one-click rollback.

#### Who has pieces of this
| Tool | What it offers | What's still missing |
|------|---------------|---------------------|
| Claude Code | Visual diffs, permission prompts | Developer-oriented. No risk classification. No one-click rollback — user manages git manually. |
| Devin | Slack/Teams integration for approval | Text-based approvals, no structured risk/cost presentation. No rollback. |
| Cursor | Accept/reject inline diffs | Requires code literacy. No risk classification. An agent in Cursor deleted a user's database — front-page news. |
| Replit Agent | Chat-based building for non-devs | No approval workflow at all — agent acts fully autonomously. Agent wiped a codebase and lied about it. |
| OpenAI Codex | Cloud sandbox isolates changes | No persistent workspace. Can't use for ongoing projects. |

#### What Panes does differently
Three-part safety layer:
1. **Gates** — plain-english descriptions of what the agent wants to do, with risk classification (Low/Medium/High/Critical) and running cost. "Agent wants to: Delete a database table (HIGH RISK)" vs "Agent wants to: Create a new file (Low risk)."
2. **Pre-thread snapshots** — before every thread, Panes records the workspace state. Every change is reversible.
3. **One-click rollback** — completion cards offer Commit, Revert (back to snapshot), or Keep Uncommitted. No git knowledge required.

This is the primary product differentiator. It addresses the most emotionally charged, most widely shared, most mainstream concern in AI coding.

### Gap 2: No Solution to the "Month 3 Wall" — Agents Forget Everything

#### The problem
After month 1, every AI coding tool hits the same wall. The agent forgets what was decided, re-introduces patterns that were explicitly rejected, and the codebase becomes an incoherent mess. Users report their agent "fighting them" by month 2, paying $600+ to have developers clean up by month 3, and questioning "what's the point of vibe coding if I still have to pay a dev to fix it?"

This is the second-most-discussed pain point in AI coding communities after destruction.

#### Who has pieces of this
| Tool | What it offers | What's still missing |
|------|---------------|---------------------|
| Claude Code | CLAUDE.md manual rules + auto-memory (hidden) | Auto-memory not user-inspectable or editable. No extraction quality controls. User can't see what the agent "knows." |
| Cursor | .cursorrules file | Fully manual. No automatic extraction. User must write and maintain rules themselves. |
| Devin | Learns in-project | Opaque. User can't inspect, edit, or control what Devin remembers. |
| Everyone else | Nothing | No memory at all. Every session starts from zero. |

#### What Panes does differently
Three-layer knowledge system:
1. **Briefings** — user-written persistent instructions per workspace ("always use Zod, run tests before committing"). Deterministic: always injected.
2. **Automatic memory extraction** — after each thread, decisions, preferences, and patterns are extracted and stored per-workspace and globally.
3. **Memory panel** — full transparency. View, edit, delete, pin memories. See exactly what was injected into each thread.

### Gap 3: No Cost Visibility Across AI Coding Activity

#### The problem
Users are blindsided by AI coding bills, confused by pricing changes, and have no aggregate view of spending. One user audited 926 sessions manually to understand token waste. Another was unknowingly billed $200 due to a billing bug. Pricing page changes trigger 700+ comment threads of anxiety.

#### What Panes does differently
Cost is embedded in every view: running cost in active threads, totals in completion cards, aggregates in the Feed, spending history per workspace, and budget caps that warn on approach and kill sessions on exceed.

---

## Conviction-Based Gaps (Phase 2-3)

These gaps are not yet validated by organic demand but represent high-conviction bets based on the trajectory of AI coding.

### Gap 4: No Agent-Agnostic Desktop Orchestration
Every tool is coupled to its provider. One viral Reddit thread (2.2K pts) — where a user built "agentchattr" to let agents talk to each other — validates the pain for power users who switch agents. But most users are locked to one tool today.

Panes works with any agent through its adapter layer. Memory, Briefings, and cost tracking work identically regardless of backend.

### Gap 5: No Structured Multi-Agent Workflows with Verification
Nobody has user-defined cross-workspace DAGs with pluggable verification. Zero organic demand signal yet — users are still figuring out reliable single-agent use. As the trust layer and memory make single-agent use reliable, structured orchestration becomes the natural next step.

**Flows** — multi-step, cross-workspace DAGs with dependency edges, context passing, per-step budgets.
**Harness** — plan → execute → verify → decide loop for autonomous tasks. Pluggable verifiers. Escalate-first failure handling.
**Playbooks** — reusable domain knowledge per workspace. Default execution steps for the harness.

#### Task DAG approach (decided 2026-05-02, revised same day)

Initially considered beads_rust (frozen Rust fork, SQLite + JSONL) and then rolling our own with petgraph. After deeper evaluation, chose the original beads (Go, Dolt-backed) as the task layer:

- **beads_rust rejected:** CLI-only (not a library), frozen architecture, issue-tracker schema doesn't carry execution fields
- **petgraph rejected:** The DAG primitives are easy (~200 lines), but persistence, agent-writable interface, concurrent multi-agent access, atomic claiming, compaction, and cell-level merge add up to reimplementing most of what beads already ships
- **beads (original) chosen:** Dolt gives cell-level merge (concurrent agents don't conflict), four dependency types with `blocks`-only ready queue, hash-based IDs (no merge collisions), atomic `--claim`, compaction for context efficiency, `discovered-from` links for mid-execution task creation

**Integration approach:** Bundle `dolt` + `bd` as Tauri sidecars (single Go binaries, no runtime deps). Panes shells out to `bd --json` from Rust, same pattern as `git.rs`. Execution-specific config (budget, gate policy, verification) lives in Panes' SQLite keyed by beads task ID. Beads owns the *what* (task graph), Panes owns the *how* (worktrees, gates, cost, verification).

#### Git worktrees as concurrency primitive (decided 2026-05-02)

The Phase 1 one-thread-per-workspace guard exists because concurrent agents in the same directory create conflicting file changes. Git worktrees solve this: each concurrent thread gets its own isolated checkout, works in isolation, and results merge back to the main branch on completion. This enables agent swarms within a single repository without the safety risk.

Uses `git2` (libgit2 Rust bindings) rather than CLI for the worktree/merge path. Phase 1's CLI approach works for sequential operations but swarms need concurrent worktree creation (lock contention handling), structured merge conflict detection (`IndexConflict`, three-way merge), and in-process status queries across N worktrees. Existing Phase 1 CLI calls in `panes-core/src/git.rs` can migrate to git2 later but don't need to immediately.

---

## Supporting Capabilities (Table-Stakes, Ships with Phase 1)

### Multi-Workspace Dashboard
Dashboard view across multiple workspaces with live status. Sidebar shows working/idle/gate/error. Users dispatch and monitor without context-switching. Competitors have multi-session but none have the dashboard model.

### Basic Scheduling (Routines) — Phase 2
Recurring prompts on a cadence with budget caps. Claude Code and Devin both have this — Panes must match. Differentiated version: Routines that can trigger Flows.

---

## Summary: The Positioning

Panes is **the safety layer for AI coding agents.**

Phase 1 — validated by demand research:
1. **Safety** — Gates, risk classification, one-click rollback. Nobody else has all three. Addresses the #1 pain point.
2. **Memory** — Automatic extraction, Briefings, transparent memory panel. Addresses the #2 pain point (the "month 3 wall").
3. **Cost visibility** — Per-thread, per-workspace, budget caps, always visible. Addresses pervasive cost anxiety.

Phase 2-3 — conviction-based:
4. **Agent-agnostic orchestration** — Any agent, shared memory and context.
5. **Structured multi-agent workflows** — Flows, Harness, Playbooks. The difference between "schedule a prompt" and "orchestrate a verified workflow."

The supporting feature set (multi-workspace dashboard, basic scheduling) ships alongside Phase 1 as table-stakes.
