# Proposal: Grow — the action → verify → measure → learn loop

**Status:** draft for ruling
**Surface:** Grow ▸ Actions (`ui/command-center/src/components/grow/GrowView.tsx`,
`crates/goose-server/src/routes/growth_actions.rs`)

## What exists today

`GrowthAction` is `{ title, evidence, recommendation, steps[], artifactKind,
artifact, category, impact, confidence }`. Two producers: a deterministic
growth inbox and the model's read of `AnalyticsSummary`. Both are **recomputed
on every load**.

That single fact is the blocker. An action has no identity, so nothing can be
attached to it — not "I did this", not a baseline, not an outcome. Everything
below exists to fix that first.

## What the loop must do

1. Copy the prompt, run it elsewhere.
2. Press **Verify change** → the system checks the change actually happened.
3. It watches the metric the action claimed it would move.
4. It renders a verdict: helped, hindered, no detectable effect, or not enough
   data to say.
5. Future suggestions are informed by what actually worked *here*.

## The hard part, stated first

Step 4 is where this feature either earns trust or destroys it.

`MIN_PAGEVIEWS` is 20. At the traffic these projects have, **most single
actions will not produce a statistically detectable change.** Week-to-week
variance, day-of-week effects, one link shared somewhere, a bot wave that
`is_bot` didn't catch — any of these dwarf the effect of a copy tweak.

A feature that says "this helped, +12%" off 40 pageviews is not measuring; it
is pattern-matching noise and presenting it as evidence. It then does something
worse: step 5 **feeds that noise back into future strategy**, so the system
confidently learns superstitions and gets more wrong over time, not less.

So the design commits to three rules:

- **Pre-registration.** The metric, the direction, and the expected magnitude
  are recorded *before* the action is marked done. A verdict computed against a
  metric chosen afterwards is unfalsifiable.
- **"Inconclusive" is the expected outcome, not a failure.** It must be the
  visually neutral default, not a sad grey state, or there is pressure to
  manufacture verdicts.
- **Only confirmed outcomes feed learning.** Inconclusive results teach
  nothing and must not shift future ranking.

## Schema

Two additive tables, following the `spectral_schema` migration pattern
(idempotent, base-independent, v41 → v42):

```sql
CREATE TABLE growth_actions (
  id             TEXT PRIMARY KEY,      -- uuid, assigned on first persist
  project_id     TEXT NOT NULL,
  fingerprint    TEXT NOT NULL,         -- hash(project_id, title, recommendation)
  title          TEXT NOT NULL,
  recommendation TEXT NOT NULL,
  category       TEXT,
  artifact_kind  TEXT,                  -- prompt | post | none
  artifact       TEXT,
  -- pre-registration, captured when the user marks it done
  target_metric  TEXT,                  -- e.g. 'sessions', 'bounce_rate'
  target_dir     TEXT,                  -- 'up' | 'down'
  baseline_json  TEXT,                  -- AnalyticsSummary snapshot at t0
  status         TEXT NOT NULL,         -- suggested|dismissed|done|verified|measuring|judged
  verified_by    TEXT,                  -- how: git|content|event|self
  verified_at    TEXT,
  created_at     TEXT NOT NULL,
  UNIQUE(project_id, fingerprint)
);

CREATE TABLE growth_action_outcomes (
  action_id      TEXT NOT NULL REFERENCES growth_actions(id),
  window_days    INTEGER NOT NULL,
  before_json    TEXT NOT NULL,
  after_json     TEXT NOT NULL,
  delta_pct      REAL,
  verdict        TEXT NOT NULL,         -- helped|hindered|no_effect|inconclusive|confounded
  rationale      TEXT NOT NULL,         -- why, in one sentence, always populated
  confounders    TEXT,                  -- other action ids overlapping the window
  judged_at      TEXT NOT NULL,
  PRIMARY KEY(action_id, window_days)
);
```

`fingerprint` is what survives regeneration: the same advice recomputed
tomorrow resolves to the same row rather than a duplicate card.

## Verify — evidence, not self-report

"Verify change" should mean the system *checked*, otherwise it is a checkbox
with extra steps. Strategy by `artifact_kind`, best available wins:

| kind | check | `verified_by` |
|---|---|---|
| `prompt` | a commit in the project repo touching the named area since the action was issued — the project's `root_path` is already known and now resolves correctly | `git` |
| `post` | a new `utm_source`/referrer or `answer_engine_visit` appearing that was absent in the baseline | `event` |
| any | the live page contains the change (fetch + assert) | `content` |
| fallback | the user asserts it | `self` |

`verified_by` is **shown on the card**. "Verified from a commit" and "you told
me so" are different claims and must not look identical. When automatic
verification fails, offer self-attestation rather than blocking — but label it.

This is where the codebase map you mentioned fits: for `prompt` actions it is
the natural oracle for "did this actually land", better than diffing blind.

## Measure

At verify time, snapshot `AnalyticsSummary` as the baseline. Then compare
equal-length windows — **whole weeks** (7/14/28 days), never arbitrary spans,
so day-of-week composition matches on both sides.

Before rendering any verdict, run a power check:

> Given baseline variance, is the observed delta larger than this metric's
> normal week-to-week swing for this project?

