# Settings → Agents: design decisions

Written 2026-08-15, before implementation, so the decisions survive a machine — the lesson from the Kronos plan that existed only in one Mac's session memory.

**Ask:** one place in Settings to see every agent, grant it powers (API keys, capabilities), review its work, and see its HUD.

---

## 1. What is an "agent" here? Three populations, not one

This is the decision that makes or breaks the page. The codebase has three distinct things that all get called agents:

| Population | Where | Count | Nature |
|---|---|---|---|
| **Background workers** | `WORKER_DESCRIPTORS` — scheduler, librarian, steward, initiative, echo (Watcher), onboarding coach | 6 | Always-on, run themselves, not dispatchable |
| **Dispatch personas** | `WorkerPersona` in `agent.yaml` (e.g. `claude_code`) | N | Goals are dispatched *to* them; have an `engine` (external_cli / supervised_cli / internal subagent / registered-but-unrunnable) and an `availability_check` |
| **Capabilities** | `PLATFORM_EXTENSIONS` — 30, incl. The Financier | 30 | Tools the agent calls. Some are agent-*like* (Librarian, Git Steward appear in both lists) |

The user's own example — "the financial datasets key for the Financier" — is about the **third** population. The Financier is a platform extension, not a worker or a persona. So the page must cover all three or it will not answer the question that prompted it.

**Decision: one page, three sections, each honest about what it is.**

- **Workers** — status, last run, what it did, live queryable state. Not dispatchable; do not offer a "dispatch" affordance.
- **Dispatch roster** — engine, live availability probe, cost rank, recent dispatches. This is where per-agent grants primarily apply.
- **Capabilities** — enabled/disabled, which keys they require, whether each key is present. This is where the Financier + financial-datasets key lives.

A single flat list would imply a background worker can be dispatched and a persona is always running. Both false.

## 2. Powers: per-agent grants (chosen)

**Today:** `is_extension_enabled(key)` is **global** — an extension is on or off for everything. There is no per-agent scoping.

**The seam already exists in adjacent form:** `resolve_extensions_for_new_session(recipe.extensions, …)` scopes extensions per *run*, which recipes and headless scheduled jobs already use. Per-agent grants extend that seam rather than inventing one.

**Secrets need no new subsystem.** `Config::set_secret` is a flat keyspace, and `grow_analytics.rs` already scopes secrets per project by namespacing the key (`api_key_secret_key(&project.id)`). The same convention scopes a secret to an agent. Follow it exactly — do not build a parallel store.

**Constraint carried from the audit:** secrets are presence-only on every read path. `observe_app`'s settings surface returns `present: bool` and never a value, with an explicit `// NEVER include the value`. The new API must hold that line, and the test that proves it must run in CI — that test was `#[ignore]`d until P2-20 and is now flaky-fixed; do not let a new endpoint escape it.

## 3. Consolidation: canonical, others link in (chosen)

There are already three partial surfaces, and a fourth that ignores them would recreate exactly the drift the 2026-08 audit existed to fix:

- **World view** — `WorldHUD`, `StrixHUD`, `ReaderHUD`, `HudShell`, `AgentPicker`. Real HUDs, but inside a 3D rotunda.
- **Automate → "Agents at work"** — live roster of workers, scheduled jobs, active sessions, with one-click stop.
- **Settings → Models** — the worker roster each role can dispatch to.

**Decision:** Settings → Agents becomes canonical. The other two remain as live views and deep-link into it. Do **not** duplicate their state; read the same sources.

**HUD:** do not rebuild it. `HudShell` is already a component. Reuse it in the agent detail panel, or link to the World view focused on that agent. Rebuilding would give two HUDs that disagree.

## 4. Work review: wire up what exists (chosen)

Nothing new needs recording. Filter existing sources by agent:

- `activity_journal` — what it did, when, with an evidence pointer (now append-only, per P2-14)
- goal outcomes — landed / blocked / failed, and the review decision
- cost ledger — what it spent (note P1-9: unpriced calls used to book as `$0.00`; that is fixed, so the number is now trustworthy)
- scheduled-job run history — success/failure, `consecutive_failures`, paused state
- Grow predictions and how the 7/14/28-day sweep judged them, where applicable

**Do not add a new table.** If a fact is not already recorded, that is a separate decision, not a side effect of building a page.

## 5. Standing rules this must satisfy

- **Self-knowledge descriptor in the same change.** `dispatch.md`: "A capability Henry can DO but can't DESCRIBE is a bug." Precedent exists for Settings pages — "Session history" and "Execution trace" are both Settings pages with `SURFACE_DESCRIPTORS` entries.
- **The descriptor must be true.** The audit found four false absolutes shipping in this brief every turn. If grants are advisory rather than enforced, say advisory.
- **Empty vs unavailable.** A worker whose status could not be read must not render as idle. This is the defect class the whole observability feature exists to prevent (`picker.rs` states it best: "a stale pick rendered as today's pick is worse than an empty surface").
- **Phantom-tool guard.** Descriptor prose naming a non-tool token needs a `NON_TOOL_PROSE_TOKENS` entry with a reason — this bit two PRs already.

## 6. Sequencing

**Phase 1 (backend, must land first):** per-agent grant model on `WorkerPersona`, agent-scoped extension resolution extending `resolve_extensions_for_new_session`, per-agent secret namespacing, and the `/agents` API — roster, detail, work review, grant/secret mutation. One lane, so the grant shape and the API contract cannot disagree.

**Phase 2 (UI):** Settings → Agents page over that API, HUD reuse, deep links from Automate and World, and the self-knowledge descriptor.

Gate between them: the API is real and tested before any UI is written against a guessed shape.
