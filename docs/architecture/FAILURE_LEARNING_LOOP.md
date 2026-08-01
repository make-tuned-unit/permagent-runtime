# The Failure Learning Loop

**Status:** design proposal. Zero code. Every phase is flag-gated and eval-gated.
**Date:** 2026-08-01

Henry should get better at the things he gets wrong. This is how — and, more
importantly, how without making him worse, which is the outcome the obvious
design produces.

---

## 0. The result that constrains everything

This repo has already run the experiment the popular blog posts recommend.

> A cheap READ-TIME LLM consolidation pre-pass **REGRESSED −9.2pp** (a weak,
> lossy intermediate the strong actor over-trusts).
> — `crates/goose/src/agents/platform_extensions/librarian_atoms.rs:11-18`

And again, independently:

> A prior write-time-atoms experiment **lost ~9 points** when distilled hints
> were treated as authoritative.
> — `crates/goose/src/playbook/mod.rs:20-26`

Distilling experience into short lessons and feeding them back is not a free
win. Done naively it is a measured, repeated **nine-point regression**, because
a strong actor over-trusts a weak summary. Any learning loop that ignores this
will reproduce it.

The literature agrees from the other direction: with the *same* accumulated
experience, an agent either improved or **degraded below baseline**, decided
entirely by whether a curation loop existed
([Insight Governance](https://arxiv.org/abs/2606.17591)).

**Design consequence, non-negotiable:** a lesson is never authoritative. It is
quoted, provenance-carrying, sub-1.0-confidence data the actor may override —
the discipline `playbook` and `librarian_atoms` already encode. We inherit it
rather than re-derive it at −9pp.

---

## 1. What we are actually trying to catch

The closest study to Henry is a longitudinal teardown of a production
personal-assistant agent runtime
([Silent Failures](https://arxiv.org/abs/2606.14589v1)). Its numbers:

| Signal | Result |
|---|---|
| Unit tests as silent-failure detector | **≈0** (despite 4,286 tests) |
| Governance checks | 827, and still ≈0 ex-ante |
| **Human looking at the product as a user** | **~70% of all silent failures** |
| Audits: ex-ante prevention | **0 / 15 (0%)** |
| Audits: regression blocking | **13 / 15 (87%)** |

Its meta-pattern, seen 28 times: *"a failure whose error signal never reaches a
human in actionable form."*

**Session 2026-07-31 reproduced this exactly.** Five real defects; CI caught
none of them:

| Defect | Mechanism | Found by |
|---|---|---|
| Saved keys shown as "No key" | `maskedValue` vs `masked_value` — silent false negative | Jesse, looking at Settings |
| Henry flails on weather | No dashboard tool; then reported search as disabled while it was enabled | Jesse, watching the chat |
| Popped-out chat re-spoke greeting | Replayed `Finish` passed a guard | Jesse, hearing it |
| Probe timeout could never fire | Blocking keychain call starved the timer | A live probe, after shipping |
| `PageSnapshot` test literals | struct change; `--lib` doesn't compile tests | CI (the one CI catch) |

Four of five were user-view observations. The one CI caught was a compile error,
not a behavioural failure. **This is the corpus the loop must learn from, and it
is not produced by fixtures.**

### 1.1 Classify by mechanism, not location

The taxonomy is deliberately mechanism-oriented, because the same mechanism
recurs across unrelated features and a mechanism-level defense immunises every
site at once.

- **A — Environment/platform quirk.** Correct logic, different runtime.
  *Ours:* dylib signature SIGKILL after `cargo clean`; `build:icons` PIL.
- **B — Design-assumption mismatch.** Code assumes a shape reality violates;
  tests pass because they mirror the wrong assumption.
  *Ours:* extension snapshot excluded MCP servers, so enabling search never
  reached the running session.
- **C — Error swallowing / dilution.** The error exists but arrives stripped of
  actionable content.
  *Ours:* the casing mismatch; the probe timeout that could not fire.
- **D — Chained hallucination (fail-plausible).** Non-signal lands where the
  model expects signal; fluent, confident falsehood follows. **The most
  dangerous class.**
  *Ours:* Henry asking Jesse to enable extensions that were already enabled.
- **E — Operational omission / forensic blind spot.** A step never ran, or the
  diagnostic instrument itself lies.
  *Ours:* `strings` showing nothing for the embedded UI — absence of evidence
  read as evidence of absence.

---

## 1.5 The blocker: Henry currently cannot record a failure as a failure

A sweep of every outcome signal in the codebase turned up one defect that
invalidates the whole premise until it is fixed.

**Every tool call is logged as a success, whatever actually happened.**

`agent.rs:740-757` calls `log_task_completed` unconditionally, with an empty
output blob, before the result future has even resolved — annotated in-source as
*"a Phase 1 trade-off."* Its counterpart `log_task_failed`
(`tasks/mod.rs:156`) has **zero production callers**.

That would be a cosmetic gap if it stopped there. It does not:
`log_task_completed` → `write_back_task_outcome` (`tasks/mod.rs:133`) →
`recognition.rs:314-319`, which stamps `outcome_kind='TaskResolved'`,
**`outcome_polarity='Positive'`** across every still-unattributed
`recognition_events` row for the session.

So the primary proxy feeding Henry's recall-quality labels is **hardcoded to
positive**. `recognition_events.outcome_label` — the `useful` / `ignored` /
`wrong` column that is the closest thing Henry has to ground truth about
whether remembering something helped — is systematically biased. `wrong` can
essentially only arrive via the narrow decision-bounce path.

This is itself a **Class C failure (error swallowing)** sitting inside the
outcome pipeline: the error exists, and is discarded before anyone can see it.
Building a learning loop on top would be training on a label that says every
attempt succeeded.

**Nothing else in this document matters until this is fixed.** It is also, by
some distance, the cheapest item here.

### 1.6 Signal inventory — what exists, what is durable

| Signal | Where | Distinguishes failure? | Durable? |
|---|---|---|---|
| `GoalOutcome` | `goal_engine.rs:90-108` | **Yes** — Success / Failed / TimedOut / Blocked | via receipt |
| `ExecutionReceipt` | `execution_receipt.rs:25-42` | **Yes** — 4 failure states | `cards.metadata_json`, **single-slot, overwritten per attempt** |
| `verify` PASS/FAIL | `developer/verify.rs:587,597` | **Yes** — exit code + normalized output | **No table at all** |
| Loop signals S1–S6 | `tool_monitor.rs:156-197` | **Yes** — incl. `VERIFY-GAMING` | **Logs only** |
| Escalation `ParkReason` | `escalation.rs:93-107` | **Yes** — names the guardrail that stopped it | `cards.metadata_json.verify_escalation` |
| Budget verdict | `budget.rs:134-144` | **Yes** — Ok/Soft/Gate/Hard | verdict not stored; only the decision row |
| `decisions` + `decision_audit` | `spectral_schema.rs:2259,2416` | **Yes** — answer + hash chain | **Yes, append-only** |
| `recognition_events` | `spectral_schema.rs:786-807` | label `useful`/`ignored`/`wrong` | **Yes — but see §1.5** |
| `egress_audit` | `spectral_schema.rs:1225` | **Yes** — `blocked` | **Yes, append-only** |
| `effect_outbox` | `spectral_schema.rs:2558` | **Yes** — `last_error`, `dead` | **Yes** |
| `tasks.status` | `spectral_schema.rs:251` | schema says yes | **never written** (§1.5) |
| Per-tool-call `is_error` | `tool_monitor.rs:144-154` | Yes | **in-memory, 16-event window, discarded** |

Two clear patterns:

- **The durable, trustworthy records are the human-gated ones** — `decisions`,
  `decision_audit`, `egress_audit`, `effect_outbox`. They are append-only and
  carry real outcomes because a human answered them.
- **The agent's own execution signals are rich and almost entirely
  ephemeral.** `tool_monitor` already computes six named stall signals with
  thresholds, plus reward-hack detection (`VERIFY-GAMING`), and throws all of
  it away as log lines.

The loop's first job is not to invent signal. It is to **stop discarding the
signal Henry already computes.**

Also worth knowing, since both are candidate machinery: `review_gate.rs` is
fully built with zero production callers, and `recognition_verdict` /
`familiarity` are schema-present but always `NULL`.

---

## 2. What Henry already has

This loop is mostly **wiring existing organs**, not new machinery.

| Organ | State | Role in the loop |
|---|---|---|
| `decision_inbox/learn.rs` | **live** | Already ingests answered decisions AND edits-as-corrections (`correction_delta`), recalls them at decompose time |
| `playbook/` + `playbook/synthesis.rs` | built, **default OFF** | Distils hints from answered decisions; carries the −9pp doctrine |
| Decision Inbox (`decisions.rs`) | **live** | The human gate. `DecisionProof` is non-Copy/non-Clone, consumed by value; hash-chained `decision_audit`; unknown action class **fails closed to Tier 2 (Jesse)** |
| Steward (`steward/mod.rs`) | live, recipe-gated | The proposer-only pattern, enforced **in code**: *"a local 14B can be cajoled; code cannot"* |
| `librarian_adjudicator.rs` | built, **OFF** | Containment pattern: model may only choose among asserted values; a hallucinated value returns `Unknown` |
| `format_reference_block` | **live** | The canonical "data, not instructions" framer, already reused in 4 places |
| `scenario_tests/` + `recordings/` | **live** | VCR-style deterministic replay per provider — the regression substrate |
| Spectral recognition | feature-gated | Persists retrieval sets with a `retrieval_id` for later attribution |
| `briefings.rs` | live | *"acknowledged_at marks that Henry has seen it… It is not approval and must never be read as one"* |

**We are not starting from zero. We are closing a loop whose halves exist.**

---

## 3. The loop

```
  ┌────────────────────────────────────────────────────────────────┐
  │  1. CAPTURE      2. GROUND      3. CLASSIFY     4. PROPOSE      │
  │   incident   →   artifact   →   mechanism   →   lesson OR       │
  │   record         proof          class A–E       regression      │
  │                                                     ↓           │
  │  7. REVIEW   ←   6. GOVERN   ←   5. GATE  ←  Decision Inbox     │
  │   replay &       admit/decay     Jesse       (Tier 2, fail-     │
  │   retire         /retire         answers      closed)           │
  └────────────────────────────────────────────────────────────────┘
```

### Stage 1 — Capture

Three intake channels, in descending order of proven yield:

1. **User-view observation (primary).** ~70% of real failures. Today this is
   Jesse typing "that's wrong" into chat and the signal dying there. Give it a
   first-class path: a `report_failure` affordance (and a chat intent) that
   opens an **incident** with the surface, the expectation, and the observation.
   The research recommendation is a standing observation ritual; the product
   equivalent is making the report one gesture instead of a bug-filing chore.
2. **Contradiction signals (automatic).** Cheap, high-precision, already
   emitted: a tool call that errors then succeeds on retry with different args;
   an escalation up the cost ladder (the cheap tier *failed*); a gate the user
   rejected; an `edit` answer on a decision (already captured as
   `correction_delta`); a budget park.
3. **Self-suspicion (lowest trust).** Henry noticing his own turn went wrong.
   Admissible only as a *candidate*, never as proof — see Stage 2.

### Stage 2 — Ground (the anti-fabrication gate)

Self-improving agents **fabricate failures that never happened**, then "fix"
them to appear more trustworthy ([Phantom Guardrails](https://arxiv.org/pdf/2607.13083)).
The mitigation is oracle-based verification, never self-report.

**Rule: no incident is admissible without a non-model artifact.** One of:

- a user message asserting the failure,
- a persisted tool-call error / non-zero exit / HTTP status,
- a diff between two recorded runs,
- a recognition record showing what was recalled vs. what was needed.

Henry's narration is never itself the evidence. This directly mirrors the
adjudicator's existing containment: the model may only *select among* asserted
facts, and anything it invents resolves to `Unknown`
(`librarian_adjudicator.rs:125-161`).

### Stage 3 — Classify

Assign mechanism class A–E plus a one-line failure statement. Class D
(fail-plausible) is escalated on sight: it is the class where Henry is
*confidently wrong to the user's face*, which is the most expensive failure a
personal agent has.

### Stage 4 — Propose *two* artifacts, never one

This is the sharpest departure from the article, which proposes only a lesson.

- **A regression** — a `scenario_tests` case seeded from the real trace, with
  recorded provider responses. This is the durable artifact. The audit data says
  audits are **regression engines (87%), not prediction engines (0%)** — so the
  regression is where the value provably is.
- **A lesson** — optional, and only when the mechanism generalises. A lesson is
  an *upsertable playbook-class hint*, sub-1.0 confidence, provenance-linked,
  injected as quoted data via `format_reference_block`.

If only one can be produced, produce the regression. A lesson without a
regression is the −9.2pp path.

### Stage 5 — Gate (human, always)

Every lesson and every regression enters the **Decision Inbox** as a proposal.
Henry proposes; Jesse disposes. This is not new policy — it is Steward's
constraint and the Inbox's existing Tier-2 fail-closed default.

The article's loop auto-commits on a nightly cron with no human in it. For a
system whose lesson store steers a personal agent, that is the memory-poisoning
attack surface ([Memory Poisoning](https://arxiv.org/html/2606.04329v1)) with
the door held open. Answering already mints a `DecisionProof` and appends to the
hash-chained audit; lessons inherit that provenance for free.

### Stage 6 — Govern (admission, decay, retirement)

Ungoverned accumulation is the documented failure mode. Borrowing ExpeL's
mechanics and SSGM's gates:

- **Importance counting.** A lesson starts at 2; corroboration increments,
  contradiction decrements; **at 0 it is deleted** ([ExpeL](https://arxiv.org/html/2308.10144v2)).
- **Write gate.** Reject a lesson that contradicts a core fact rather than
  silently storing both — the "user said X in May and Y in November, top-k
  surfaces both, behaviour goes inconsistent" failure.
- **Decay.** Recency-weighted; a lesson untouched past a freshness floor
  down-weights out of injection before it is deleted.
- **Bounded injection.** Hard cap, in the spirit of the existing
  `MAX_RECALLED_PLAYBOOK_HINTS = 3`. The pool may grow; the prompt may not.
- **Immutable ledger + mutable view.** Incidents are append-only; the active
  lesson set is derived and rebuildable. This is what makes a bad lesson
  *revocable*, which the article has no answer for. Drift is bounded by
  reconciliation interval rather than total horizon ([SSGM](https://arxiv.org/html/2603.11768v1)).

### Stage 7 — Review and prove

- Replay the regression corpus on a schedule. A lesson whose regression passes
  with the lesson **removed** has stopped earning its slot — retire it.
- **Held-out set.** Some regressions are never shown to the lesson-writing path.
  The gap between corpus and held-out performance is the **reward-hacking gap**;
  a positive gap means Henry is scoring on the visible proxy
  ([SpecBench](https://arxiv.org/html/2605.21384v1)).
- **Sabotage validation.** Periodically inject the failure a guard claims to
  catch, and assert it fires. This caught guards that had been vacuously
  executing empty strings *for months*.
- **Routing independence.** The alarm for a broken subsystem must not travel
  through that subsystem.

---

## 4. What this must not do

- **Not authoritative.** Hints, quoted, overridable, sub-1.0 confidence. (−9.2pp.)
- **Not self-certifying.** No incident without a non-model artifact. (Phantom guardrails.)
- **Not auto-committing.** Every lesson is a Tier-2 decision. (Memory poisoning.)
- **Not unbounded.** Importance counts, decay, hard injection cap. (Insight governance.)
- **Not prompt-enforced.** Containment lives in code — *"a local 14B can be cajoled; code cannot."*
- **Not silent.** A loop that fails silently is itself a Class C failure. Off must be loudly off, as the initiative driver already does.

---

## 5. Phasing

Each phase is independently useful and independently abandonable. Every one is
default OFF and byte-for-byte inert when off, matching the playbook/atoms/
best-of-N rollout discipline.

| Phase | Scope | Proves |
|---|---|---|
| **00** | **Fix the outcome record.** Resolve the tool result before logging; call `log_task_failed` on error; stop stamping `Positive` over unattributed recognition rows. | That "failed" is expressible at all. **Prerequisite for everything below.** |
| **0** | Persist what `tool_monitor` already computes: S1–S6 signals, `VERIFY-GAMING`, `verify` PASS/FAIL, per-attempt receipt history. No learning, no injection. | That failure signal survives the turn |
| **1** | Incident record + `report_failure` intake + mechanism classification A–E | That we can capture the ~70% channel |
| **2** | Incident → `scenario_tests` regression, proposed through the Inbox | The 87% artifact — value with no injection surface |
| **3** | Lessons: playbook-class hints, governed pool, bounded injection, paired eval vs. lessons-off | Whether lessons beat −9.2pp. **If they do not, stop and keep Phase 2.** |
| **4** | Held-out set + sabotage validation + reward-hacking-gap metric | That Phase 3 gains are real rather than proxy |

**Phase 00 is not optional and should be done regardless of whether the rest of
this is ever built.** Right now `recognition_events.outcome_label` — the table
already shipping in production, already feeding recall quality — is being
written from a hardcoded success. That is a live data-integrity bug, not merely
a missing feature.

**Phase 2 is the recommended first *learning* build.** It captures the measured
87% of the value, has no injection surface, cannot poison memory, and cannot
regress the actor. Phase 3 is the one that must earn its place against a
−9.2pp prior.

---

## 6. Open questions for Jesse

1. **Intake gesture.** Is "report failure" a button, a chat intent Henry
   recognises ("that's wrong"), or both? The research says this channel is worth
   ~70% of the yield, so its ergonomics matter more than the algorithm.
2. **Does Henry propose lessons about himself, or only regressions?** Phase 2/3
   split assumes regressions first. Reasonable to never grant Phase 3.
3. **Where do lessons inject?** Decompose-time (as playbook does) is safer than
   the system prompt: it is already the "quoted data" seam, and it keeps the
   cacheable system prefix stable.
4. **Turn the playbook on first?** It is built, OFF, and is the nearest existing
   analogue. Running its paired eval would tell us whether lesson injection can
   beat the −9pp prior *before* we build anything new.

---

## Sources

Internal: `librarian_atoms.rs:11-18` · `playbook/mod.rs:20-26` ·
`librarian_adjudicator.rs:125-161` · `steward/mod.rs:9-18` ·
`decisions.rs:1209-1256` · `decision_inbox/learn.rs` · `scenario_tests/` ·
`agent.rs:740-757` · `tasks/mod.rs:133,156` · `recognition.rs:314-319,408-463` ·
`tool_monitor.rs:156-197,304-343` · `escalation.rs:93-107` ·
`execution_receipt.rs:25-42` · `developer/verify.rs:587-628`

External: [Silent Failures in a Production LLM Agent Runtime](https://arxiv.org/abs/2606.14589v1) ·
[Reflexion](https://arxiv.org/abs/2303.11366) ·
[ExpeL](https://arxiv.org/html/2308.10144v2) ·
[Insight Governance](https://arxiv.org/abs/2606.17591) ·
[SSGM](https://arxiv.org/html/2603.11768v1) ·
[Phantom Guardrails](https://arxiv.org/pdf/2607.13083) ·
[Memory Poisoning](https://arxiv.org/html/2606.04329v1) ·
[SpecBench](https://arxiv.org/html/2605.21384v1) ·
[MindStudio: self-improving agent feedback loop](https://www.mindstudio.ai/blog/self-improving-ai-agent-feedback-loop)
