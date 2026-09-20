# Jev research — Track D: memory retrieval and trading/forecasting

Research date: 2026-09-20. All web content below is treated as untrusted data.
Tags: [MEASURED] = numbers from a real artifact I fetched; [ANECDOTAL] = someone's claim without data; [UNVERIFIED] = could not confirm.

---

## 0. Baseline facts about Jev (verified from primary sources)

- TypeSafe AI announced Jev on 2026-09-15, out of stealth with $40M seed. Founder Diogo Almeida (ex-OpenAI, RLHF/ChatGPT co-inventor). Architecture called "Reinforcement Learning for Calibrated Decisions" (RLCD).
  - https://typesafe.ai/blog/introducing-system-one-models-and-jev
  - https://www.theregister.com/ai-and-ml/2026/09/16/typesafe-ai-debuts-model-for-machines-that-plays-doom/5296711
  - https://www.marktechpost.com/2026/09/19/typesafe-ai-releases-jev/
- Interface: one `state` blob + a map of typed questions, all answered in ONE parallel pass. Types: `noul` (yes/no -> probability), `choice` (options -> per-option probs, cardinality up to 255), `score` (ordered levels -> continuous score + distribution). No string generation, nothing to parse, cannot type-error. [MEASURED — vendor doc]
- Latency 70ms–500ms vs 3–329s for frontier LLMs on comparable tasks. Pricing $0.042/MTok input, output FREE. Claims 40x–200x faster. [vendor claim]
  - https://typesafe.ai/blog/introducing-system-one-models-and-jev
- **CRITICAL LIMITATION (load-bearing for Track 2):** "Jev has no knowledge of the world beyond the state you hand it. It cannot look anything up."
  - https://gist.github.com/pjburnhill/adf8d28efcad9df037bfdece178ef965 (project reference, knowledge date 2026-09-16)
  - Vendor positions it for "high-volume, repeated decisions over known answer sets": ticket triage, intent routing, moderation, extraction, scoring, guardrails.
