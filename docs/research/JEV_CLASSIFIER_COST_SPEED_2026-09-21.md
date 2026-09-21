# Jev as a classifier for harness speed and cost — where it belongs, and where it does not

**Date:** 2026-09-21 · **Status:** research contribution to the four-way harness-cost debate (Codex Astra leading;
`permagent-runtime-46`, Cursor, and this session contributing; the Permagent Council to debate the result).
**Brief**, relayed by `permagent-runtime-46` — Jesse: *"consider how to best include Jev as a classifier model for
speed and cost optimization."* Where does a typed, non-generative classifier beat **(i)** a deterministic rule,
**(ii)** the status quo, **(iii)** a small local generative model — on **our** surfaces? Held to two constraints
already agreed: **never a classifier over explicit state**, and **a cheap gate must fail awake.** The answer must say
where the classifier's error is **cheap and recoverable**, *"because that is the only place it belongs."*

**Evidence discipline** (the lesson of `CODEX_DEBATE_2026-09-21.md`): every code claim below is marked **located** (I
found the line) or **traced** (I followed the path end to end). Most are *located*. The peer session owns this code and
should be believed over me where we differ. Ledger figures are the peer's corrected ones. "Jev" means a **local
typed-decision backend**, never TypeSafe's hosted service for private state.

---

## 0. Answer

**Of the four surfaces asked about, a classifier earns a place on one, conditionally on a second, and on neither of
the other two. The biggest lever found in this debate — 33–43k tokens of tool definitions per request — is not a
classifier problem at all.** And on the one surface where it fits, the arithmetic shows the selector's *accuracy barely
matters*; what matters is that a wrong choice is **visible**. So the honest recommendation is smaller than the
question: *start deterministic, use the cheapest semantic picker that exists (Apple's on-device model — free, no RAM
cost to us), and bring a Jev-like in only if a measured experiment says the alternatives are too slow or too
inconsistent.*

There is a **fifth surface nobody listed** that fits the cheap-and-recoverable test better than any of the four and
attacks the price-independent finding (input : output = 90 : 1) directly: **triaging large tool outputs before they
enter the context.** It is also the natural home for the one experiment the Codex debate left open.

| Surface | Error if wrong | Cheap? Recoverable? | Verdict |
|---|---|---|---|
| 1. Tool/extension subset before the first request | one wasted small cache write + one turn, then expand to the full set | **yes / yes — if the escape hatch is visible** | **Fits.** But the win is the *stable small set*, not the classifier. |
| 2. "Does this sweep need a model call?" | a wrong skip is a **silent failure** | no / no | **Does not belong.** Deterministic gates already do it. |
| 3. Goal complexity → tier (`assess.rs`) | wrong-down cost **$2.19 and 38 turns for zero files** | recoverable, **not cheap** | **Offline experiment only** — and beat a logistic regression first. |
| 4. "Does this output need a verification pass?" | a wrong skip is a **false "done"** | no / no | **Tighten-only, marginal.** The evidence gate is deterministic and exists. |
| 5. Tool-output triage at ingest *(added)* | a dropped line, re-fetchable from a stub | **yes / yes** | **Best fit. The experiment to run.** |

---

## 1. The tool/extension subset — fits, but read the arithmetic first

**What is known.** `[PEER, measured]` full-extension sessions ship **106–134 tool definitions, 33–43k tokens per
request** — 82.8 % of one request, 50.3 % of another. Astra's arithmetic: a **volatile** 4k subset costs more per turn
($0.0100) than a **stable cached** 43k block ($0.0086), so naive per-turn "dynamic toolsets" *lose*; a **stable** 4k set
over 38 turns is **$0.0396 vs $0.4257.** *Select once, early, fixed per phase, never rerank per turn.* **located:**
`cost_router/cache.rs:1-25` states the same rule from the other side — the cacheable prefix is ordered
**tools → system → repo-map → files → tail**, and *"any reordering or mid-prefix insertion invalidates every token
after the change."* Tools are **first**: changing the toolset mid-session throws away the entire cache.

**The error arithmetic** (Sonnet-class pricing as Astra used it: input $2/M, cache write 1.25×, cache read 0.1×):

