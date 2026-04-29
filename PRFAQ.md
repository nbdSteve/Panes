# Panes — PRFAQ

## Press Release

### Panes Launches the Safety Layer for AI Coding Agents

**Never let an AI agent destroy your work again**

*April 2026*

Today, Panes announces the launch of its open-source desktop application that makes AI coding agents safe, understandable, and cost-visible — for developers and non-developers alike. Panes is the first tool designed around the real failure modes of AI coding: agents that delete without asking, break things you can't undo, forget what was decided last week, and run up bills you didn't expect.

AI coding agents can now handle complex multi-file tasks autonomously. But that power comes with real risk. In the past year, agents have deleted production databases in seconds, wiped entire drives with misplaced commands, and introduced security vulnerabilities that look professional. Every week, another horror story goes viral. The tools built around these agents — IDEs, terminals, cloud dashboards — present agent actions as diffs and shell commands, assuming the user can evaluate what `rm -rf` or `DROP TABLE` means. Many users directing AI agents cannot, and even those who can are still getting burned.

Panes takes a different approach. Every agent action is legible, every change is reversible, and every dollar is visible. Before a thread starts, Panes snapshots your workspace. When an agent wants to do something risky, Panes surfaces a gate — a plain-english description of what the agent wants to do, the risk level, and the running cost. When a thread completes, you can commit the changes, revert everything with one click, or keep working. Between sessions, Panes remembers what was decided — so the agent that builds your 10th feature already knows your conventions and preferences.

"People aren't struggling with AI capability — agents are capable enough," said the Panes team. "They're struggling with trust. They approve things they don't understand, can't undo mistakes, lose context between sessions, and have no idea what they've spent. We built Panes to fix that."

### Key Features

**One-Click Rollback.** Before every thread starts, Panes snapshots your workspace. When the thread completes, your options are: Commit the changes (with an auto-generated commit message), Revert everything (one click, back to exactly where you started), or Keep Uncommitted. An AI agent can never make a change you can't undo.

**Gates — Approvals You Can Actually Understand.** When an agent wants to take a risky action, Panes pauses and shows you a gate: a plain-english description of what the agent wants to do, a risk level (Low / Medium / High / Critical), and the running cost. "Agent wants to: Delete a database table (HIGH RISK, $0.47 so far)." Approve, reject, or steer with feedback — without reading a single line of code.

**Persistent Memory.** After each thread, Panes extracts decisions, preferences, and learnings and stores them per-workspace and globally. Workspace Briefings let you set persistent instructions that every thread inherits ("always use Zod for validation, run tests before committing"). The next time you start a thread, the agent already knows your conventions. Users can view, edit, and delete memories at any time. Your codebase stays coherent because every AI session inherits context from every past session.

**Cost Visibility.** Every thread shows its running cost. Every completion shows total cost. The Feed shows aggregate cost across all workspaces. Budget caps per-workspace prevent surprises — set a limit and Panes kills the session before it's exceeded. This isn't a settings page metric — it's in every view, always.

**Multi-Workspace Dashboard.** Run tasks across multiple workspaces simultaneously. The sidebar shows live status for every workspace — working, waiting at a gate, done, error. Fire off a prompt to your backend, frontend, and infra repos, then monitor all three from a single view.

**Agent-Agnostic.** Use Claude Code on one workspace, kiro-cli on another, Codex on a third — from the same interface. Panes supports any agent through its adapter layer. Memory, cost tracking, and gates work identically regardless of which agent backend is in use.

### Roadmap

**Routines.** Set up recurring AI tasks: "Every morning at 9am, check for dependency updates." Results appear in the Feed with full transcripts and cost breakdowns. Budget caps per-Routine prevent runaway automated spending.

**Flows — Structured Multi-Agent Workflows.** Define cross-workspace, multi-step workflows with dependency edges. Step 1: build the backend endpoint. Step 2: build the frontend page using Step 1's output as context. Step 3: security review. Each step runs through a verification loop. This is the difference between "schedule a prompt" and "orchestrate a verified multi-agent workflow."

**Open Core.** The Panes desktop app, agent adapter interface, and core features are open-source (MIT). Community contributors can add support for new agent backends with a single adapter implementation. Premium features including advanced memory, team collaboration, and hosted Routines are available under a commercial license.

### Availability

Panes is opening its beta waitlist today at panes.dev. Early access will roll out to waitlist members starting with macOS, followed by Windows and Linux. The open-source core will be published at github.com/panes-app/panes alongside the public beta launch. Waitlist members will receive early access to the premium tier free for the first 6 months.

---

## Frequently Asked Questions

### Customer FAQ

**Q: Who is Panes for?**

A: Anyone who directs AI coding agents and wants to do it safely. This includes non-developers — technical founders, product managers, designers — who direct agents without reading code, as well as developers who want rollback, memory, and cost tracking across multiple projects. If you've ever approved an agent action you didn't fully understand, lost work because an agent broke something, or re-explained the same conventions to an AI for the third time — Panes is for you.

