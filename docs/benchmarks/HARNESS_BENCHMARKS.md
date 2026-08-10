# Harness efficiency benchmarks

What this measures, how, and every result so far — including the ones that
didn't flatter us. The point of this file is that a claim like "the harness
is cost-effective" is either backed by a row here or it is marketing.

## Method

`scripts/bench/harness_bench.py` drives the REAL goal pipeline — card →
`goal_advance` (worker pinned) → isolated worktree → completion checks →
receipt — and harvests evidence from the worker's own transcript
(`cli-*` workers) or the daemon's session accounting (internal harness
workers). Nothing is taken from a model's self-report.

Rules:
- **Failures count.** A run whose completion check failed is a FAILED row,
  not a discarded one. A cheap model that fails the task is not cheaper.
- **n=1 is labeled n=1.** The report prints run counts and averages nothing
  silently.
- **Configs are labeled**, because the `permagent` worker's model comes from
  the live `PERMAGENT_ROLE_*` mapping and results must stay attributable
  after the config changes.

Task suite (one per work class): `lookup` (single-symbol navigation),
`multi_file` (structural edit across construction sites), `writing`
(~600-word launch post), `scaffold` (build-from-scratch CLI with tests
that must pass).

Provider matrix available on this machine (all keyed as of 2026-08-10):
anthropic, openai, moonshot (Kimi), minimax — via the `permagent` worker
with per-role mappings; plus the flat-rate CLIs (`claude_code`, `codex`).

## Results

### 2026-08-10 — code-map injection, lookup task (goals A/B, by hand)

Same task shape, different single-symbol targets; claude_code worker;
map absent vs 4k flat map slice injected into the worker prompt.

| | A (no map) | B (flat map) | Δ |
|---|---|---|---|
| tool calls | 5 | 5 | 0 |
| tool sequence | grep→grep→Read→Write→commit | identical | — |
| assistant turns | 10 | 8 | −2 |
| output tokens | 2,728 | 2,447 | −10% |
| billed input | 491,079 | 398,331 | −18.9% |
| cost-weighted | — | — | ≈ wash (higher cache-write) |

**Finding:** injection did not change navigation behaviour — the worker
greped exactly as without the map, because one grep is optimal for "where
is function X" and the 4k top-slice of a 1,660-file tree never contained
the goal's deep path. Consequences adopted: goal-aware subtree slice
(deep paths + ancestry ahead of the tree top), and the map's real upgrade
path is a query tool + landing-time re-index, not a bigger injection.
Multi-file rerun (goals C/D) pending.

## Open cells

The matrix worth filling next, one row at a time:

| task | claude_code (subscription) | permagent/haiku | permagent/kimi | permagent/minimax | permagent/openai |
|---|---|---|---|---|---|
| lookup | A/B above | — | — | — | — |
| multi_file | C (in flight) | — | — | — | — |
| writing | — | — | — | — | — |
| scaffold | — | — | — | — | — |

**The Haiku column is the thesis test.** Prime-agent's finding was that
harness quality moves outcome quality more than model price. If that holds
here, `permagent/claude-haiku-4-5` — the cheapest capable Anthropic model,
inside worktree isolation + authored completion checks + deterministic
verify + the escalation ladder — should pass the suite at a small fraction
of the frontier cost, and its failures should surface as ladder climbs, not
as bad results reaching the user. A Haiku column full of green is the
harness working; a Haiku column the ladder keeps rescuing is the honest
price of the cheap tier, measured.

Procedure per metered cell: set the role map, dispatch, label with the
resolved model, restore the map. e.g.
`PERMAGENT_ROLE_EDIT_PROVIDER: anthropic` + `PERMAGENT_ROLE_EDIT_MODEL:
claude-haiku-4-5` in `~/.permagent/config.yaml`, then
`harness_bench.py run --task scaffold --worker permagent --label permagent/haiku-4.5 …`.

Interpretation guide: the subscription CLI column is the flat-rate incumbent
(marginal cost ≈ $0, scarce resource = rate limit); the metered columns are
where per-token price and failure rate trade off. The decision this table
feeds is the cost-tier ranking and the per-role map — not vendor loyalty.