```
right choice, 38 turns:   small set  $0.0396   vs   full set  $0.4257      → saves ≈ $0.39 per session
wrong choice, found at turn t:   wasted 4k write $0.010  +  one wasted turn
                                 +  re-writing the history so far into a new cache (≈10k tokens at turn 5 → $0.025)
                                 then you pay what the full set would have cost anyway        → loses ≈ $0.04–0.05
break-even:   p_right × 0.39  >  (1 − p_right) × 0.05      →      p_right > ~12 %
```

**A selector that is right one time in eight already pays for itself.** So selector *accuracy* is not the constraint,
and paying for a well-calibrated classifier to pick extensions is solving the wrong problem. The constraint is the
error the arithmetic cannot see: **the agent that lacks a tool, does not know it, and quietly does worse work.** That
error is neither cheap nor recoverable — it is a quality loss with no signal.

**What makes the error visible.** **located:** a `manage_extensions` meta-tool exists
(`platform_extensions/ext_manager.rs:138`). Keep it in every small set, with a **one-line-per-extension catalogue**
(30 default-on extensions → roughly 600 tokens) so the model can *see what it is missing and ask*. Expansion happens
**at most once, to the full set** — never a rerank — because every expansion is a full cache rewrite. *Not traced:*
whether enabling an extension mid-session works end to end, and what it does to the cached prefix in practice. That is
the first thing to verify, and it is the peer's code.

**Who picks the set — in order of preference:**

1. **Explicit state → deterministic. No classifier, by the agreed constraint.** **located:** workers already get their
   extensions from `goal_worker_extensions_for_dispatch` (`orchestrator.rs:6254`) and a compact profile
   (`worker_prompt_profile` `:6326`; `agent.rs:240`). Recipes declare their own `extensions`
   (`recipe/recipe_extension_adapter.rs:184`). A coding worker does not need finance, voice or browser tools — that is
   a role→set table. **This is probably most of the saving, and it needs no model.**
2. **Free-text chat, first message → a recall-biased multi-label pick.** Only here is the input genuinely
   unstructured. Because the break-even is ~12 %, a **hard label is sufficient** (the Codex debate's test: code decides
   what to do with it, and the error is cheap). Cheapest adequate picker first:
   - a deterministic score — the message matched against each extension's description (BM25/FTS);
   - **Apple's on-device model with `@Generable` multi-select** — free, private, **no RAM cost to us** on a machine
     with none to spare, already wired via the Swift sidecar (which today is text-only — no guided generation yet);
   - the resident Ollama model with a JSON-schema `format`;
   - a Jev-like — **only if** the above are measurably too slow or too inconsistent.
   Bias toward inclusion: *when unsure, include.* A superfluous extension costs a few hundred cached tokens a turn; a
   missing one costs a cache rewrite or a worse answer.
3. **Never per turn. Never mid-tool-loop.**

**Does the classifier's speed matter here?** Barely — this runs **once per session**. Even a 1–2 s on-device pick is
small next to the first request's uncached prefill of 43k tokens, which the small set *removes*. The "300 ms on every
dispatch" worry applies to per-turn uses, which this is not.

**Where a non-generative classifier could still genuinely win:** *repeatability.* A stable cache needs the **same
input to yield the same subset**, byte for byte, across sessions. A typed classifier is deterministic by construction;
a generative model is only so at temperature 0 with a fixed prompt. If subset drift shows up as cache misses, that is
the argument for it — a measurable one.

---

## 2. "Does this background sweep need a model call?" — does not belong

I argued for this in `JEV_RETHINK` and withdrew it in the Codex debate. It fails the brief's own test: **a wrong skip
is a silent failure**, the expensive direction. The status quo already wins: **located** `proactive.rs:125-171` skips
the model for delivery spacing, quiet hours, a missing brain and an empty candidate list; `growth_sweep` never calls a
model by design; the sweeps tick every 3–6 hours, so the prize is a handful of calls a day. In the harness the
equivalent question is **explicit state** — `agent.rs:662` suspends with zero provider turns — so a classifier there
is forbidden by the first constraint. Better alternatives come first for anything new: a **state transition**, a
**conditional HTTP request** (ETag / If-Modified-Since costs nothing), or a natural-language condition **compiled once
into a reviewed deterministic predicate.** A semantic judge is for the residue that cannot compile — and even then
only with a **time-bounded forced wake**, coalescing and backoff.

