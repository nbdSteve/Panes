# Panes — Reddit Demand Research

*Research conducted: April 29, 2026*
*Sources: r/vibecoding (~100K subscribers), r/ClaudeAI, r/SideProject, r/sysadmin, r/ExperiencedDevs, r/technology, Reddit-wide search*
*Method: Reddit JSON API, ~100+ threads analyzed*

---

## Executive Summary

Reddit provides strong validation for Panes's trust layer and memory features, moderate validation for the non-developer persona, and weak-to-no validation for advanced orchestration features (Flows, Harness, Playbooks). The strongest emotional signal in the entire AI coding space is fear of agent destruction — agents deleting databases, wiping drives, and breaking codebases. The second strongest is context amnesia — agents forgetting decisions, re-explaining conventions, inconsistent patterns over time.

| Panes Value Prop | Signal Strength | Evidence Depth |
|---|---|---|
| Trust layer / rollback / approval UX | Very Strong | 30K+ upvote incidents, multiple viral destruction stories |
| Memory / context persistence | Strong | "Month 3 problem" threads, universal complaint across skill levels |
| Non-developer persona exists | Strong | Real builders, real revenue, but still minority of users |
| Cost visibility / budget tracking | Strong | Pervasive anxiety, users building their own tracking tools |
| Agent-agnostic orchestration | Moderate | One viral thread (2.2K pts), power-user niche |
| Scheduling / automation | Weak | Table-stakes, already shipped by Anthropic natively |
| Flows / structured orchestration | No Signal | Nobody is asking for cross-workspace DAGs yet |

---

## Theme 1: The Non-Developer Builder Persona

### Signal: Strong — the persona exists and is growing, but remains a minority

r/vibecoding has ~100K subscribers and is one of the fastest-growing coding-adjacent subreddits. A meaningful fraction of posts come from people with no traditional coding background. However, the highest-engagement posts are still from developers using AI as an accelerant. Non-developer "I built this" posts typically get 5-100 upvotes, while developer-perspective posts and memes get 1,000-5,000+.

### Key Threads

**"claude code is fucking insane"** — r/vibecoding, 3,238 pts, 230 comments
> "i know literally NOTHING about coding. ZERO. and i just built a fully functioning web app in minutes"

Pure excitement from a zero-experience user. High engagement but note: the app is running on localhost — shipping is a different story.

**"my entire vibe coding workflow as a non-technical founder (3 days planning, 1 day coding)"** — r/vibecoding, 1,128 pts, 112 comments
> "I learned to code at 29. before this I studied law, then moved to marketing... 3 products later, 1's finally working: [Oiti] solo founder, currently at $718MRR, $5K net, 1000 users. the entire thing is built with Claude Code."

This is the strongest evidence of the Panes persona in practice — a non-technical founder with real revenue built entirely with AI. Notably, this person developed a deliberate planning-first workflow, suggesting the need for structure that Panes could provide.

**"Not a programmer but Claude Code literally saves me days of work every week"** — r/ClaudeAI, 555 pts, 91 comments
> "I know most people here are probably using Claude Code for actual coding, but I gotta share what I've been doing with it... I do a lot of data indexing work (boring, I know)"

Non-programmer using Claude Code for data work, not app building. Broader persona than "founder building a SaaS."

**"As a non-technical PM, I built a real-time multilingual social platform where everyone speaks their own language. Claude wrote 100% of the code"** — r/ClaudeAI, 21 pts, 45 comments

Direct match to the Panes "PM who owns a repo" persona. Lower engagement suggests this is still niche.

**"I'm not a developer. I built a multi-agent AI system that runs my business."** — r/vibecoding, 0 pts, 13 comments
> "I'm a business consultant with an MBA, not an engineer. I've never written a line of code in my life. Two months ago, I started experimenting with Claude Code..."

Exact Panes persona. But 0 upvotes — the community didn't engage much.

**"Civil engineer here - finally discovering Claude Code and AI agents"** — r/ClaudeAI, 6 pts, 20 comments
> "I'm a civil engineer finishing my Master's thesis... I've always been fascinated by tech and coding... unsure where to go from 'beginner' to 'actually useful'"

Non-developer with technical aptitude trying to bridge the gap. Represents the "technical adjacent" population.

**"From PMO to Code: My 3-Month Journey with Claude Code (Advice for Non-Dev Vibecoders)"** — r/ClaudeAI, 12 pts, 7 comments
> "Coming from IT as a PMO who's delivered products to 300k+ users in finance, I thought I knew what building software meant..."

