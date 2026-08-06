# `Brain::turn` outcome prototype

Spectral asked Permagent to call `Brain::turn` and report outcomes, because the
labelled corpus that would settle several open retrieval questions does not
exist anywhere. This is that prototype: **sampled, shadowed, and off by
default.**

## Why it exists

`recall_*` auto-reinforces every hit at retrieval time, which credits
*exposure*, not usefulness. `turn` retrieves read-only, hands back a receipt,
and learns nothing until told what was actually used. The resulting
`turn_events` / `turn_members` rows are real queries against a real wing
taxonomy with recorded use — which benchmark corpora (LongMemEval, LoCoMo)
cannot provide, because they have no wing structure at all.

Open questions it is meant to feed:

- Does the constellation/fingerprint tier earn its 39%-of-every-write cost?
  (The "0 wins, 2 losses, 9 ties" verdict that nearly justified deleting it was
  measured on corpora with no wings.)
- Does wing-scoped retrieval beat plain FTS on real project queries?
- Do the reranking levers convert into better answers?

## How it is wired

`inject_recall` (`crates/goose-server/src/brain_ops.rs`):

1. If `turn_sampling::should_sample_turn()` fires, call `SafeBrain::turn`
   **in addition to** `recall_cascade` — a shadow.
2. The reply is still built from `recall_cascade`, so a sampled turn **cannot
   change what the user sees**. It only produces a corpus row.
3. At end of turn, `RecallInjection::finish` reports outcomes via
   `record_turn_outcome`, detached and best-effort.

Attribution reuses `recognition::cited_memories_by_content_overlap` — the same
rule the recognition write-back already uses. Two definitions of "used" would
make the turn corpus incomparable with the recognition data it should be
validated against.

Every delivered hit is reported, not only the used ones: silence about a
delivered-but-unused memory is indistinguishable from one that was never
delivered, and the negatives are half the signal. Non-cited hits are reported
`Ignored` (delivered, not used) and never `Wrong` — `Wrong` means *actively
misleading*, and content overlap cannot tell unhelpful from unused. Overstating
it would poison the corpus.

## Turning it on

```bash
# One turn in ten. Deterministic: exactly 10 per 100, not approximately.
export PERMAGENT_TURN_SAMPLE_RATE=0.1
```

Absent, unparseable, `0`, or negative ⇒ **off**. Values above `1.0` clamp to
always-on. The rate is read per turn, so it can be changed without a rebuild
(the daemon must restart to pick up a new environment).

## Why it remains sampled

The daemon enables Spectral's asynchronous turn-delivery mode, removing the
synchronous ledger write from the reply hot path. Sampling remains deliberate:
the turn path is still a shadow whose purpose is to compare its delivered set
with `recall_cascade`, not to replace the context shown to the model.

## Verifying it is producing data

```sql
-- ~/.permagent/brain/memory.db
SELECT COUNT(*) FROM turn_events;   -- was 0 before this prototype
SELECT COUNT(*) FROM turn_members;
```

If `turn_events` grows but `turn_members` outcomes stay empty, the retrieval is
happening and the reporting is not — that is the failure mode that produces
pure overhead and no signal, and it should be treated as a bug.

## Pin

Requires Spectral `028a2864783fcab74fc265a9836ed862bb777567` or later
(`crates/spectral/src/turn.rs`).