If not → `inconclusive`, with the honest rationale ("30 pageviews/week; a
change under ~40% is indistinguishable from normal variance here"). If another
action was verified inside the same window → `confounded`, naming it. Attribution
across overlapping actions is not solvable at this traffic; saying so is correct.

A nightly job re-evaluates open windows — the `health-review` starter is the
pattern (`crates/goose-server/src/automation/`).

## Learn

Only `helped` / `hindered` outcomes feed back, as a compact record injected
into the action-generation prompt:

```
Previously tried on this project:
- "Add FAQ schema" (seo) -> helped, sessions +34% over 28d, verified from commit
- "Daily posting" (social) -> no effect over 28d
- "Rewrite hero copy" (copy) -> inconclusive, insufficient traffic
```

Two guards on the feedback:

- **Never suppress a category outright** on one bad outcome; downweight it in
  `rank()` (which already exists) rather than hiding advice the user can judge.
- **Say the sample size.** "Worked once" is not "works". The prompt should
  carry the count so the model does not over-generalise from a single result.

## How much confidence the traffic actually buys

Projects differ by orders of magnitude — some sit near `MIN_PAGEVIEWS`, others
run ~1000 views/week. The verdict a project can support scales with that.
Taking week-over-week noise as Poisson with a 1.5× overdispersion allowance
(bursts, referral spikes, bot leakage), the smallest change distinguishable
from normal variance:

| weekly views | min detectable change |
|---:|---:|
| 40 | ~66% |
| 100 | ~42% |
| 300 | ~24% |
| **1000** | **~13%** |
| 3000 | ~8% |

So the rule is not one threshold, it is: **each project contributes at the
level of confidence its own traffic supports.**

- **≳300 views/week** — per-project verdicts are meaningful. A 13-24% lift on a
  1000-view project is a real signal and should be reported as one.
- **≲100 views/week** — per-project verdicts stay `inconclusive` essentially
  always. These projects still contribute: they feed the pooled signal below,
  where N comes from the number of projects rather than the number of views.

The power check computes this per project from its own baseline variance
rather than reading a table — the table is the intuition, not the
implementation.

## Cross-project learning

This is what rescues the low-traffic projects. One action on one small project
is underpowered forever; the *same strategy* tried across nineteen projects has
N=19, and that is a sample worth reasoning about.

Pool outcomes by `category` (the field already on `GrowthAction`) across all
active projects:

```
Strategy: "FAQ / answer-engine schema"  (seo)
  tried on 11 projects · helped 7 · no effect 3 · hindered 1
  median lift where measurable: +18% sessions (4 projects ≳300 views/wk)
  weakest on: single-page sites (2 of the 3 no-effects)
```

**The trap, and it is a real one: projects are not exchangeable.** A tactic
that lifts a B2B SaaS landing page can do nothing for a community site or a
local-services page. Pooling naively produces textbook Simpson's paradox — an
aggregate that says "helped" while quietly failing on the segment you are about
to apply it to.

Two guards:

- **Segment before pooling.** Group by the attributes that plausibly moderate
  the effect: traffic tier, whether the site is content-heavy or single-page,
  and dominant acquisition channel (`top_sources` already gives this). Report
  the aggregate *and* the segment.
- **State transfer explicitly.** When a strategy is proposed for project B
  because it worked on A and C, the card says so — "worked on 3 similar
  projects (content sites, ~500 views/wk)". A recommendation the user cannot
  audit is one they cannot overrule.

## Grading itself

The generator already emits `impact` and `confidence` on every action. Those
are **predictions**, and predictions can be scored — which means the system can
grade its own strategy-making rather than only its actions.

Track calibration: of the actions it labelled high-confidence, what fraction
actually helped?

```
Strategy calibration — last 90 days, all projects
  said high confidence   -> helped 9/22  (41%)
  said medium confidence -> helped 6/19  (32%)
  said low confidence    -> helped 2/8   (25%)
  → confidence is weakly discriminating; treat "high" as ~40%, not ~80%
```

This is the most robust number in the whole feature, because it aggregates
across every project and every category — it stays meaningful exactly where
individual verdicts do not. Two uses:

1. **Show it to the user.** "When I say high confidence I am right about 40% of
   the time" is honest, and it tells them how much weight to give the next card.
2. **Feed it back as a discount, not a silencer.** If high-confidence claims
   land at 41%, the generator is overconfident and its labels should be
   recalibrated — not its advice suppressed.

Grading must never be self-assessed prose. It is computed from
`growth_action_outcomes`, or it is not a grade.

## Non-goals

- Causal inference. This is a before/after with an honesty gate, not an
  experiment. It should never use the word "caused".
- A/B testing. Meaningless at this traffic.
- Auto-running the prompts. The user runs them; the loop observes.

## Open decisions

1. **Who picks `target_metric`?** The generator proposing it (and the user
   confirming at verify time) is my recommendation — it forces the action to
   be falsifiable at the moment it is written.
2. **Default window** — 28 days is the shortest that is usually meaningful at
   this traffic, but it is slow feedback. 7/14/28 evaluated progressively, with
   early windows explicitly labelled provisional, is the honest compromise.
3. **Does an unverified action still get measured?** I would say no: without
   knowing the change landed, a delta is unattributable to anything.
4. **Where does the cross-project comparison run?** It is a nightly aggregate
   over every active project, which is the `health-review` starter's shape
   exactly — a scheduled recipe that reads durable state and writes a report.
   Reusing that seam beats a bespoke loop.
5. **Does a pooled result auto-propose to other projects?** I would surface it
   as a suggestion carrying its provenance, never as an auto-created action.
   "Worked on 3 similar projects" is an argument the user can weigh; a card
   that silently appeared because of a correlation elsewhere is not.
