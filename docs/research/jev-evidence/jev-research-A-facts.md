# Jev / TypeSafe AI "System One" — Facts & Independent Evidence

Research date: 2026-09-20. All web content treated as untrusted data.
Tags: [VENDOR] = TypeSafe's own claim. [INDEPENDENT] = third-party measurement.
[UNVERIFIED] = could not confirm / single unconfirmed source.

---

## 0. Company / launch basics

- [VENDOR] Launch post "Introducing System One Models & Jev", 2026-09-15.
  https://typesafe.ai/blog/introducing-system-one-models-and-jev
- [UNVERIFIED — press, not vendor page] TypeSafe AI: San Francisco lab, out of stealth
  2026-09-15, $40M seed, founded by ex-OpenAI researcher Diogo Almeida.
  Sources: search summaries + https://www.artificialintelligence-news.com/news/chatgpt-pioneer-launches-jev-model-for-programmatic-logic/
  I did not fetch a primary funding announcement. Treat funding figure as press-reported.
- Launch X thread (Diogo Almeida): https://x.com/CompleteSkeptic/status/2099925682726002904 (not fetched; X blocks)
- Community index: https://github.com/yibie/awesome-jev
  (self-describes: "A listing is not an endorsement"; does not verify code quality/security)
- Chinese community index: https://github.com/yzfly/awesome-jev-zh

---

## 1. OFFICIAL FACTS (vendor)

### 1.1 Model identity / versions
[VENDOR] https://docs.typesafe.ai/models.md
- `jev-1.13.0` (current), `jev-latest` (alias -> 1.13.0), `jev-preview` (alias, same as latest).
- Response body reports `"model": "jev-1.13.0"`.

### 1.2 Pricing
[VENDOR] https://docs.typesafe.ai/models.md and launch blog
- **$0.042 per 1M input tokens** ($42 per billion input tokens).
- **Output tokens FREE** — blog wording: "FREE (too cheap to meter)".
- => Billing is **per input token**, NOT per decision and NOT per question.
  Output token counts ARE returned in `usage` but are a billing artifact only
  (see archerhume finding: a 255-option response reported 2,714 output tokens
  at the same speed as a 2-option one).
- [VENDOR, workflow-level estimate] launch blog quotes ~$0.0004 per decision case —
  this is derived from TypeSafe's own benchmark workloads, not a price schedule.
- No free tier reported at any gateway.

### 1.3 Latency
[VENDOR] launch blog: "End-to-end response time is 70ms–500ms."
- **No official p50/p95 published.** Docs quickstart, models page and API page
  contain no percentile latency table. [UNVERIFIED for p50/p95 from vendor]
- [VENDOR] "40x–200x faster" than frontier models on System One tasks;
  "193.6x faster" and "193.6x cheaper / 444.6x cheaper" in workflow evals,
  with the blog itself conceding these are "the higher end of real world gains".
- [VENDOR] Hallucination rate listed as "0%", explicitly annotated as
  non-empirical / guaranteed by construction (typed output), not measured.

