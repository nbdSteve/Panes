# Panes — Product Brief

## Vision

AI coding agents can now build entire features autonomously. But they also delete databases in 9 seconds, wipe drives with a single misplaced command, and silently introduce security vulnerabilities that look professional. Every week, another horror story goes viral.

The tools built around these agents — IDEs, terminals, cloud dashboards — assume the user can evaluate a diff, read a shell command, and understand what `rm -rf` means. A growing number of people directing AI agents cannot do any of that, and the ones who can are still getting burned.

Panes is the safety layer between AI agents and your codebase. It makes agent work legible, reversible, and cost-visible — so you can direct agents confidently without needing to read every line they write.

## Core Thesis

The #1 problem in AI coding today is not capability — agents are capable enough. It's trust. People don't understand what agents are doing, can't undo it when things go wrong, and lose all context between sessions. Panes solves this with three things: plain-english approval gates, one-click rollback, and persistent memory that compounds over time.

## Target Persona

**Primary: Anyone directing AI coding agents who has been burned — or fears being burned.**

This includes technical founders, product managers, tech leads, designers, and developers. What unites them isn't their skill level — it's that they've experienced or fear the core failure modes of AI coding:

- The agent deleted something and there was no undo
- The agent did something they didn't understand and they approved it anyway
- The agent forgot what was decided last week and re-introduced a pattern they explicitly rejected
- They have no idea how much they've spent across sessions
- Their codebase works but became an unmaintainable mess because every AI session made locally correct but globally incoherent decisions

Concrete examples:
- A non-technical founder who built a working SaaS with Claude Code but hit the "month 3 wall" — the agent forgets conventions, the codebase is a mess, and they'd pay $600 to have a developer clean it up
- A product manager who owns a repo, approved an agent action they didn't understand, and it broke production
- A developer running Claude Code across three repos who can't track what they've spent or what was decided in which project
- A tech lead who wants to let junior team members use AI coding but needs guardrails

**Secondary persona:** power developers who manage multiple repos and want faster orchestration than juggling terminal sessions. These users will discover Panes for the multi-workspace dispatch and stay for the memory and cost tracking.

## Product Principles

### 1. Safe by default
Every agent action is reversible. Pre-thread git snapshots mean you can always undo. Gates pause destructive actions for review. Budget caps prevent runaway costs. The default posture is cautious — you opt into autonomy, not out of it.

### 2. Trust through legibility
Non-developers can't evaluate diffs, but they can evaluate intent. Approval cards describe what the agent wants to do in plain english ("Agent wants to delete a database table — HIGH RISK"). The timeline shows what happened step-by-step. Trust is built through legibility, not code literacy.

### 3. Continuity across sessions
Every conversation contributes to a growing knowledge base. Panes remembers what was decided, what was tried, what failed, and what the user prefers. The 10th conversation in a workspace is meaningfully better than the 1st. Your codebase stays coherent because the agent inherits context from every past session.

### 4. Cost-aware
Every action has a price. Panes tracks cost per-thread, per-workspace, and in aggregate. Budget caps prevent surprises. You always know what you've spent and can set limits before you walk away.

### 5. Prompt-first, not code-first
The primary input is natural language. The primary output is a result summary. Code, diffs, and terminal output exist behind progressive disclosure — available when needed, never forced.

### 6. Agent-agnostic by default
Panes is not a Claude wrapper or a Kiro wrapper. It works with any compatible agent through its adapter layer. The user picks their agent; Panes provides the safety and memory layer on top.

## Core Differentiators

### Phase 1 — Validated by user research (ship first)

#### 1. The safety layer (PRIMARY)
AI agents delete databases, wipe drives, and break codebases. Every existing tool presents agent actions in developer-centric language: file paths, shell commands, diff hunks. Nobody has built approval UX for people who can't — or don't want to — evaluate a diff.

