# Decision: disable co-retrieval reranking (live production regression)

**2026-07-03 · owner: Jesse · action: bump spectral rev after fix**
**RESOLVED 2026-07-30 — verified in the pinned rev; no rev bump needed.**

Spectral's co-retrieval reranking (`co_retrieval_weight = 0.10`) is measurably
DEGRADING real-query recall in production. Blind, non-circular LLM judge on the
real Permagent workload (`recognition_events.query`): co-retrieval ON loses to OFF
13–59 (isolation) and 17–56 (production config), p≈0. A proxy-query run showed the
opposite but was a mirage.

**Action:**
1. Fix in spectral repo (default `co_retrieval_weight` to 0.0; add config knob) —
   see `spectral/docs/internal/tickets/coretrieval-regression.md`.
2. Bump `spectral` git `rev` in `Cargo.toml:88` to the fixed commit.
3. Confirm with the blind judge on freshly logged real queries.

Also linked: `spectral-tact:166` UTF-8 slice panic crashes the cascade retrieval
path on real content — fix in the same spectral PR.

Note: no query-text logging work needed — it already exists in
`recognition_events.query` (that's what this was measured on).

---

## Resolution (2026-07-30)

All three code-side items are **already in the pinned rev** — `Cargo.toml:88`
pins `spectral` at `486459c`, and the fixes landed at or before it. Verified by
reading the source *at that rev* (`git show 486459c:<path>`), not at spectral HEAD:

| Item | Status | Evidence at rev `486459c` |
|---|---|---|
| 1. `co_retrieval_weight` defaults to 0.0 | **done** | `crates/spectral-graph/src/brain.rs:1833` — `co_retrieval_weight: 0.0`. `RecallProfile::Fast` also forces `0.0` (`crates/spectral/src/lib.rs:328`). |
| 1b. boost skipped when weight is 0 | **done** | `brain.rs:1844` — `if reranking_config.co_retrieval_weight > 0.0`; also gated at `:2027`. Upstream `7920cd3` is an ancestor of the pin. |
| 2. bump the pinned rev | **not needed** | `486459c` is at-or-after `095f234` ("co-retrieval regression fix"); `git log 486459c..7920cd3` is empty, so both fixes are ancestors of the pin. |
| UTF-8 slice panic (`spectral-tact`) | **done** | `crates/spectral-tact/src/lib.rs` `build_context_bundle` backs off to `is_char_boundary` before slicing, with a comment naming the em-dash panic. |

**Item 3 (blind-judge re-confirmation on freshly logged real queries) remains
open** — it is a measurement task, not shipped code, and it now has a natural
home: the production-replay export built for Spectral's 2026-07-29 dispatch
(`spectral/docs/internal/DISPATCH-permagent-production-replay-2026-07-29.md`)
emits exactly the recall traces that re-run needs. Fold it into the first
Tier-C replay rather than running it standalone.

Caveat on scope: this verifies the *defaults* shipped in the pinned rev. It does
not prove no Permagent call site passes a non-zero `co_retrieval_weight`
explicitly — no such call site exists in `crates/` today (the only Permagent
reference to co-retrieval is `rebuild_co_retrieval_index`, which builds the index
and does not set the rerank weight), but a future caller could reintroduce it.
