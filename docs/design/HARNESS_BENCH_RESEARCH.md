# Harness Benchmark Research — what the *harness* buys you, and the cheapest way to measure it

**Status:** research note (untracked). No code changes.
**Date of survey:** 2026-09-02.
**Author lane:** deep online research (WebFetch; WebSearch budget was exhausted this session, so every number below is anchored to a fetched URL rather than a live search).
**Audience:** Permagent harness team.

> **Why this doc exists (from memory / Jesse's framing).** This is "the harness bench page." It adopts Prime Intellect's thesis — *harness quality > model price* — and uses **a held-constant cheap model (Haiku-class) as the test column**, so that any score delta is attributable to the harness rather than the model. The sharpened goal is not merely "reach SWE-bench parity." It is to **synthesize the best of five named harnesses — Claude Code, Codex, Hermes, Prime Agent, and Pi — into Permagent**, under a hard downstream cost constraint. This doc is a blueprint for a best-of-five harness plus the cheapest credible protocol to tune it.

---

## 0. TL;DR — the four answers, up front

1. **Benchmark that best isolates HARNESS quality:** **Terminal-Bench** (agentic, end-to-end, standardized `Terminus` harness slot, model held separately) and **SWE-Lancer Diamond** (real multi-file tasks graded by end-to-end Playwright tests) are the best *harness* discriminators as of 2026. Plain **SWE-bench Verified is saturating** (frontier models now ~88–96%, see §1.5) and is therefore a *weakening* harness discriminator at the top — but its **Lite (300)** and a **~50-task curated Verified subset** remain the cheapest way to get harness-tuning signal. **SWE-bench Pro** (Scale) is the harder successor when Verified saturates.
2. **Ranked harness techniques by score-impact** (evidence in §3): (1) localization / find-the-right-file; (2) edit-apply reliability + recovery; (3) test-driven self-verification loop; (4) multi-attempt / search (MCTS, majority selection); (5) context compaction + sub-agent fan-out; (6) tool/ACI design; (7) planning-first (to-do list); (8) tool-result pruning. The top three are worth *more* than a model tier on the cheap end.
3. **Prime Agent** is real: Prime Intellect's self-improving harness, **built on top of `pi`** (earendil-works), organized around a **Recursive Language Model (RLM)** and a **"Continual Harness."** It publishes ARC-AGI-3 and long-context numbers, **not** a SWE-bench head-to-head — **there is no Prime vs. Permagent (or vs. Claude Code) SWE-bench number to compare against.** We compare against its *primitives and thesis*, not a score.
4. **Cheapest credible protocol** (§5): three tiers — **$0 offline harness unit-metrics** (does the loop retry / prune / apply-edits / compact correctly, run against a mock model) → **Haiku-held-constant on a ~50-task Verified subset** for harness A/B → occasional **SWE-bench Lite 300** for full-signal. Anchor the cost ceiling to **Agentless's $0.70/instance**.

---

## 1. The benchmarks

### 1.1 SWE-bench family
Source: <https://www.swebench.com/>, <https://github.com/SWE-bench/SWE-bench>, <https://github.com/swe-bench/experiments>

- **What it measures:** given a real GitHub repo + issue, the system must produce a patch. Scored by hidden tests: a `FAIL_TO_PASS` set must flip to passing and a `PASS_TO_PASS` set must stay green. Metric is **pass@1 % resolved**.
- **Variants & task counts:**
  - **Full:** 2,294 tasks.
  - **Lite:** 300 tasks (the canonical cheap subset).
  - **Verified:** 500 human-confirmed-solvable tasks (Epoch runs 484 of them; 16 don't run reliably — <https://epoch.ai/benchmarks/swe-bench-verified>).
  - **Multimodal:** 517 tasks (JS/visual; "480 available for local evaluation" in v2 per the repo README).
  - **Multilingual** and a **Bash-only** leaderboard also exist.
- **Harness vs. model:** SWE-bench is *scaffold-agnostic by construction* — you submit predictions from whatever harness you like. That is exactly why it exposes harness effect. **Epoch performed a "major upgrade of scaffolding, environments, and token limits" (v2.0.0, Feb 2026) that "led to model performance improving significantly"** — i.e. the same models scored materially higher purely from a better scaffold. That is the single cleanest published statement that the *harness* moves the number.
- **Cost:** highly harness-dependent. Agentless-style pipelines cost **~$0.70/instance** (§3.1); heavy agent loops with a frontier model can run **$1–$10+/instance**, so a full Verified pass with an expensive agent is easily $500–$5,000; Lite (300) is ~⅗ of that; a 50-task subset is ~1/10 of Lite.
- **Submission bar (11/2025 policy):** leaderboard submissions now require an arXiv/tech-report link and an academic/established-lab author affiliation — relevant if Permagent ever wants an *official* entry vs. an internal number.

### 1.2 Terminal-Bench
Source: <https://www.tbench.ai/>, <https://github.com/laude-institute/terminal-bench>, Warp writeup <https://www.warp.dev/blog/terminal-bench>

- **What it measures:** end-to-end *terminal* tasks — compile code, train a model, set up a server — done **autonomously** in a sandboxed shell. Each task ships a **test script** that verifies success (pass/fail), plus an oracle solution.
- **Task count:** "~100 tasks" (Core v0.1.1 was the first leaderboard set; the site now shows **Terminal-Bench 2.1 / "4.0"** generations). Hosted by **Stanford, Harbor, and the Laude Institute**; explicit "keep out of training data" note.
- **Harness isolation — this is the important one:** Terminal-Bench deliberately splits **model** from **agent/harness adapter**. It ships a reference harness (**`Terminus` / Terminus-2**) into which any model is dropped, and the leaderboard columns are literally `MODEL | AGENT | RESOLUTION RATE | COST | TOKENS`. Because the task is raw shell (no special ACI handed to you), *the harness has to supply its own file-nav, edit-apply, long-running-command handling, and recovery* — so the score reflects harness engineering more directly than SWE-bench does.
- **Reference numbers:** **Warp reached #1 at 52% on v0.1.1** (~June 2025), "almost 9 percentage points ahead of the next submission." Anthropic reports **Claude Opus 4.5 (Nov 24 2025)** as SOTA-class here (Warp's CEO: **"+15% over Sonnet 4.5 on Terminal Bench"**), run with a **128K thinking budget** on the **Terminus-2** harness. Current frontier (Vellum, §1.5) sits ~85–89%.
- **Cost:** the leaderboard publishes per-run **cost and tokens** per system — the only major benchmark that makes harness *efficiency* a first-class, visible metric. Good target for a cost-conscious harness.

### 1.3 SWE-Lancer
Source: <https://arxiv.org/abs/2502.12115> (v3 HTML: <https://arxiv.org/html/2502.12115v3>), <https://openai.com/index/swe-lancer/>

- **What it measures:** 1,488 real freelance tasks from the Expensify repo on Upwork, **valued at $1M total in real payouts**, from "$50 bug fixes to $32,000 feature implementations." Maps agent capability to **dollars earned**.
- **Split:** **IC SWE** (write the patch) = 764 tasks / $414,775; **SWE Manager** (pick the best of several proposals) = 724 tasks / $585,225.
- **Public cheap subset — "SWE-Lancer Diamond":** **502 tasks worth $500,800** (237 IC / $236,300 + 265 Manager / $264,500), with a private $499,200 holdout to resist contamination. Unified public Docker image is provided.
- **Scoring:** **end-to-end tests written by professional engineers using Playwright browser automation** (triple-checked) — not unit tests, so it rewards a harness that can actually run the app and observe UI behavior. Manager tasks graded against the real hiring manager's choice.
- **Harness setup (as OpenAI ran it):** isolated Docker, no internet, **pass@1, max 100 tool calls, 3-hour limit**, plus a **"user tool"** that runs Playwright scripts and returns logs + screenshots for the agent to iterate on. Best reported: **Claude 3.5 Sonnet earned $208,050 on Diamond** (26.2% IC pass@1 / 44.9% Manager); frontier "still can't solve the majority."
- **Why it's a good harness discriminator:** the value is in *running and verifying against a real app via the browser tool* — a harness capability, not a model fact. It's more expensive to run than SWE-bench (Playwright, 3h budgets), so treat it as a *periodic* signal, not the daily loop.

### 1.4 SWE-bench Pro (the successor when Verified saturates)
Source: <https://labs.scale.com/leaderboard/swe_bench_pro_public>

- **What it is (Scale/Labs):** **1,865 tasks over 41 professional repos** — public 731 (GPL repos), private 276 (startup codebases), held-out 858. Deliberately targets Verified's four weaknesses: contamination, low task diversity, over-simple problems, flaky envs.
- **Difficulty:** **longer-horizon, multi-file, averaging 107.4 LOC per solution.** At launch the best frontier models (GPT-5, Claude Opus 4.1) scored only **~23.3% / 23.1%** on the public set — vs. >70% on Verified. As of this survey the public leaderboard shows **Muse Spark 1.1 61.5%, gpt-5.4 (xHigh) 59.1%, Muse Spark 55.0%, Claude Opus 4.6 (thinking) 51.9%**.
- **Use for us:** the benchmark to graduate to once Verified stops separating harnesses (which, per §1.5, is happening now).

### 1.5 The saturation caveat (why the top of Verified no longer isolates harness)
Source: Vellum leaderboard <https://www.vellum.ai/llm-leaderboard> (a third-party aggregator; treat exact figures as *directional* — several listed model names are newer than this author's training data and were not independently confirmed).

- Current top SWE-bench ("agentic coding") figures sit around **88–96%** for the frontier tier, and Terminal-Bench 2.1 around **85–89%**. When the ceiling is ~95%, a 2-point harness improvement is inside the noise band and swamped by model choice. **Implication:** for *harness* A/B, either (a) hold a **weak model constant** so headroom is large (the Haiku column), or (b) move to **SWE-bench Pro / SWE-Lancer / Terminal-Bench**, where scores are still 20–60% and a harness change is visible.

### 1.6 Which benchmark best isolates HARNESS quality?
| Benchmark | Harness-isolation | Why | Cheap subset |
|---|---|---|---|
| **Terminal-Bench** | **Best** | raw shell, no ACI given, model dropped into a named agent slot; cost/tokens published | ~100 tasks is already small; run the Core set |
| **SWE-Lancer Diamond** | **High** | graded by running the real app (Playwright); value = harness's verify loop | Diamond 502 (public) |
| **SWE-bench Pro** | **High** | multi-file/long-horizon defeats "lucky one-file" model wins | public 731 |
| **SWE-bench Verified** | **Medium, falling** | scaffold-agnostic (good) but saturating at the top (bad) | **Lite 300 / ~50-task subset** |
| **Aider polyglot** | **Medium** | isolates *edit-format / edit-apply* specifically | 225 Exercism tasks |

**Verdict:** to *isolate* harness quality, prefer **Terminal-Bench** (cheap, ~100 tasks, cost-aware) as the daily discriminator and **SWE-Lancer Diamond** as the periodic "does it verify against a real app" check. Use **Verified-subset-with-Haiku** as the ultra-cheap daily A/B (§5).

---

## 2. The five-harness spine (identify → signature technique → public standing)

### 2.1 Claude Code — *context & sub-agent management + skills*
Source: <https://code.claude.com/docs/en/best-practices>, <https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents>, <https://www.anthropic.com/engineering/swe-bench-sonnet>
- **Identity:** Anthropic's agentic coding harness (CLI + web + desktop).
- **Signature technique — treating the context window as *the* scarce resource.** Concretely: **automatic compaction** + manual `/compact <focus>` + `/clear` between tasks; **sub-agents for investigation** (research runs in a *separate* context and reports back a 1,000–2,000-token summary — keeps the main loop clean); **CLAUDE.md** persistent project memory (loaded every session, ruthlessly pruned); **on-demand Skills** (`SKILL.md`) so domain knowledge loads only when relevant; **hooks** for deterministic must-happen actions; **plan mode** (explore→plan→implement→commit); **checkpoints/`/rewind`**; **`/batch` fan-out** across 5–30 subagents each in its own worktree/PR; and a first-class **verification stance** — `/goal` re-checks a condition after every turn, an **adversarial review subagent** grades the diff in a fresh context, and the docs push "show evidence, don't assert success."
- **Public standing:** Anthropic's own minimal scaffold got **Claude 3.5 Sonnet to 49% on SWE-bench Verified** (beating the then-45% SOTA) with just **bash + edit** tools — and stated the load-bearing claim: *"the performance of an agent on SWE-bench can vary significantly based on this scaffolding, even when using the same underlying AI model."* Opus 4.5 is current SOTA-class on Verified/Terminal-Bench.

### 2.2 Codex — *sandboxed execution + the approval / auto-review loop*
Source: <https://github.com/openai/codex>, <https://learn.chatgpt.com/codex/sandboxing>
- **Identity:** OpenAI's "lightweight coding agent that runs in your terminal" (Codex CLI, Apache-2.0) + the GPT-5-Codex model line. **We already integrate Codex locally as `codex-companion`**, so its shape is familiar precedent.
- **Signature technique — safety-gated execution as a first-class harness layer.** Platform-native sandboxing applied to *spawned commands* (not just file writes): **Seatbelt on macOS, bubblewrap/user-namespaces on Linux/WSL2, native sandbox on Windows**, with **network disabled by default** behind an allowlist. Three approval modes — **Ask for Approval** (default: read/edit workspace + routine commands, ask to leave the sandbox), **Approve for Me** (escalations route to an **automatic reviewer agent**, `approvals_reviewer = "auto_review"`), and **Full Access** (`danger-full-access`, `approval_policy = "never"`). **`AGENTS.md`** is its project-memory analogue. The **auto-review agent** is the distinctive bit: an automated reviewer adjudicates boundary-crossing actions instead of always bouncing to the human.
- **Public standing:** OpenAI positions GPT-5-Codex as top-tier on SWE-bench Verified (frontier band, §1.5); the harness itself is praised for *unattended safety* — you can grant more autonomy because the sandbox+approval loop contains blast radius.

### 2.3 Hermes — *the built-in learning loop (self-evolution)*
Source: <https://github.com/NousResearch/hermes-agent>, <https://github.com/NousResearch> (also `hermes-agent-self-evolution`, `atropos`)
- **Identity (disambiguated):** **Nous Research's `hermes-agent`** — "the self-improving AI agent," *not* the Hermes *models* and not an unrelated project of the same name. This is the correct referent because it is (a) an agent *harness* (multi-platform gateway, tools, memory), and (b) explicitly self-improving, matching the "harness that grows" framing. (Nous also ships `hermes-agent-self-evolution`, which optimizes it via **DSPy + GEPA**, and `atropos`, an RL environment framework.)
- **Signature technique — procedural memory + autonomous skill creation.** It **generates new skills from complex tasks and refines them in use** (closed-loop, agentskills.io-compatible), keeps **agent-curated memory** with "periodic nudges," models the user (Honcho dialectic), and does **cross-session recall via FTS5 full-text search + LLM summarization**. Also: 40+ pluggable tool backends (Docker/SSH/Modal/…) and **parallel isolated sub-agents**. The ONE distinctive thing vs. the other four: **self-improvement is a runtime property, not a manual config step** — the harness edits its own skill/memory set from experience.
- **Public standing:** community/qualitative (large GitHub following); **no published benchmark scores.** Honestly: its claim is *personalization + longevity*, not a leaderboard number.

### 2.4 Prime Agent — *the harness-over-model thesis + RLM/goal primitives*
Source: <https://www.primeintellect.ai/blog/prime-agent>, <https://www.primeintellect.ai/>
- **Identity:** Prime Intellect's self-improving harness, installable via CLI, **built on top of `pi`** (see 2.5). Thesis, verbatim in spirit: *"modern harness designs were built around the capabilities of earlier generations of models"* and fail to exploit frontier models — so the **harness itself should be adaptive and learnable** (this is the "harness quality > model price" thesis our memory records).
- **Signature techniques (its two abstractions):**
  - **Recursive Language Model (RLM):** context is *variable*; the agent runs in a **persistent IPython kernel** and delegates to sub-agents *as async function calls* (`rlm(...)` — "Programmatic Tool Calling"), keeping history/tools/sub-agents as **REPL variables** so arbitrarily long sessions don't lose state.
  - **Continual Harness:** harness state (prompts, skills, memory, sub-agents) is **CRUD-able by the agent at runtime**, plus **A2A messaging** across a "nuclear family" (parent/sibling/child) and even across separate sessions.
  - Plus **`/refine`** (reads its own trajectories, applies targeted CRUD edits in the background) and **Autonomous Mode** (persistent goals + optional **token budgets** + scheduled heartbeats).
- **Public standing / honesty:** it reports **ARC-AGI-3: 95.5% RHAE Best@1 with Opus 5 (> the 95.4% human-expert baseline), at lower token usage than native harnesses**; long-context wins vs. **Pi-mono** (OOLONG 0.700 vs 0.420, OOLONG-Pairs 0.874 vs 0.556, LongBench-Pro 0.777 vs 0.768); and **EmulatorBench** (rebuilt SEGA Genesis + Game Boy Color emulators in Rust). It candidly reports reward-hacking in Factorio. **Crucially: no SWE-bench / Terminal-Bench head-to-head is published.** So the comparable is its **primitives and methodology**, not a score — consistent with memory's "no Prime head-to-head benchmark exists." (Our repo already tracks closing these six primitive gaps: rework-budget, async fan-out, A2A goal addressing, RLM kernel, executable skills — see `docs/architecture/PRIME_INTEGRATION_SEAMS.md`.)

### 2.5 Pi — *the minimal, provider-neutral agent substrate*
Source: <https://github.com/earendil-works/pi>, <https://github.com/earendil-works>
- **Identity:** **`earendil-works/pi`** — "AI agent toolkit: unified LLM API, agent loop, TUI, coding agent CLI." A monorepo: **Pi Coding Agent** (CLI), **Pi Agent Core** (runtime: tool invocation + state), **Pi AI** (unified multi-provider LLM API). **Prime Agent is built on top of pi** — so pi is the *substrate*, Prime Agent is the *self-improving layer* on it.
- **Signature technique — deliberate minimalism + externalized safety.** Pi **ships no built-in permission system**; it explicitly pushes isolation *out* to containerization (`gondolin` microVMs, Docker, OpenShell). This is the opposite design pole from Codex (which builds the sandbox *in*). Its value is a clean, provider-agnostic loop + TUI with differential rendering and vendor-neutral telemetry. Note also **`pi-review` / `pi-review-loop`** repos in the same org — a review-loop primitive.
- **Public standing:** infra/plumbing; **no benchmark numbers.** Its endorsement is that Prime Intellect chose it as the base for Prime Agent.

---

## 3. Harness techniques ranked by score-impact (with evidence)

> Ranking is by *published, attributable* score movement, weighted toward the cheap end (weak model, tight budget) where harness leverage is largest.

1. **Localization / find-the-right-file (highest leverage).** **Agentless** — a plain *localize → repair → validate* pipeline with **no agent, no tools** — hit **32.0% on SWE-bench Lite at $0.70/instance**, beating contemporary open agents. Getting the *right file* is most of the battle; a weak model that edits the right file beats a strong model editing the wrong one. Source: <https://arxiv.org/abs/2407.01489>.
2. **Edit-apply reliability + malformed-output recovery.** Aider's leaderboard exposes this directly: **% well-formed responses ranges 64–100%**, and the harness's recovery pass lifts results materially (e.g. **gpt-5 (high) 52% → 88%** between pass-1 and pass-2 by tolerating/repairing imperfect edits). **Diff-format > whole-file** edits. A harness that never drops a valid edit is worth a model tier here. Source: <https://aider.chat/docs/leaderboards/>.
3. **Test-driven self-verification loop.** Devin: **72% of passing runs took >10 min**, i.e. iterating against test feedback is *how* it passes; **handing it the final unit tests raised its success from 13.86% → 23%.** Claude Code's whole "give it a check it can run / `/goal` / adversarial-review-subagent" doctrine is the productized version. Sources: <https://cognition.com/blog/swe-bench-technical-report>, <https://code.claude.com/docs/en/best-practices>.
4. **Multi-attempt / search / selection.** **SWE-Search (MCTS + value + discriminator agents): +23% relative across five models** vs. non-search agents; scales with inference-time compute. Best-of-N with a good selector is the reliable-but-costly lever. Source: <https://arxiv.org/abs/2410.20285>.
5. **Context compaction + sub-agent fan-out.** Anthropic's context-engineering guidance: **compaction** (summarize-and-reinitialize), **structured note-taking** (`NOTES.md` outside the window — cited via Claude-plays-Pokémon keeping tallies over thousands of steps), and **sub-agents returning 1–2k-token summaries** enable "long-horizon strategies impossible when keeping everything in-context." Impact is qualitative ("substantial improvement") but it's the enabler for everything above on long tasks. Source: <https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents>.
6. **Tool / Agent-Computer-Interface (ACI) design.** SWE-agent's thesis: a purpose-built ACI for nav/edit/test "significantly enhances" the agent; **"the design of the ACI can impact agents' behavior and performance."** Same model + better tool descriptions = better score (Anthropic echoes: "we put a lot of effort into the descriptions and specs for these tools"). Sources: <https://arxiv.org/abs/2405.15793>, <https://www.anthropic.com/engineering/swe-bench-sonnet>.
7. **Planning-first (to-do list) + long-running-command control.** Warp's #1 Terminal-Bench run credited an **editable to-do list** ("forces reasoning before implementation"), an **Opus-4 planner in front of a Sonnet-4 executor**, and **pty control** for interactive tools (REPL/vim). Source: <https://www.warp.dev/blog/terminal-bench>.
8. **Tool-result pruning / clearing.** Called out as a "lightweight compaction" in Anthropic's context doc; cheap to implement, protects the window, but smallest standalone score effect. Source: same as #5.

**Meta-evidence that harness (not model) moves the number:** Epoch's Feb-2026 **scaffold upgrade (v2.0.0) alone "led to model performance improving significantly"** on identical models (<https://epoch.ai/benchmarks/swe-bench-verified>); and **mini-swe-agent scores >74% on SWE-bench Verified in ~100 lines of bash-only Python** (<https://github.com/SWE-agent/mini-swe-agent>) — proof that most of the score is reachable with a tiny-but-correct loop, and therefore that *loop correctness*, not baroque tooling, is what pays.

---

## 4. Synthesis — the best-of-five blueprint

For each capability: which of the five does it best, what Permagent should borrow, and the **seam to hand to the parallel code-audit lane** (this doc does not assert what Permagent already has — it flags where to look).

| Capability | Best-in-class (of the 5) | What to borrow | Permagent seam to verify (audit lane) |
|---|---|---|---|
| **Context compaction / window discipline** | **Claude Code** (`/compact`, `/clear`, auto-compaction preserving files+decisions) | Summarize-and-reinitialize with a *configurable preserve-list* (modified files, test cmds) | daemon loop's compaction/summarization path; is there a preserve-list? |
| **Sub-agent fan-out for exploration** | **Claude Code** (investigation subagents → 1–2k-token summaries) / **Prime Agent** (`rlm()` async PTC) | Research in a *separate context* that returns only a summary; async fan-out with mid-flight steer | orchestrator/subagent spawn; does exploration pollute main context? (`feat/prime-async-fanout`) |
| **Self-verification / test-driven stop** | **Claude Code** (`/goal` re-check every turn; adversarial review subagent; "show evidence") | A stop-gate that a *fresh* model grades against a runnable check before "done" | is there a verify-before-done gate + evidence requirement? (ties to the repo's review-gate memory) |
| **Localization** | (field: **Agentless**) — none of the 5 advertises it | A cheap localize-first pass before edit; it's the #1 lever and the cheapest | is repo-mapping/localization a distinct step, or implicit in the model? |
| **Edit-apply reliability + recovery** | (field: **Aider**) / Codex & Claude Code productize it | Diff-format edits + a *repair* pass on malformed edits; never silently drop a valid edit | edit-apply tool: does it retry/repair malformed patches? |
| **Sandboxed execution + approval loop** | **Codex** (Seatbelt/bubblewrap; Ask/Auto-review/Full; net-off default) | Command-level sandbox + an **auto-review agent** for boundary escalations (not just human bounce) | we already run `codex-companion` + Cursor — reuse that sandbox shape for native tools? |
| **Runtime self-improvement (skills/memory)** | **Hermes** (autonomous skill creation; FTS5 memory; DSPy/GEPA) / **Prime Agent** (`/refine` CRUD on own prompts/skills) | Executable skills the agent can create/refine; procedural memory with cross-session recall | `feat/prime-executable-skills`, `PRIME_RLM_AND_SKILLS.md`; Brain vs. RLM-kernel storage |
| **Persistent state / long-horizon** | **Prime Agent** (RLM: history/tools as REPL variables; token-budgeted autonomous goals) | RLM-kernel as durable state (memory: **transactional SQLite in the daemon**, write-through summary to Brain) | `feat/prime-rlm-kernel` — is state a durable table, not just context? |
| **A2A / goal addressing** | **Prime Agent** (A2A messaging; persistent goals w/ budgets) | Address goals to peer sessions; rework-budget per goal | `feat/prime-a2a-goal-addressing`, `feat/prime-rework-budget` |
| **Provider-neutral minimal loop** | **Pi** (unified LLM API; ~100-line-agent ethos, cf. mini-swe-agent) | Keep the core loop tiny + provider-agnostic; push isolation to containers | is the core loop small/testable enough to unit-metric offline? |
| **Planning-first** | **Warp** technique (via Terminal-Bench) / Claude Code plan mode | An editable to-do/plan before implementation on multi-file tasks | plan-mode equivalent for multi-file changes |
| **Cost/efficiency as a metric** | **Terminal-Bench discipline** (cost+tokens published per run) | Emit per-run cost/tokens so harness changes are judged on cost, not just score | telemetry: do we record cost/tokens per run? |

**One-line blueprint:** *Pi's minimal provider-neutral loop* + *Codex's sandbox/auto-review safety* + *Claude Code's context/subagent/verify discipline* + *Prime Agent's RLM-kernel + A2A/goal primitives* + *Hermes's self-created executable skills & procedural memory* — measured the *Terminal-Bench way* (cost+tokens per run).

---

## 5. The cheap-signal protocol (the gating constraint)

**Goal:** minimum spend that still tells you a harness change helped. Three tiers, cheapest first.

**Tier 0 — $0 offline harness unit-metrics (run in CI, no model spend).**
Mock the model and assert the *loop* behaves. These are the behaviors §3 says matter, tested deterministically:
- **Retry/recovery:** feed a malformed edit / a failing tool call → assert the harness repairs or retries instead of aborting.
- **Edit-apply:** feed a valid diff against a fixture → assert it applies byte-exact; feed a near-miss → assert repair.
- **Localization:** given a seeded repo + issue, assert the localizer surfaces the known-correct file in top-k.
- **Compaction/pruning:** drive context past threshold → assert summary retains the preserve-list (modified files, test cmds) and drops tool spew.
- **Verify-gate:** assert "done" is blocked until a runnable check passes.
These cost $0, run every commit, and catch regressions *before* any benchmark spend. This is the highest-ROI tier and should exist regardless.

**Tier 1 — Haiku-held-constant on a ~50-task Verified subset (the "Haiku column").**
- **Model:** a fixed Haiku-class model for *every* harness variant, so score deltas are attributable to the harness, not the model. (Weak model = large headroom = harness change is visible, avoiding the §1.5 saturation trap.)
- **Tasks:** a **curated ~50-task subset of SWE-bench Verified** (stratify by repo + difficulty; freeze the list so runs are comparable). Not an official split — an internal fixed set.
- **Cost anchor:** hold per-instance cost near **Agentless's $0.70** ceiling; 50 tasks ≈ **~$35/run** with a cheap model, so A/B'ing two harness variants is ~$70 — a coffee, not a budget line.
- **Cache** aggressively (prompt/tool-result caching; the 15-min fetch cache pattern) so repeated runs during tuning are near-free.

**Tier 2 — periodic full-signal (weekly / pre-release).**
- **SWE-bench Lite (300)** with the Haiku column for a broader read (~6× Tier-1 cost).
- **Terminal-Bench Core (~100)** for the *cleanest harness-isolation* signal, recording **cost + tokens** per run (adopt its metric).
- **SWE-Lancer Diamond** occasionally to confirm the *verify-against-a-real-app* loop works (Playwright; the expensive one — quarterly, not weekly).
- Graduate to **SWE-bench Pro public (731)** if/when the Haiku column on Verified stops separating variants.

**What NOT to pay for early:** full SWE-bench Verified/Full with a frontier model (hundreds–thousands of $ and, at saturation, low harness signal); ensembling/MCTS best-of-N during *tuning* (it's a score-buying deploy-time lever, not a signal for whether the *base loop* improved).

---

## 6. Honest gaps / caveats
- **Prime Agent has no SWE-bench/Terminal-Bench head-to-head** — we compare primitives/thesis, not a number (confirmed against memory and the blog).
- **Hermes and Pi publish no benchmark scores** — their case is qualitative (self-evolution; minimal substrate).
- **Codex's exact SWE-bench Verified figure** was not fetched from a first-party page this session (openai.com 403s WebFetch); treat "frontier band" as the claim, not a precise %.
- **§1.5 top-line numbers** come from a third-party aggregator and include model names newer than the author's training cutoff — directional, verify before quoting externally.
- **WebSearch budget was exhausted**, so this survey is WebFetch-only; a follow-up with search could add SWE-bench Multimodal detail, exact Codex/GPT-5-Codex figures, and Terminal-Bench 2.1's current top table.

---

## 7. Sources
- SWE-bench: <https://www.swebench.com/> · <https://github.com/SWE-bench/SWE-bench> · <https://github.com/swe-bench/experiments> · <https://epoch.ai/benchmarks/swe-bench-verified>
- Terminal-Bench: <https://www.tbench.ai/> · <https://github.com/laude-institute/terminal-bench> · <https://www.warp.dev/blog/terminal-bench>
- SWE-Lancer: <https://arxiv.org/abs/2502.12115> · <https://arxiv.org/html/2502.12115v3> · <https://openai.com/index/swe-lancer/>
- SWE-bench Pro: <https://labs.scale.com/leaderboard/swe_bench_pro_public>
- Aider leaderboards: <https://aider.chat/docs/leaderboards/>
- Agentless: <https://arxiv.org/abs/2407.01489>
- SWE-agent (ACI): <https://arxiv.org/abs/2405.15793>
- SWE-Search (MCTS): <https://arxiv.org/abs/2410.20285>
- mini-swe-agent: <https://github.com/SWE-agent/mini-swe-agent>
- Devin technical report: <https://cognition.com/blog/swe-bench-technical-report>
- Claude Code: <https://code.claude.com/docs/en/best-practices> · <https://www.anthropic.com/engineering/swe-bench-sonnet> · <https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents> · <https://www.anthropic.com/news/claude-opus-4-5>
- Codex: <https://github.com/openai/codex> · <https://learn.chatgpt.com/codex/sandboxing>
- Hermes: <https://github.com/NousResearch/hermes-agent> · <https://github.com/NousResearch>
- Prime Agent: <https://www.primeintellect.ai/blog/prime-agent> · <https://www.primeintellect.ai/>
- Pi: <https://github.com/earendil-works/pi> · <https://github.com/earendil-works>
- Frontier leaderboard (directional): <https://www.vellum.ai/llm-leaderboard>