---

## 3. Goal complexity → tier — an offline experiment, not a deployment

**The design stance stands:** `cost_router/assess.rs:14-31` is zero-LLM on purpose; prose may only route **down**; the
peer has just made local dispatch require **positive structural evidence**. A classifier may not raise a tier from
prose (attacker-influenced) and may not sit over structure that code already reads.

**The error is recoverable but not cheap.** `[PEER, measured]` a two-file goal sent to a 7B cost **~38 turns and $2.19
with zero files written.** The ladder escalates on failed verification, so the mistake heals — after the money is
spent. A down-routing classifier therefore belongs **only if the cheap rung is bounded first**: a hard cap of a few
turns or a few cents before escalation. With that cap, a wrong-down costs cents and the brief's test is met; without
it, no selector — classifier or keyword list — should be trusted to route down more aggressively.

**Why it is still worth an experiment:** this is the **one surface with real ground-truth labels** — not approvals,
not clicks. Every dispatched goal has a tier, an outcome (verification passed or failed), and a cost. So it can be
tested **offline, on history, with nothing deployed**: *predict "would the cheap rung have succeeded?" from the goal's
text and structure, and compare against today's heuristics.* Two honest caveats: **selection bias** — outcomes exist
only for the tier actually chosen, so the counterfactual is unobserved for most goals; and **baseline first** — the
richest feature vector in the repo is already extracted (`planner_complexity.rs:559`, with JSON receipts), and a
**logistic regression over those structural features** is the opponent to beat. If it cannot be beaten, there is no
case for a text classifier here at all.

---

## 4. "Does this output need a verification pass?" — tighten-only, marginal

Skipping verification on a classifier's say-so is a **false "done"** — vendor-admitted as this product category's
defining failure, and the most expensive error in the harness. So a classifier may only **add** verification, never
remove it. That leaves it little to do: the evidence gate that matters is deterministic and **located** already —
`tool_monitor.rs:352 verify_pass_after_edits`, `:371 diff_touches_only_tests`, orchestrator `:20863
scan_verify_evidence`; reviewer strength comes from diff statistics (`recommend.rs:1475`). The published numbers agree:
the strong result (AUROC 0.976) came from a *deterministic* "no fresh check since the last file mutation" rule, while
the honest semantic result on 2,645 hand-labelled stops was **AUROC 0.60–0.62**. **Do not build this.**

---

## 5. The surface nobody listed — triage large tool outputs *at ingest*

The one finding that survives every pricing correction is **90 : 1 input to output.** Tool definitions are one part of
that input; **tool results are the other** — file dumps, grep hits, test logs — appended to the conversation and then
re-read, at cache-read price, on **every subsequent turn**.

- **It does not break the cache.** Trimming a result *before it is first appended* changes only the mutable tail. That
  is the opposite of history compaction, which rewrites the middle of the prefix — the thing the Nous study found tied
  plain recency and the cache rules forbid. **Ingest-time, never retroactive.**
- **The error is cheap and recoverable by construction.** Replace what is dropped with a stub —
  `[214 lines elided — read_more(result_id, range)]` — so nothing is lost, only deferred. A wrong drop costs **one
  extra tool call.** No silent failure: the model can see that something was elided and where.
- **It is high-frequency and per-item**, which is where a non-generative scorer has a real speed case: this runs on
  *every* large tool result, so a small generative model's seconds-per-call would show up in every turn, and a
  tens-of-milliseconds scorer would not. **This is the one place "speed" is actually on the classifier's side.**
- **Deterministic rule first:** **located** `agents/large_response_handler.rs` already truncates; head/tail windows and
  a BM25 score against the goal text are the baseline.
