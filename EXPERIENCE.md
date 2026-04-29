# Panes — Experience Design

## Core Interaction Model

The fundamental loop in Panes is: **Prompt -> Monitor -> Decide -> Review -> Commit or Revert**

This is not a chat interface. It's closer to a dispatch system with a safety net. You send work out, agents execute, you engage at decision points, and when the work is done you choose: keep the changes, undo everything, or commit. Every change is reversible. Every agent action is legible.

---

## The Three Surfaces

Panes has three primary surfaces, each serving a distinct purpose:

### 1. Workspaces (the dispatch view)
Where you send work to agents. A sidebar lists your workspaces, each pointing to a folder on disk. Selecting a workspace shows its conversation threads — both manual and automated. Starting a new conversation is: select workspace, type prompt, pick agent (or use default), send.

### 2. Feed (the morning briefing)
Where you review what happened while you weren't watching. Aggregates results from Routines and completed background work across all workspaces. Each item shows: workspace name, prompt summary, outcome (success/failure/needs-approval), cost, and timestamp. Routine results are badged so they're visually distinct from manual threads. Think activity feed, not chat history.

### 3. Memory Panel (the knowledge base)
Where you inspect and manage what Panes has learned. Shows extracted memories organized by workspace and global scope. Each memory has a type (decision, preference, constraint, pattern), the source thread, and the extracted content. Users can edit, delete, or pin memories. Workspace Briefings (persistent user instructions injected into every thread) are also managed here. This is not a primary interaction surface — most users will rarely visit it — but it builds trust that the system is learning sensibly.

---

## Core User Flows

### Flow 1: First-Time Setup

1. User opens Panes for the first time
2. Welcome screen: "Add your first workspace"
3. User selects a folder (native file picker)
4. Panes detects available ACP agents installed on the system (kiro-cli, etc.) and any configured adapters (Claude Code)
5. If no agents found, Panes shows install guidance for supported agents
6. User sees their workspace with an empty conversation list and a prompt input
7. They type their first prompt, select an agent, and send

**Design note:** No account creation, no API key entry in Panes itself. The agents handle their own authentication. Panes is a client, not a provider.

### Flow 2: Dispatch a Task

1. User selects a workspace from the sidebar (or is already in one)
2. Types a prompt: "Add a /health endpoint that returns 200 OK with the current timestamp"
3. Optionally selects which agent to use (default is remembered per workspace), or overrides the model per-message
4. Hits send
5. A new thread appears showing the agent working:
   - Thinking indicator while agent reasons
   - Step cards as the agent takes actions ("Reading package.json", "Creating src/health.ts", "Running tests")
   - Each step shows elapsed time
6. If the agent spawns sub-agents, they appear as **branches** — collapsible nested sections within the thread, each with their own step cards and status

**The user does not see:** raw terminal output, file diffs, or streaming text (unless they expand a step card for details).

### Flow 3: Gate (Approval Request)

1. Agent reaches a point requiring approval
2. The thread shows a **gate** — an inline approval card:

```
┌─────────────────────────────────────────────────┐
│  Agent wants to:                                │
│                                                 │
│  Create new file: src/health.ts                 │
│  Modify existing file: src/routes/index.ts      │
│  Run command: npm test                          │
│                                                 │
│  Risk: Low (new file + minor edit + test run)   │
│  Estimated cost: $0.12 so far                   │
│                                                 │
│  [ Approve ]  [ Reject ]  [ Steer... ]          │
└─────────────────────────────────────────────────┘
```

3. **Approve** — agent continues. Optionally "Approve for this thread" to auto-approve similar actions for the rest of the session.
4. **Reject** — agent stops, thread notes why
5. **Steer** — opens a text input for feedback: "Approve but use Express instead of Fastify"

**Design note:** Gates translate technical actions into human-readable descriptions. "Run command: npm test" is acceptable because even non-developers understand "run tests." But "Run command: sed -i 's/old/new/g' config.yaml" would be translated to "Modify configuration file: config.yaml."

### Flow 4: Task Completion — The Safety Moment

1. Agent finishes work
2. The thread shows a completion card — the most important UI surface in Panes:

```
┌─────────────────────────────────────────────────┐
│  Done                                           │
│                                                 │
│  Added a /health endpoint that returns 200 OK   │
│  with the current server timestamp.             │
│                                                 │
│  Files changed: 2 (1 created, 1 modified)       │
│  Tests: 14 passed, 0 failed                     │
│  Cost: $0.47                                    │
│  Duration: 2m 34s                               │
│                                                 │
│  [ Commit changes ]  [ Revert all ]  [ Keep ]   │
│                                                 │
│  [ View Timeline ]  [ View Transcript ]         │
└─────────────────────────────────────────────────┘
```