IT project manager turned builder. Direct persona match.

**"Built an OS/Dashboard for my golf sim company with no experience"** — r/ClaudeAI, 1 pt, 5 comments
> "Last summer I spent 4 months and spent way too many claude credits to build my program 'ClubOS'... I used Claude, Warp, and ChatGPT, then stitched together a bundle..."

Small business owner building internal tools. Not a developer, real business need.

**"Any Non-Technical Founder Vibe Coding Success Stories?"** — r/vibecoding, 2 pts, 8 comments
> "I'm a non-technical founder of a very fast growing virtual construction permitting and inspection firm. I am trying to figure out if it's reasonable to think that I could vibe code multiple business apps with very complex business logic..."

Someone actively looking for confirmation the persona works. Low engagement but signals demand for guidance.

### Implications for Panes

The persona is real but early. Most non-developer builders today are:
- Technical founders who can evaluate output but don't want to write code
- PMs/PMOs from IT backgrounds with enough context to direct agents
- Small business owners automating internal processes

They are NOT (yet) the completely non-technical "describe what you want and walk away" user. They still engage deeply with agent output. This suggests the Phase 1 trust layer should be designed for people with *some* technical intuition, not zero.

---

## Theme 2: Trust, Destruction, and Fear of Agent Actions

### Signal: Very Strong — the most emotionally charged topic in AI coding

Agent destruction stories regularly reach the front page of Reddit with tens of thousands of upvotes. This is not a niche concern — it's mainstream news when AI agents delete databases.

### Key Threads

**"Claude-powered AI coding agent deletes entire company database in 9 seconds — backups zapped"** — r/technology, 34,804 pts, 2,697 comments
The PocketOS incident. An AI agent running in Cursor (powered by Claude) deleted a production database and its backups in 9 seconds. This thread appeared simultaneously in r/technology (34K), r/whennews (5K), and r/nottheonion (14K). Total engagement across duplicates: ~55,000+ upvotes.

**"GPT 5.3 Codex wiped my entire F: drive with a single character escaping bug"** — r/vibecoding, 1,122 pts, 398 comments
> "I asked codex to do a rebrand for my project, change the import names and stuff, it was in the middle of the rebrand then suddenly everything got wiped. It said a bad rmdir command wiped the contents of F:\Killshot"

User asked for a simple rebrand. Agent destroyed the drive. 398 comments of shared horror and advice.

**"Cursor deletes vibe coder's whole database"** — r/vibecoding, 1,616 pts, 212 comments

Another database destruction incident, specific to Cursor users.

**"never touching cursor again"** — r/vibecoding, 3,882 pts, 565 comments

565 comments of shared frustration. One of the highest-engagement threads in the sub.

**"Oh shit."** — r/vibecoding, 4,103 pts, 105 comments

Universal reaction meme to agent destruction. 4,103 upvotes.

**"Replit's CEO apologizes after its AI agent wiped a company's code base in a test run and lied about it"** — r/Futurology, 5,873 pts, 331 comments

The agent didn't just destroy code — it lied about what it did. Trust violation on top of destruction.

**"What the actual f.."** — r/ChatGPT, 6,171 pts, 449 comments
> "No where in the thread did I mention it to do that"

Agent took unauthorized destructive action. User had no idea it would happen.

**"Amazon blames human employees for an AI coding agent's mistake"** — r/technology, 11,189 pts, 478 comments

Two AWS outages caused by AI coding agents. Amazon blamed humans. 11K upvotes of concern.

**"I just watched an AI agent take a Jira ticket, understand our codebase, and push a PR in minutes and I'm genuinely scared"** — r/cscareerquestions, 4,734 pts, 1,123 comments

Professional developer's existential fear watching agents operate autonomously.

**"Vibe coding without a security audit is not negligence. Change my mind."** — r/vibecoding, 22 pts, 103 comments
> "I have audited enough AI-generated SaaS products to have a strong opinion on this... AI writes insecure code that looks professional. Clean variable names, proper structure, but with hidden flaws."

Security professional warning about invisible risks in AI-generated code.

### Implications for Panes

This is the strongest demand signal in the entire dataset. The problems are:
1. **Agents take destructive actions without meaningful approval** — users click "approve" without understanding the risk
2. **No rollback** — once the agent acts, there's no undo button
3. **Agents lie about what they did** — the Replit incident shows agents can misrepresent their actions
4. **Risk is invisible** — destructive commands look the same as benign ones in a terminal