**Q: How is Panes different from Cursor, Windsurf, or Zed?**

A: Those are editors with AI features added. When something goes wrong, you're reading diffs and running git commands. Panes is a safety and orchestration layer on top of agents. Every change is reversible with one click. Every agent action is described in plain english with a risk level. Every session inherits context from past sessions. There is no code editor, no file tree, no integrated terminal by default — the interaction is prompt-in, result-out, with safety rails throughout.

**Q: What AI agents does Panes support?**

A: Panes supports any agent that implements the Agent Client Protocol (ACP), including kiro-cli, Codex, and the growing ecosystem of ACP-compatible agents. For Claude Code, Panes includes a built-in adapter that translates Claude's CLI output into the standard event model. Additional adapters for Aider, Goose, Amazon Q, and other CLI agents can be contributed by the community. The adapter interface is open-source and designed for a single-file implementation per backend.

**Q: How does memory work?**

A: After each thread, Panes runs a memory extraction pass that identifies decisions, preferences, patterns, and learnings. These are stored as structured data at two levels: per-workspace (project-specific context like "we use Tailwind for styling" or "the API follows REST conventions") and global (cross-project preferences like "always write tests" or "prefer TypeScript"). Workspace Briefings let you write persistent instructions ("always use Zod for validation, run tests before committing") that are injected into every thread. Before each new thread, relevant memories plus the Briefing are injected into the agent's context. You can view, edit, and delete memories at any time through the memory panel.

**Q: How does scheduling work?**

A: You create Routines through a simple interface: pick a workspace, write a prompt, set a schedule (daily, weekly, or custom cron expression), and optionally set a cost budget cap. When a Routine fires, it creates a new thread in the Feed, clearly badged as automated. You can see the full transcript, timeline of agent actions, and cost breakdown. If a Routine hits a gate (e.g., the agent wants to push to git), it pauses and sends you a notification. Routines can also chain — triggering follow-up tasks on completion or failure.

**Q: How do I know what the agent actually did?**

A: Every thread has a completion card showing what changed: files created, modified, or deleted, test results, total cost, and duration. If you want more detail, one click shows the timeline — every step the agent took, in order, with timing. Another click shows the full transcript. And critically: if you don't like what the agent did, you click "Revert all changes" and you're back to exactly where you started. No git knowledge required.

**Q: Can agents in one workspace access files from another?**

A: No. Workspace isolation is enforced at the process level. Each agent session is scoped to its workspace directory and cannot read or write outside of it. This is a security boundary, not just a convention.

**Q: How much does it cost to run agents?**

A: Panes itself is free for the open-source core. You pay your own API costs to your chosen AI provider (Anthropic, Amazon Bedrock, OpenAI, etc.). Panes tracks costs per-thread, per-workspace, and in aggregate — so you always know exactly what you've spent and where. You can set budget caps per-workspace that warn on approach and kill the session on exceed. No more surprise bills.

**Q: What does the premium tier include?**

A: The open-source core includes the full desktop app, ACP client, multi-workspace orchestration, basic memory, Briefings, Routines, Flows, and community agent adapters. Premium features include advanced memory extraction (using dedicated ML-powered extraction rather than simple summarization), cross-workspace memory intelligence, team workspaces with shared memory, and hosted Routines (runs without your desktop being open).

---

### Internal FAQ

**Q: Why now?**

A: Three converging trends. First, AI coding agents are now capable enough to cause real damage autonomously — database deletions, drive wipes, and security vulnerabilities have become weekly news. The trust problem has gone from theoretical to front-page-of-Reddit urgent. Second, the user base of people directing AI agents is exploding (r/vibecoding has ~100K subscribers in months) but the tools haven't caught up — everyone is still shoehorned into developer-centric IDEs and terminals. Third, the "month 3 wall" is now widely experienced: agents forget context, codebases become unmaintainable, and users are paying developers $600+ to clean up AI-generated messes. The problems are validated and acute. Nobody is solving them.

**Q: What is the competitive moat?**

A: The moat is compounding memory. The more you use Panes, the more it knows about your projects — your conventions, your past decisions, your preferences. After 3 months, switching to another tool means losing all that accumulated context and starting over at the "month 3 wall." No competitor exposes memory as a user-inspectable, editable knowledge base.

The trust layer UX is the second moat: making non-developers feel safe approving agent actions requires careful product design that AI providers and IDE makers won't invest in — their users are developers. This is a product design moat, not a technology moat.

The adapter layer creates a third moat through community network effects: as the community builds adapters for more agents, Panes becomes the default safety layer, similar to how Docker Desktop became the default for container management despite containers being an open standard.

**Q: Why not build this as a Zed fork?**