3. **Commit changes** — opens a minimal commit dialog with an auto-generated commit message from the thread summary. User can edit the message. No terminal or git knowledge required.
4. **Revert all** — restores the workspace to its pre-thread state. One click. Everything the agent did is undone. This is the core safety guarantee.
5. **Keep** (default) — changes stay on disk, uncommitted. User handles git themselves.
6. **View Timeline** — expands to show every step the agent took, in order, with timing. If the agent used branches (sub-agents), each branch is a collapsible section:
   - Read src/routes/index.ts (0.3s)
   - Planned approach (thinking) (1.2s)
   - ▸ Branch: "Write auth middleware" (3 steps, 1.1s, $0.08)
   - Modified src/routes/index.ts (0.3s)
   - Ran npm test (12.1s) — 14 passed
   
7. **View Transcript** — shows the full conversation between the prompt and the agent, including the agent's reasoning

**Design note:** The commit/revert decision is the most consequential moment in the UX. It must be prominent, clear, and frictionless. "Revert all" should feel as safe as Ctrl+Z — the user should never hesitate to try something because they fear they can't undo it.

### Flow 5: Parallel Multi-Workspace Threads

1. User has 3 workspaces: backend, frontend, infra
2. From the workspace sidebar, they start a thread in each:
   - backend: "Add rate limiting to the /api/users endpoint"
   - frontend: "Update the pricing page to show the new Enterprise tier"
   - infra: "Check if any Terraform modules are using deprecated AWS provider versions"
3. The sidebar shows live status for each workspace:
   - backend: Working... (step 3 of ~5)
   - frontend: Gate — needs approval
   - infra: Working... (step 1 of ~3)
4. User clicks into frontend, reviews the gate, approves
5. Switches to backend, sees it completed while they were approving frontend
6. Infra finishes with a summary: "2 modules using deprecated providers, details below"

**Design note:** The sidebar is the orchestration surface. Status indicators (working, gate, done, error) give at-a-glance awareness without requiring the user to click into each workspace.

### Flow 6: Create a Routine

1. User navigates to a workspace
2. Opens the Routines panel (clock icon or menu)
3. Fills in:
   - **Prompt:** "Check for dependency updates. If any are security-related, create a summary of what needs updating and why."
   - **Schedule:** Every weekday at 8:00 AM
   - **Budget cap:** $2.00 per run
   - **On completion:** Notify (default)
   - **On failure:** Retry once, then notify
4. Saves the Routine
5. The workspace now shows a "Routines" section listing active routines

### Flow 7: Morning Feed Review

1. User opens Panes in the morning
2. The Feed shows what happened overnight:

```
Feed (3 new)
─────────────────────────────────────
● backend — Dependency check            8:00 AM
  No security updates found. 2 minor   $0.34
  updates available (details inside).    ⟳ Routine
  
● frontend — Lighthouse audit           8:15 AM  
  Performance: 94 → 91. Largest         $1.12
  contentful paint regressed. See        ⟳ Routine
  details for suggested fixes.
  
○ infra — Cost anomaly check            8:30 AM
  ⚠ Gate: Agent wants to run AWS CLI    $0.08
  to fetch billing data.                 ⟳ Routine
  
─────────────────────────────────────
                          Total: $1.54
```

3. User can click into any item to see the full thread
4. The infra Routine is paused at a gate — user approves from the Feed
5. It resumes, completes, and updates in place

### Flow 8: Memory in Action

1. In workspace "backend," user has a thread where they say: "Use Zod for schema validation, not Joi — we're standardizing on Zod across all services"
2. After the thread completes, Panes extracts a memory:
   - **Type:** Decision
   - **Scope:** Workspace (backend)
   - **Content:** "Use Zod for schema validation, not Joi. Standardizing across all services."
   - **Source:** Thread #14, April 28 2026
3. Two weeks later, user starts a new thread: "Add input validation to the /orders endpoint"
4. The agent's context includes the Zod preference (from memory) and the workspace Briefing. It uses Zod without being told.
5. The thread shows a subtle indicator: "Using 2 memories · 1 briefing" (expandable to see which ones were injected)

**Design note:** Memory injection is mostly invisible. The indicator exists for trust — the user can verify that the agent is using relevant context. But the primary experience is that the agent "just knows" things from past threads.

### Flow 9: Flows (Cross-Workspace Orchestration)