- **Hard limits and documented incapabilities** (https://flaviocopes.com/jev/ — independent deep dive, corroborates vendor docs):
  - State + all questions share a budget of **~64,000 tokens**; **state + the longest single question must fit in ~32,000 tokens** (~150,000 chars of English).
  - `choice`: up to **255 options**. `score`: **2 to 10 levels**.
  - Early-access rate limits: **250,000 tokens/sec and 1,200 requests/minute** (stated to be adjusting dynamically).
  - Latency: vendor's 70–500ms, "most queries around 100ms" — but **measured from US West Coast**; measure from your own location before promising anything.
  - **Jev explicitly CANNOT: do maths reliably, count items accurately, COMPARE DATES/TIMES, handle indirection, or generate text.** Author's summary: "pick a card from the deck" rather than "ask it to name a card."
  - **This date/time incapability is load-bearing for both tracks.** Memory *staleness* ("is this memory still valid as of today?") and pre-trade *stale-data* checks ("is this quote older than N minutes?") are both fundamentally date comparisons. **Compute recency deterministically in your own code and hand Jev the already-computed fact ("this memory is 412 days old") as part of the state — never ask Jev to do the comparison.**
- Available via direct API, Cloudflare Workers AI, Vercel AI Gateway, Pydantic AI.
  - https://developers.cloudflare.com/ai/models/typesafe/jev/
  - https://pydantic.dev/docs/ai/models/typesafe/
- Community index: https://github.com/yibie/awesome-jev (note: dozens of forks/clones exist with the same name — cite the original).
- Build directory: https://madewithjev.com — categories: agents and browsers, games and real time, triage and routing, **trading and markets**, content and growth, research and data, robotics and devices, tools and apps. There is **no memory category** (closest: "research and data"). [MEASURED]

---

## 0.1 INDEPENDENT CALIBRATION AUDITS — bears on BOTH tracks, and it is the most important finding in this report

The vendor claims "Calibrated: higher confidence means higher accuracy." **Four independent audits say this is true in-distribution and false out-of-distribution, and that the raw probabilities need per-task recalibration before you can threshold on them.**

### AnthusAI/Jev-Calibration [MEASURED]
https://github.com/AnthusAI/Jev-Calibration — "Does Jev's confidence mean what it says?"
- Sentiment task, **8,801 unique examples** (5,280 calibration / 3,521 test). Neutral-tier examples carried deliberately arbitrary labels, tracking ~50% accuracy.
- **Raw Jev ECE 0.117. After Platt scaling 0.052. After isotonic regression 0.008.**
- Across all question variants: **raw ECE 0.064–0.160; isotonic-calibrated 0.006–0.018.**
- Isotonic matched or beat Platt **at every calibration-set size from 20 to 5,280** — the miscalibration is not sigmoid-shaped; there is a plateau in the mid-probability range Platt cannot fit.
- Stated conclusion: does Jev's confidence mean what it says? **"Not without calibration."** After isotonic, yes.
- **Practical: you can fit a usable isotonic calibrator from as few as ~20 labelled examples.** That is a very cheap thing to add.

### scienthoon/jev-ood-calibration [MEASURED] — the sharpest warning
https://github.com/scienthoon/jev-ood-calibration — 900 rule-generated support tickets (a task Jev cannot have seen) + OpenBookQA / CommonsenseQA / HellaSwag, via Vercel AI Gateway. Reproducible for ~$0.06.

| Question type | ECE | Refit T | Direction |
|---|---|---|---|
| choice (queue) | 0.082 | 3.29 | **overconfident** |
| score (priority) | **0.325** | 3.40 | **overconfident** |
| noul/boolean (angry) | 0.079 | 0.66 | **underconfident** |

- **Miscalibration direction differs BY QUESTION TYPE within the same task.** `score` is badly overconfident; `noul` is underconfident. A single global temperature cannot fix both. Stated recommendation: **"Calibrate per question, not per model."**
- **The killer datapoint:** on the "priority" task, where the correct answer is determined by organisational policy and is *not recoverable from the ticket text*, **Jev averaged 0.74 confidence at 44.7% accuracy — "confidently wrong where the rule is unknowable."**
- Noise floor for the 900-item set is 0.024; measured ECE 0.107 is **4.4x the floor** — the miscalibration is real, not sampling noise.
- Public benchmarks calibrated near-perfectly (refit T ≈ 1.0), which the author reads as evidence **those benchmarks are likely in Jev's training mix**. So vendor-style benchmark calibration does not predict your calibration.

### Running-Dolphins/jev-bench [MEASURED]
https://github.com/Running-Dolphins/jev-bench — 12 business-like classification tasks, 500 examples each, 7 experiments.
- Accuracy **when Jev claims ≥0.9 confidence**, by task: spam 98.6–100%, **duplicate detection 98.9%**, intent routing 95–97%, contract clause classification 86.7%, star rating 81.6%.
- **"The threshold has to be set per task"** — in the 0.8–0.9 confidence band, real accuracy ranged **73% to 99%** across tasks. Calibration is "a property of the task, the dataset and the prompt together," not of the model.
- **Human-in-the-loop, duplicate detection: gating on Jev cut errors from 16.6% to 1.1%, at the cost of 35% of correct answers being sent to human review.** This is the best available number for a memory dedup gate.
- Cardinality hurts: expanding choice options 5 -> 20 -> 59 dropped accuracy **97.7% -> 86.7%**.
- English vs Italian: no measurable difference on short requests.
- **Out-of-scope requests averaged 0.54 confidence but still landed at ≥0.9 about 15% of the time.** Jev does not reliably know when a question is out of its scope.

### AHTOOOXA/jev-cyrillic-audit
https://github.com/AHTOOOXA/jev-cyrillic-audit — independent RU vs EN audit (ECE, reliability diagrams, paired bootstrap) on parallel human-labelled data. [not fetched in depth — exists, methodology sound-looking]

### Other calibration-adjacent work
- https://github.com/themsquared/jev-benchmark — agent tool-call risk classification; "whether the confidence score is worth routing on." Reported that calibration held on that task set (low confidence where wrong, high where right). [ANECDOTAL — not fetched in depth]
- https://github.com/jmanhype/jev-dspy-lab/pull/1 — "Add own-distribution calibration refit"
- https://github.com/jmlusu/light-speed-holdings/issues/340 — "JEV-020: Calibration monitoring dashboard"

### Cross-cutting conclusion (drives both tracks)
**Jev returns a well-behaved probability about *what the text it was handed implies*. It is NOT a probability about the world.** Where the answer is not recoverable from the supplied state — organisational policy, a future event, anything requiring a lookup — it does not abstain; it returns high confidence and is wrong (0.74 confidence / 44.7% accuracy; out-of-scope items hitting ≥0.9 fifteen percent of the time). Combined with the vendor's own statement that "Jev has no knowledge of the world beyond the state you hand it. It cannot look anything up," this settles the Track 2 question before any trading repo is examined.

**Operational rule: fit a per-question isotonic calibrator on your own labelled data (≈20+ examples is enough to start), monitor ECE, and never threshold on raw Jev probabilities for anything that matters.**

---

# TRACK 1 — MEMORY RETRIEVAL / RAG / RERANKING

## 1.1 The headline reranking benchmarks

### anessbelbati/jev-rerank-bench — the most rigorous head-to-head [MEASURED]
https://github.com/anessbelbati/jev-rerank-bench
"Can a decision model beat dedicated rerankers?" 14 datasets, every raw API response, bootstrap ranges on every gap.

nDCG@10 over 8 English datasets, 1,617 questions:

| System | nDCG@10 |
|---|---|
| Jev (4-level rubric) | **0.692** |
| Cohere Rerank 4 Pro | 0.691 |
| ZeroEntropy zerank-2 | 0.682 |
| DeepSeek V4.1 Flash (chat baseline) | 0.682 |

- "Jev rubric minus Cohere Pro is +0.001, with a 95% interval of −0.009 to +0.012" — **statistical dead heat, neither a win nor proven equivalence.**
- Per-dataset splits: Cohere Pro +3.9 on Natural Questions, +5.0 on FiQA; ZeroEntropy +2.1 on TREC-COVID.
- **NevIR (negation pairs): Jev 71% vs Cohere Pro 67%, gap +4.2 (range +1.5 to +6.9).** This is the one place Jev clearly wins — negation/contradiction-sensitive relevance.
- Per-query weighting flips the winner: Cohere 0.756 vs Jev 0.738.
- Cost/latency per query:

| System | Latency | Cost / 1,000 queries |
|---|---|---|
| Jev rubric | 422 ms | $0.45 |
| Cohere Rerank 4 Pro | 844 ms | $2.51 |
| ZeroEntropy zerank-2 | 1.8 s | $0.22 |

**Takeaway: Jev is ~2x faster and ~5.6x cheaper than Cohere Pro at statistically indistinguishable quality. The win is economics + negation handling, not ranking quality.**

### zhuyansen/jev-search-rerank-eval — the big negative result [MEASURED]
https://github.com/zhuyansen/jev-search-rerank-eval
This is the rigorous version of the GoSailGlobal "not a free win" claim. Agent Skills Hub catalog: **33,047 entries, 164 real zh/en/mixed queries, 9,831 graded (query, skill) pairs**, snapshot 2026-09-18.

nDCG@10:

| System | nDCG@10 | vs ash-0.4.0 (95% CI) |
|---|---|---|
| rrf(bge-m3, jev@30) | **0.864** | +0.255 [+0.221, +0.290] |
| jev-score(bge-m3@30) | 0.785 | +0.176 [+0.138, +0.213] |
| bge-m3 alone | 0.774 | +0.165 [+0.133, +0.199] |
| bm25 | 0.633 | +0.024 [−0.013, +0.060] |
| ash-0.4.0 (shipped keyword ranker) | 0.609 | — |

Key findings:
- **Jev reranking bge-m3's top-30 gained only +0.012 nDCG@10 over bge-m3 alone** (0.785 vs 0.774) — i.e. essentially nothing. THIS is "not a free win."
- **RRF fusion of bge-m3 with Jev's rerank gained +0.090 over bge-m3 alone**, and held +0.064 under the conservative label set. Fusion works; replacement does not.
- **Judge-circularity bias, measured explicitly** — Jev was both judge and system under test. Same system (jev-score(bge-m3@30)) scored:
  - +0.012 under merged/adjudicated labels
  - **−0.028 under LLM-only labels (Claude Haiku, Jev excluded)** — i.e. NEGATIVE
  - +0.053 under Jev-only labels
  This is the single most important methodological warning in the whole corpus: **if you evaluate a Jev reranker with Jev-generated relevance labels you will manufacture a win that doesn't exist.**
- The shipped keyword ranker's real problem was **recall (0.497 vs 0.708 for bge-m3), not ranking.** Reranking cannot fix a recall problem — it only reorders what first-stage retrieval already found.
- Cost: ~$0.0002 per Jev rerank call; 328 rerank calls = $0.066; whole eval ≈ $2.6.
- Stated conclusion: "Jev adds value only as a second signal atop semantic candidates, not as the primary ranker." Standalone it is not better than bge-m3 "and should not be sold as such."

### WiktorB2004/llama-index-jev [MEASURED, but weak protocol]
https://github.com/WiktorB2004/llama-index-jev
- BEIR nfcorpus test, 323 queries: **nDCG@5 0.340 -> 0.396, delta +0.056 (95% CI 0.042–0.072)**. Cost ~$0.096 total (~$0.0003/query).
- SciFact, 300 queries, same protocol: **0.629 -> 0.715 (+0.086)**.
- Protocol: MiniLM dense top-10, then JevRerank(mode="score", top_n=5).
- **Author's own caveat, quoted: "This is not a BEIR leaderboard vs Cohere: first-stage is MiniLM, not Pyserini."**
- Reconciliation with zhuyansen: the lift here is large because the first stage (MiniLM top-10) is weak. Against a strong first stage (bge-m3), the lift collapsed to +0.012. **The measured rule: Jev's rerank lift is inversely proportional to first-stage quality.**

### hotchpotch/jev-reranker (Python) [no numbers]
https://github.com/hotchpotch/jev-reranker — also on PyPI as `jev-reranker`
- `rerank()` (order by relevance, default threshold 0.0) and `relevance_rerank()` (filter for useful evidence, default threshold 0.2).
- Modes: listwise (default, shared context), pointwise (independent), pairwise (n(n−1)/2 requests, small sets only).
- Auto-splits long candidate lists at `split_state_budget` 26000 / `split_request_budget` 48000 units.
- **Stated limitation, important: "Partitioned listwise scoring is not guaranteed to produce the same scores as an unsplit request."**
- README publishes **no** nDCG/recall/MRR/latency/cost numbers. References a "Nano-set benchmark" with no results. [UNVERIFIED on quality]

### shinpr/jev-reranker (Rust CLI) [no numbers]
https://github.com/shinpr/jev-reranker
- JSON in / JSON out. Three modes: rerank (score 0–1), filter (drop candidates lacking useful evidence), **compress (judge individual sentences/lines and keep only query-relevant ones)**.
- Default filter/compress threshold 0.5. Batch 30 candidates per request (`--batch-size` 1–30).
- Compress mode is directly interesting for a memory store: sentence-level pruning of a retrieved memory before it enters context.
- Limitations: compress works best on prose; "code, tables, and unusual formatting" want filter mode instead. `--top` does not reduce API work. Retries only on 429/529.
- No published latency/cost/accuracy numbers. Related writeup: https://dev.to/shinpr/what-retrieval-still-hasnt-decided-3haa

### superagents-lab/jev-search
https://github.com/superagents-lab/jev-search — ranks Search1API results using Jev relevance judgments. Listed in awesome-jev; no published metrics. [UNVERIFIED on numbers]

### hev/jev-rerank
https://github.com/hev/jev-rerank — "one call, up to 30 documents, a probability per document." Apache-2.0. Confirms the canonical batching shape: **N documents as N questions against one shared state, one HTTP call.**

## 1.2 Selection, sorting, linking — the keltokhy trio

### keltokhy/jselect — evidence selection within a token budget [MEASURED]
https://github.com/keltokhy/jselect
- Picks passages that fit a token budget, avoiding repetition; explicitly asks Jev to surface evidence that **contradicts** the task/question, not just supports it.
- Batches up to **256 passages per request, up to 8 requests in flight**. Caches scores per (endpoint, model, prompt version, task, exact passage text).
- Benchmarks dated 2026-09-19:
  - Indexing 1,000,000 log records: **13.64 s**
  - Repeated local queries: **0.40 s median**
  - SciFact (semantic shortlist, 256 candidates): **86.7% mean recall vs 78.3% for BM25**; **1.86 s median**; **$0.149 total for 30 queries** (≈$0.005/query — note this is 25x the cost of a 30-doc rerank because it scans 256 candidates).
- This is the closest published analogue to "assemble the right memory context under a budget."

### keltokhy/jsort — pairwise ranking, Bradley-Terry [MEASURED]
https://github.com/keltokhy/jsort
- Sorts text by a plain-English comparative descriptor ("more hawkish about inflation"). Bradley-Terry scale fitted to Jev pairwise comparisons.
- Avoids n(n−1)/2: round 1 is a random ring (connectivity), later rounds are Swiss-system pairings of scale-adjacent items, with scores jittered by remaining uncertainty to direct comparisons at under-determined items. `-k` default 10 comparisons per text.
- Readability benchmark, 300 texts: **Pearson r = 0.824 with 1,500 calls at $0.046** — "94% of the theoretical ceiling (0.88)."
- ~200 ms per comparison; ~60 s per 1,000 short lines; **$0.0000135 per short-line comparison (~320 tokens); ~$0.07 per 1,000 lines.**
- **Limitation that matters for memory: "scores are relative to texts in the same run; they don't transfer across files or dimensions."** So you cannot use jsort scores as a persisted importance field. Reliability below 0.80 means the descriptor doesn't coherently rank the texts.

### keltokhy/jlink — record linking / dedup under natural-language rules [MEASURED]
https://github.com/keltokhy/jlink
**This is the most directly transferable artifact for memory dedup/merge.**
- You write the match rule in English ("a subsidiary counts as its parent company"); Jev returns a probability per candidate pair, ~0.2 s/pair.
- **Two-phase: local blocking proposes candidate pairs on your machine with ZERO API calls (n-gram similarity, initial matching, exact field match), then only the proposed pairs go to Jev.**
- Results on five public ground-truth datasets:

| Dataset | Precision | Recall | F1 | Cost |
|---|---|---|---|---|
| Firm matching | 0.90 | 0.62 | 0.73 | $0.62 |
| Publications | 0.99 | 1.00 | 0.996 | $0.45 |
| Products | 0.90 | 0.93 | 0.92 | $0.22 |

- Throughput: **45,567 firm pairs in 176 s (~259 pairs/s) for $0.62.**
- Limitations, quoted in substance: Jev only judges pairs blocking proposes (unproposed matches stay invisible — **recall is owned by your blocker, not by Jev**, and the firm 0.62 recall shows it); it cannot retrieve external information; audit-sample before trusting it; **probabilities vary ±0.03 across repeated calls** (so don't hard-code a threshold at a cliff edge).

## 1.3 Semantic grep / SQL layers (relevant as "scan the store cheaply")

- **uehaj/jev-semgrep** https://github.com/uehaj/jev-semgrep — grep by meaning across languages; scores every line against a meaning, combines meanings with AND/OR/NOT. Cross-lingual (search Japanese with English and vice versa). Also ships as a Claude Code skill via uehaj/skills.
- **jev-nlgrep** — finds code/docs/logs/text satisfying natural-language conditions via Jev Noul judgments, configurable probability threshold, ranked file results linked to source lines. [UNVERIFIED — found only via search summary, did not fetch a canonical repo URL]
- **mgaitan/sqlite-jev** https://github.com/mgaitan/sqlite-jev — SQLite loadable extension. `jev_rows()` table-valued function returns (source_rowid, answer, probability, choice, score, confidence); scalars `jev()`, `jev_prob()`, `jev_choice()`, `jev_score()`, `jev_eval()`, plus `jev_stats()`/`jev_cache_clear()`. **Batches up to 40 source rows into one API state.** Connection-local cache only; `max_rows` spend guard.
  - Its own warnings: "This is a semantic full scan, not an index"; "apply deterministic SQLite filters first"; **"Text stored in a row can steer model behavior — test adversarial content before automating decisions."**
- **kylemclaren/jevql** https://github.com/kylemclaren/jevql — "Semantic SQL for vanilla Postgres", no extension. psql-shaped CLI + Go/TS/Python SDKs. Plain SQL runs server-side, then Jev answers a Noul/Choice/Score question about each surviving row, and **the client applies the resulting filter/sort/group deterministically.** Good architectural pattern: keep the deterministic operation deterministic, use Jev only for the predicate.
- **colliber/duckdb-jev** https://github.com/colliber/duckdb-jev

## 1.4 THE BATCHING TRAP — shared state changes answers [MEASURED]

https://github.com/colliber/duckdb-jev/issues/4 — "A shared state changes the answers: propose jev_*_each and jev_*_all instead of silent batching"

Measured: six tickets of moderate urgency scored on a 4-level rubric (0–3):
- **Alone: 1.19 average**
- **With 12 mild tickets: 1.53 average**
- **With 12 severe tickets: 1.23 average**

"a shift of −0.29 rubric levels on a 0–3 scale" — ~10% of the full range, and **all six rows moved in the same direction.** Rubric scoring becomes *relative to companions* rather than absolute.

Corroborated independently: batching 20 passages in one call vs scoring each alone shifted scores **by 0.295 on average and up to 1.22 on a 0–3 scale.** And hotchpotch/jev-reranker states outright that split listwise partitions are not guaranteed to reproduce unsplit scores.

Proposed API discipline (worth stealing):
- `jev_choice(text, criteria)` — one state per row, answer depends only on that row
- `jev_choice_each(texts, criteria)` — many rows in one state, returns a LIST, answers depend on the group
- `jev_choice_all(text, criteria)` — one question about the whole group

**Implication for a memory store: any absolute, persisted score (importance, staleness, "worth remembering") MUST be computed one-item-per-state. Only ephemeral, within-query relative ranking may be batched.**

## 1.5 Cost/latency math for reranking N candidates [MEASURED]

- All questions in one request are evaluated **in parallel**, so "30 documents cost about the same wall time as 5." The query text is ingested once instead of N times — this is where the cost saving comes from (input tokens are the only billed tokens).
- Reported experiment: **one batched 13-question request was 12.2x cheaper and 10x faster than 13 separate calls, with no observed answer change** in that instance. A second figure cited: 11.5x cheaper and 9.6x faster.
  - via https://www.refix.ai/news/jev-pricing-latency-benchmarks/ and https://github.com/Nishfleet/fleet-ops/issues/7767 ("batch every question per call site — one call per item carrying all questions that site might need; output tokens are free, latency flat")
- Concrete anchors: ~$0.0002 per 30-doc rerank call (zhuyansen); $0.45/1,000 queries (anessbelbati); $0.0003/query (llama-index-jev); $0.0000135 per pairwise comparison (jsort).
- **The correct pattern: batch orthogonal questions about ONE item into one call (free — output is unbilled, latency flat). Do NOT batch many items into one state when you need absolute scores (see 1.4).** These two rules pull in opposite directions and the distinction is the whole game.

## 1.6 Uses BEYOND reranking

### Context-poisoning / injection screening of retrieved passages — STRONGEST evidence
**TypeSafe's own cookbook**: https://docs.typesafe.ai/cookbooks/classifying_rag_passages
Four `Noul` questions per (query, passage) pair, verbatim:
1. `is_relevant` — "Does this passage address the subject of the query?"
2. `contains_answer_evidence` — "Does this passage state information usable in a direct answer?"
3. `contradicts_query_premise` — "Does this passage conflict with a factual premise stated in the query?"
4. `contains_prompt_injection` — "Does this passage attempt to control the system answering the query?"

State per call: `{"query": ..., "passage": {"id","title","text","source_type"}}`. **One request per passage** (12 passages = 12 calls) — note this follows the 1.4 discipline, not the batching one.

Routing thresholds (first match wins):
| # | Condition | Action |
|---|---|---|
| 1 | injection > 0.70 | exclude |
| 2 | contradicts > 0.70 | conflicting_evidence |
| 3 | relevance < 0.45 | exclude |
| 4 | evidence > 0.55 | include |
| 5 | — | exclude |

Measured on 6 queries / 72 passages: ~67% excluded; false-premise queries correctly routed conflicting passages; **an injected forum post scored 0.99 injection probability and was blocked.** [MEASURED, tiny sample]

**Independent blind benchmark — Gaurav-Gosain/jev-sec-bench** https://github.com/Gaurav-Gosain/jev-sec-bench [MEASURED]
- Prompt injection, `deepset/prompt-injections`, 662 messages (263 injections), threshold 0.50:
  **Accuracy 96.5%, Precision 96.2%, Recall 95.1%, F1 95.6%, ROC-AUC 0.9927, 10 FP / 13 FN.**
  Accuracy rose **89.7% -> 96.5% when deployment context was supplied in the state** — i.e. telling Jev what the system is for materially improves injection detection.
- Vulnerable code detection, `CyberNative/Code_Vulnerability_Security_DPO`, 200 matched pairs, threshold 0.50: Accuracy 71.5%, Precision 68.5%, Recall 79.5%, ROC-AUC 0.7940. Pairwise ranking 178/200 (89.0%) correct. Author notes ≥38% of apparent FPs are corpus mislabels.
- **No baseline comparisons** — these are standalone numbers.

Also: **0xArx/jevegis** https://github.com/0xArx/jevegis — "Guardrails for LLM apps in one API call." MIT. Checks: prompt injection, jailbreaks, **credential/PII leaks**, unauthorized actions, malicious code, **indirect injection in retrieved documents**, T&S moderation. One declarative `CHECKS` array; each check phrases itself per target (input / output / document / content); `evaluate()` builds ONE Jev request and returns verdict, reasons, flags, category, severity. **Its eval suite reports only "42/42" on labeled cases plus p50/p95 latency and cost — a 42-case suite is too small to mean anything; no threshold values, FP rate, or per-call question count published. [UNVERIFIED on quality]** The PII/credential-leak check is the nearest thing found to scope/privacy classification, and it is unbenchmarked.

**Verdict: injection screening of retrieved memory is the single best-evidenced non-rerank use of Jev. 0.9927 AUC on a public set, one sub-second call, ~$0.0002.**

### Memory write gates ("is this worth remembering?")
- **No dedicated, benchmarked Jev write-gate project found.** [UNVERIFIED]
- Nearest: the context-compaction family in awesome-jev, which is a *read-side* analogue of a write gate:
  - **https://github.com/compozy/yoshi — the ONLY one with published numbers, and they are discouraging. [MEASURED]**
    - Gate design (reusable, and well-formulated): for each candidate span it asks two **loss-framed** Nouls — *"would a required fact be lost?"* and *"would a binding user constraint be lost?"* — and removes the span only if **both probabilities are ≤ 0.2**. Eligible: completed tool arguments (Bash/Write/Edit), tool results, teammate reports. **Protected and never judged: human instructions, assistant notes, errors, opaque blocks.** Judgment only activates above ~50,000 estimated input tokens. Tool results additionally follow a "calibrated Atomic relevance/coverage/conflict rule."
    - v12 study, Fable diagnostic: **34.39% accumulated-input reduction, peak context −41.46% — but latency rose 39.77s -> 210.54s, with 3 Jev failures during judging.**
    - v12 study, Sonnet mixed session: **−0.03% accumulated input reduction (i.e. nothing) despite 142 Jev calls and 3 failures.**
    - The repo itself states "Yoshi latency increased in both studies" and that these are limited trials, not generalizable claims.
    - **Lesson for a memory write gate: the per-call latency is small but the per-session call COUNT is what kills you. 142 calls at ~200ms serialised is ~30s. Gate on a cheap deterministic precondition first (yoshi uses a 50K-token floor), and run the Jev pass offline in the nightly Librarian rather than on the write path.**
  - https://github.com/joelhooks/pi-fast-jev-compaction — preserves verbatim text while pruning stale tool history
  - https://github.com/tamaratran/fast-jev-compaction — replaces summarization with scored tool-call filtering
  - https://github.com/leonaaardob/fast-dev-compaction — Codex port
  - https://github.com/tamaratran/jev-pruner — trims Bash output before Claude sees it
- **Notable pushback on the whole compaction framing:** https://x.com/theo/status/2100762304862384257 — "This is a terrible compaction strategy... compaction is reconstruction, not filtering; [it] misunderstands context management." (could not fetch — x.com returns HTTP 402; summary via awesome-jev's own description) [ANECDOTAL]
  - This critique transfers directly to memory writes: a write gate that only *filters* raw turns produces a worse memory than one that *rewrites* them into a durable claim. Jev cannot rewrite — it generates no text. So Jev can only ever be the gate, never the writer.

### Staleness / contradiction scoring of memories
- https://github.com/Nishfleet/fleet-ops/issues/7425 — "jev: memory prune — staleness/contradiction score per memory file; keep the index under the 24 KB silent-truncation limit." Three booleans per memory file: is-stale, contradicts-another-file, keep-in-index. **This is a requirements spec only — no thresholds, no batching design, no measured results.** [UNVERIFIED as an outcome]
- The `contradicts_query_premise` Noul from the RAG cookbook is the reusable primitive.
- NevIR result from anessbelbati (Jev 71% vs Cohere 67% on negation pairs) is the only measured evidence that Jev is comparatively *good* at contradiction-shaped judgments.

### Scope / privacy classification — the best-documented REJECTION
https://github.com/ojungo69/oboete/issues/303 — "Evaluated TypeSafe AI's Jev for oboete: not adopted, and the rules that decided it" [MEASURED — as a decision, not as numbers]

oboete is a memory capture + lexical search system. They surveyed **ten candidate Jev integration sites** across product runtime, repo tooling, and measurement backlog, each getting **two independent adversarial reviews (one feasibility, one conflict-with-stated-commitments)**. **All ten were rejected by both reviewers.** Four rules decided it:
1. **Egress**: remote calls cannot include `local_only` or `private` rows (per their CONSTITUTION.md).
2. **Zero-credential floor**: capture and search must work with no credentials; fallback must be "deterministic, no network."
3. **Determinism**: secret classification needs testable exact oracles — "a probabilistic, unversioned scorer cannot hold those positions."
4. **Labels already determined**: the required ground truth is mechanical, so calibration buys nothing.

**This is the sharpest available "when NOT to use Jev for memory" document, and rule 3 + rule 1 apply squarely to a private personal memory store.**

### Retrieval-need gating ("does this query need memory at all?")
- No Jev-specific project found. [UNVERIFIED]
- The concept is named in the 2026 memory literature as "retrieval-or-not gating" (see https://arxiv.org/pdf/2603.07670 "Memory for Autonomous LLM Agents"). A single `noul` per turn at ~$0.0002 and ~100 ms is an obvious fit, but nobody has published numbers on it.

## 1.7 Moritz Kremb "Full Jev Tutorial" — what the memory part does
- Tutorial: https://x.com/moritzkremb/status/2100715237267660873 (x.com returns HTTP 402 to WebFetch — could not read directly)
- Structure recovered from search + madewithjev: **Jev explained -> API setup -> Demo 1 voice-controlled browser -> Demo 2 AI memory (at ~17:27) -> Demo 3 YouTube predictor.**
- **The memory demo's actual contents could not be verified. [UNVERIFIED]** Do not cite specifics.
- What IS verified is the adjacent Demo 1 pattern, which matters for memory: https://github.com/moritzkremb/jev-voice-browser — Jev decides intent + target in ~300 ms per spoken word, Playwright acts. "The speed comes from streaming partial transcripts and firing Jev on every update, with stale requests cancelled." MIT-licensed. The **speculative-fire-and-cancel** pattern (see also https://github.com/jem-computer/bb-live/issues/7 "Speculative pre-resolution on partial transcripts (streaming Jev path)") is the transferable idea: because Jev is cheap and fast, you can speculatively issue retrieval-gating decisions and cancel the losers.
- Kremb's roundup of Jev projects: https://x.com/moritzkremb/status/2100895894287839255

## 1.8 2026 memory papers (all verified to exist, abstracts fetched)

| Paper | arXiv | Verified headline |
|---|---|---|
| STALE: Can LLM Agents Know When Their Memories Are No Longer Valid? | 2605.06527 | 400 conflict scenarios -> 1,200 queries, contexts to 150K tokens. **Best model 55.2% accuracy.** Names "Implicit Conflict": a later observation invalidates an earlier memory without explicit negation. Three axes: State Resolution, Premise Resistance, Implicit Policy Adaptation. Proposes CUPMem. Authors: Chao, Bai, Sheng, Li, Sun. |
| Remember the Decision, Not the Description: A Rate-Distortion Framework for Agent Memory | 2605.10870 | Memory quality should be measured by **decision degradation under compression**, not descriptive fidelity. "Memory is valuable not because it faithfully describes the past, but because it preserves the distinctions between histories that must remain separated." Proposes DeMem, which only refines its partition when data show merging would create conflicting decisions. Consistent gains at fixed memory budget. |
| Memory-Induced Tool-Drift in LLM Agents | 2605.24941 | Personality biases in memory (**cost-consciousness**, impatience, risk tolerance) silently distort tool-call parameters where irrelevant. 105 scenarios, 5 bias dimensions, 7 domains, 7 frontier models. **Deflection up to +3.6 on a 1–5 scale.** Scanned 6,062 tools across 288 MCP servers, **608 flagged with susceptible parameters.** Persisted across three production memory architectures. Authors: Dabas, Jeong, Jin, Jia. |
| MemFail | 2605.26667 | Diagnostic benchmark isolating summarization / storage / retrieval failures. Five datasets, four tasks, four SOTA memory systems. Abstract gives no headline number. Authors: Garg, Kolhe, Song, Zhao (Berkeley). |
| DeferMem | 2605.22411 | **NOT about deferring writes** — it defers *evidence distillation to query time*. Broad candidate retrieval, then query-conditioned distillation via an RL method (DistillPO). Best QA accuracy + fastest runtime on LoCoMo and LongMemEval-S, zero commercial-API token cost for memory ops. |

Supporting 2026 work found incidentally:
- https://arxiv.org/pdf/2606.26511 "Temporal Validity in Retrieval Memory: Eliminating Stale-Fact Errors" — argues staleness detection must be **structural**: if an incoming fact and a stored fact share a (subject, relation) key but assert different objects, the newer supersedes, *independent of embedding similarity*. Also claims **"gate-judge contradiction calls are unreliable; in abstention regimes they leak stale facts 25–60% of the time."** [MEASURED — that paper's claim; directly cautions against a pure-Jev staleness gate]
- https://arxiv.org/pdf/2606.06240 TOKI: bitemporal operator algebra for contradiction resolution
- https://arxiv.org/pdf/2606.30306 Always-On Agents: survey of persistent memory, state, governance
- https://arxiv.org/pdf/2606.15903 Control-Plane Placement Shapes Forgetting (13 system configs)
- https://arxiv.org/pdf/2605.29630 Entity-Collision: stratified protocol for attributing retrieval lift in agent memory

## 1.9 Named critiques of Jev (all x.com — HTTP 402, summaries via awesome-jev) [ANECDOTAL]
- https://x.com/bojie_li/status/2100688989006475580 — "Jev is just a classifier": latency follows mechanically from parallel token decoding; nothing novel.
- https://x.com/jiayuan_jy/status/2100876273061102006 — Chinese scepticism after one day: fast classifier LLMs already do this; **world knowledge on complex scenarios unclear.**
- https://x.com/theo/status/2100762304862384257 — compaction pushback (see 1.6).
- https://x.com/GoSailGlobal/status/2100877682972258619 — "Jev reranking is not a free win", 33,047 entries. **Could not fetch the post itself (402); the claim is corroborated in detail by zhuyansen/jev-search-rerank-eval above, which I treat as the load-bearing source.**
- The r/AI_Agents thread "Has anyone tried Jev as a relevance filter for RAG?" — **could not be located. [UNVERIFIED]** Do not cite it.

## 1.10 Other evaluation artifacts spotted (not fetched in depth)
- https://github.com/kenn-io/msgvault/issues/877 — "Add optional Jev reranking for search results"
- https://github.com/franklinmdev/docmatch/issues/91 — "Phase 3: TypeSafe Jev as a reranker ablation row"
- https://github.com/SaplingLearn/Sapling/issues/641 — "Move upload decisions and reranking onto the decision seam"
- https://github.com/carlaiau/jev-reranking/issues/72 — "Experiment: JEV duo with Noul on the top 20 documents"
- https://github.com/sibukixxx/rag-poc/issues/40 — Japanese: semantic context filter / reranker for RAG passage relevance + context compaction
- https://github.com/Nishfleet/fleet-ops/issues/7779 — prompt-injection screen at intake for public-repo issue/PR bodies
- https://github.com/Nishfleet/0509/issues/3621 — injection screen on scraped pages/ad copy before llama generation
- https://github.com/kentcdodds/kody/pull/2368 — "Batch Jev Score calls to fix incomplete-score-answers"
- https://x.com/juanmacias/article/2100463318209048850 — "Testing TypeSafe's Jev - Legal Corpus" (402, not read)

---

## 1.11 TRACK 1 SYNTHESIS — what to do in a hybrid vector+keyword personal memory store

**Where Jev helps (evidence-backed):**
1. **Injection / context-poisoning screen on every retrieved memory passage before it enters context.** 0.9927 AUC, 96.5% accuracy on a public set; vendor cookbook gives exact question wordings and thresholds; supply deployment context in the state (+6.8pp). Cheapest, highest-confidence win. One call per passage.
2. **RRF fusion, never replacement.** +0.090 nDCG@10 measured when fused with bge-m3; +0.012 when used to rerank the same candidates. Do not let Jev be the primary ranker.
3. **Dedup/merge with local blocking + Jev adjudication** (jlink pattern): 0.90–0.99 precision, 259 pairs/s, cents per run. Your blocker owns recall.
4. **Contradiction detection as a signal** — Jev's one clear quality edge is negation (NevIR +4.2 over Cohere Pro). Pair it with a structural (subject, relation) supersession rule; do not rely on the judgment alone (25–60% stale-fact leak reported for pure gate-judge approaches).
5. **Budgeted evidence assembly** (jselect pattern): 86.7% recall vs 78.3% BM25 on SciFact at ~$0.005/query.
6. **Retrieval-need gating** — no published evidence, but ~$0.0002 / ~100ms per turn makes it nearly free to A/B. Treat as an experiment, not a known win.

**Where it does not help / where it hurts:**
- If your first stage already has good recall, reranking gains ~nothing (+0.012 measured). **Diagnose recall first** — the zhuyansen case showed the real deficit was recall 0.497 vs 0.708, which no reranker can fix.
- **Never evaluate a Jev reranker with Jev-generated labels.** Same system: +0.053 (Jev labels) vs −0.028 (independent labels). The circularity manufactures a phantom win.
- **Never batch items into a shared state for an absolute persisted score.** Measured drift −0.29 levels on a 0–3 rubric, all items moving together; up to 1.22 in another test. Batch orthogonal *questions* about one item; never batch *items* when the score must be absolute.
- **Determinism/egress:** oboete rejected all ten integration sites because a probabilistic unversioned scorer cannot back a testable secret/privacy oracle, and private rows cannot leave the machine. For a personal memory store with `private`/`local_only` scoping, **scope and privacy classification is exactly the wrong job for a remote probabilistic model.**
- Jev cannot rewrite a memory. It can gate and score; the Librarian's consolidation/summarisation work still needs a generative model. Theo's critique — filtering is not reconstruction — applies to memory writes.
- Probabilities wobble ±0.03 run-to-run; never place a threshold on a decision cliff.
- **Raw probabilities are not ready to threshold on.** Raw ECE 0.064–0.160 on sentiment, 0.325 on an out-of-distribution `score` question. `score` runs overconfident, `noul` runs underconfident — opposite directions, so one global temperature cannot fix both. Fit a **per-question isotonic calibrator** (ECE -> 0.006–0.018, works from ~20 labelled examples). For a memory store this means: label ~50 of your own memories per gate question once, fit isotonic, store the calibrator, re-check ECE monthly.
- **Cardinality hurts:** 5 -> 20 -> 59 choice options dropped accuracy 97.7% -> 86.7%. Keep scope/tag classification to a handful of options, or decompose into several `noul`s.
- **Jev does not know when it is out of scope:** out-of-scope inputs still hit ≥0.9 confidence ~15% of the time. A memory write gate will confidently admit junk it has no basis to judge.
- **Best-evidenced gate economics, from jev-bench duplicate detection:** gating cut errors 16.6% -> 1.1% but pushed 35% of *correct* answers into human review. For an autonomous nightly Librarian with no human in the loop, that 35% has to go somewhere — e.g. "defer to next night" rather than "merge" or "drop."

---

# TRACK 2 — TRADING / PREDICTION MARKETS / FORECASTING

## 2.1 THE DECISIVE EVIDENCE: Jev is not a forecaster

Three independent, properly-designed studies all reach the same answer. This is unusually clean for a two-week-old model.

### egrm07/jev_bitcoin_backtest — BTC/USD, no edge in any configuration [MEASURED]
https://github.com/egrm07/jev_bitcoin_backtest
- Tested **ten distinct state representations** (semantic, raw OHLCV, indicators, narrative, etc.) against the same questions and simulator. Model pinned at **Jev 1.13.0**.
- A representation had to clear **five gates simultaneously**: significant dev-set performance above chance; survive Benjamini–Hochberg multiple-testing correction; replicate on holdout; Sharpe CI excluding zero; beat buy-and-hold.
- **Result: "No tested representation of Jev 1.13.0 demonstrated a tradable edge."** No configuration cleared all five gates.
- **Untouched holdout: real-arm AUC 0.471 to 0.503** — chance. **Every Brier skill score was negative** (worse than baseline). **Every policy lost money after a 14 bps round trip.**
- Best arm: `raw_ohlcv` + linear policy at **−15.73% on holdout**, vs **+25.55% for simulated buy-and-hold** over the same window.
- This is the single most rigorous negative result in the corpus: pre-specified gates, multiple-testing correction, untouched holdout, realistic fees.

### waderylan/jev-mlb — beaten by the market, and by the base rate [MEASURED]
https://github.com/waderylan/jev-mlb
- MLB win probability, **2,118 games (2025 season) + 300 games (2026)**, vs Kalshi pre-game implied probability and a base-rate baseline.
- **Brier (lower better): Jev v2 0.2631 | Kalshi 0.2426 | base rate 0.2487.**
  **Jev is worse than the market (+0.0205, 95% CI 0.0146–0.0265, statistically significant) AND worse than simply predicting the base rate.**
- Jev's probabilities were far more extreme (range 0.06–0.90) than Kalshi's (0.23–0.79), correlation ~0.78–0.94. It picked favourites about as often as the market but with **overconfidence** that destroyed accuracy — exactly the OOD pattern from §0.1.
- **Ceiling-check failure: in the v3 experiment Jev was GIVEN the Kalshi price as input and still produced a less accurate output than the input price itself.** It actively degraded a good forecast.
- Cost/latency were excellent and irrelevant: **$0.12 total, ~165 ms**.
- Author's conclusion, quoted: **"Structured sports forecasting against a liquid market appears to be a poor fit for the model, which is designed for fast judgments over text."**

### cristiancolon/jev-hft — the sharpest test: Jev's signal is FULLY explained by order-book imbalance [MEASURED]
https://github.com/cristiancolon/jev-hft (`docs/backtest-and-analysis.md`)
**Methodologically the best idea in the whole corpus.** The author orthogonalised Jev's signal against four trivial rules (order-book imbalance etc.) and measured the *residual*, reasoning: *"Jev is shown the same numbers the rules are built from, so a good score on its own doesn't tell you whether Jev understood something or just repeated book imbalance back to us."*
- **Result, verbatim: "On our 31-minute recording Jev scored 0.10 to 0.17 on its own at 2 and 10 seconds, and between −0.14 and 0 on this line."** i.e. **zero incremental information after the order-book features it was handed.**
- README: *"On market data, Jev doesn't beat a one-line rule: ... plain order-book imbalance did about twice as well for free, and Jev added nothing beyond it."*
- Also demolishes naive significance: *"In a 2.5-minute test recording, 178 decisions at the 60-second horizon are 2 separate pieces of evidence."* Overlapping horizons collapse effective n. **Anyone quoting "n=178" from a short recording is quoting n≈2.**
- The project publicly **pivoted away from market data to news triage** after this: *"over a few seconds Bitcoin barely moves, so even perfect predictions would earn less than trading fees. That's why the news part exists."*

### sosopop/jev_stock — HK stock backtest: 45% hit rate, worse than a coin flip [MEASURED, but badly flawed]
https://github.com/sosopop/jev_stock
- Universe: **four** HK stocks (Xiaomi, MiniMax, Pop Mart, MIXUE Group), **30 consecutive eligible T+1 pairs each = 120 observations total.**
- **Hit rate: 54/120 = 45.0%.** Per-stock range **30.0% to 56.7%.** On a binary up/down call, 45% is below chance.
- Methodology gaps, several admitted by the repo itself:
  - **Look-ahead bias admitted: "Retrospective model calls can also benefit from information that was available to the model after the historical date."** Fatal for a backtest of a model whose training data covers the test period.
  - "Financial context is excluded from historical states" for timing reasons — so it is a price-only judgment.
  - **No fees, no slippage.** In-sample historical replay, not out-of-sample.
  - 120 observations cannot distinguish anything; Wilson CIs computed but not quoted.
- **The repo's own calibration warning, quoted: "confidence: 0.52 is not a calibrated 52% chance of being correct."** The author of the most-cited Jev stock project explicitly disclaims the calibration interpretation.
- Multiclass Brier reported in the harness but **no values published. [UNVERIFIED]**

### johnhughes3/LegalForecastBench — no Jev results exist yet [UNVERIFIED]
https://github.com/johnhughes3/LegalForecastBench
- Task: forecast federal **motion-to-dismiss** rulings from the judge's written record; predict P(each claim-defendant pair dismissed in full). Primary metric **micro-Brier over prediction units with clustered CIs**.
- Cycle 1 is a **91-case** evaluation. **Only GPT-5.6 Luna and Meta Muse Spark 1.3 have completed Cycle 1.**
- Status quoted: results "recently received... compiling and validating the complete report... Preliminary model rows are withheld here while that report is assembled."
- **There are NO published Jev calibration or Brier numbers from this benchmark. Do not cite any.** Note also this is arguably not forecasting at all — predicting a ruling *from the judge's written record* is a text-classification task, which is precisely Jev's strength; worth re-checking when Cycle 1 publishes.

### valentynkit/jev-plays-pokemon-red — [UNVERIFIED]
Brier scoring of faint predictions. Referenced in the brief; **I did not independently verify this repo or its numbers.** Treat as unconfirmed.

### michellepellon/nfl-data-stack#62 — a pre-registered live test (no results yet)
https://github.com/michellepellon/nfl-data-stack/issues/62 — "Jev as a forecaster: pre-registered live test of a calibrated-decision model against the closing market"
- Well-designed: three arms — `jev_blind` (teams/date/venue only), `jev_news` (+ injuries, QB), `jev_market` (+ devigged market odds) — against **closing market probabilities** and an **elo_plain** baseline. Model pinned at jev-1.13.0, never "latest". Frozen records of exact requests/raw responses/version/timestamps. Determinism tested by duplicate requests in week one.
- Metrics: log skill score with CI, Brier, paired gap vs market and vs elo, recalibration slope and intercept.
- **Honest power analysis: ~270 games/season can only detect gross miscalibration or a forecaster no better than base rate; ~400 games needed to separate close forecasters.** Worth watching, but **no results reported — this is a pre-registration. [UNVERIFIED as an outcome]**
- The stated *hypotheses* are themselves informative: the author expects `jev_market` to be "indistinguishable from or potentially degrade market performance" — which is exactly what jev-mlb's v3 measured.

**Synthesis: against any liquid, efficiently-priced market, Jev's probability is not alpha. It loses to the closing price, and in the MLB case to the base rate. Combined with §0.1 (0.74 confidence at 44.7% accuracy where the answer isn't in the text) and the vendor's own "it cannot look anything up," the mechanism is clear: Jev outputs a calibrated-ish classification confidence about the state you handed it, not a forecast of the world.**

## 2.2 What people actually BUILD with Jev in trading

Survey (2026-09-20) by drillan: https://gist.github.com/drillan/6916b16e8ea31a8ec36c8f59d6483150

| Project | URL | What Jev does | Status |
|---|---|---|---|
| jev-trader | https://github.com/jarrodwatts/jev-trader | Monad/Kuru MON-USDC market maker; Jev decides buy/sell every ~300 ms block | 1.3k stars; template others copied |
| jev-trade | https://github.com/aowang-ai/jev-trade | Hyperliquid, five isolated sleeves (BTC/ETH/SOL/DOGE/BNB) | Real fills, live desk |
| QuantDinger | https://github.com/OpenByteInc/QuantDinger | Crypto/stocks/forex OS; Jev **gates** trade decisions | 11.7k stars; largest integration |
| jev-trader (Python) | https://github.com/buberlo/jev-trader | **Six atomic judgments** (regime, direction, toxic flow, …) feeding a policy engine | Noted as most rigorous architecture |
| jev-market-reflex | https://github.com/zzsong1023/jev-market-reflex | Kraken BTC/ETH/SOL -> simulated portfolio | Paper only |
| jev-trading-bot | https://github.com/rnjsxodyd90/jev-trading-bot | $1,000 virtual spot on Kuru MON/USDC | Paper only, risk-limited |
| jev_stock | https://github.com/sosopop/jev_stock | HK stock direction | Calibration addressed; **no P&L in the survey** |
| jev-signals-lab | https://github.com/Gamma-Software/jev-signals-lab | 12 independent Jev questions per market snapshot | Research POC |
| jev-typesafe-real-financial-use-cases | https://github.com/IslamBaraka90/jev-typesafe-real-financial-use-cases | 50 financial demos + backtest lab with replay | Demos |

**The survey's own bottom line, quoted: "No measured P&L reported in any projects reviewed."** And: **"No prediction-market projects (Polymarket/Kalshi) identified as of survey date."** Also: **"Jev judges, code executes is universal"** — every project delegates only typed judgments to Jev and keeps deterministic control of execution.

Adjacent non-trading finance results that ARE measured, and show where Jev genuinely wins (classification, not prediction):
- https://github.com/simonmesmith/jev-banking77-experiment — BANKING77 intent classification: **Jev 92.40% vs BERT 93.66%** (slightly worse, vastly cheaper/faster, zero training).
- https://github.com/kyotofin/tax-doc-classifier — IRS form classification across **261 forms: 100% accuracy at $0.001/page, ~34x cheaper** than the incumbent.
**Note the pattern: near-perfect on "which of these known categories is this document", mediocre-to-negative on "what will happen next".**

## 2.3 Prediction markets — what exists

- **markusbug/jevymarket** https://github.com/markusbug/jevymarket — "Polymarket trading bot driven by Jev (TypeSafe AI) via OpenRouter". **This is the one real Jev-Polymarket project found.** [MEASURED as to design, UNVERIFIED as to profitability]
  - State fed to Jev: market question, description, resolution date, **days until resolution (pre-computed — correctly working around Jev's date-comparison incapability)**, market-implied probability, and a **research brief with dated evidence**.
  - **Three typed questions, ~100 ms:**
    - `resolves_yes` -> probability of YES
    - `answerable` -> whether sufficient current information exists to form an estimate
    - `clarity` -> objectivity of the resolution criteria, 0–4 score
  - Pipeline: a cheap generative model with web search (default DeepSeek) produces a dated, sourced evidence brief **excluding odds sites**; cached up to 6 h; then handed to Jev. **This is the correct architecture — the LLM supplies world knowledge Jev lacks; Jev only judges the supplied text.**
  - **Trades LIVE by default** (GTC limit orders at best ask); `--dry-run` available.
  - Only published measurement: one live market in Sept 2026 — **"without the brief Jev reported `answerable=0.27`; with it, `0.78`."** That is a sanity check on the brief pipeline, not evidence of edge.
  - **No P&L, Brier or hit rate disclosed. Standard "educational purposes, not financial advice, no liability" disclaimer.**
  - The `clarity` (resolution-criteria ambiguity) and `answerable` questions are exactly the two prediction-market-specific uses worth stealing — and notably neither of them is a forecast.
- General search for "Jev Polymarket / Jev Kalshi / System One prediction market" otherwise returns **nothing** beyond jevymarket and the jev-mlb Kalshi *comparison*. Market-question classification, news->market relevance matching, duplicate/correlated-market detection and sentiment with Jev: **no published projects found. [UNVERIFIED]**
  - **jevymarket's own "Known limitations" — the best cautionary material in the track, verbatim:**
    - *"**Longshot bias.** Jev tends to be **less confident than the market at the extremes**, which shows up as 'edge' on 5-cent contracts. The trade band (`MIN_TRADE_PRICE`) is the guard."* -> **a naive `p_jev − ask ≥ 0.08` filter systematically selects the tails where Jev is miscalibrated. The "edge" IS the miscalibration.** (Note the sign is not even stable: jev-mlb measured Jev *more* extreme than the market, 0.06–0.90 vs 0.23–0.79.)
    - *"**Edge ≠ profit.** Limit orders at the ask are taker orders; adverse selection and stale briefs eat into an 8% threshold."*
    - *"**No exits.** Positions are held to resolution. No stop-loss, no re-pricing on new evidence."*
    - *"**Jev interprets literally and degrades with irrelevant context** — keep the state small."*
  - Gates used: `MIN_EDGE 0.08`, `MIN_ANSWERABLE 0.70`, `MIN_CLARITY 2`, trade band 0.10–0.90, `MAX_USD_PER_TRADE $5`, `MAX_OPEN_EXPOSURE_USD $50`, `MAX_TRADES_PER_RUN 3`, `KELLY_FRACTION 0.25`.
  - **Cost breakdown that reframes everything: Jev ≈ $0.00006/decision; the research brief ≈ $0.010–0.020; a 20-market pass ≈ $0.20–0.40. The researcher LLM costs ~200x more than Jev.** Jev's cheapness is a rounding error on total system cost once you need actual world knowledge.
  - Repo created 2026-09-20, **0 stars, hours old at research time.**
- **marianimatteo-lexroom/poly-jev ("Reflex")** https://github.com/marianimatteo-lexroom/poly-jev — created 2026-09-19. One `noul` per market (P(YES)) **plus a `manipulation_risk` question** that "screens out thin, ambiguous, or already-decided-but-mispriced markets." Fractional Kelly 0.25, `DRY_RUN=true` default, paper ledger.
  - **Its own damning and correct self-assessment, verbatim: "Reflex's edge is only as good as Jev's calibration on markets it's never specifically been evaluated against, and `edge_engine.build_state()` today only feeds it the market's own metadata — no live news feed."** By construction it asks Jev to beat the market using only the market's own metadata. That cannot work. **P&L: none; the live order path is "verified for construction, not for an actual fill."**
  - Useful non-Jev finding: Gamma's `restricted` field is `true` on **100% of the top 100 markets by volume** (useless as a quality filter); `py-clob-client` is **deprecated** — Polymarket moved to `polymarket-client` and **pUSD** collateral after an April 2026 exchange upgrade. Author's lesson: *"verifying against a real API doesn't catch it having been superseded."*
- **Team-Win-and-Make-Monney/polymarket-arb-scanner PR #147 — MERGED** https://github.com/Team-Win-and-Make-Monney/polymarket-arb-scanner — "feat: TypeSafe Jev-1.13 System One Decision Engine & Calibration Suite", merged 2026-09-18. **The richest source for what prediction-market jobs people actually give Jev.** Repo pre-dates Jev (created 2026-02-06).
  1. **Semantic news sniping** (`scans/news_snipe.py`) — replaced regex keywords with `RESOLVES_YES` / `RESOLVES_NO` / `NEUTRAL_OR_SPECULATIVE` **with graceful regex fallback**. (Honest framing: Jev is a better regex, and you keep the regex.)
  2. **Resolution Equivalence Gate** (`market_discovery.py`, `matcher.py`) — `JevJudge` scores **contract rule divergences on a 0–2 scale**; `verify_cross_platform_equivalence_jev()` is a **pre-execution safety gate**. This is duplicate/correlated-market detection + resolution-criteria ambiguity, used as a **gate, not a signal**.
  3. **Adverse-selection / toxic-flow quoting shield** — auto-pauses quoting during sweeps. Kill-switch role.
  4. **Calibration harness**: `jev_calibration.py` computes **Brier, Brier Skill Score vs market pricing, ECE, and 10-bin reliability diagrams**; auto-syncs Gamma resolutions; `GET /api/jev/calibration`.
  - Gates: `JEV_CONFIDENCE_THRESHOLD 0.70`, `JEV_MIN_EDGE 0.04`. `fees.py` models **Polymarket's 7% crypto fee schedule** plus Polygon gas.
  - **RESULTS: NONE. Zero Brier, zero BSS, zero ECE values reported** — verification is "54 automated tests pass" + "paper-trading dry-run". **They shipped the measuring instrument into a live order-routing path before taking a single reading** (the PR's own bot flagged it "High Risk — errors could affect real capital").
- **Manifold / Metaculus: NOTHING EXISTS.** [UNVERIFIED/absence] Explicit searches for "Jev Manifold" and "Jev Metaculus" return nothing; the Metaculus/Manifold bots that surface use Claude/LLMs, not Jev.
- **`opaielsheikh/jev-polymarket-arb` DOES NOT EXIST** — appears in search results with a plausible description but returns **HTTP 404** on the repo and both raw README branches. **Do not cite it.**
- For contrast, the mature non-Jev Polymarket tooling is statistical, not LLM/Jev: https://github.com/BallesJr/polymarket-edge-model (Random Forest calibration model + backtesting + paper trading), https://github.com/ImMike/polymarket-arbitrage (10,000+ markets, cross-venue), https://github.com/suislanchez/polymarket-kalshi-weather-bot (31-member GFS ensemble + Kelly sizing; claims "highest profits $1.8k" — [ANECDOTAL], self-reported).

## 2.4 Latency vs cost — the value is COST, decisively

### Nobody actually gets 70–500 ms end-to-end
**cristiancolon/jev-hft `docs/latency.md`** [MEASURED, 2026-09-19, US West Coast, Coinbase BTC] — the most thorough published teardown:

| Step | Typical |
|---|---|
| Coinbase holds the update before sending | 47 ms (book), 85 ms (trades) |
| Coinbase -> our machine | 32 ms |
| Processing one market update | 0.02–0.05 ms |
| Building the state text | 0.25–0.6 ms |
| **Jev round trip via Vercel AI Gateway** | **~260 ms** (slow ~320 ms) |
| **Total exchange-event -> decision** | **~0.37 s** (slow 0.49 s) |

Decomposition: 95 ms gateway reachability floor, 160 ms gateway's own wait on TypeSafe, 40 ms direct-to-TypeSafe network floor, ~100 ms new-connection penalty. Author: **"Roughly 60 ms of that is the Cleveland-to-West-Coast trip, which leaves about 100 ms for Jev itself. So more than half of the round trip is the route, not the model."**
**Keep-alive fix [MEASURED]: holding the connection open (25 s idle keep, GET ping every 20 s — must be GET; Node closes the socket after every HEAD) took decisions from 460–550 ms down to 255–300 ms.**

Corroborating: jev-mlb **~165 ms** provider latency over 2,118 calls; jev-plays-pokemon-red **median 621 ms / mean 759 ms (n=6)** through a gateway shim. **Go direct to `api.typesafe.ai`, not through a gateway, and reuse connections.**

### The arithmetic that kills speed as a strategy
jev-hft, verbatim: *"**This isn't high-frequency trading.** Even the fastest route gives decisions about 0.2 seconds after something happens."* And: *"**Trading costs matter more than speed at short horizons.** In our recorded data, Bitcoin's average price move was **0.05 to 0.1 bp over 1 second, 0.5 to 0.9 bp over 10 seconds, and 1 to 4 bp over 60 seconds. A round trip of buying and selling costs around 10 bp.** No amount of speed fixes that gap."*

**If that kills sub-minute BTC on a tight CEX book, it annihilates Polymarket** — wide spreads, thin books, resolution days-to-months away, and PR #147 models a **7% Polymarket crypto fee schedule** plus Polygon gas. A 250 ms vs 2,500 ms decision is indistinguishable against a market resolving next month.

The one genuine exception is **jarrodwatts/jev-trader**: a **300 ms Monad block is a hard deadline**, so latency is a *feasibility constraint*, not an alpha source — and its published 100 ms p50 was measured **with the mock model, not Jev** [UNVERIFIED for real Jev]. Contrast aowang-ai/jev-trade running the same idea on a **2,000 ms tick with a 4,000 ms Jev deadline — 20x slack.** Nobody on a CLOB needs the speed.
For Polymarket specifically, no venue CLOB latency figures were found; a strategy guide (https://pnlpro.fit/blog/polymarket-trading-bot-guide.html) asserts sub-100 ms matters but publishes no measured Polymarket numbers and reads as promotional. [ANECDOTAL]
**And the 100 ms is irrelevant in the only real Jev-Polymarket design anyway:** jevymarket's Jev call sits behind a web-search research brief **cached for 6 hours**. The pipeline's information latency is hours.

### THE COST MATH — exact, published, reproducible
jev-hft's token model [MEASURED]:
> - about **300 tokens of fixed overhead** per call,
> - plus the text you send, **counted once however many questions you ask about it**,
> - plus **35 to 95 tokens per question**.
> *"That's why one call about three assets is much cheaper than three calls, and why adding a few lines of context to a news item is nearly free."*

| What | Tokens | Cost |
|---|---|---|
| Market-data decision (3 questions + summary) | ~820–855 | ~$0.000035 |
| News decision, one asset | ~580–650 | ~$0.000025 |
| Asking once a second, all day | | **~$3/day** |
| Back-to-back (~2.7/s) | | **~$8/day** |
| **10,000 backtest decisions** | | **~$0.35** |

**Latency is flat in N** [MEASURED, same doc]:

| Changed | Result | Meaning |
|---|---|---|
| State 446 -> 1,687 -> 6,633 tokens | **257, 254, 274 ms** | *"Shorter text saves money (**15x cheaper across this range**), not time."* |
| **Questions per request 1 -> 4 -> 16** | **257, 268, 277 ms** | *"asking several at once is nearly free in time"* |
| 8 concurrent requests | **282 ms each** | concurrency doesn't slow any of them |

**So 16 questions in one call = +8% latency and ~$0.0001, versus 16 separate calls at ~$0.0009 — a ~9x cost saving and ~16x wall-clock saving.** That is the actual economic argument.

Corroborating costs [all MEASURED]: jev-mlb **2,118 calls for $0.12**; jev-ood-calibration **4,621 calls for ~$0.06**; jev-orderby-bench 350 requests / 359,013 tokens for **~$0.013**; jevymarket **$0.00006/decision**.

**Rate-limit ceiling: 250,000 tokens/sec, 1,200 requests/minute** (early access, dynamic).

**Direct answer: the value on Polymarket/Kalshi is COST, not speed.** At ~$0.00006/decision you can gate a 10,000-market universe for **$0.60** and ask on every pass: is this answerable? are the resolution criteria objective? is this a duplicate of that market? is this headline actually new? is this flow toxic? The sub-100 ms merely lets you do that screening *synchronously inside a scan loop* instead of as a batch job. It is not an execution edge. **And the moment you need a forecast rather than a screen, the researcher LLM (~$0.015/brief) dominates the bill by ~200x and Jev's price advantage stops mattering.**

## 2.4b More independent measurements that bear on trading use

**yodablocks/jev-orderby-bench** https://github.com/yodablocks/jev-orderby-bench — pre-registered gates, jev-1.13.0 [MEASURED]
- In-domain (360 human-labelled rows, 20 Newsgroups): passes all six gates. `jev_bool` **Brier 0.0524, ECE 0.0453, AUC 0.9637**. Systematically **under**confident.
- **Hard probe (Amazon ESCI, 306 graded pairs, 30 hard queries): FOUR OF SIX GATES FAIL.** `jev_bool` **ECE 0.242**, inversion 0.255; `jev_score` ordinal inversion 0.254 vs human grade; **choice confidence ECE 0.279**.
- **Wording instability: "a plain paraphrase moves the answer by 0.164 on average."** And the vendor's "negation consistency" claim was shown indistinguishable from plain paraphrase sensitivity (0.0161 vs a 0.0159 paraphrase control, ratio 1.01x) — **the negation-consistency claim carries no information.**
- **Quantization: two decimals. 360 rows -> only 45 distinct `jev_bool` values; 21.6% of row pairs tie exactly; 53 rows tied at 0.99.** A 0.01-quantized probability **cannot express an 8%-edge threshold finely and cannot distinguish 0.985 from 0.99.**
- **Batching changes the answer: the same rows through a 40-row batch FAIL the ranking gate (inversion 0.171) that they PASS one-per-request (0.0363). Send one market per request.**
- On the vendor: *"the only figure its vendor publishes is **67.8% agreement against averaged frontier judgments**, self-run and unreproduced. **Agreement with other models is not calibration.**"*

**Additional quantization hazard from jev-ood-calibration [MEASURED]:** *"Probabilities are quantised to 0.01 and frequently exactly 0 or 1. In OpenBookQA, **1,051 of 2,000 option probabilities are exactly 0**. One item assigned probability 0.00 to the correct answer. **A probability of exactly 0 cannot be repaired by temperature scaling.**"* For a market position that is a catastrophic log-loss event. Also: ***"Don't threshold on the `confidence` field."*** On those sets it was never better than the max probability and sometimes much worse (confidence-field ECE 0.18 on the synthetic set).

**scienthoon/luce** [INDEX-REPORTED, one hop removed]: a LoRA on Qwen3-4B trained for the specific task reportedly beats Jev substantially — rule-generated tickets **91.1 vs 75.1**, phishing **97.4 vs 62.6**. Also relayed: `anisselbd/jev-phishing-bench` — 2,000 emails, **Jev 62.6% [60.5, 64.7], ECE 0.154** vs Claude Haiku 4.5 ECE 0.097.

## 2.5 Skeptics

**x.com posts could NOT be fetched (HTTP 402; xcancel/nitter mirrors suspended). Substance corroborated only via secondary sources — treat direct quotes as [UNVERIFIED].**
- https://x.com/bojie_li/status/2100688989006475580 — "Jev is just a classifier". Corroborated substance: architecture assessed as *"Transformer encoder + classification/regression heads"* rather than a language model; Sean Goedecke cited that constrained decoding on existing LLMs already achieves fast structured output, so **there may be limited moat**. Chinese roundup (https://github.com/lizliz404/agent-skills issue #3) characterises it as **"更快的通用分类器"** — *"a faster universal classifier."*
- https://x.com/jiayuan_jy/status/2100876273061102006 — same roundup summarises the one-day findings as **"复杂场景准确性存疑、尚无长上下文 benchmark"** (*accuracy questionable in complex scenarios; no long-context benchmark exists*) and **"世界知识/长上下文待验证"** (*world knowledge and long context await verification*). jev-mlb and jev_bitcoin_backtest are the measured confirmation of exactly that doubt.

**juejin deep-dive** https://juejin.cn/post/7686669083098775562 [fetched] — the most useful skeptical secondary source, containing an independent calibration failure:
- Poker test: *"**它没法从原始牌面自己推理出'我输了'，得有人把答案嚼碎喂到嘴边**"* — *it cannot reason "I've lost" from the raw cards; someone must chew the answer up and spoon-feed it.* Success required pre-digested state like `"Flush, Ace high"`.
- **置信度倒挂 ("inverted confidence")**: highest confidence (**0.86**) on the worst errors; lowest (**0.09**) near the correct choices. [MEASURED, small-n]
- ***"校准是针对预测群体衡量的，不保证单个答案正确"*** — ***calibration is measured over a POPULATION of predictions; it does not guarantee any single answer is correct.*** **This is the single most important sentence for a trading application.** Aggregate calibration over a training distribution says nothing about the probability attached to *this* market.
- *"官方那套成本/速度数据依然没有独立复现"* — the official cost/speed data remain **independently unreproduced**; the 193.6x speedup / 444.6x cost claims are self-reported against competitors run with reasoning **on**, in a comparison designed by TypeSafe.
- Verdict: *"方向大概率是真的，数据有水分"* — *the direction is probably real, the data is watered down.*

**pearpages.com critique** [fetched] — verified: Mike Taylor (Every) ran 11 early-access experiments, "777 judgments over 37 documents in under 0.7 seconds"; schema compliance is guaranteed by construction. Unverified but pointed: *"**Calibration** has a precise meaning... The word is in the name of the training method and in the launch copy, and **it is absent from the documentation**"*; *"**There is no human ground truth anywhere in the eval.** Jev's 67.8% means it agrees with the two-frontier-model consensus about two thirds of the time"*; RLCD has *"no reward function, no training procedure, no ablation"*; and on hallucination: ***"A wrong answer that type-checks is still a wrong answer... the model constrains the shape of the output. It does not constrain the judgment."***

**Hacker News launch thread** https://news.ycombinator.com/item?id=49717558 (~256 comments):
- **elil17:** *"What we would want to see is a confidence value that is in line with the actual correctness. If the value is 0.9 for 1000 different answers, then approximately 900 of those answers should be correct."*
- **8note:** *"if it puts a high confidence value on a wrong answer, that's still hallucinating"*
- **WhitneyLand:** "40-400x cheaper and 20-200x faster" is *"potentially misleading when comparing fundamentally different model classes."*
- **ramoz:** the home-assistant demo *"had to kick out to an anthropic model"* for complex reasoning.
- Absence of open weights flagged as an adoption blocker specifically for **financial** environments that cannot send data externally. (Note: this is the same objection oboete raised for private memory — see §1.6.)
- https://news.ycombinator.com/item?id=49745752 is a **dead end** — meta-commentary about article scope, no substantive critique. Do not cite.

**Reddit: NOTHING FOUND.** [UNVERIFIED/absence] Targeted `site:reddit.com` and r/algotrading / r/AI_Agents / r/MachineLearning queries surfaced **no Jev threads at all** — neither the trading discussion nor the "Has anyone tried Jev as a relevance filter for RAG?" thread named in the brief. One search-engine summary asserted "ongoing independent testing in the algotrading community on Reddit"; it could not be corroborated with any link and should be treated as **search-engine confabulation**. As of 2026-09-20 the Jev conversation lives on **GitHub and X, not Reddit.**

## 2.5b Unverifiable P&L claims — do not repeat
- **Brendan Playford, X post 2100614031845539975: "375 completed paper trades / 41.1% win rate / 1.5:1 reward-to-risk / +63 units of risk-adjusted return / 1.38 profit factor."** [UNVERIFIED — post could not be fetched (402); numbers come from a search snippet only.] Even at face value: **paper**, no period, no fee model, no instrument, no code, no artifact, and it is "Jev **and a fine-tuned model**" so Jev's contribution is unattributable. **Ignore.**
- **"$10,000 in Jev's hands"** — https://madewithjev.com/builds/10k-trading [fetched]: BTC/USDT, $10,000 bankroll, posted 2026-09-17, a video of a dashboard. **No P&L, no final balance, no outcome metric, no statement of live vs paper.** A promo.
- **jarrodwatts/jev-trader `totals: pnlUsd -0.003`** — **NOT A RESULT.** It is a 3-block JSON **schema example** documenting the SSE event shape. Anyone citing it is misreading a type definition. The deployed endpoint is labelled **"dry run, mock model"** — the public demo isn't even running Jev.
- **aowang-ai/jev-trade's "live desk"** (https://www.jev-trade.com/) — page renders column headers with **no populated data** and does not state testnet vs mainnet. Not usable as evidence. Its own `plan.ts` comment is a structural warning: *"Exits cross as IOC takers: a resting exit only fills when the market moves your way, which **caps winners at the spread and lets losers run**."*

**Bottom line: zero live, fee-inclusive, independently reproduced P&L exists for any Jev trading system as of 2026-09-20. Every positive claim is paper or a screenshot; every rigorous measurement is negative.**

## 2.6 TRACK 2 SYNTHESIS — recommended architecture for polybot

**Do NOT use Jev's probability as the forecast.** Measured: negative Brier skill on BTC across 10 representations; worse Brier than both Kalshi and the base rate on 2,418 MLB games; degrades a market price even when handed one. Mechanism: no live data, no lookup, and demonstrable confident-wrongness when the answer is not recoverable from the supplied state.

**DO use it for the three things it measurably is good at — all of them judgments about text you already have:**
1. **News/event triage and market matching.** Per-item `noul` at ~$0.0002 / ~100 ms; screen thousands of headlines against your open markets. This is the cost play, and it is the strongest fit.
2. **Pre-trade checklist gate.** Copy jevymarket's `clarity` (resolution-criteria objectivity, 0–4 score) and `answerable` (is there enough information to form an estimate) verbatim — these are the two highest-value prediction-market questions found anywhere in this research, and neither is a forecast. Add liquidity/position-limit/stale-data checks, but **compute every numeric and date comparison in your own code and hand Jev the result** (Jev cannot compare dates or count reliably).
3. **Anomaly / kill-switch monitor.** jev-bench's gate economics (errors 16.6% -> 1.1% at the cost of 35% deferrals) is the right shape for a kill switch, where a false halt is cheap and a missed halt is not.
4. **Resolution-ambiguity and duplicate/correlated-market detection** via the jlink pattern (local blocking + Jev adjudication, 0.90–0.99 precision, 259 pairs/s) — plausible and cheap, but **no one has published this for markets. Unproven.**

**Leave the probability estimate to a statistical model or an LLM-with-search.** The correct division of labour is the one jevymarket already implements and the drillan survey found to be universal. The crispest statement of it, from **buberlo/jev-trader** (noted as the most rigorous architecture in the ecosystem), verbatim:

> "**One principle carries the whole build: Jev should not own the trading system. It should own selected judgments inside it. Code calculates the state. Jev interprets the state. Code applies policy. Execution places the order.**"
>
> "Everything computable stays in code (mid, microprice, spread, imbalance, realized vol, inventory, drawdown, queue position). **Everything that is a judgment goes to Jev.** Atomic questions, composed in code — change a coefficient, not a prompt."

Its six atomic judgments in one call — `regime | direction | toxic_flow | liquidity_stressed | quote_environment | inventory_pressure` — feed a policy engine whose thresholds live in code, behind **hard risk vetoes that are "never delegated."** Its fallback ladder is directly reusable:
```
healthy + high confidence  -> normal operation
healthy + low confidence   -> reduce size
late past block deadline   -> hold, never quote stale state
Jev unavailable            -> deterministic fallback judgments only
hard limit breached        -> kill switch, flatten, stop
```
And it **explicitly rejects the vendor's calibration as sufficient**: *"fits Platt scaling so policy thresholds can be recalibrated on your venue's data — **RLCD calibrates against TypeSafe's distribution, not yours.**"* (Status: Phase 0, paper loop, no results.)

Also worth stealing, from jev-hft: ***"Only ask what can be measured... an answer that can never be checked against a price move teaches us nothing."*** Every Jev question in polybot should have a resolvable label attached so you can accumulate a calibration set.

**Pitfalls specific to trading:**
- Fit a per-question isotonic calibrator on your own resolved markets before thresholding. Raw `score` questions run badly overconfident (ECE 0.325 OOD).
- Pin the model version (`jev-1.13.0`, never "latest") as both nfl-data-stack and jev_bitcoin_backtest do — an unversioned probabilistic scorer silently invalidates your backtest.
- Probabilities vary ±0.03 across identical calls; don't put a gate threshold on a cliff edge.
- Never batch multiple markets into one shared state for an absolute score — measured drift up to 1.22 on a 0–3 scale, with all items moving together. That is a systematic, correlated error across your whole book.
- Be suspicious of every P&L claim here: **no Jev trading project reported measured P&L at all.** The star counts (11.7k, 1.5k) measure novelty, not profitability — jev-trader had ~5 hours of development and was untouched afterwards, and its author is Lead AI Engineer at Monad, so it is vendor-adjacent promotion.
- **Paraphrase sensitivity 0.164** — an equivalent rewording of a market question moves Jev's probability ~16 points on hard data. **Prediction-market question text is not standardized. This alone disqualifies Jev as the pricing oracle.**
- **Quantization**: probabilities at 2 decimals, frequent exact 0/1, 53/360 rows tied at 0.99. You cannot express an 8%-edge threshold finely, and an exactly-0 probability is an unrepairable log-loss catastrophe if the market resolves the other way.
- **Don't threshold on the `confidence` field** — measured never better than max probability, sometimes much worse (ECE 0.18 on an OOD set).
- **Ecosystem age caveat:** every trading repo found was created between **2026-09-16 and 2026-09-20** — 1–5 days old. There is **essentially zero live out-of-sample track record for anything in this space.**
- **Rate limits vs calibration:** jev-plays-pokemon-red's lesson generalises — at realistic call rates you cannot accumulate enough labelled outcomes to establish calibration quickly, and **markets that resolve in weeks make this worse, not better.** nfl-data-stack's power analysis says **~400 resolved events to separate forecasters 0.042 bits apart**. Budget months before you can claim an edge.

---

## APPENDIX — things that could NOT be verified (do not cite as established)
- Direct text of the bojie_li and jiayuan_jy X critiques, the GoSailGlobal post, the theo compaction critique, and Brendan Playford's P&L post — **x.com returns HTTP 402; all nitter/xcancel mirrors suspended.**
- Brendan Playford's "375 paper trades / 41.1% win rate / 1.38 profit factor" — snippet only, mixed Jev + fine-tuned model. **Do not repeat as a Jev result.**
- Any Jev Brier/AUC on LegalForecastBench — **does not exist publicly as of 2026-09-20.** (Jev support was added 2026-09-17 and patched daily through 09-19; only GPT-5.6 Luna and Meta Muse Spark 1.3 completed Cycle 1. Note also: full-text legal requests ranged 21,479–607,966 bytes, median 124,580 — **only 17 of 91 fit the 64,000-byte budget**, so the Jev condition runs on LLM-generated summaries and measures the summarizer too. Its docs state plainly: *"Its Boolean probability is not a confidence score."*)
- Any Reddit discussion of Jev — none found, in either track.
- Any Manifold or Metaculus Jev bot — none exists.
- `opaielsheikh/jev-polymarket-arb` — **404, does not exist.**
- The r/AI_Agents "Jev as a relevance filter for RAG" thread named in the brief — **could not be located.**
- Moritz Kremb tutorial's memory-demo contents — **tutorial is on x.com (402); only the section list is known.**
- valentynkit/jev-plays-pokemon-red's Brier scores — **deliberately not published by the author:** *"the longest labelled run so far is n=5 turns with a confidence interval that covers everything, including the constant predictor. A Brier score on that sample would be decoration."* Its measured latency (median 621 ms, n=6, via gateway shim) is the only number. **Its reusable discipline: always report Brier beside the base rate AND a constant predictor.**
- jarrodwatts/jev-trader's 100 ms p50 with *real* Jev (measured with the mock).
- Whether jev-trade.com / jev-trader run on mainnet with real capital at all.
- scienthoon/luce's head-to-head numbers (index-reported, one hop removed).
