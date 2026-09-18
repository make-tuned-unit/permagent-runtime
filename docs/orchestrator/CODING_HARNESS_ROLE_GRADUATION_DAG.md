# Coding Harness Role Graduation DAG (X-grad)

Status: design — 2026-09-09. Owns Excellence DAG E3.1/E3.5 and the Cheap-model-usability row
(`CODING_HARNESS_EXCELLENCE_DAG.md`: "eligible for a role only after passing that role's smoke
plus ≥80% of the curated micro-suite and 95% of tool/DAG-selection checks"). Companion to
`CODING_HARNESS_ROUTING_DAG_5.md` R2/R3 (role-specific graduation, cheapest-capable selection),
which this DAG supplies the evidence store for. No code in this document; a code lane implements it.

## Objective

One capability card per provider/model, built only from retained `permagent-eval` evidence, that
dispatch consults before letting a model coordinate and that clears graduated mechanical/edit/review
work without re-litigating capability every turn. No model gets a default grant — the gate in
`config/model_roles.rs:76-106` already states this for coordinators; this DAG extends it to every
role and backs it with a durable, inspectable store instead of a compiled-in single trusted route.

```text
G0 freeze card schema + evidence-id contract
 ├── G1 durable card store (spectral_schema ladder step, beside skills/rlm_context precedent)
 └── G2 evidence pipeline: retained tier runs -> per-role fields
          │
          ▼
 G3 tool/DAG-selection micro-fixtures (the 95% metric — does not exist today)
          │
          ▼
 G4 dispatch consult: extend assess_harness_coordinator's fallback to
    list_workers/check_worker/ranked selection; mechanical roles allowed
          │
          ▼
 G5 deterministic + adversarial tests (card-from-fixture -> routing decision)
          │
          ▼
 G6 migrate today's four models onto the card; promote gate
```

## Gate ledger

| Node | Evidence required to pass |
|---|---|
| G0 | schema rejects a graduation claim with no evidence-run id, stale evidence, or an unrated role |
| G1 | card persists across restart; one row per (provider, model, role), additive migration |
| G2 | every card field traces to a specific retained `permagent-eval` JSON, not a human label |
| G3 | tool-selection and DAG-selection fixtures exist and run deterministically |
| G4 | ungraduated coordinator dispatch is refused with a reason the user sees, not just a log line |
| G5 | routing-matrix test: same card + fixtures -> same decision, twice |
| G6 | glm-4.7, deepseek-chat, qwen2.5:7b, claude-sonnet-5 each have a card row (even if `unrated`) |

## G0 — Capability card schema

| Field | Type | Source |
|---|---|---|
| `provider`, `model` | string | matches `cost_router::knowledge::lookup` key (`knowledge.rs:63-67`) |
| `family` | string | `ModelKnowledge::family` (`knowledge.rs:70-73`), review-diversity only |
| `context_window` | u32 | `ModelKnowledge::context_window` (`knowledge.rs:85`) |
| `billing_class` | enum | `WorkerBillingClass::{LocalFree,Subscription,PaidApi}` (`config/agent_identity.rs:344-352`) — never inferred from a name |
| `role_graduation[coordinator\|edit\|mechanical\|review]` | struct | see below |
| `last_observed_cost_usd`, `last_observed_latency_s` | f64 | per-role, from the retained run's `cost_ledger` sum / wall time (`report.rs::TierReport`) |
| `evidence_snapshot_date` | date | reuses the staleness pattern in `knowledge.rs:481-506` (`KB_SNAPSHOT_STALE_AFTER_DAYS`) |

`role_graduation` entry per role:

| Field | Meaning |
|---|---|
| `smoke_passed` | bool — the role's named smoke (e.g. FizzBuzz-class coordinator smoke, `EXCELLENCE_DAG.md:207`) |
| `micro_suite_pass_pct` | ≥ 80% required (Excellence gate) |
| `tool_reliability_pct` | correct tool selection rate, from G3 fixtures |
| `dag_selection_pct` | ≥ 95% required (Excellence gate) — correct roadmap/dispatch tool chosen over an equivalent (e.g. `shell`-run tests vs the literal `verify` tool, the tool-name mismatch found in `after_turn.rs:205-266`/`orchestrator.rs:6816-6822`) |
| `evidence_run_ids` | `Vec<String>` — retained `permagent-eval --out` file paths / `HARNESS_LOOP_LOG.md` section anchors, never a caller-supplied status string (mirrors `qualification.rs`'s refusal to trust labels) |
| `graduated` | bool — `smoke_passed && micro_suite_pass_pct>=0.80 && dag_selection_pct>=0.95`, three consecutive qualifying runs per the parent contract (`CODING_HARNESS_EXCELLENCE_DAG.md:52`, `qualification.rs:99` `REQUIRED_CONSECUTIVE_RUNS`) |

## G1 — Where it lives today, and where the card goes

| Existing piece | File:line | What it already does | Gap |
|---|---|---|---|
| `ModelKnowledge` / `KNOWN_MODELS` | `crates/goose/src/cost_router/knowledge.rs:62-464` | Static, cited leaderboard proxy (aider, SWE-bench) per provider/model — cost, context, two scalar strengths | Not measured on Permagent's own harness; no per-role pass %, no tool/DAG-selection %, no evidence ids — the module doc itself says the numbers are "approximate," a heuristic pre-filter, not graduation |
| `CoordinatorEligibility` / `assess_harness_coordinator` | `crates/goose/src/config/model_roles.rs:74-128` | The ONE existing graduation gate: `TrustedDefault` (compiled-in), `HeuristicCandidate` (KB clears 0.60 floor), `Unrated`, `BelowFloor`. Its own doc comment (`model_roles.rs:67-71`) admits: *"the coding-suite pass/tool/DAG rates needed to promote a model are not yet recorded"* | Coordinator-only; no edit/mechanical/review roles; no durable evidence store, so a `HeuristicCandidate` can never actually graduate |
| `resolve_decomposition_provider` | `orchestrator.rs:1179-1226` | Consults `assess_harness_coordinator` (`:1193`) before routing decomposition to a cheap model; ineligible → falls back to the session coordinator with a `tracing::warn` (`:1194-1202`) | Fallback is silent to the user (log only, no Decision Inbox / receipt); wired into exactly this one call path, not `list_workers`/`check_worker`/ranked dispatch |
| `list_workers` / `check_worker` | `orchestrator.rs:3422-3479`, `3481-3514` | Report `cost_tier`, `capabilities()`, live availability probe | No graduation field at all — a caller cannot tell "available" from "graduated for this role" |
| `WorkerPersona.workflow_role` / `routing_role()` | `config/agent_identity.rs:618-626`, `:711-713` | Pins/derives one of `WorkflowRole::{Orchestrate,Edit,Mechanical,Review,Local}` (`cost_router/recommend.rs:73-84`) per configured worker | Role is declared, not graduated; `CapabilityFloor` (`recommend.rs:199-218`) gates on the same static KB numbers, not retained evidence |
| `default_roster()` | `config/agent_identity.rs:753-1030` | Seeds `claude_code`, `codex`, `cursor`, … with engine + availability, no evidence | New card is keyed the same way (`provider`/`model`, not roster key) so it applies to in-process session models too, which have no roster entry |

**Decision:** the card is a new table, additive to the same ladder `rlm_context` used
(`docs/architecture/PRIME_RLM_AND_SKILLS.md` Part A, `session/spectral_schema.rs`) —
`capability_graduation(provider, model, role, ...)`, primary key `(provider, model, role)`. It does
not replace `ModelKnowledge` (kept as the pre-graduation heuristic filter) or
`CoordinatorEligibility` (kept as the compiled coordinator default needing no DB read); it is the
durable evidence layer neither has. One concept, one place: graduation data lives beside the other
control-plane tables in `permagent.db`, not a second store.

## G2 — Evidence pipeline: existing runs -> card fields

| Existing artifact | Maps to |
|---|---|
| `permagent-eval run --tier <t>` per-task result rows (`report.rs::TierReport`, e.g. Lane A's `glm47` 7/7, `HARNESS_LOOP_LOG.md:402`) | `micro_suite_pass_pct` for the role the tier's fixtures exercise; `last_observed_cost_usd`/`latency_s` from the same run |
| `qualification.rs::QualificationRun.areas["cheap_model_usability"]` | rolls up into `graduated` — reuses `qualify()`'s three-consecutive-run rule (`REQUIRED_CONSECUTIVE_RUNS`, `qualification.rs:12`) rather than a new counter |
| Retained coordinator smoke, `ollama/qwen2.5:7b`: **0/1**, `EXCELLENCE_DAG.md:207` | `coordinator.smoke_passed = false` |
| Lane A/A2 MAIN-lineage (no DAG-approval gate) on `zai/glm-4.7`: 14/15 (`HARNESS_LOOP_LOG.md:2397`) vs. Lane M2 dogfood-lineage (DAG-approval gate present, no headless answer path): 1/10 (`:2425`), later 0/10 (`:4055`) | **Attribution matters**: the causal chain (`:2475-2498`) shows the drop is the harness's missing `answer_decision` tool on the Auto path, not glm-4.7's own diagnosis quality (frequently correct even in failing runs, `:2468-2473`). The card must record `edit`/`review`-style capability from the MAIN-lineage numbers and track the DAG-protocol defect separately (owned by X-dag), never blend an infra defect into a model's graduation score |
| No retained tier run yet for `sonnet5`/`dschat` in `HARNESS_LOOP_LOG.md` | `evidence_run_ids = []`, `graduated = false` — objectivity requires the same bar for every vendor, matching the no-bias test in `knowledge.rs:657-705` |

**New fixtures needed for tool/DAG-selection checks (do not exist today):**

1. A fixed set of ~10 "which tool" prompts (e.g. "run the tests" → must call `verify`/the
   detected shell command, not an unrelated tool) scored against `detect_checks`'s own command
   (the fix already proposed for the `verify`-vs-`shell` mismatch, `after_turn.rs:205-266`).
2. A fixed set of ~10 "which DAG node" prompts against a frozen roadmap (dispatch the correct
   Ready node, not edit directly, not skip decomposition) — reuses `RoadmapMockProvider`'s
   deterministic pattern (`HARNESS_LOOP_LOG.md:2920`) so no network call is needed.
3. Both suites run per (provider, model, role) and populate `tool_reliability_pct` /
   `dag_selection_pct` directly — no proxy from `ModelKnowledge`.

### G3 — implemented (Lane G3)

The tool/DAG-selection micro-fixture family this node called for now exists as `category:
"selection"` fixtures under `crates/permagent-eval/tasks/select-*/` (10 fixtures:
`select-failing-test-decompose-before-create`, `select-trivial-typo-fix-skips-roadmap`,
`select-named-check-uses-verify-tool`, `select-review-request-does-not-mutate`,
`select-no-repo-facts-escalates-not-invents`, `select-missing-credential-escalates-not-guesses`,
`select-checks-worker-before-dispatch`, `select-existing-roadmap-resumes-not-redecomposes`,
`select-ambiguous-choice-escalates-decision`, `select-mechanical-rename-skips-roadmap`), each
declaring a `selection_check` (`must_call`/`must_not_call`/`before` ordering rules over tool
names, new `crates/permagent-eval/src/selection.rs`) evaluated purely against
`harness_log::LogSignals::tool_names` — the harness's own already-extracted, first-seen-order
tool-call sequence — rather than against the code a run produced. `runner.rs` evaluates each
fixture's check into `TaskResult::selection_correct`; `metrics::dag_selection_pct` rolls attempted
selection fixtures up into a real `[0,1]` rate (`Aggregate::dag_selection_pct`, `report.rs`'s
per-tier JSON summary and a new run-wide `report::overall_dag_selection_pct` that `main.rs`'s
`cmd_run` stamps onto the JSON report's top level); `routing_matrix::capability_for_category` maps
`category: "selection"` to a new `FixtureCapability::Selection` variant (additive — no exhaustive
match on that enum exists outside `routing_matrix.rs`). This is exactly the top-level
`dag_selection_pct` field `permagent-eval graduate` (Lane Z4, `graduate.rs::RunFile`) already
parses and gates `graduated` on but that no real run ever populated before this lane — a retained
`permagent-eval run --format json --out FILE` against the `select-*` fixtures now produces a real
measurement instead of an always-absent field, closing G3's stated gap ("the 95% metric — does not
exist today").

## G4 — How dispatch consults the card

1. **Refuse ungraduated coordinators, honestly.** Extend the existing pattern at
   `orchestrator.rs:1193-1202` (today: `tracing::warn` + silent fallback) so an ineligible
   coordinator also surfaces a user-visible reason — reuse the Decision Inbox receipt shape
   `decisions.rs` already has for other refusals, not a second ad hoc message format. Apply the
   same check inside `handle_list_workers`/`handle_check_worker` (`orchestrator.rs:3422`, `:3481`)
   so a `graduated: false` role is visible before dispatch, not only discovered by failure.
2. **Allow graduated mechanical roles.** `WorkerPersona.routing_role()` (`agent_identity.rs:711`)
   already carves out `Mechanical`/`Edit`/`Review` independently of coordinator eligibility;
   the card's per-role `graduated` flag becomes an additional filter next to `CapabilityFloor`
   (`recommend.rs:199-218`) in the ranked-selection path, not a new selection mechanism.
3. **Explicit operator selection still wins.** `select_requested_worker`'s comment is exact:
   *"An explicitly requested worker PINS the choice and skips cost ranking"* (`orchestrator.rs:2019`,
   behavior asserted by `select_requested_worker_ignores_the_ranked_selection_restriction`,
   `:11618-11639`). No literal harness.log line matching "explicitly selected… automatic fallback
   is refused" was found in `HARNESS_LOOP_LOG.md`'s M2 section on re-check; the closest recorded
   statement of the same invariant is `CODING_HARNESS_ROUTING_DAG_5.md:56-58` ("An explicit user
   selection outranks automatic routing until the user clears it… the system explains the smallest
   required fallback rather than silently switching"). The card must not override an explicit pin
   — it only narrows what automatic/ranked selection may choose, and it must emit the same kind of
   fallback receipt that gate already prescribes, not a silent skip.

## G6 — Migration path for today's four models (retained evidence only)

| Model | KB row? | Coordinator eligibility today | Retained evidence | Target card state |
|---|---|---|---|---|
| `zai/glm-4.7` | yes, `knowledge.rs:383-398` | `HeuristicCandidate` (orch 0.79 ≥ 0.60 floor, not the compiled default) | MAIN-lineage 14/15 edit/review-shaped (`:2397`); dogfood-lineage coordinator-shaped runs blocked by the X-dag defect, not model quality (`:2475`) | `edit`/`review`: run the G3 micro-suite once, likely graduates. `coordinator`: stays `unrated` until X-dag closes the answer-path gap and a clean re-run exists — do not graduate on confounded data |
| `custom_deepseek/deepseek-chat` | yes, `knowledge.rs:257-271` | `HeuristicCandidate` (orch 0.62 ≥ 0.60) | none in `HARNESS_LOOP_LOG.md`; tier `dschat` exists (`tier.rs`) but unrun | run `permagent-eval run --tier dschat` on the frozen 10-fixture slice + G3 suites before any card row is non-empty |
| `ollama/qwen2.5:7b` | **no** — not in `KNOWN_MODELS`; family-prefix fallback needs an untagged base match and none exists (`knowledge.rs:108-464`) | `Unrated` | retained coordinator smoke 0/1 (`EXCELLENCE_DAG.md:207`); local $0 arm explicitly **not run** on 2026-09-07 (`HARNESS_LOOP_LOG.md:400,654-657`, memory-pressure directive) | `coordinator: unrated, smoke_passed=false`. Bounded mechanical nodes remain a separate, lower-bar claim (`CoordinatorEligibility::BelowFloor` already says mechanical is unaffected, `model_roles.rs:87-88`) — needs its own mechanical-role micro-suite run before even that graduates |
| `anthropic/claude-sonnet-5` | yes, `knowledge.rs:125-140` | `HeuristicCandidate` (orch 0.80) — not `TrustedDefault` (`DEFAULT_HARNESS_PROVIDER_ID`/`_MODEL_ID` = `openai`/`gpt-5.4-mini`, `model_roles.rs:211,221`) | Used throughout as the *analyst/lane worker*, not as the eval's pinned fixture-solving tier — no retained `permagent-eval --tier sonnet5` micro-suite row found in `HARNESS_LOOP_LOG.md` | same requirement as every other vendor: run the tier before any card entry claims graduation — no default trust for the runtime's own vendor, matching `knowledge.rs`'s explicit no-bias design (`:657-705`) |

## Tests

| Kind | What it proves |
|---|---|
| Deterministic | `card_from_fixture_evidence()` given a canned `QualificationRun`-shaped JSON produces the exact expected `graduated`/`pass_pct` fields — mirrors `model_roles.rs`'s existing `assess_harness_coordinator` unit tests (`:520-545`) and `qualification.rs`'s `qualify()` fixtures |
| Deterministic | routing-matrix test: same card set + same fixture set → same worker chosen twice (restart-safety, per `CODING_HARNESS_ROUTING_DAG_5.md` R3 gate) |
| Adversarial | stale evidence (past `KB_SNAPSHOT_STALE_AFTER_DAYS`-style window) is flagged, never silently treated as current |
| Adversarial | an explicit worker pin still dispatches even when that worker's card says `graduated: false` for the role — G4's rule 3 is not accidentally inverted |
| Integration | `list_workers` output includes `graduated` per role and matches the card store after a restart |

## Node order and effort

| Node | Depends on | Effort |
|---|---|---|
| G0 schema freeze | — | S (half day) |
| G1 durable store (migration + read/write) | G0 | S |
| G2 evidence pipeline (retained runs → fields) | G1 | M (needs `permagent-eval` report parsing) |
| G3 tool/DAG-selection micro-fixtures | G0 | M (new fixtures, ~20 prompts total) |
| G4 dispatch consult (3 call sites) | G1, G2 | M |
| G5 deterministic + adversarial tests | G2, G3, G4 | S |
| G6 migrate the four named models | G2, G3 | S (mostly running existing/new tiers, not code) |

G0/G3 can start immediately in parallel. G1/G2 depend on G0. G4 cannot land before G2 has at least
one real evidence row, or every coordinator becomes refused on day one with no graduation path.