1. User creates a new Flow from the sidebar menu: "Ship user profiles"
2. Defines steps:
   - Step 1: [kiro-cli @ backend] "Add /users/:id endpoint returning profile data"
   - Step 2: [kiro-cli @ frontend] "Add a user profile page that calls /users/:id" — depends on Step 1
   - Step 3: [claude @ backend] "Review the new endpoint for security issues" — depends on Step 1
3. Starts the Flow
4. The sidebar shows a Flow card with a mini DAG visualization:
   - Step 1: Working...
   - Step 2: Blocked (waiting on Step 1)
   - Step 3: Blocked (waiting on Step 1)
5. Step 1 completes → Steps 2 and 3 unblock and run in parallel
6. User can click into any step to see its thread
7. On completion, the Feed shows a single Flow summary with per-step cost and status

**Design note:** Flows are the power-user feature. Most users will use single-workspace threads. Flows exist for technical leads managing cross-repo changes — the secondary persona.

---

## Information Architecture

```
┌──────────────────────────────────────────────────────┐
│  Panes                                    [settings] │
├──────────┬───────────────────────────────────────────┤
│          │                                           │
│ Feed (3) │  [Active workspace or Feed content]       │
│ ──────── │                                           │
│ backend  │  ┌─────────────────────────────────────┐  │
│  ● task  │  │  Thread                             │  │
│          │  │                                     │  │
│ frontend │  │  User: Add a /health endpoint...    │  │
│  ✓ idle  │  │                                     │  │
│          │  │  ┌─ Step: Reading project files ──┐ │  │
│ infra    │  │  └─ 0.3s ────────────────────────┘ │  │
│  ✓ idle  │  │                                     │  │
│          │  │  ┌─ Gate ────────────────────────┐  │  │
│ Routines │  │  │  Agent wants to create 2 files │  │  │
│  3 active│  │  │  [Approve] [Reject] [Steer]   │  │  │
│          │  │  └───────────────────────────────┘  │  │
│ Flows    │  │                                     │  │
│  1 active│  │  ┌─ ▸ Branch: Write middleware ──┐  │  │
│          │  │  └─ 3 steps · $0.08 ─────────────┘ │  │
│          │  │                                     │  │
│          │  └─────────────────────────────────────┘  │
│          │                                           │
│          │  ┌──────────────────────────────────────┐ │
│          │  │ [prompt input]    [agent ▾] [send ▶] │ │
│          │  └──────────────────────────────────────┘ │
└──────────┴───────────────────────────────────────────┘
```

### Sidebar elements:
- **Feed** — activity feed, always at top, shows unread count from Routines and completed background work
- **Workspaces** — listed below, each showing status (working/idle/gate/error) and active thread count
- **Routines** — count of active routines across all workspaces
- **Flows** — count of active cross-workspace flows

### Main panel:
- Shows the selected workspace's thread, or the Feed
- Prompt input at the bottom (when viewing a workspace)
- Agent selector in the prompt bar (per-message override supported)

### Progressive disclosure layers:
1. **Default:** Prompt input, step summaries, gates, completion cards
2. **One click deeper:** Timeline view (ordered step list with timing, branches collapsed)
3. **Two clicks deeper:** Full transcript (raw conversation with agent reasoning)
4. **Settings/panels:** Memory panel, Briefings editor, Routines manager, cost dashboard, agent configuration

---

## Design Principles for the UX

### Reversible by default
The user should never hesitate to try something because they fear they can't undo it. Every thread starts with a snapshot. Every completion offers revert. The emotional posture is "experiment freely" — the safety net is always there.

### Legible over precise
"Agent wants to delete a database table (HIGH RISK)" is better than `DROP TABLE users CASCADE;` for the default view. "Modified 3 files" is better than a full diff. Precision lives in the detail layers — one click deeper for the timeline, two clicks for the transcript.

### Calm over busy
The default state is quiet. No animations, no real-time streaming text, no blinking cursors. A thread is "working" — you see a progress indication and step summaries appear as they complete. The interface demands attention only when a gate is hit or a result arrives.

### Cost is always visible
Every thread shows its running cost. Every completion shows total cost. The Feed shows aggregate cost. This is not a settings page metric — it's embedded in every view because users are blindsided by AI costs and it erodes trust.

### Consistent across agents
Whether the backend is kiro-cli, Claude Code, or a custom agent, the user experience is identical. Gates look the same. Completion summaries look the same. Rollback works the same.

### Flat over nested
Two levels of hierarchy (workspaces → threads) is enough. No sub-groups, no folders-within-folders. Tags provide cross-cutting organization when needed. Search and filter replace nesting.

---

## Glossary

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