Panes's trust layer — gates with plain-english risk descriptions, one-click rollback, pre-thread snapshots — directly addresses every one of these. This should be the lead marketing message, not "dashboard for directing agents." The pitch is: **"Never let an AI agent destroy your work again."**

---

## Theme 3: Context Amnesia and the "Month 3 Problem"

### Signal: Strong — universal complaint across all skill levels

The pattern is consistent: AI coding works great for the first project or first month. Then context decay sets in. Agents forget decisions, re-introduce patterns that were explicitly rejected, and the codebase becomes inconsistent.

### Key Threads

**"The real cost of vibe coding isn't the subscription. It's what happens at month 3."** — r/vibecoding, 828 pts, 284 comments
> "Month 1: This is incredible. You go from idea to working product in days... Month 2: You want to add features or fix something and the AI starts fighting you. You're re-explaining context. It forgets what it did last week."

This thread describes the exact problem Panes's memory system solves. The user is experiencing context decay — the agent doesn't remember past decisions, conventions, or architecture choices.

**"vibe coded for 6 months. my codebase is a disaster."** — r/vibecoding, 1,751 pts, 559 comments
> "the app works. users are happy. revenue is coming in. but i just tried to onboard a dev to help me and he opened the repo and went quiet for like 2 minutes. then said 'what is this.' 6 months of cursor and lovable and bolt. every feature worked when i shipped it. but nobody was thinking about structure. the AI just kept adding."

1,751 upvotes. The codebase works but is unmaintainable because there was no persistent memory of conventions or structure. Each AI session made locally correct but globally incoherent decisions.

**"What's the point of vibe coding if I still have to pay a dev to fix it?"** — r/vibecoding, 1,275 pts, 509 comments
> "sure it feels kinda cool while i'm typing... but when stuff breaks it's just dead weight. i cant vibe my way through debugging"

509 comments. The frustration cycle: build fast, break fast, can't fix without expert help.

**"How much would you pay for someone to fix your mess?"** — r/vibecoding, 1,085 pts, 125 comments
> "Lowkey I'd pay 600 bucks to hire a dev to fix my vibe coded mess in a couple days."

People are willing to pay $600+ to fix what AI broke. Market signal for "keep it from getting messy in the first place."

**"4 months of Claude Code and honestly the hardest part isn't coding"** — r/ClaudeAI, 955 pts, 323 comments
> "I've been building a full iOS app with Claude Code for about 5 months now. 220k lines, real users starting to test it. The thing nobody talks about is that the coding is actually the easy part at this point. The hard part is making design decisions."

955 upvotes. The problem isn't code generation — it's decision continuity. This maps directly to Panes's Briefings (persistent instructions) and Memory (extracted decisions).

**"How I keep AI generated code maintainable"** — r/vibecoding, 1,166 pts, 298 comments
> "I love how fast I can build stuff using AI, but I was having trouble maintaining the project as it got larger. So I built this tool that gives you an overview of your code..."

User built their own tool to manage AI-generated code. Validation that existing tools don't solve the problem.

**"Can a LLM write maintainable code?"** — r/vibecoding, 1,406 pts, 262 comments

262 comments debating whether AI can write code that's maintainable long-term. The concern is widespread.

**"I let my interns vibe code from day one but with rules. here's what happened after 2 months"** — r/vibecoding, 1,018 pts, 105 comments
> "someone who doesnt know coding jumping straight into vibe coding with no guidance is a recipe for disaster. they hit enter, stuff works, they think theyre a developer. then something breaks and they have no mental model for why."

A manager's perspective: AI coding needs guardrails and persistent rules. This is the Briefings use case.

### Implications for Panes

The memory/Briefings system addresses the #2 pain point in AI coding. Specifically:
- **Briefings** solve the "re-explaining conventions" problem — persistent instructions that every thread inherits
- **Memory extraction** solves the "it forgets what we decided" problem — automatic capture of decisions and preferences
- **Memory panel** solves the "the codebase became incoherent" problem — visible, editable knowledge base that maintains consistency

The marketing angle: **"Your AI remembers what you decided last month."**

---

## Theme 4: Cost Anxiety

### Signal: Strong — pervasive and under-addressed

Cost is a recurring source of anxiety, particularly around Claude Code pricing changes. Users don't have good visibility into what they're spending or why costs spike.

### Key Threads

**"PSA: The string HERMES.md in your git commit history silently routes Claude Code billing to extra usage — cost me $200"** — r/ClaudeAI, 1,420 pts, 194 comments

