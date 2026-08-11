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

## Why it is not the default

Spectral's preregistered latency gate **FAILED**: recall-only p95 regressed
**+87–100%** against a +5% kill line. The cause is the synchronous
delivery-write commit, not retrieval — p50 actually *improved* ~19%.

This repo has already taken a latency incident from slow pre-stream recall on
the voice path. Running `turn` on every turn would walk straight back into it.
When the deferrable delivery write lands upstream, the rate can go to `1.0`,
the shadow can become the primary path, and `turn_sampling` can be deleted.

## Verifying it is producing data

```sql
-- ~/.permagent/brain/memory.db
SELECT COUNT(*) FROM turn_events;   -- was 0 before this prototype
SELECT COUNT(*) FROM turn_members;
```

If `turn_events` grows but `turn_members` outcomes stay empty, the retrieval is
happening and the reporting is not — that is the failure mode that produces
pure overhead and no signal, and it should be treated as a bug.

## Corpus eras — the pre-void boundary

Spectral's queued pin bump adds `voided_at TEXT DEFAULT NULL` to `turn_events`,
which gives the corpus a verb for *abandoned* as distinct from *ignored*. It
also, on the day it lands, stamps every pre-existing row with `voided_at = NULL`
— byte-identical to a post-bump turn that ran to completion and simply was not
voided. From that moment the corpus cannot describe its own eras, and a later
"what fraction of turns were abandoned?" query sweeps the old aborts in as
`unreported`, re-creating the exact conflation the void verb exists to remove
(Spectral dispatch `2026-08-06v`). It costs one timestamp now and is
unrecoverable afterwards.

**Census taken 2026-08-07, before the bump** (`scripts/turn_corpus_era.sh`):

```
voided_at:    absent — bump not landed
rows:         19 total, 4 committed, 15 uncommitted, all policy v1
delivered_at: 2026-08-04T19:16:47.580410+00:00 .. 2026-08-06T21:36:53.649053+00:00
```

**The boundary is the `delivered_at` of the first turn served by a bumped
daemon.** Below it, `voided_at IS NULL` means *voiding was impossible here* —
never *this turn was not voided*. Above it the NULL is meaningful. Any row
delivered between the census above and the install is still pre-void, which is
why the boundary is defined by the first post-bump row rather than by the census
timestamp.

Finalizing it is step 8 of the bump: run `scripts/turn_corpus_era.sh` once the
bumped daemon has served a turn, and record the boundary here.

> **Boundary:** _not yet fixed — the bump has not landed._

`voided_at` appearing in the live brain is also the agreed definition of "the
bump landed": it proves both the dependency rev *and* that the daemon opened the
brain with it, which an install date cannot (dispatch `2026-08-06u`).

## Pin

Requires Spectral `c2c8381` or later (`crates/spectral/src/turn.rs`). The
previous pin `486459c` had no `turn` API at all.

The sampler is **deterministic by contract** (`decide()` in
`crates/goose/src/turn_sampling.rs`): rate `0.1` fires on exactly 10 turns in
100, pinned by `one_in_ten_fires_ten_times_per_hundred`. At rate `1.0` a counter
and a coin are indistinguishable, so a probabilistic reconstruction would pass
every check runnable today and only bite if the rate is ever dialled back. When
the pin-bump branch is ported onto main, **exactly one sampler must survive and
it must be this one** — delete the loser deliberately rather than letting a
merge choose.
