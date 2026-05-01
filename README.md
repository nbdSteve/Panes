# Panes

[![CI](https://github.com/nbdSteve/Panes/actions/workflows/ci.yml/badge.svg)](https://github.com/nbdSteve/Panes/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/nbdSteve/Panes/branch/main/graph/badge.svg)](https://codecov.io/gh/nbdSteve/Panes)

A desktop app that wraps AI coding agents with safety gates, one-click rollback, persistent memory, and cost visibility.

Built with [Tauri 2](https://v2.tauri.app/) (Rust backend, React frontend). macOS only for now.

## What it does

- **Safety gates** — High-risk agent actions (destructive bash commands, etc.) pause for your review. Continue or abort with one click.
- **Git rollback** — Every thread snapshots your repo before the agent starts. Revert all changes instantly if things go wrong.
- **Persistent memory** — Decisions, preferences, and patterns are extracted from sessions and injected into future ones. Agents stop forgetting what you told them last week.
- **Briefings** — Write per-workspace instructions that get prepended to every prompt. "Always use TypeScript, never JavaScript."
- **Cost tracking** — See running cost per thread, per workspace, and in total. Set budget caps that kill runaway sessions.
- **Multi-workspace** — Manage multiple repos from one app. Each workspace has its own threads, memory, and briefings.

## Project structure

```
panes/
├── crates/
│   ├── panes-events/       Shared types (AgentEvent, RiskLevel, etc.)
│   ├── panes-adapters/     Agent adapter trait + Claude Code implementation
│   ├── panes-core/         Session manager, git snapshot/rollback, SQLite schema
│   ├── panes-cost/         Cost accumulator and budget enforcement
│   ├── panes-memory/       Memory store (SQLite FTS5), briefings, context injection
│   └── panes-app/          Tauri entry point and IPC command handlers
├── src/                    React frontend
├── sidecar/                Mem0 memory sidecar (optional)
├── e2e/                    Playwright end-to-end tests
└── docs/                   Product docs, architecture, research
```

## Prerequisites

- Rust 1.80+
- Node 20+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and authenticated

## Getting started

```bash
npm install
npx tauri dev
```

To run with the test adapter (no real Claude CLI needed):

```bash
PANES_TEST_MODE=1 npx tauri dev
```

## Configuration

| Env var | Default | Description |
|---------|---------|-------------|
| `PANES_CLAUDE_PATH` | `claude` | Path to Claude Code CLI binary |
| `PANES_TEST_MODE` | unset | Use fake adapter instead of real Claude |
| `CLAUDE_CODE_USE_BEDROCK` | unset | Passed through to Claude CLI |
| `AWS_PROFILE` | unset | Passed through to Claude CLI |

## Tests

```bash
# Rust unit tests
cargo test --workspace --exclude panes-app

# TypeScript type check
npx tsc --noEmit

# Frontend unit + component tests (with coverage)
npx vitest run --coverage

# E2E tests (mock Tauri backend)
npm run test:e2e

# E2E tests (full-stack, real Rust backend)
npm run test:e2e:fullstack
```

## Docs

Product documentation lives in [`docs/`](docs/):

- [PRODUCT.md](docs/PRODUCT.md) — Product brief and vision
- [ARCHITECTURE.md](docs/ARCHITECTURE.md) — Technical architecture
- [BRD.md](docs/BRD.md) — Business requirements
- [PRFAQ.md](docs/PRFAQ.md) — Press release / FAQ
- [EXPERIENCE.md](docs/EXPERIENCE.md) — User experience design
- [GAPS.md](docs/GAPS.md) — Known gaps and future work
- [REDDIT-RESEARCH.md](docs/REDDIT-RESEARCH.md) — Demand research

## License

Not yet decided.
