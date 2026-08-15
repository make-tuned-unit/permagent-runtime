# Phase 5 — Final baseline, and the delta

Written 2026-08-15. The audit protocol's definition of done: re-run the Phase 0 baseline with every remediation item merged, and diff it against `00-baseline.md`.

Measured on `main` @ `d16e1de2`, all 20 items landed.

---

## The delta

| Check | Baseline (2026-08-14) | Now | |
|---|---|---|---|
| `clippy -p permagent -p permagent-daemon --all-targets -- -D warnings` | exit 0 | **exit 0** | ✅ |
| `scripts/test-daemon.sh` | 1252 passed, 0 failed | **1279 passed, 0 failed** | ✅ +27 |
| `cargo test -p permagent --lib` | 2940 passed, **8 failed** | **3017 passed, 1 failed** → **0 failed** after #1012 | +77 tests, **all 8 failures resolved** |
| `tsc --noEmit` | exit 0 | **exit 0** | ✅ |
| `vite build` | exit 0 | **exit 0** | ✅ |
| Coverage tooling | **none anywhere** | `.github/workflows/coverage.yml` | ✅ |
| `cargo audit` | 7 advisories / 12 allowed warnings | 7 / 12 | unchanged |
| `npm audit` | 6 (1 critical) | 7 (2 critical) | see note |
| **main CI** | red, repeatedly | **GREEN** (run 31881139055) | ✅ |

## What the failure count actually means

8 → 1 matters less than *which* seven went.

The baseline's eight split as four real (the `prompt_manager` snapshots) and four machine-dependent (three `posthog::*_without_opt_in`, one search test). The four machine-dependent ones failed **here and passed in CI**, because they read this developer's live config — telemetry on, Guard on.

**They now pass on this machine, with that same config unchanged.** That is the evidence that P0-4 did what it claimed: green no longer depends on whose laptop ran the suite. It is also what makes every other number in this table trustworthy — before P0-4, a green run was not evidence of anything.

## The one remaining failure was a real bug, not a flake

`developer::search::zero_match_suggests_dropping_filters` went unexplained through all three independent reviews and was listed in `03-plan.md` under "not doing" as an unexplained local failure. Being the last one left, it got chased.

ripgrep 15.2.0 exits **code 2** with `No files were searched, which means ripgrep probably applied a filter you didn't expect` when a `--glob` or `--type` filter excludes every candidate. `search.rs` treated every exit-2 as a hard error. So **any search whose filter matched no files told the user "Search failed"** — instead of the message written for exactly that case: *"the matches are outside your glob filter, retry without it."*

That is the empty-vs-broken confusion this entire audit was about, reintroduced one layer beneath the code that exists to prevent it. It read as an environment flake because CI runs an older ripgrep that exits 1 there.

Fixed in **PR #1012**, keyed on ripgrep's own stderr line rather than the exit code, because a bad regex also exits 2 and must stay an error — with `an_invalid_regex_is_still_an_error` pinning that half so the fix cannot trade one confusion for its mirror image.

**Worth noting as a process finding:** three independent reviewers, a rebuttal round, and an adjudicated plan all classified this as environmental. The thing that caught it was re-running the baseline at the end and refusing to wave away the single remaining red. The final baseline is not ceremony.

## npm audit: 6 → 7, honestly

Not a new vulnerability. P2-16 added `@vitest/coverage-v8`, which inherits the **existing** vitest advisory (`via: ["vitest"]`), so the same underlying risk is now counted twice. The P2-17 gate had to allowlist it precisely because it is critical-by-inheritance with no GHSA id of its own.

The real exposure is unchanged and still dev-tooling-only — on a machine that also holds the user's keychain, which is why it was flagged rather than dismissed.

## Claim status

Nine of twenty-three claims failed at audit. The false statements in the always-injected self-knowledge brief — the audit's central finding — are corrected, and the code gaps behind them closed rather than papered over:

| Claim | Then | Now |
|---|---|---|
| SK-1 sovereignty | NOT MET | Descriptor scoped to inference calls; **all three unaudited egress paths now write audit rows** (#1002) |
| SK-2 credentials | NOT MET | Guard scans the pushed commit chain from committed blobs, fails closed (#998) |
| SK-4 pooling types | NOT MET | Claim corrected to what the wire actually carries (#996) |
| SK-5 destructive gate | NOT MET | Push refspec asserted (#997); internal engine can no longer run unisolated (#1006) |
| SK-6 automation | NOT MET | Three floor bypasses closed, runtime kill-switch removed (#1004) |
| SK-11 test integrity | NOT MET | Config pinned in the test harness (#995) — verified above |

A fifth false statement, missed by the plan, was found during implementation: the "Sovereign controls (Settings)" *surface* descriptor repeated the same "every cloud call" claim ~60 lines from the guard descriptor that was being fixed. Corrected in #996.

## What remains open

- ~~PR #1012 — the ripgrep fix~~ **merged** (`815d54f6`). All eight baseline failures are now resolved; `permagent --lib` is clean.
- **P1-5 behaviour change** — an internal goal in a *non-git project* can no longer be dispatched at all. That is the isolation promise being enforced, but it is a real user-visible regression and its implementer specifically asked for a second opinion. Merged; easy to revisit.
- **Eleven Dependabot PRs** (#953–#989) untouched by this work.
- **`developer::search`** is fixed, but the class deserves a sweep: how many other places treat a tool's non-zero exit as failure without asking what the tool meant?

## Verdict

Three reviewers independently returned **DO NOT SHIP** on 2026-08-14. Every P0 and P1 they raised is now on main, the suite that proves it is no longer machine-dependent, and main CI is green.

The honest remaining caveat is coverage: `coverage.yml` now exists and ratchets on non-regression, but it has one data point. "Well tested" is measurable for the first time and not yet measured. That is a better position than the baseline — where it was unfalsifiable — but it is not the same as proven.