A billing bug cost someone $200 with no warning. Anthropic acknowledged the bug but refused a refund. 194 comments of outrage.

**"PSA: Claude Pro no longer lists Claude Code as an included feature"** — r/ClaudeAI, 2,954 pts, 733 comments

Pricing page change triggered massive anxiety. 733 comments — one of the most-engaged threads on the sub.

**"Does Claude's $20 Plan No Longer Include Claude Code?"** — r/ClaudeAI, 992 pts, 271 comments

More pricing confusion and anxiety.

**"Anthropic: Stop shipping. Seriously."** — r/ClaudeAI, 3,143 pts, 387 comments
> "Claude Max user here... I've spent hundreds of dollars on Claude subscriptions"

Power user frustrated by quality regressions despite high spending.

**"anthropic isn't the only reason you're hitting claude code limits. I did audit of 926 sessions and found a lot of the waste was on..."** — r/ClaudeAI, 30 pts, 35 comments

Someone manually audited 926 sessions to understand their own token waste. This is the kind of analysis Panes's cost tracker would automate.

**"Open Letter to Anthropic - Last Ditch Attempt Before Abandoning the Platform"** — r/ClaudeAI, 1,120 pts, 478 comments
> "We've hit a tipping point with a precipitous drop off in quality in Claude Code and zero comms that has us about to abandon Anthropic."

Team running 5 platforms across fintech, gaming, media — frustrated by quality and cost unpredictability.

### Implications for Panes

Cost tracking isn't just a nice feature — it's a trust mechanism. Users feel blindsided by unexpected bills, pricing changes, and opaque token usage. Panes's per-thread cost tracking, budget caps, and aggregate dashboards address this directly. The value isn't "save money" — it's "never be surprised by your bill."

---

## Theme 5: Multi-Agent Management Pain

### Signal: Moderate — real for power users, not mainstream

### Key Threads

**"I got tired of copy pasting between agents. I made a chat room so they can talk to each other"** — r/vibecoding, 2,267 pts, 256 comments
> "Whoever is best at whatever changes every week. So like most of us, I rotate and often have accounts with all of them and I kept copying and pasting between terminals wishing they could just talk to each other. So I built agentchattr."

2,267 upvotes. This is the strongest direct validation for agent-agnostic orchestration. The user built a tool to solve the exact problem Panes addresses. The solution (agentchattr — MCP-based agent chat room) is a hack compared to Panes's vision, but it validates that the pain exists for users who switch between agents.

**"Wall between claude.ai and claude code"** — r/ClaudeAI, 5 pts, 21 comments
> "I use claude.ai for planning and architecture discussions, and claude code for implementation, but I cannot directly share any files, context or memory."

Lower engagement but exact description of the context fragmentation problem.

**"We run 14 AI agents in daily operations. Here's what broke."** — r/ClaudeAI, 4 pts, 8 comments
> "We run a digital marketing agency with 14 AI agents handling daily briefings, ad spend monitoring, client email drafting... After 7 months in production, we learned..."

A team running 14 agents in production. Low engagement but represents the power-user segment.

### Implications for Panes

Agent-agnostic orchestration has real demand among a specific power-user segment — people who actively use multiple AI tools and switch based on task type. However, most users are locked into a single tool (usually Claude or Cursor). The agent-agnostic value prop is more useful as engineering insurance (decoupling from a single provider) than as a user-facing selling point.

---

## Theme 6: Scheduling and Automation

### Signal: Weak — table-stakes, already shipped

**"Anthropic just made Claude Code run without you. Scheduled tasks are live."** — r/ClaudeAI, 1,198 pts, 247 comments
> "Daily commit reviews, dependency audits, error log scans, PR reviews — Claude just runs it overnight while you're doing other things. This is a big deal."

1,198 upvotes of excitement, but this is excitement about Anthropic's native feature, not demand for a third-party tool. Anthropic already shipped Routines in Claude Code, making scheduling table-stakes.

### Implications for Panes

Scheduling must be present but isn't a differentiator. The differentiated version (Routines that trigger Flows) has no organic demand signal yet.

---

## Theme 7: Organizational Governance Risk

### Signal: Emerging — the "adult supervision" narrative

**"I'm quitting my job due to vibe coders and poor leadership"** — r/sysadmin, 1,891 pts, 513 comments
> "Our exec leadership this year is making a big push for AI. They're encouraging everyone to generate ideas and try to make them real with vibe code. The team with the best idea that generates real results gets a bonus. This has led to a huge influx of..."

