# Backlog Tier Sort — what's batchable vs what needs you

The sort that turns "a long list I prompt one-by-one" into "a batch I let run
under /goal + a short list of decisions that are actually mine."

Sort rule:
- **Tier 1 (batchable under /goal + auto-mode):** mechanical, bounded, the
  "done" condition is objectively checkable by a script (test/lint/tsc passes,
  a string changed, a render works). CC can iterate to green without you.
- **Tier 2 (you — Plan Mode, detailed spec, your judgment):** design,
  architecture, product decisions, taste. Getting it wrong compounds. The
  slowness IS the work.
- **Tier 3 (loops, but bounded):** diagnosis/audit. Runs semi-autonomously as
  a single dispatch, you review the report.

---

## TIER 1 — batchable now (verifiable, run under /goal + auto-mode)

These have a checkable acceptance condition. Bake the check into the goal and
let them run. Each is its own worktree + PR.

| Item | Goal condition (the /goal) | Verification |
|---|---|---|
| Voice anti-narration (#13A/#267) | voice.rs:714 prompt has an explicit anti-narration clause; cargo check+clippy green | clippy -p <crate>, the clause is present |
| Voice preview Web Audio (#14) | preview routes through Web Audio not `<audio>`; tsc+vite green | tsc --noEmit, vite build |
| Search & tools surfacing (#5/#6) | the section is discoverable (own nav/above Providers); tsc+vite green | tsc, vite build, visual |
| Pronunciation lexicon (#245) | lexicon populated + consulted; TTS says "Claude Code"; cargo green | unit test on lexicon lookup |
| Terminal reflow on tab-focus | fit()/ResizeObserver re-fits terminal on tab show; tsc+vite green | tsc, vite, manual tab-switch |
| Chat-window clipping regression | separate-window always-on-top restored; tsc+vite green | tsc, vite, visual above browser |
| Voice-latency: migrate heavy query | list_sessions_by_types uses the lean SessionSummary projection (#341b); cargo green + the slow-query warning gone | the SQL no longer SELECTs the blob columns; query <1s |
| Entity-description render (merge #265) | #265 rebased + green | CI green, descriptions render |

**Run mode:** `/goal <condition>` in auto-mode, one CC per item (sequential or
2 parallel max for disk). Add the **escalate-clause** to every one: "stop and
flag if you hit a judgment call, don't guess." That's what makes the auto-run
safe on this codebase.

**Note:** the two voice-latency pieces — migrate the query (Tier 1, mechanical)
vs profile/cut the 14s ctx build (Tier 2/3, needs diagnosis). Split them.

---

## TIER 2 — YOU (Plan Mode, detailed spec, your decision)

These are the epics and the design calls. The research is explicit: detailed
specs + Plan Mode for these, NOT terse goals. The slowness is the work.

- **Orchestrator supervised-enable (#9)** — the keystone. You-present. Lights
  up the Decision Inbox. Activates autonomous goal-creation — never auto-run.
- **Cave Phase 2 (Turn-on-boot) + Phase 3 (Mouth=Mesh, Brain-throat)** — design
  decisions about the origin experience + the allegory. Spec them; you rule.
- **Terminal-supervision epic (#399-402)** — esp. #402, the whitelist of safe
  auto-advance gates. Needs YOUR rulings on what's whitelist-safe. The
  supervision boundary is yours to draw.
- **File-intake epic** — the inbox-hub architecture + the routing model
  (where does a download go?). Design-first, scope build-free.
- **Voice subsystem hardening (the pattern call)** — is it piecemeal fixes or a
  state-lifecycle overhaul? Your architecture call.
- **Voice interrupt / barge-in** — a real primitive; needs you to decide the
  interaction (space-to-stop) + how it composes with the speaking-state reset.
- **Arrow-key puppeting ↔ cave-descent composition** — already flagged in the
  bible §8a; the arm/release hand-off design is yours when the descent is spec'd.

---

## TIER 3 — bounded loops (single dispatch, you review the report)

Diagnosis/audit. Ran great today as one-shot dispatches. Semi-autonomous.

- **Voice-latency ctx=14102ms profiling** — diagnose what the 14s context
  build does. Audit dispatch → report → then the fix is Tier 1.
- **Stuck speaking/thinking state-lifecycle audit** — find every exit path that
  fails to clear the state. Audit → report → fix.
- **Any "find all stale X" / reconciliation pass** — the kind that found the
  cave misread and the dogfooding map. Single dispatch, you read the output.

---

## The immediate move

1. Run the **Tier 1 batch** under /goal + auto-mode (with the escalate-clause)
   while you do Tier 2 thinking or step away. ~8 items, mostly small.
2. Keep yourself on **Tier 2** — the epics and decisions. That's where your
   time actually creates value.
3. Fire **Tier 3** audits as single dispatches when you need a diagnosis, then
   the resulting fix drops into the Tier 1 batch.

The split itself IS the speed-up: ~8 batchable items stop being 8 separate
hand-prompted sessions and become one auto-run you supervise by exception.