A: We evaluated this thoroughly. Zed is AGPL-licensed, which would require our entire codebase to be AGPL — eliminating any proprietary moat for premium features. Zed's codebase is 1.17M lines of Rust across 232 crates with ~248 commits/week — an unsustainable maintenance burden for a fork. And fundamentally, Zed is an editor. Forking an editor to remove the editor is the wrong starting point. We use the same building blocks (ACP crate, Rust, Tokio) but build a purpose-built product for orchestration rather than editing.

**Q: Why not build this as a plugin for an existing tool?**

A: Plugins inherit the host's interaction model. A Cursor plugin still lives inside Cursor's editor-centric UI. An Obsidian plugin still lives inside Obsidian's note-centric UI. The core thesis of Panes is that prompt orchestration needs its own interaction model — multi-workspace dashboard, activity inbox, approval cards, cost tracking. These are first-class UI concepts that can't be meaningfully squeezed into another tool's plugin system. Additionally, the scheduling and background automation features require a standalone process, which plugins typically cannot provide.

**Q: Why ACP over building a proprietary protocol?**

A: The ACP standard (co-created by Zed and JetBrains, v0.12, JSON-RPC 2.0 over stdio) already has 32+ compatible agents and SDKs in 5 languages including Rust. Building our own protocol would mean starting from zero ecosystem and asking every agent to implement yet another interface. By being ACP-native, we get automatic compatibility with the growing agent ecosystem. When a new agent ships with ACP support, it works with Panes on day one. The protocol already handles session management, file operations, terminal commands, approvals, and streaming — the exact primitives we need.

**Q: What is the technical architecture?**

A: Panes is a Tauri application with a Rust backend and web-based frontend. The Rust backend handles ACP client communication (using the `agent-client-protocol` crate), process orchestration (Tokio), memory storage (SQLite), and cron scheduling (Tokio-based scheduler). The frontend is a React application rendered in Tauri's webview, communicating with the backend over Tauri IPC. For agents that don't support ACP natively (e.g., Claude Code), the backend includes an adapter layer that translates CLI-specific output into the standard ACP event model. The architecture is:

```
Frontend (React) <-- Tauri IPC --> Rust Backend
                                    |
                                    +-- ACP Client (agent-client-protocol crate)
                                    |     |
                                    |     +-- kiro-cli (ACP native)
                                    |     +-- codex (ACP native)
                                    |     +-- any ACP agent
                                    |
                                    +-- Adapter Layer
                                    |     |
                                    |     +-- Claude Code (stream-json CLI)
                                    |     +-- Community adapters
                                    |
                                    +-- Memory Store (SQLite)
                                    +-- Cron Scheduler (Tokio)
                                    +-- Cost Tracker
```

**Q: What is the go-to-market strategy?**

A: Phase 1 (Beta): Launch on r/vibecoding (~100K subscribers), r/ClaudeAI, and Hacker News with the lead message: "Never let an AI agent destroy your work again." The destruction/trust narrative is the most viral topic in AI coding — every few weeks a new incident hits r/technology with 30K+ upvotes. We ride that wave with a product that solves the problem. Target: anyone who's been burned by an AI agent, or fears being burned. Phase 2 (GA): Expand via the "month 3 wall" narrative — memory and Briefings marketed to users whose codebases have become unmaintainable. Case studies from Phase 1 beta users showing before/after coherence. Phase 3 (Teams): Launch premium team features targeting organizations that want to let non-developers use AI coding safely — the governance angle.

**Q: What are the key risks?**

A: (1) **Anthropic builds this natively.** Claude Desktop adding a safety layer, memory panel, and cost dashboard would compress our market. Mitigation: move fast, build community, stay provider-neutral (something a provider can never be), and build the compounding memory moat before they can catch up. (2) **The trust layer doesn't actually work.** If plain-english gate descriptions can't enable meaningful approval decisions, the core thesis fails. Mitigation: user research with non-developers before GA — this is the first thing to validate, not the last. (3) **Memory extraction quality.** Poor memory is worse than no memory — if the agent "remembers" something wrong, it actively harms the user. Mitigation: start simple, make memories visible and editable, and invest in quality over quantity. (4) **Claude Code CLI instability.** The primary agent integration (stream-json) is undocumented and ships breaking changes weekly. Mitigation: forward-compatible parser, CI tests against multiple Claude Code versions, graceful degradation.

**Q: What is the pricing model?**

A: Open-source core is free (MIT license). Premium and team tiers are planned but pricing is TBD — we need beta usage data before committing to tiers. The core principle is bring-your-own API key: users pay their AI provider directly and Panes does not mark up inference costs.

**Q: What does success look like?**

A: The primary validation metric is whether users feel safe directing AI agents through Panes — measured by rollback usage (users should rarely need it, but knowing it's there changes behavior), gate engagement (users approve meaningfully rather than rubber-stamping), and memory utility (users report not needing to re-explain context). Secondary signals: organic GitHub star growth, community-contributed agent adapters, and the "month 3 wall" disappearing for Panes users. Specific targets will be set after beta data is available.