- **It has an oracle we already own.** `permagent-eval` scores by **exit code** — no human labels, no LLM judge, no
  selection-biased clicks. Same fixtures, same model: **no triage vs head/tail vs BM25 vs k-sample agreement vs
  calibrated logits**, measured on **task success, total input tokens and cost.** That is precisely the first open
  question the Codex debate ended on — *which uncertainty/ranking signal wins on a real workload* — with a free,
  objective label. The evidence that this works is `[UNVERIFIED]` ecosystem claims only (a Bash-output pruner; "30 %
  token savings" in a browser tool); I have no measured result. **That is the point of running it.**

---

## 6. Speed and hardware — what is actually known

**No measurement exists on our hardware.** Everything below is the authors' own, mostly on bigger chips.

| Backend | Footprint | Reported latency | Notes |
|---|---|---|---|
| Hosted Jev | — | **p50 379 ms / p95 481 ms** (the only independent figure; more than half is network) | off-limits for private state; a public-content bench arm is legitimate |
| Laya 421M, `laya serve` | 404 MB (Q4), static Metal binary, HTTP | **none published for Apple Silicon** | claims 0.766 acc / 0.081 ECE vs hosted Jev 0.727 / 0.144; default port **:8080 collides with the Picker** |
| laya-mlx | 421M FP16 | 13.4 ms P50 — **on an M3 Max 40-core** | Python only, no server → ruled out by the no-sidecar rule |
| von 395M | 1.5 GB | ~18 ms on MPS (chip unnamed) | claims `/v1/systemone` compatibility; no calibration figure |
| `llama-server` prefill + logprobs | whatever model is already served | **unmeasured** | **one suffix pass per question** — hosted Jev's "latency flat in question count" does *not* transfer; and there is **no logprob plumbing anywhere in `crates/goose/src`** |
| Apple on-device, `@Generable` | **0 to us** | unmeasured here | hard choices only — **no probabilities**; sidecar is text-only today |
| Resident Ollama 7B, schema-constrained | 4.7 GB — **often not resident** | seconds; a cold load is worse | the M4 showed 0 MB available and 11–13 GB of swap on every reading |

**The constraint that dominates is memory, not FLOPs.** On a machine with no free RAM, a 0.4 GB resident model gets
paged out and its "13 ms" becomes whatever a page-in costs. That is a reason to prefer **Apple's on-device model for
once-per-session picks**, and to put any always-resident scorer on the **M1** — by agreement with the Sidebar session,
whose battery runs there — or not at all. **A 30-minute measurement on real hardware would replace this whole table**;
it needs a ~400 MB download, so it needs Jesse's go-ahead and a heads-up to both peer sessions.

---

## 7. What I would tell the Council

1. **Cut the tool block deterministically first** — per-role and per-recipe sets, stable per phase, with
   `manage_extensions` and a one-line catalogue as the visible escape hatch. That is where the dollars are, and it needs
   no model.
2. **The remaining open cost question is caching, not classification:** why Anthropic reads 53 % from cache where zai
   reads 75 %. `cache.rs` names the usual culprit itself — byte drift in the system prompt. That is a diff of two
   consecutive request bodies in `llm_request.*.jsonl`, not an inference problem.
3. **For free-text sessions, use the cheapest picker that works** — a deterministic score, then Apple's on-device
   model. The break-even is ~12 % accuracy; do not buy calibration you do not need.
4. **Run one experiment: tool-output triage at ingest, on the eval bench, scored by exit code.** It is the only
   candidate here with a cheap recoverable error, a genuine speed argument for a non-generative scorer, *and* an
   objective label — and it settles the debate's open question about which signal wins.
5. **Bound the cheap rung before routing anything else down**, then test tier prediction **offline** against a
   logistic regression on features that already exist.
6. **Do not put a classifier in front of sweeps or verification.** Both fail the brief's own test.

**What would change my mind:** a measurement showing subset drift causing cache misses (→ the repeatability case for a
typed classifier on surface 1); a triage experiment in which calibrated logits beat BM25 and k-sample agreement at the
same task-success rate; or an on-hardware latency number showing the cheap pickers are too slow for the first turn.

**Not established here:** whether `manage_extensions` works mid-session and what it does to the cache (*located, not
traced*); the real per-extension token cost, and therefore whether ~4k is an achievable small set; any latency on our
chips; any measured benefit from ingest-time triage; how often the Apple on-device path is actually available.