Panes provides:
- **Gates** — plain-english descriptions of what the agent wants to do, with risk classification (Low/Medium/High/Critical) and running cost
- **One-click rollback** — pre-thread git snapshots mean every change is reversible. Completion cards offer Commit, Revert, or Keep Uncommitted
- **Budget caps** — per-workspace and per-routine spending limits that kill sessions before costs spiral
- **Workspace isolation** — agents are process-scoped to their workspace directory. No cross-workspace file access.

The Reddit data is unambiguous: agent destruction stories are the most viral, most emotionally charged topic in AI coding (30K+ upvote incidents). This is the lead value prop.

#### 2. Persistent memory that compounds
After month 1, every AI coding tool hits the same wall: the agent forgets what was decided, re-introduces rejected patterns, and the codebase becomes incoherent. Users report paying $600+ to have developers clean up "vibe coded" messes.

Panes provides:
- **Automatic memory extraction** — after each thread, decisions, preferences, and patterns are extracted and stored per-workspace and globally
- **Briefings** — persistent user instructions ("always use Zod for validation, run tests before committing") injected into every thread
- **Memory panel** — view, edit, delete memories. Full transparency into what the agent "knows"
- **Context indicator** — every thread shows what memories and briefings were injected

The 10th conversation in a workspace is meaningfully better than the 1st. Your codebase stays coherent because the agent inherits context from every past session.

#### 3. Cost visibility everywhere
Users are blindsided by AI coding bills, confused by pricing changes, and building their own tracking spreadsheets. No tool provides real-time cost visibility at the level Panes does.

- Running cost in every active thread
- Total cost in every completion card
- Aggregate cost in the Feed
- Per-workspace spending history
- Budget caps that warn on approach and kill on exceed

### Phase 2+ — High-conviction roadmap (ship later)

#### 4. Agent-agnostic orchestration
Every AI coding tool is locked to its provider. Panes works with any agent through its adapter layer — run kiro-cli on one workspace, Claude on another, Codex on a third. Memory, Briefings, and cost tracking work identically regardless of backend.

#### 5. Structured orchestration — Flows, Harness, Playbooks
Nobody has user-defined cross-workspace workflows with verification. This is the difference between "schedule a prompt" and "orchestrate a verified multi-agent workflow."
- **Flows** — multi-step DAGs across workspaces and agents with dependency edges and context passing
- **Harness** — plan → execute → verify → decide loop for complex autonomous tasks. Pluggable verifiers.
- **Playbooks** — reusable domain knowledge per workspace that guides agent behavior

Users aren't asking for this yet — they're still figuring out how to use a single agent reliably. But as the trust layer and memory make single-agent use reliable, structured orchestration becomes the natural next step. This is the conviction bet.

#### 6. Routines
Recurring prompts on a cadence with budget caps. Claude Code and Devin both have basic scheduling — Panes must match them and integrate Routines with the agent-agnostic model. The differentiated version: Routines that can trigger Flows, not just single prompts.

### Supporting capability — ships with Phase 1

#### Dashboard, not editor
Panes is not an editor — it's a dashboard. Sidebar with live workspace status, Feed with aggregated results, cost overview across everything. The mental model is CI/CD dashboard, not editor with chat. Multi-workspace support is the cost of entry.

## What Panes Is Not

- **Not an IDE.** No code editor, no syntax highlighting, no file management (by default).
- **Not an agent.** Panes does not run AI models. It wraps agents with safety, memory, and cost visibility.
- **Not a chatbot wrapper.** Unlike TypingMind or Jan.ai, Panes is not about chatting with LLMs. It's about directing agents that take actions on your codebase — safely.
- **Not a workflow builder (yet).** Flows and structured orchestration are on the roadmap, but Phase 1 is laser-focused on the validated pain points: safety, memory, and cost.

## Business Model

Open-core. The desktop app, ACP client, adapter layer, and basic memory are MIT-licensed open-source. Premium features (advanced memory extraction, hosted scheduling, team collaboration) are commercial. Users bring their own API keys — Panes never intermediates or marks up inference costs. Specific pricing TBD post-beta.
