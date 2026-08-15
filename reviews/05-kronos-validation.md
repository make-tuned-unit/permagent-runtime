# Kronos → Financier: validation against the evidence

Written 2026-08-15. The integration plan itself (uncertainty/scenario sidecar, 3-step spike, kill criteria) lives only in the primary Mac's session memory; this file records what the external evidence says about whether that plan should proceed, so the judgement survives a machine.

**Model:** [shiyu-coder/Kronos](https://github.com/shiyu-coder/Kronos) — decoder-only foundation model over OHLCV candlestick tokens. Tsinghua (IIIS + Automation), arXiv [2508.02739](https://arxiv.org/abs/2508.02739), AAAI-26. MIT licence. Pretrained on 12B+ K-lines, 45 exchanges, cutoff June 2024.

**Intended use here:** part of The Financier's brain, reviewing the outputs of the user's own pre-surge picker (`~/dev/Picker/pre_surge_scanner`, a Flask service behind an HTTP seam).

---

## Verdict — the kill criterion was run, and it FAILED

**Do not build it.** Superseded the literature-based recommendation below; the test was run on 2026-08-15 and Kronos loses to a 40-year-old GARCH model on the one job it was proposed for. Experiment, data and output are preserved in `reviews/kronos-experiment/`.

### Result

606 non-overlapping forecasts, 6 tickers (SPY, AAPL, MSFT, NVDA, AMD, TSLA), daily bars, 5-day horizon, 30 sampled paths each, windows from 2024-08-01 to 2026-07-31 — all strictly after the June 2024 pretraining cutoff. Kronos-small on MPS, 24 minutes.

**1. Standalone — Spearman rank correlation with next-window realized volatility**

| Predictor | ρ | 95% CI |
|---|---|---|
| **Kronos dispersion** | **0.592** | [0.538, 0.642] |
| Kronos path-vol | 0.538 | [0.479, 0.592] |
| Trailing RV(20) | 0.741 | [0.703, 0.775] |
| EWMA(0.94) | 0.744 | [0.706, 0.777] |
| **GARCH(1,1)** | **0.745** | [0.707, 0.778] |

Kronos loses to every baseline including a 20-day standard deviation. Steiger test against GARCH: **t = −7.94, p = 9.6e-15** — the gap is not noise.

**2. Incremental — does it add anything a GARCH user does not have?**
`log(realized) ~ log(GARCH) + log(Kronos)`, HC3 errors: **β_Kronos = +0.029, p = 0.558.** R² rises from 0.5258 to 0.5262 — **+0.0004**. Nothing.

**3. Calibrated QLIKE** (scale fitted per predictor, so level bias is not penalised): Kronos 0.892 vs GARCH 0.525. Diebold–Mariano against all three baselines: **significantly worse** (p = 1.6e-05, 3.1e-08, 1.2e-10).

**4. Directional** (recorded to put the claim on the record): **49.0% ±4.0pp, p = 0.655.** Indistinguishable from chance, matching every independent test in the literature review below.

### The finding that matters most: the pooled number is an illusion

Per-ticker Spearman with realized vol:

| Ticker | Kronos | GARCH | RV(20) |
|---|---|---|---|
| AAPL | **−0.134** | 0.168 | 0.283 |
| MSFT | **−0.089** | 0.313 | 0.345 |
| SPY | 0.055 | 0.384 | 0.332 |
| TSLA | 0.208 | 0.193 | 0.358 |
| NVDA | 0.214 | 0.300 | 0.424 |
| AMD | 0.281 | 0.325 | 0.304 |

The pooled ρ = 0.592 looks respectable. **Stratified by ticker it collapses, and goes negative on two of six.** The pooled figure is mostly *cross-sectional*: Kronos has learned that TSLA is a more volatile instrument than SPY, which is true, stable, and worth nothing — you already know it. What a picker reviewer needs is the *time-series* question — is *this week* unusually risky for *this name* — and there Kronos is near zero or actively inverted.

This is the exact trap the literature review warned about, and it is worth noting that the one encouraging external result (Spearman +0.375 dispersion↔realized vol) was reported pooled. **A pooled correlation across instruments of differing volatility manufactures signal from level differences alone.** Had the kill criterion been run pooled-only, it would have read as a marginal pass.

### Cost of the finding

Roughly 30 minutes of compute and no integration work — which is the entire argument for running kill criteria before building rather than after.

---

## Original recommendation, superseded above

**Proceed only as a volatility/dispersion tool, never as directional confirmation — or not at all.**

The framing in the handoff note ("uncertainty/scenario sidecar") is better targeted than it probably realised: it is the *only* Kronos use case with any independent supporting evidence. Every independent test of its *directional* output found no edge. If any part of the plan treats the ensemble mean as a signal about which way a pick will go, cut that part.

## The evidence, separated by source

### What the authors claim
+93% RankIC over the leading time-series foundation model, +87% over the best non-pretrained baseline, 9% lower volatility-forecast MAE. Test window strictly post-cutoff, 25 baselines, **and a 0.15%/trade transaction cost in the backtest** — better hygiene than most financial-ML papers.

Two problems on their own terms:
- **No random-walk or naive-persistence baseline.** In this domain that is the single most important comparator, and its absence is conspicuous.
- **The 499M model carrying the headline numbers was never released.** A year of requests (#325, #347, #270, #360) are unanswered, including from people offering to benchmark it.

### What independent parties found
Consistently negative on direction:

| Source | Test | Result |
|---|---|---|
| [#354](https://github.com/shiyu-coder/Kronos/issues/354) | 1,800 rolling AAPL forecasts, 2 lookbacks × 3 horizons × 3 seeds | **Every configuration worse than persistence** (12–22% higher MAPE); beat it in 33–41% of runs; directional accuracy 48.5–54.3% |
| [#323](https://github.com/shiyu-coder/Kronos/issues/323) + [gist](https://gist.github.com/moscowmule2240/1bb5bf350ad58e42199ac350f8768672) | Audit of the project's **own live demo**: 4,682 published BTC forecasts vs realized | 53.0% ±1.5 directional; de-overlapped **53.1% ±7.0 — indistinguishable from chance** |
| same | Calibration | **Inverted.** Forecasts saying 22% P(up) realized 45.8%; saying 94% realized 52.8%. **Brier 0.298 vs 0.250 for a constant 50% — worse than predicting nothing** |
| [#387](https://github.com/shiyu-coder/Kronos/issues/387) | Walk-forward futures backtests | No directional edge, worse than random walk, "cones too narrow" |
| comment on #323 | ORCL/AAPL/MSFT/SPY, daily + 5-min | Beat a naive random walk in 1 of 8 ticker-timeframe cases |

**The one positive independent finding, and it is the one that matters here:** sampled-path **dispersion** correlates with realized volatility at **Spearman +0.375**. The ensemble *spread* carries real information about how uncertain the next window is, even though the ensemble *mean* carries little about direction. That is precisely a scenario/uncertainty sidecar and nothing more. Caveat: one person's unreviewed result on crypto and gold.

### What is unknown
- **Whether the published weights contain baked-in leakage.** Window-normalisation leakage was reported Sept 2025 (#84, #83) and fixed in the *training code* only in April 2026, via a community PR. [#307](https://github.com/shiyu-coder/Kronos/issues/307) asks whether the released checkpoints were retrained afterwards. **Never answered.** A commenter states they were not.
- **Behaviour in an unseen regime.** No out-of-regime evaluation exists.
- **Latency at scenario scale.** Cost is `sample_count × pred_len` sequential decode steps; a 100-path fan is 100× a point forecast. Nobody has published figures.

## Two findings that directly change the build

**1. The public API throws away the thing you need.** `KronosPredictor.predict()` averages the sampled paths internally and returns a point forecast — `preds = np.mean(preds, axis=1)` at `model/kronos.py:467`. The README documents `sample_count` as "number of forecast paths to generate **and average**." There is no public parameter to return the fan.

So **step 1 of the spike must be "expose per-path output"** — calling `auto_regressive_inference()` directly, or patching the reshape/mean. It is a small change to one function, but it is a vendor/fork decision rather than a config flag, and anyone who wrote the plan against the README would not know it. MIT licence makes vendoring fine.

**2. The correlation problem is worse than it looks, but resolves cleanly.** The picker is a *pre-surge ranking algorithm* — almost certainly OHLCV/momentum-derived. Kronos is trained on OHLCV. Two models on the same data are weak independent evidence. Combined with the finding that Kronos has **no directional edge at all**, using it to confirm a pick's direction would be rubber-stamping noise with noise.

The resolution: Kronos must answer a *different question* than the picker. The picker asks "which names look like they are about to move." Kronos should only ever answer "how wide is the distribution of outcomes for this name over the next window" — a risk filter applied after selection, not a vote on selection. That is non-redundant, and it is the one thing the evidence supports.

## Deployment

- **Apple Silicon: works natively.** MPS auto-detect at `kronos.py:498` (added Dec 2025), independently confirmed (#294, #86, #384).
- **Tiny models.** 4.1M / 24.7M / 102.3M params. The #354 study ran 1,800 forecasts on CPU. Memory is a non-issue.
- **No ONNX** (requested #41, never actioned), **no Rust bindings.** PyTorch only.
- **Integration path is already established in this repo.** `crates/goose/src/picker.rs` documents the pattern: the scanner "is a long-lived service with an HTTP API, so this module is an HTTP client and nothing more… we do not spawn its Python." Kronos should ride the same seam — a local Python service, Permagent as HTTP client. That reasoning transfers exactly.
- **Context limit 512** for small/base (2048 for mini only) — a real lookback constraint.

## Maintenance risk

Not dead, clearly winding down. Last push **13 April 2026** (4 months stale). 11 commits in all of 2026, mostly one backlog-merge sitting. **262 open issues, 56 open PRs, zero tagged releases.** The official live demo **stopped updating 4 July 2026** (#386, unanswered). No v2, no successor, no fork with independent momentum. High HF download counts are bot/CI-inflated and are not evidence of production adoption.

Pattern: published, got the AAAI acceptance, disengaged. The unanswered issues are precisely the substantive correctness ones.

## Kill criteria — make them quantitative and dispersion-specific

The instinct to write kill criteria was right. Concretely:

- **Primary:** sampled-path dispersion must beat a trailing-realized-volatility and a GARCH(1,1) baseline at predicting next-window realized volatility, on *our* instruments, out of sample. If it cannot beat GARCH, it has no job here.
- **Reject outright** any use of the ensemble mean as a directional input. The calibration is inverted; the probabilities are actively misleading.
- **Do not accept the paper's RankIC numbers as evidence.** Single-source, unreproduced, possibly contaminated by checkpoint-level leakage, and the model behind the headline was never released.
- **Latency budget** measured before committing: `sample_count`-linear, unmeasured by anyone.
- **Abandonment clause:** vendored under MIT, with no expectation of upstream fixes.

## Inherit the picker's honesty rule

`picker.rs` already distinguishes "unreachable" from "reachable and has nothing," because "a stale pick rendered as today's pick is worse than an empty surface." A Kronos sidecar must do the same — a silently-down reviewer must never read as approval. This is the same empty-vs-broken discipline the 2026-08 audit found violated elsewhere in the codebase.

---

**Sources:** [arXiv 2508.02739](https://arxiv.org/abs/2508.02739) · [AAAI-26 entry](https://ojs.aaai.org/index.php/AAAI/article/view/39730) · [repo](https://github.com/shiyu-coder/Kronos) · [model/kronos.py](https://raw.githubusercontent.com/shiyu-coder/Kronos/master/model/kronos.py) · [#354](https://github.com/shiyu-coder/Kronos/issues/354) · [#323](https://github.com/shiyu-coder/Kronos/issues/323) · [benchmark gist](https://gist.github.com/moscowmule2240/1bb5bf350ad58e42199ac350f8768672) · [#387](https://github.com/shiyu-coder/Kronos/issues/387) · [#307](https://github.com/shiyu-coder/Kronos/issues/307) · [#84](https://github.com/shiyu-coder/Kronos/issues/84) · [#199](https://github.com/shiyu-coder/Kronos/issues/199) · [#294](https://github.com/shiyu-coder/Kronos/issues/294) · [#41](https://github.com/shiyu-coder/Kronos/issues/41) · [#386](https://github.com/shiyu-coder/Kronos/issues/386) · [Kinlay commentary](https://jonathankinlay.com/2026/02/time-series-foundation-models-for-financial-markets-kronos-and-the-rise-of-pre-trained-market-models/)