### 1.4 Context / state limits
[VENDOR] https://docs.typesafe.ai/models.md
- **64k tokens per request total; 32k tokens for `state` plus the longest question.**
- NOTE: Cloudflare and Vercel AI Gateway both list the context window as **32,000**
  (https://developers.cloudflare.com/ai/models/typesafe/jev/ ,
   https://vercel.com/ai-gateway/models/jev). Gateway-listed limit is lower/simplified
  than the vendor's own 64k figure. [DISCREPANCY — flag it.]

### 1.5 Question / option limits
[VENDOR] https://docs.typesafe.ai/api.md
- Max Choice options: **255 per question** (blog: "Maximum cardinality supported is 255 options").
- Score levels: **min 2, max 10** per question.
- **Max questions per request: NOT documented.** [UNVERIFIED]
  Only indirect evidence: archerhume observed 1,500 questions in one call returning
  in a few hundred ms (independent, not a documented limit).

### 1.6 Rate limits
[VENDOR] https://docs.typesafe.ai/models.md
- **250,000 tokens per second / 1,200 requests per minute.**
- Docs note "Rate limits are adjusting dynamically" and may change without notice;
  higher limits via custom/enterprise plans.

### 1.7 Endpoint + request/response shape
[VENDOR] https://docs.typesafe.ai/api.md
`POST https://api.typesafe.ai/v1/systemone`
Headers: `Authorization: Bearer <API_KEY>`, `Content-Type: application/json`
Env var: `TYPESAFE_API_KEY`.

Request:
```json
{
  "state": "Help! My payouts have been failing for 3 days.",
  "model": "jev-latest",
  "questions": {
    "is_urgent": {
      "type": "noul",
      "instructions": "Does this convey urgency?",
      "criteria": { "true": "Explicitly time-sensitive", "false": "No urgency expressed" }
    },
    "department": {
      "type": "choice",
      "instructions": "Which team should handle this?",
      "criteria": {
        "billing": "Payments, invoicing, refunds",
        "technical": "Bugs, outages, integrations",
        "sales": "Pricing, upgrades, new accounts"
      }
    },
    "frustration": {
      "type": "score",
      "instructions": "How frustrated is the customer?",
      "criteria": ["Calm", "Frustrated", "Very angry"]
    }
  }
}
```

Response:
```json
{
  "model": "jev-1.13.0",
  "answers": {
    "is_urgent": { "type": "noul", "noul": 0.95 },
    "department": {
      "type": "choice",
      "choice": "billing",
      "probabilities": { "billing": 0.88, "technical": 0.12, "sales": 0.0 },
      "confidence": 0.81
    },
    "frustration": {
      "type": "score",
      "score": 1.05,
      "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
      "probabilities": { "0": 0.0, "1": 0.95, "2": 0.05 },
      "confidence": 0.92
    }
  },
  "usage": { "input_tokens": 304, "output_tokens": 18 }
}
```
Note: `confidence` is a SEPARATE field from the probability distribution.
Docs distinguish "certainty" vs "probability": https://docs.typesafe.ai/confidence.md
`noul` answers return only a probability — no separate `confidence` in the doc example.

Error codes [VENDOR]: 401 invalid key, 422 validation failure, 429 rate limit,
529 overloaded (retry w/ exponential backoff).

### 1.8 Modality
[VENDOR] https://docs.typesafe.ai/concepts/system-one and /models.md
- **Text only.** "String, JSON object, or array of text values.
  No image, audio, or video input." Images/audio/video explicitly NOT supported.

### 1.9 SDKs
[VENDOR] https://docs.typesafe.ai/sdk.md
- **Python** (requires Python >= 3.10): sync `TypeSafeClient` + `AsyncTypeSafeClient`,
  `RetryPolicy`, typed exceptions.
- **JavaScript/TypeScript**: `TypeSafeClient`, `choice()`, `noul()`, `score()` helpers,
  full typed interfaces (`SystemOneRequest`, `ChoiceResponse`, etc.).
- HTTP API for everything else. No other first-party SDK languages documented.
- [VENDOR] There is a drop-in **agent skill** for Claude and other environments:
  https://docs.typesafe.ai/agent-skill.md
- Third-party: Pydantic AI has a TypeSafe (Jev) provider:
  https://pydantic.dev/docs/ai/models/typesafe/

### 1.10 Availability / gateways
- [VENDOR] TypeSafe direct API is **early access, waitlist-gated**, keys issued in batches.
- [INDEPENDENT/3rd-party docs] Available WITHOUT the waitlist via:
  - **Vercel AI Gateway** — https://vercel.com/changelog/typesafe-ai-jev-now-available-on-ai-gateway
    and https://vercel.com/ai-gateway/models/jev ; exposed through AI SDK 7's
    experimental `evaluate` API. Lists $0.042/1M input, 32k context.
  - **Cloudflare Workers AI** — https://developers.cloudflare.com/ai/models/typesafe/jev/
    Called as `typesafe/jev` via `env.AI.run()` or
    `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run`.
    Listed as a **"Third-party" model**; Cloudflare's docs do NOT state whether it
    runs on Cloudflare infra or is proxied to TypeSafe. [UNVERIFIED where inference runs.]
  - **OpenRouter** — reported as `typesafe/jev-1.13` via a dedicated decisions endpoint
    at $0.042/M input, output free. NOTE: https://openrouter.ai/typesafe/jev-1.13
    returned **HTTP 404** when I fetched it on 2026-09-20. [UNVERIFIED — could not confirm
    the OpenRouter listing directly; only search-summary + secondary sources assert it.]
  - **LLM Gateway** — https://llmgateway.io/changelog/system-one-typed-decisions
  - **Opper** — https://opper.ai/typesafe/jev-1-13-0

---

## 2. DATA HANDLING (most important for a privacy-sensitive runtime)

### 2.1 Training on customer data
[VENDOR] Privacy Policy https://typesafe.ai/legal/privacy-policy — verbatim:
> "We (1) will not train or fine tune any artificial intelligence or machine learning
> models on Input, and (2) will not disclose any Input to a third party other than our
> service providers."

[VENDOR] Master Customer Agreement https://typesafe.ai/legal/mca — verbatim:
> "The foregoing license does not grant TypeSafe the right to, and TypeSafe will not,
> include Customer Data in a dataset used to train any artificial intelligence or
> machine learning models without Customer's prior consent."

[VENDOR] Models page also states Jev is not fine-tuned or LoRA-adapted with customer data.

=> Training-on-your-data: **contractually excluded** (with a "prior consent" carve-out in the MCA).

### 2.2 Retention
[VENDOR] Privacy Policy:
> "We retain personal data about you for as long as reasonably necessary to provide you
> with the Services, or otherwise in support of our business or commercial purposes."
- **No stated retention period for API inputs/outputs.** [UNVERIFIED — no number published.]
[VENDOR] MCA:
> "TypeSafe will be under no obligation to store or retain Customer Data and may delete
> Customer Data at any time in its sole discretion."
  (This is a disclaimer of a duty to RETAIN — it is not a commitment to delete.)
[VENDOR] Deletion on request:
> "When you request that we do so, we take measures to delete your personal data or keep
> it in a form that does not permit identifying you when this personal data is no longer
> reasonably necessary for the purposes for which we process it."

### 2.3 Zero Data Retention (ZDR)
[VENDOR] https://docs.typesafe.ai/legal.md — ZDR available for **enterprise customers**;
contact privacy@typesafe.ai. Not available on standard/self-serve tiers.
[THIRD-PARTY, conflicting] Opper's provider page (verified 2026-09-18)
https://opper.ai/provider/typesafe says: "Zero data retention posture is not established
for this route" — i.e. an independent gateway could not verify ZDR. Opper also lists
"No training on customer data" and "DPA available" with transfer mechanism "unknown".
[INDEPENDENT] scienthoon's calibration study called Jev through **Vercel AI Gateway with
`zeroDataRetention: true`** — so a ZDR flag is at least exposed at the Vercel gateway layer.
(That is Vercel's flag; it does not by itself prove TypeSafe honours ZDR end-to-end.)

### 2.4 Regions / residency
[VENDOR] Privacy Policy: "The Services are hosted in the United States ("U.S.")."
- **US-only. No EU/other region option documented.** No data-residency choice found.

### 2.5 Legal docs
- Privacy Policy: https://typesafe.ai/legal/privacy-policy
- Data Processing Addendum: https://typesafe.ai/legal/data-processing
  (DPA states TypeSafe will not "sell" or "share" Customer Personal Data, will not retain/
  use/disclose it outside the direct business relationship, and will not combine it with
  third-party personal data.)
- Master Customer Agreement: https://typesafe.ai/legal/mca
- Legal index: https://docs.typesafe.ai/legal.md
- **Not found**: SOC 2, ISO 27001, HIPAA/BAA, GDPR SCC transfer mechanism, or a public
  subprocessor list. [UNVERIFIED — absent from what I could fetch.]

### 2.6 Self-hosting / on-prem / on-device / open weights  ← DECISIVE FOR LOCAL-FIRST
- **No self-hosting, no on-prem, no on-device, no open weights.** API-only, closed weights.
- [VENDOR] The launch blog and docs contain **no statement** about self-hosting,
  on-prem deployment, or future open weights.
- [INDEPENDENT] https://www.modemguides.com/blogs/ai-news/jev-typesafe-reality-check-run-locally
  > "No, you cannot run Jev locally, and TypeSafe has not said you ever will."
  > "Jev is available only through TypeSafe's hosted API, currently behind an early-access
  > waitlist. TypeSafe has published no model weights."
  Notes TypeSafe's adapter repo contained only a README as of 2026-09-15.
- [INDEPENDENT] HN commenter **VladVladikoff**: "I don't really want to bounce all my home
  automation commands to the cloud... there are obvious privacy problems."
  https://news.ycombinator.com/item?id=49717558

### 2.7 Contractual restrictions relevant to evaluating it
[VENDOR] MCA prohibits:
> "use the Services or any Output to perform model distillation, train a model to imitate
> the output of the Services, or develop (or to facilitate the development of) a similar
> or competing product"
> "reverse engineer, decompile, disassemble, or attempt to access or derive the source code
> or underlying data with respect to the Services"
- [UNVERIFIED but notable] The **jevcal** author asserts:
  "TypeSafe's customer agreement restricts publishing performance numbers for Jev, so this
  README contains none, on purpose." https://github.com/abhixhek/jevcal
  I did **NOT** find an explicit no-benchmark-publication clause in the MCA text I fetched.
  The clauses I did verify are the anti-distillation and anti-reverse-engineering ones.
  Treat "benchmarks are contractually gagged" as one practitioner's reading, not confirmed.
  => Implication either way: some independent evaluators are self-censoring numbers, which
     thins the independent evidence base.

---

## 3. INDEPENDENT CALIBRATION & ACCURACY EVIDENCE

### 3.1 scienthoon/jev-ood-calibration — STRONGEST independent evidence
https://github.com/scienthoon/jev-ood-calibration
Method: 2026-09-19, via Vercel AI Gateway (`experimental_evaluate`, `zeroDataRetention: true`).
3,721 public-benchmark items + 900 rule-generated synthetic support tickets.
4,621 calls, **zero failures, zero invalid responses**. Total cost ~**$0.06**.
Publishes raw responses, seeded generator, scoring script. MIT / CC0.

Public benchmarks (likely in-distribution):
| Dataset | n | Acc | NLL | ECE | Refit T |
|---|---|---|---|---|---|
| OpenBookQA (val) | 500 | 94.2% | 0.172 | 0.024 | 0.96 |
| CommonsenseQA (val) | 1,221 | 88.1% | 0.395 | 0.032 | 1.35 |
| HellaSwag (val, 2k) | 2,000 | 86.1% | 0.420 | 0.029 | 1.00 |

Synthetic OOD support tickets:
| Question | Type | n | Acc | NLL | ECE | Refit T |
|---|---|---|---|---|---|---|
| Which queue? | choice(4) | 300 | 89.0% | 0.697 | 0.082 | 3.29 |
| Is customer angry? | noul/bool | 300 | 91.7% | 0.275 | 0.079 | 0.66 |
| Priority (org rule) | score(4) | 300 | 44.7% | 1.331 | 0.325 | 3.40 |
| **All** | | **900** | **75.1%** | **0.768** | **0.107** | **2.74** |

ECE noise floor for n=900 (perfectly calibrated, 200 resamplings) = 0.024.
Measured 0.107 = **4.4x the noise floor** => genuinely miscalibrated OOD.

Headline findings:
1. **Calibration is in-distribution-dependent.** Refit temperature **2.74 on the unseen task**
   vs **0.96–1.35 on public benchmarks**. Calibration does NOT transfer to your data.
2. **Sign of miscalibration differs by question type on identical inputs**:
   Choice **overconfident** (T=3.29), Score **overconfident** (T=3.40),
   Boolean/Noul **underconfident** (T=0.66).
   => "calibrate per question, not per model".
3. **Unknowable-task failure mode**: the priority label depended on an org policy not present
   in the text. Accuracy 44.7% (barely above chance) but "the chosen level carries **0.74
   probability on average**" — it is confidently wrong when the answer isn't derivable.
4. **Probability quantization**: "1,051 of 2,000 option probabilities are exactly 0."
   One item assigned **0.00 to the correct answer while `confidence` read 0.99.**
5. **The separate `confidence` field is worse than max-probability**: ECE 0.078 vs 0.029 on
   HellaSwag; 0.18 on synthetics. => prefer max-prob over `confidence` for routing.
Caveats stated by the author: single synthetic task family (n=900, 300/type) limits bin-level
claims; public-benchmark contamination unknowable; gateway exposes no model version;
5% label corruption intentionally injected; calibration measured on returned probabilities.

### 3.2 lindfors.no early-access test
https://lindfors.no/blog/a-first-look-at-typesafes-jev/
Method: 24 Norwegian documents (bokmål + nynorsk) from the 412 responses to the 2022
Norwegian salmon-farming resource-rent-tax hearing; 1 PDF dropped (no text layer).
11 questions/doc: 1 Choice(4) stance, 1 Choice(6) respondent type, 8 Noul arguments,
1 Score(0–3) substance. Reference labels by Claude Fable 5.1, two independent passes
(24/24 stance agreement, 23/23 respondent, 97% arguments).

| Metric | Jev 1.13 | DeepSeek V4.1 Flash (no reasoning) | DeepSeek Flash (reasoning) |
|---|---|---|---|
| Stance (4 opt) | 20/24 | 20/24 | 22/24 |
| Respondent (6 opt) | 21/23 | 22/23 | 23/23 |
| Arguments (192 binary) | 0.86 | 0.89 | 0.88 |
| Substance (exact level) | **19/24** | 14/24 | 14/24 |

Calibration on the 192 argument judgments:
| p-range | n | agreement |
|---|---|---|
| 0.0–0.1 | 14 | 0% |
| 0.1–0.3 | 56 | 4% |
| 0.3–0.7 | 41 | 34% |
| 0.7–0.9 | 38 | 97% |
| 0.9–1.0 | 43 | 98% |
**ECE 0.040 (draft wording) -> 0.116 (careful wording).** Wording alone tripled ECE.
High-confidence (p>=0.9): 14/15 correct stance, 20/20 correct respondent type
(the "34/35 when p>=0.9" figure).
Note author observes Jev is **underconfident at the extremes** here (0.7–0.9 bin -> 97%).
DeepSeek-with-reasoning 0.7–0.9 bin -> only 48% agreement. (LLM confidence far worse.)

Cost per 1,000 documents: **Jev $0.22**, DeepSeek no-reasoning $1.31, DeepSeek reasoning $3.08.
Latency: **Jev median 0.32s, slowest 1.3s**. DeepSeek no-reasoning median 2.7s / p95 17.9s.
DeepSeek reasoning median 26s / p95 250s. (OpenRouter spread requests over 13 endpoints,
so baseline latency is "a mix".)
Caveats: "Twenty-four documents and an early-access model make a first look and not a
benchmark"; 95% interval on a stance number ~±15 points; labels are frontier-model, not human
("Where the reference is wrong and Jev is right, Jev scores as wrong"); circularity risk.
**No repeatability/determinism test performed** (one version, one day).

### 3.3 LangChain — "Can Jev Be a Better Agent Evaluator?"
https://www.langchain.com/blog/jev-agent-evals-langsmith
NOTE: the URL in the task brief, https://www.langchain.com/blog/building-a-harness-with-jev ,
is a DIFFERENT, non-experimental post. I fetched it: it is an explainer/integration piece
that repeats the vendor's "up to 200x faster inference and 400x lower cost" claim, describes
choice/score/noul, and covers model routing and "AutoMode" guardrails that classify risky
tool calls before execution. It contains **no independent measurements** — treat it as
[VENDOR-DERIVED], not evidence. The measured work is at the jev-agent-evals-langsmith URL.
Method: 5 captured weather-agent runs in a LangSmith dataset; each judge scored `quality`
(continuous) and `does_pass` (binary); **500 repeated decisions** per judge.
Compared: Jev, GPT-5.6 Luna, GPT-5.6 Terra, Claude Sonnet 4.6.

Agreement with human oracle on `does_pass`:
- **Jev: 500/500 (100%)**
- Terra: 99.8%
- Luna: 96.4%
- Claude Sonnet 4.6: 80.0%

Repeatability (mean per-case variance on the continuous score):
- **Jev: 0.0000149** (lowest)
- Claude: 92x higher; Luna: 433x higher; Terra: 913x higher.

Cost/latency: **Jev averaged 0.44s and $0.00035/call ($0.34 total)**; Claude $28.17 total.
LangChain's own caveats (important): sample is tiny (5 cases); "This experiment cannot tell
us why Jev's scores varied less" and it is "not evidence that its training objective caused
the lower variance"; "We still need to see whether the results in this experiment carry over
to other agents and production workflows"; "low cost can amplify mistakes — a consistently
wrong evaluator can produce bad feedback at scale"; **"The Jev service version was not
available in the experiment metadata"** (incomplete reproducibility).

### 3.4 agentjournal.dev — judge call vs dimension scores
https://agentjournal.dev/blog/llm-judge-vs-feature-extraction/
Three tasks: synthetic B2B replies (200 rows), aggregate-weak-cues (300 rows),
Japanese NLI (8,000 train / 2,434 test).
Compared: one direct Jev question per row vs 12–14 narrow Jev-scored dimensions with
locally fitted weights (CV).

| Task | Direct | Dimensions | Delta |
|---|---|---|---|
| B2B synthetic | 100% | 98.0% | -2.0 |
| Weak cues | 64.7% | 74.0% | **+9.0pt** |
| Japanese NLI | **83.73%** | **90.76%** | **+7.03pt** |

**Critical safety finding**: on 339 hard-benign rows (security docs that *mention* attack
techniques), direct call FP rate **1.5%**, decomposed-dimensions FP rate **37.2%** (~25x worse).
Dimensions learned surface markers like "obfuscated_encoding" that appear in both benign and
malicious text. => decomposition buys accuracy on the main metric and destroys precision on
adversarial-looking benign input.
Cost: dimensions need **1.6–2.3x more tokens** (1,010–1,036 vs 444–619 tokens/row).
Whole experiment: 34.1M input tokens, **$1.43** across 5,477 test rows.
Author's recommendation: "Write the cheapest baseline first. If plain n-grams pass,
Jev is not involved at all." Use decomposition only where single calls genuinely fail.
Caveats: hand-written dimensions; JNLI has 55% neutral skew + label noise; output tokens
counted but not priced.

### 3.5 archerhume — "Jev's Architecture Unmasked" (iwashi86 circulated it)
https://archerhume.com/posts/jevs-architecture-unmasked/
Method: `jev-1.13.0`, 2026-09-17, one early-access account, one region.
1,029 probe records + 6,800 benchmark records + 1,322 follow-up requests (~10k calls).
Latency taken from `x-envoy-upstream-service-time` (upstream service time, unknown queueing).

Measurements:
- **Latency vs state length**: ~50ms at 360 tokens -> ~220ms at 30,000 tokens (roughly linear).
- **Latency vs question count**: flat up to ~**100 questions**, then rises steadily.
  Token-for-token, question text costs ~**2x** state text.
  **1,500 questions still return in a few hundred ms**; median 610ms, fastest 408ms.
- **Token additivity**: 1 minimal yes/no question = 268 input tokens; 2 = 276; 3-question
  batches summed exactly => common prefix + question suffixes (shared-state encoding).
- **Output tokens are billing-only**: a 255-option response reported 2,714 output tokens
  at identical speed to a 2-option one.
- **Questions are isolated**: a secret referenced in `state` -> 0.90–0.92 probability;
  the same secret isolated in another question's text -> **0.00**. Questions do NOT see
  each other. (Good for privacy reasoning about fan-out, bad for cross-question context.)
- **Options interact**: adding irrelevant options shifted log-odds by **-0.28
  (95% CI -0.36 to -0.19)** across ten blocks => NOT fixed independent logits + softmax.
- **Position sensitivity**: reference card placed last = 16/16 correct; placed first = 12/16.
- **Calibration on MMLU (1,200 items): ECE 0.0313** — but **990 of 1,200 fell in the
  0.9–1.0 bin**, so the ECE is dominated by one bin.
- **Fresh math tasks** (novel, not memorizable): 3-digit product 83.0% predicted vs 86.7%
  actual; two-step word problems **30.4% predicted vs 32.0% actual** — calibrated but
  near-chance competence on novel arithmetic/multi-step.
Author's explicit unknowables: dense vs sparse MoE ("the least certain part"); exact
tokenizer (192 public tokenizers tested over 415 probes, no exact match; closest is Qwen,
agreeing on 348/415, differing on digit splitting); base model identity unknown; training
recipe unpublished; worker/routing mechanism not identified.
NOTE: this work is arguably contrary to the MCA's anti-reverse-engineering clause.

### 3.6 Shogo-nfrealmusic/jev-eval
https://github.com/Shogo-nfrealmusic/jev-eval
60 synthetic multilingual booking-inquiry messages (4 languages), 3 rounds each, serial,
via Vercel AI Gateway. Jev vs GPT-4o-mini vs Claude Sonnet 4.5. One warm-up discarded,
no retries.
RESULTS.md: https://raw.githubusercontent.com/Shogo-nfrealmusic/jev-eval/main/RESULTS.md

| Metric | Jev | GPT-4o-mini | Claude Sonnet 4.5 |
|---|---|---|---|
| Category accuracy (exact) | **96.1%** | 93.9% | 94.4% |
| Urgency agreement (>=0.5) | **96.7%** | 50.0% | 91.7% |
| needs_human agreement | 81.7% | 78.9% | **85.0%** |
| Latency p50 | **379 ms** | 1,209 ms | 1,910 ms |
| Latency p95 | **481 ms** | 1,462 ms | 2,162 ms |
| Cost / 1,000 messages | **$0.032** | $0.122 | $3.41 |

=> 3.2x faster than GPT-4o-mini and 5.0x faster than Sonnet at p50; 3.8x / 106x cheaper.
**This is the only source I found, vendor or independent, with explicit p50/p95 latency.**
Tight p50->p95 spread (379 -> 481 ms) is itself a notable result vs LLM tail latency.
Calibration datapoint: "category probability exceeded 0.95 on 157 of 180 calls and was
never wrong there." BUT Jev **missed more `needs_human` decisions (21 of 69 cases)** —
it under-escalates, which is the dangerous direction for an agent runtime.
Hardest-case split: Jev 88.3% vs GPT-4o-mini 81.7%.
Caveat stated: **"Labels were assigned by a single annotator."** n=60 synthetic messages.

### 3.7 hegargarcia/jev-playground
https://github.com/hegargarcia/jev-playground
Tic-tac-toe / Connect Four legal-move selection; Jev (`typesafe-ai/jev`) vs GPT-5.6 Luna,
Claude Haiku 4.5, Gemini 3.5 Flash Lite, GPT-6 Astra, via Vercel AI Gateway.
**NO aggregate metrics reported.** README: "Automated tournaments and aggregate scores are
future work." Tests use mocked evaluations. Interactive only.
=> Listed in awesome-jev as a benchmark but currently **produces no numbers.**
Caveats: option weights are the Choice distribution, "not win probabilities"; confidence is
provider-specific and not comparable across models.

### 3.8 abhixhek/jevcal
https://github.com/abhixhek/jevcal
A tool, not a study: fits a per-question confidence threshold to a target accuracy on your
own labeled data, verifies on a held-out split, reports how much traffic must escalate to an
LLM, and fails CI when a model update breaks locked thresholds. Splits data in half
(pick on one half, verify on the other). Lints question wording against known model weaknesses.
**Publishes no Jev numbers**, explicitly: "TypeSafe's customer agreement restricts publishing
performance numbers for Jev, so this README contains none, on purpose."
Its shown metrics come from a deliberately-overconfident built-in simulator.
=> Useful as a *pattern* (threshold + escalation + CI regression gate), not as evidence.

### 3.9 Other independent results worth noting
- **jev-orderby-bench** https://github.com/yodablocks/jev-orderby-bench — pre-registered gate
  on whether SQL `ORDER BY` over a Jev probability is defensible. jev-1.13.0 **passes on
  20 Newsgroups topics** but **fails four of six conditions on Amazon ESCI product relevance**.
  Also: a DuckDB extension's default **40-row batching fails the ranking gate that one row
  per request passes** (batching changes the answer).
- **"Jev reranking is not a free win"** https://x.com/GoSailGlobal/status/2100877682972258619
  33,047 catalog entries, 164 real queries, 9,831 graded pairs: **Jev reranking alone did not
  beat vector retrieval.** (X link not fetchable; description is awesome-jev's, verbatim.)
- **BTK audit studies** https://boringtoolskit.com/blog/seo-audit-cost-2026/ — 1,204 pages
  judged per run, 4,816 judgments in under 3 minutes, $0.0048 per 12-query batch.
  (Vendor-adjacent marketing content; throughput datapoint only.)
- **Reddit: "I reviewed 287 open-source Jev projects"**
  https://reddit.com/r/LLMDevs/comments/1wko2e5/i_reviewed_287_opensource_jev_projects_here_are/
  (not fetched; flagged by awesome-jev as "critical filtering of actual working
  implementations" — i.e. most of the ecosystem is same-day scaffolds.)
  awesome-jev itself warns: same-day releases sharing scaffolds satisfy inclusion rules yet
  remain "unproven".

---

## 4. CRITICISM & FAILURE MODES

### 4.1 VENDOR'S OWN admitted limitations (unusually candid — cite this)
https://docs.typesafe.ai/model-jaggedness/jev-1.13.md
1. **Literal reading** — "answers the question you wrote, not the one you meant. Scoping
   words, negations, and implied conditions are read at face value."
2. **Math/counting unreliable** — do arithmetic in code, not in Jev.
3. **Dates as text** — "reads dates as text, not as ordered quantities. Asking which of two
   dates comes first, how far apart they are, or whether one falls inside a window is
   unreliable."
4. **Indirection** — multi-hop reasoning and double negatives substantially reduce accuracy.
5. **Long-context degradation** — "Accuracy falls as the state grows with content unrelated
   to the decision. Unrelated detail acts as a distractor."
6. **ADVERSARIAL VULNERABILITY** — "does not treat input data as inherently hostile",
   susceptible to injected instructions and misleading framing.
   => A Jev guardrail/classifier is itself prompt-injectable. Vendor-admitted.
7. **Instruction/criteria conflict** degrades performance.
8. **Structural invariants violated** — a question and its negation do not necessarily sum
   to 1.0. (Matches scienthoon's per-type sign-of-miscalibration finding.)
9. **Cannot generate text** — generation tasks are slow and ineffective.
10. Bounded context window.
[VENDOR, calibration caveat] "Calibration is a group property... it does not guarantee the
correctness of any single answer. Jev can still make mistakes."

### 4.2 Behavior when the right answer is NOT among the options
- [INDEPENDENT, scienthoon] This is the clearest measured failure: on the "priority" question
  whose true label depended on an org rule absent from the text, accuracy was **44.7%**
  (near chance for 4 options) yet **the chosen level carried 0.74 probability on average**.
  It does not abstain and it does not report low confidence. **No `unknown` escape hatch is
  built in.**
- [VENDOR mitigation] The documented workaround is to add an explicit "uncertain/unknown"
  option yourself — see cookbooks:
  https://docs.typesafe.ai/cookbooks/consistency_choice_cookbook.md ("Add uncertain outcome
  to moderation decisions") and
  https://docs.typesafe.ai/cookbooks/consistency_noul_cookbook.md ("Route uncertain
  probabilities to human review").
- [INDEPENDENT, scienthoon] Probability mass quantizes to exact 0 (1,051/2,000 option
  probabilities were exactly 0), and at least one item put **0.00 on the correct answer while
  `confidence` read 0.99** — so `confidence` cannot be used as an out-of-scope detector.

### 4.3 Hacker News launch thread
https://news.ycombinator.com/item?id=49717558 (~1,800 points, ~480 comments)
- **jacobgold**: the comparison isn't valid — Jev "can only generate structured output"
  while generative models are doing Turing-complete work; speed comparisons ignore that
  LLMs could be constrained the same way.
- **bigglebear**: the speed advantage evaporates — "comparing to LLMs... is pointless because
  you could train a transformer on the same sort of task... and get even better performance
  yet again." Claims the Doom demo used "structured state, not images" and "cheated" with
  shortcuts unavailable to general models.
- **8note**: "if it puts a high confidence value on a wrong answer, thats still hallucinating."
- **nkozyra**: "Type safety is not factual correctness."
- **thduabmd**: "You've already agreed that this doesn't establish correctness. An approve for
  an unauthorized action still meets the schema guarantee."
- **WhitneyLand**: the "40-400x cheaper and 20-200x faster" framing is "false" because Jev
  targets different use cases entirely.
- **elcomet**: confidence scores only help if "correctness aligned with confidence", not
  merely structural constraint.
- **VladVladikoff** (privacy): won't send home-automation commands to the cloud; no
  open-weight alternative exists.
Related: https://news.ycombinator.com/item?id=49760225 ("Ask HN: What do you think of Noul")
and https://news.ycombinator.com/item?id=49765997 (practitioner classification trial).

### 4.4 "It is the inference technique, not the training" (anderslie / Sean Goedecke line)
https://www.seangoedecke.com/jev-means-structured-output-is-interesting-again/
- "It's unclear to me how much of Jev's value is in the model itself, compared to the
  inference strategy of only generating one token per question."
- You can "simply prefill the response with `\"choice\": \"` and generate one token,
  restricted to the user-provided choices". Author tested this with **Qwen2.5-1.5B-Instruct**
  and got a **2x–3x speedup** over non-prefixed structured output.
- On calibration: "I haven't seen anything to suggest that these aren't just regular logit
  probabilities." => the calibration claim is the only part not obviously reproducible.
- [UNVERIFIED] I could not locate the specific **anderslie** post; the identical argument is
  documented at the Goedecke URL above. Do not attribute the quote to anderslie.

### 4.5 "Jev is just a classifier, and that is fine" (bojie_li)
- [UNVERIFIED — X post, not fetchable] Described (via awesome-jev/search) as arguing Jev is a
  representation model rather than a new class, and that its latency follows mechanically from
  one prefill plus a single parallel token per question with logprobs. Posted in Chinese on X.
- The same argument is available in fetchable form:
  https://explainx.ai/blog/jev-vs-xgboost-bert-classifiers-2026 (Jev vs XGBoost/BERT — the
  concept of a fixed pre-enumerated output set is decades old; what's new is generalization
  across tasks without per-task training).
  and https://daily.dev/posts/it-s-easy-to-dismiss-jev-as-just-a-classifier-mrc5ya3xo
- Video restatement: https://www.youtube.com/watch?v=FVzI3QAxwNs

### 4.6 Isaac Flath "Six things I'll still use Jev for" (60-day review)
[UNVERIFIED] I could not retrieve this post. It is referenced in awesome-jev forks as
"a practitioner lists the six Jev uses he still expects to rely on after 60 days, an early
usefulness review rather than a launch reaction." It appears to be an X post.
**Chronology problem**: the public launch was 2026-09-15 and today is 2026-09-20, so a
"60-day" review implies ~60 days of private early access. I could not confirm the post,
its date, or its six items. Do not cite specifics from it.

### 4.7 Chinese-language skeptic notes
[PARTIALLY VERIFIED] Chinese community index: https://github.com/yzfly/awesome-jev-zh
(also mirrored at https://github.com/shitianfang/awesome-jev-zh). Chinese-language overview:
https://www.eigent.ai/zh-CN/blog/typesafe-ai-jev-system-one-models
The recurring skeptical line found in Chinese-language material: "This approach currently has
no public papers. Whether calibration is good depends on measuring it on your own data,
don't trust anyone's slides." I could not fetch individual Chinese X threads (X blocks
fetching); treat specific attributions as unverified.

### 4.8 No public paper
- **RLCD ("Reinforcement Learning for Calibrated Decisions") has no published paper**
  that I could find — only the launch blog's description. [UNVERIFIED as a method.]
  archerhume independently notes "the exact recipe is unpublished" and that log-loss vs
  Brier-loss optimization cannot be distinguished from the outside.

---

## 5. CONSENSUS: GOOD FOR vs ABANDONED

### Converging evidence — Jev is GOOD for:
1. **High-volume bounded classification/routing over text you already have.**
   89–94% on in-distribution choice tasks (scienthoon), 91.7% on sentiment (OOD),
   stance/respondent parity with a frontier LLM at ~1/6 to 1/14 the cost (lindfors).
2. **Online eval / LLM-judge replacement where REPEATABILITY is the point.**
   LangChain: 500/500 oracle agreement, per-case variance 92–913x lower than LLM judges,
   $0.00035/call vs $28.17 total for Claude. This is the strongest independent result.
3. **Latency-critical interactive decisions.** p50 379ms / p95 481ms (jev-eval — note the
   very tight tail), 0.32s median (lindfors), 0.44s mean (LangChain), 408–610ms even at
   1,500 questions (archerhume). No parse step, zero invalid responses across 4,621 calls
   (scienthoon) and 60x3 runs (jev-eval). The tail latency is the real differentiator vs
   LLMs (DeepSeek reasoning p95 250s; Sonnet 4.5 p95 2,162ms).
4. **Massive fan-out.** Question count is nearly free up to ~100 questions (archerhume);
   output tokens are unbilled, so batching many speculative questions into one call is the
   intended and measured-cheap pattern (docs /patterns/fan-out.md).
5. **Scoring/ordinal judgments** — lindfors found Jev BEAT both DeepSeek configs on exact
   substance level (19/24 vs 14/24).
6. **Confidence-gated cascades**: use Jev, escalate the low-confidence tail to an LLM
   (jevcal pattern; docs /patterns/confidence-routing.md).

### Tried and ABANDONED / didn't pay off:
1. **Reranking / search relevance.** GoSail: 33,047 entries, 164 queries, 9,831 graded pairs
   — Jev reranking alone did NOT beat vector retrieval. jev-orderby-bench: fails 4/6
   pre-registered conditions on Amazon ESCI product relevance (passes on 20 Newsgroups).
2. **Anything requiring a rule not present in the state.** 44.7% accuracy at 0.74 average
   confidence (scienthoon). Confidently wrong.
3. **Dates, arithmetic, counting, ordering** — vendor-admitted unreliable.
4. **Adversarial/guardrail use without defense in depth** — vendor-admitted injectable;
   agentjournal's decomposed-dimension variant hit a 37.2% false-positive rate on hard-benign
   security text.
5. **Trusting the `confidence` field** — worse-calibrated than max-probability on both
   HellaSwag and the OOD synthetics (scienthoon). Multiple people converge on: threshold the
   probability distribution, not `confidence`.
6. **Assuming vendor calibration transfers** — refit T=2.74 on unseen data vs ~1.0 on public
   benchmarks. Every serious evaluator (scienthoon, jevcal, the Chinese-language note)
   concludes: **measure calibration on your own data and fit your own thresholds.**
7. **Batching rows into one call for ranking** — jev-orderby-bench: 40-row batching fails a
   gate that one-row-per-request passes.

### Relevance to a LOCAL-FIRST, PRIVACY-SENSITIVE runtime
- **Blocking issue**: API-only, closed weights, US-only hosting, no on-prem/on-device,
  no public statement of intent to ever ship weights. ZDR is enterprise-tier only and an
  independent gateway (Opper) reports ZDR posture as "not established".
- **Open alternatives that reproduce the INTERFACE locally** (none reproduce RLCD):
  https://apidog.com/blog/openjev-open-source-jev-alternatives/
  | Project | Repo | Base model | Runtime | Reported vs Jev |
  |---|---|---|---|---|
  | OpenJev | github.com/TheoLeeCJ/openjev | Qwen3.5-4B | CUDA (RTX 3090+) | 0.845 modal agreement |
  | mini-jev | github.com/r-ms/mini-jev | Qwen3-4B-Instruct-2507 | **MPS** or CUDA | -0.22 pts (within CI) |
  | jevlike | github.com/vinnylarouge/jevlike | custom trainable encoder | CPU/MPS/CUDA | ~26% on Wikispeedia |
  | MLX parallel constrained decoding | huggingface.co/spaces/drinkmoonshine/parallel-constrained-decoding | Qwen2.5-1.5B-Instruct-4bit | **Apple Silicon only** | no accuracy reported |
  | vLLM PR #57250 | github.com/vllm-project/vllm/pull/57250 | DiffusionGemma | vLLM GPU | ~90% on classification |
  Article's own verdict: **"None of them reproduce RLCD. That's the honest headline."**
  For a Mac-mini local tier, **mini-jev (MPS, Qwen3-4B, within-CI of Jev on its test)** and
  the **MLX parallel-constrained-decoding** space are the two directly relevant leads.
- The mechanism is cheap to reproduce: prefill KV cache once, broadcast across fields,
  score only valid candidate token IDs per field, softmax over that set, assemble JSON in
  code (Goedecke; 2–3x speedup on Qwen2.5-1.5B). The contested question is only whether
  RLCD's calibration is real and worth the cloud dependency — and scienthoon's OOD result
  says the calibration does NOT transfer to your data anyway, which weakens the single
  strongest reason to pay for the hosted model.

---

## 6. EXPLICITLY UNVERIFIED / COULD NOT CONFIRM
- Official p50/p95 latency percentiles (only the 70–500ms range is published by the vendor).
  The only p50/p95 numbers anywhere are Shogo-nfrealmusic/jev-eval's: p50 379ms / p95 481ms
  (n=60 synthetic messages, single annotator) — independent but very small.
- Max questions per call (undocumented; ~100 is where latency starts rising, per archerhume).
- OpenRouter listing: https://openrouter.ai/typesafe/jev-1.13 returned 404 on fetch.
- Whether Cloudflare runs Jev on its own infra or proxies to TypeSafe.
- Context window discrepancy: vendor 64k total / 32k state vs gateways listing 32k.
- SOC 2 / ISO 27001 / HIPAA / GDPR SCCs / subprocessor list — none found.
- Any TypeSafe statement about future open weights or on-prem — none found (absence, not denial).
- An explicit MCA clause banning publication of benchmarks (jevcal asserts it; I could not
  find the clause text; the anti-distillation and anti-reverse-engineering clauses ARE real).
- Isaac Flath's "Six things I'll still use Jev for" — post not retrieved; "60-day" timing is
  hard to reconcile with a 2026-09-15 public launch.
- bojie_li and anderslie X posts — not fetchable; equivalent arguments sourced elsewhere.
- $40M seed / Diogo Almeida founder detail — press-reported, no primary source fetched.
- RLCD has no public paper.