A sysadmin watching non-developers deploy AI-generated code into production without governance. 513 comments of shared frustration from IT professionals.

**"AI AGENTS today are far more DANGEROUS than you think"** — r/ArtificialInteligence, 531 pts, 423 comments
> "I built a multi-agent AI system that has root shell access to any Linux environment... this one I can share is open-sourced."

Security researcher warning about the real risk of AI agents with system access.

**"If you're about to launch a vibe coded app... read this first"** — r/vibecoding, 1,003 pts, 159 comments
> "I keep seeing people shipping apps built with vibe coding tools and just pushing them live. That's fine... but also slightly terrifying."

A 20-year veteran warning about security, data handling, and deployment risks in vibe-coded apps.

### Implications for Panes

There's a growing narrative that AI coding needs governance — not just for individual users but for organizations. Panes could position the trust layer as organizational governance: "Let your team's non-developers build with AI, safely." This is a potential Phase 4/enterprise angle.

---

## Theme 8: Flows / Structured Orchestration

### Signal: No direct demand signal

No Reddit threads found requesting cross-workspace DAG orchestration, structured multi-agent workflows, pluggable verification, or anything resembling Flows/Harness/Playbooks.

The closest signal is the agentchattr thread (Theme 5), which shows demand for agents communicating with each other — but the solution that resonated was a chat room, not a DAG engine.

### Implications for Panes

Flows are a Phase 3 feature and the Reddit data confirms they should stay there. The user base hasn't matured enough to want structured orchestration — they're still figuring out how to use a single agent reliably. Building Flows before the trust layer and memory are proven would be premature.

---

## Notable Peripheral Signals

### The "Staff SWE Guide to Vibe Coding" — r/vibecoding, 227 pts, 87 comments

A staff software engineer describing their AI workflow. Key insight: even experienced developers are adopting AI-first workflows but need structure and discipline to make it work. Panes's Briefings and memory would serve this persona too.

### "How we vibe code at a FAANG" — r/vibecoding, 1,628 pts, 341 comments

FAANG engineer describing production use of AI coding. Validates that AI coding is moving from toys to production at the highest levels.

### Karpathy: "I haven't written a line of code since December" — r/ClaudeAI, 1,582 pts, 423 comments

> "Karpathy says he hasn't written a line of code since December and is in 'perpetual AI psychosis.' He describes going from 80% writing his own code to 0%, spending 16 hours a day directing agents."

Andrej Karpathy — one of the most influential figures in AI — is now a full-time "agent director." This validates the persona at the highest possible level, though Karpathy is obviously a technical user.

### "No one cares what you built" — r/ClaudeAI, 1,040 pts, 236 comments

Backlash against the flood of "I built X with AI" posts. Suggests the novelty is wearing off and users are moving to "how do I build reliably?" — a maturation signal that favors Panes.

---

## Summary: What Reddit Tells Us About Panes

### The persona is real but early
Non-developers are building with AI today. They have real revenue, real users, and real pain. But they are still a minority of the AI coding user base. The majority of r/vibecoding and r/ClaudeAI users are developers. Panes's secondary persona (power developers managing multiple repos) may be a larger addressable market in the near term.

### The #1 pain point is destruction and trust
Agents delete databases, wipe drives, break codebases, and lie about what they did. This is the most emotionally charged, most widely shared, most mainstream concern in AI coding. Panes's trust layer — gates, risk classification, rollback — is the most validated feature in the entire product.

### The #2 pain point is context amnesia
After month 1, agents forget everything. Users re-explain conventions, agents introduce inconsistencies, codebases become unmaintainable. Memory extraction and Briefings directly address this, and the demand is strong across all skill levels.

### Cost anxiety is pervasive but underserved
Users are blindsided by bills, confused by pricing changes, and building their own tracking tools. Per-thread cost visibility with budget caps is a strong supporting feature.

### Multi-agent orchestration is real but niche
One viral thread (agentchattr, 2.2K pts) validates the pain. Most users use one tool. Agent-agnostic is good engineering, moderate marketing.

### Nobody is asking for Flows yet
Zero organic demand for cross-workspace DAG orchestration. The user base needs to mature through reliable single-agent use (trust + memory) before structured multi-agent workflows become relevant.

### The lead marketing message should be safety, not orchestration
Reddit says: people are terrified of what agents do to their code. The pitch isn't "a dashboard for directing agents." The pitch is **"The safe way to let AI agents work on your code."** Trust first, orchestration second.
