# The Financier: validation against regulation and practice

Written 2026-08-15. Companion to `05-kronos-validation.md`. Base case: single operator, Atlantic Canada, local-first, extension `default_enabled: false`. **HARD** = legal requirement, **PRUDENT** = convention, **UNSETTLED** = no consensus exists.

---

## 1. The headline correction: the boundary clause guards the wrong perimeter

The Financier's descriptor currently ends:

> "Reports numbers; never sizes a position and cannot place an order."

All three clauses are good. As a *regulatory* boundary, two of them are aimed at the wrong thing:

| Clause | Perimeter it actually guards |
|---|---|
| "cannot place an order" | **Dealer/broker** registration and discretionary authority — a *different* perimeter from advice |
| "never sizes a position" | On-point: sizing is where tailoring lives |
| "reports numbers" | Weak — reporting numbers about *securities the tool selected for this user* is already the interesting case |

**The advice perimeter turns on TAILORING, not on order placement and not on naming securities.**

**NI 31-103 s.8.25 "Advising generally"** (the Atlantic Canada position) exempts advice that "does not purport to be tailored to the needs of the person receiving it" — and **s.8.25(3) explicitly contemplates recommending a "specified security"** under that exemption. Naming tickers is not the line. Two conditions attach:

- If the operator or its officers hold a recommended security, that interest must be disclosed **concurrently with the advice** (s.8.25(3)–(4)) — same output, not a footer. **HARD, if ever distributed.**
- **s.8.25(5): the section does not apply in Ontario.** Do not assume it if this ever ships to Ontario users.

**UK PERG 8.30B is the sharpest guidance anywhere on screeners specifically**, and it maps almost exactly onto `picker_*`:
- **8.30B.28G** — filtering on a *single* factor is generally not a personal recommendation.
- **8.30B.33G** — **multi-factor filtering requiring substantial customer input about their circumstances can be** a personal recommendation.
- **8.30B.15G(1)** — output presented as circumstance-based is a personal recommendation **even if the firm never used the information**, provided the user reasonably expected it to.

That last one is the design-relevant one: *appearing* to use the user's holdings is enough.

### The clause that is missing

> *"Screens on instrument attributes only. Never reads the user's holdings, trade history, or financial circumstances as screener input, and never infers risk tolerance, objectives, or suitability."*

Suggested full replacement:

> *"Reports timestamped market numbers and runs the user's own screening criteria. Screens on instrument attributes only — never on the user's holdings, circumstances, or risk tolerance. Does not size positions, does not assess suitability, cannot place an order."*

**`picker_top_picks` is the highest-risk surface in the extension — not `record_trade`.** A ranked list can read as tailored without anything ever being sized or ordered.

### Where the risk actually lives

Zero today. It becomes non-zero along exactly three axes: **distribution to others**, **compensation**, and **tailoring**. In all three jurisdictions the perimeter tests are conjunctive and self-use fails several at once (US Advisers Act §202(a)(11) requires advising *others*, *for compensation*, *in the business*; Canada's business trigger, 31-103CP s.1.3; UK RAO Art. 53).

**Caveat on a widely-repeated claim:** the "inanimate tool applying objective, non-discretionary criteria" formulation attributed to SEC no-action letters could not be verified against a primary source — both hits traced to the same secondary page. **Treat as practitioner lore, not authority.** The verifiable US position is IM Guidance Update 2017-02: automation is irrelevant; personalised ongoing management is advisory regardless of what performs it.

---

## 2. Design changes, ordered by value

1. **Firewall `record_trade`/`list_trades` from `picker_scan`/`picker_top_picks` at the type level.** This is the single highest-value architectural rule: it makes "not tailored" true *by construction* rather than by prompt discipline. Per PERG 8.30B.15G(1) the firewall must also be *visible*, since appearing to use holdings is enough.
2. **Split `resolve_symbol` from `research_ticker`.** Never let the model emit a ticker — measured ~10% error rate from parametric memory. Return `{symbol, MIC, share_class, FIGI/CUSIP, is_delisted, effective_from/to}`; ask on ambiguity, never guess. Symbols are exchange-scoped and reused after delisting (`V` was Vivendi before Visa) and spelled inconsistently across vendors (`BRK.B`/`BRK-B`/`BRK/B`). **Key the trade ledger on a stable ID + symbol-as-of-date; symbol is a display label, never a key.**
3. **Make the quote type impossible to render without provenance:** `{value, currency, as_of (exchange-local + UTC), received_at, source, delay_seconds, market_state, is_stale, session}` with `market_state ∈ {pre, regular, post, closed, halted, holiday, unknown}`. **Compute `is_stale` at read time, not fetch time** — a quote cached at 10:00 and rendered at 14:00 is stale. **Fail closed on `unknown`**; never infer session from wall-clock (holidays, half-days, DST break it several times a year). During close it must say **"previous close"**, never "current price."
4. **Add a deterministic numeric verification pass.** Extract every price/volume-shaped number from a response; verify it appears verbatim in a tool result *from this turn*; regenerate or strike otherwise. Highest value per line of code in this list — LLM guardrail frameworks are "fast and deterministic but logically shallow" and **will not catch a wrong number that is well-formed.**
5. **Store raw OHLCV as system of record** (immutable) plus a separate **corporate-action table**; compute adjustments on demand. Tag every series with `adjustment ∈ {none, split_only, split_and_dividend, total_return}` and **make mixing enums a type error.** On each trade store `price_unadjusted`, `cumulative_adjustment_factor_at_entry`, `data_source`, `retrieved_at`.
6. **Explicit refusal path** for "should I buy / how much / is this right for me." Worth more than any disclaimer — a disclaimer does not convert advice into non-advice; the activity is characterised by what it does.

### Data licensing, quietly

Nasdaq's Display Requirements Policy requires a delay message on **all displays of delayed data — including audio announcements on voice-response services** — prominently, at or near the top. **A conversational agent reading a quote aloud is a display surface.** Whether this binds purely personal non-redistributed use is a **vendor-contract question, not settled by exchange policy** — but label anyway; it costs nothing. Note also that the descriptor's "no setup, no key" boast implies an unlicensed feed: low exposure personally, materially higher if redistributed.

---

## 3. Why the sidecar must be volatility-only — arrived at twice, independently

`05-kronos-validation.md` found Kronos has no independently-verified directional edge, inverted calibration (Brier 0.298 vs 0.250 for a constant 50%), and one positive external result: **path dispersion correlates with realized volatility (Spearman +0.375).**

This research reached the same place from the forecasting literature, without reference to Kronos:

> **The three quantities are not equally forecastable.** Direction ≈ chance. Magnitude ≈ the random-walk case. **Volatility/range is genuinely forecastable** at short horizons (GARCH/realized-vol), decaying quickly with horizon.

Supporting: professional 12-month price targets run ~54% directional and ~30% strict hit rate; Goyal & Welch (RFS 2008; Goyal/Welch/Zafirov RFS 2024) find essentially all equity-premium predictors fail to beat the historical mean out of sample, with over a third failing *in*-sample post-2008.

**Two independent lines of evidence converging on "range, not level" is the strongest signal in either report.** It is also the framing hardest to misread as advice — which resolves the regulatory question and the accuracy question with the same design decision.

---

## 4. If the sidecar ships: presentation rules

Empirically established:
- **Deterministic construal error** (Padilla et al.): lay readers *and professional emergency managers* read a widening hurricane cone as the object getting bigger, not the forecast getting less certain. **A drawn boundary invites a containment reading.** A shaded price fan will be read as "it'll be in the shaded region, and the line is the plan."
- **Showing uncertainty barely costs trust** (van der Bles et al., PNAS 2020, n=5,780 incl. a BBC field experiment) — but **verbal hedging is the format that costs trust.** Numbers, not adverbs.
- **A central line biases readers downward on uncertainty** (Kale/Hullman/Kay). The original Bank of England fan chart deliberately had none.
- **Countable outcomes beat bands** (Fernandes et al., CHI 2018): 50-quantile dotplots reached 97% of optimal expected payoff, +5pp over no-uncertainty control. **Textual uncertainty was the weakest format.**
- **Verbal terms need inline numbers** (Wintle et al., PLOS ONE 2019, n=924): unguided, intended-vs-interpreted correspondence is **32%**; with an inline numeric range ("unlikely (10–25%)") it is **66%**. Tooltips are roughly half as effective as inline.
- **Frequency framing** ("3 in 10" over "30%") helps most for low-numeracy readers. Always give the reference class.

**UNSETTLED — fan charts.** The Bernanke Review (2024) told the Bank of England to drop them ("weak conceptual foundations") and it did, moving to explicit scenarios; BIS Quarterly (Mar 2026) documents a sector-wide shift. But the Bank's own 2026 comprehension experiment (n=1,600) found fan charts well understood and, critically, that **point forecasts de-anchor expectations badly when an error lands while fan charts mitigate it.** Reconciliation: fans convey *magnitude* fine and *structure of risk* badly. **Never use a fan or interval over a bimodal outlook** (pending earnings, rate decision) — it puts its densest region on the least likely outcome. Use two named scenarios with probabilities.

**Never show:** a bare point target as headline; a shaded band with a hard edge and a bold central line; a bare verbal probability; a calibration curve on <30 resolved forecasts; a fan over a bimodal outlook; skill without the naive baseline in the same view.

**Always show:** a quantile dotplot (50 dots; 20 on small screens) or raw ensemble paths; a frequency gloss with explicit denominator; inline numeric ranges on verbal terms; **the naive baseline series and a signed skill score allowed to render negative**; data cutoff and horizon adjacent to the number.

**Log per forecast, immutably:** issue time, data cutoff, machine-checkable target, horizon, ≥9 predictive quantiles, **the baseline's distribution on the same data**, model/prompt version hash, realized outcome, and CRPS + log score + Brier for **both model and baseline**. **CRPS as headline** (proper, distance-sensitive, reduces to MAE so point and distributional forecasts score on one axis).

**On sample size, the honest answer is you probably cannot claim anything.** SE = √(p(1−p)/n): at p=0.5, n=25 → ±10pp, n=100 → ±5pp — and overlapping horizons are serially dependent, so daily 30-day forecasts have effective n nearer 12/year than 365. **Gate the UI: <30 resolved → log only, no curve, no claim; 30–100 → scores with CIs and an explicit "too few to assess calibration"; >100 → reliability diagram with Bröcker–Smith consistency bars and bin counts, still hedged.**

### Projections are the most regulated content class here — HARD, if distributed
- **SEC Marketing Rule 206(4)-1(d)(6)** treats model performance, back-tests and **projected returns** as "hypothetical performance", permitted only with policies ensuring relevance to the audience's financial situation — read by industry as effectively unsatisfiable for mass retail.
- **FINRA 2210(d)(1)(F)** flatly prohibits projecting performance, excepting Rule 2214 investment analysis tools, whose mandated language is reusable verbatim.
- **FCA COBS 4.5A.14R:** future performance must rest on reasonable assumptions supported by objective data and show **both negative and positive** scenarios — and a firm **should not provide it at all** absent the objective data.
- **PRIIPs is the cautionary tale:** the ESAs concluded their own mandated scenarios were systematically over-optimistic and rewrote the RTS to use a ≥10-year back-test with the unfavourable scenario from the **worst observed window**, legislating that stress ≤ unfavourable. **Borrow all three: long lookback, worst-observed-window, monotonic ordering enforced in code.**
- **UNSETTLED/live:** SR-FINRA-2026-004 (filed 10 Feb 2026) would permit projections under conditions; awaiting SEC action. Not law.

---

## 5. Backtesting: the strongest argument for refusing to backtest at all

- **Survivorship:** 7.4% vs 9.0% annualized (1926–2001) — **1.6%/yr overstatement.** Shumway (1997) substitutes **−55%** for missing performance-related delisting returns; the Nasdaq bias is **4.7×** NYSE/AMEX. Free data sources are *structurally* biased — a series that simply ends omits the −100% event that is the entire point.
- **Multiple testing:** Harvey/Liu/Zhu (RFS 2016) — a new factor needs **t > 3.0**, not 2.0. Bailey & López de Prado's **Deflated Sharpe Ratio** corrects for trial count; MinBTL implies ~1,000 trials needs 20+ years of data.
- **Novy-Marx (NBER w21329) is aimed squarely at a personal screener:** combining the best k of n signals carries bias almost as large as picking the best of **n^k**. **A 3-of-10-criteria screener is effectively a 1,000-trial search.** He produces "highly significant" backtests from randomly generated signals.
- **LLM-specific look-ahead:** Sarkar & Vafa (ICML 2025) — pretrained models use post-cutoff knowledge about earlier periods **even when explicitly instructed not to**, and prompting-based mitigations are limited. Commentary explaining why a 2021 signal worked is contaminated and cannot be decontaminated.

**Therefore:** persist a **trial counter** incremented on every screener variant ever run; report **DSR and haircut Sharpe, never bare Sharpe**; **refuse to backtest on a survivorship-biased source**; and — the single most damaging thing an agent could do here — **never let the LLM iterate thresholds against backtest output.** That is automated overfitting at machine speed.

---

## 6. The audit's theme has a regulatory edge in this domain

The 2026-08 audit's central finding was descriptors claiming what the code does not deliver. In finance that stops being a hygiene problem:

**SEC charged Delphia and Global Predictions (March 2024, $225k/$175k) for claiming AI capabilities they lacked.** Delphia's advertised ability to "make predictions across thousands of publicly traded companies up to two years into the future" is uncomfortably close to how a forecasting sidecar might describe itself. **AI-washing is an enforcement priority, and SEC FY2026 exam priorities focus on whether representations match reality.**

So: whatever The Financier's self-description says, the code must actually do — for the same reason as everywhere else in this codebase, plus one more.

---

## 7. Authoritative guidance: the honest headline

**No regulator anywhere has issued rules specific to AI agents in personal finance. Every body that has spoken says existing technology-neutral rules apply unchanged.**

| Source | Date | Use |
|---|---|---|
| **FINRA 2026 Annual Regulatory Oversight Report** | Dec 2025 | Most on-point published guidance. First dedicated GenAI section; names **agent-specific** risks (autonomy, scope overreach, multi-step auditability, domain-knowledge gaps in general-purpose agents); recommends stored logs of prompts/responses/outputs, human-in-the-loop, restricted system access |
| **IOSCO Supervisory Toolkit** (FR/02/2026) | May 2026 | Global standard-setter, explicitly spans agentic AI. Supervisor-facing, non-binding |
| **CSA Staff Notice 11-348** | Dec 2024 | The Canadian position. **Creates no new requirements.** Expects explainability, human-in-the-loop, pre-deployment testing; warns against AI-washing |
| **CRI Financial Services AI RMF** | Feb 2026 | 230 control objectives on NIST AI RMF; the most implementable checklist available |

**The vacuum is real:** the SEC withdrew its Predictive Data Analytics proposal (June 2025) with no replacement; the Fed/OCC/FDIC state generative and agentic AI are **out of scope** of revised model-risk guidance (Apr 2026); **Canada has no federal AI law** — AIDA died with prorogation Jan 2025 and will not return in that form. OSFI E-23 (effective May 2027) binds federally regulated institutions only, not securities registrants.

**Direction of travel is toward capture, in the UK first:** a new regulated activity of **"targeted support"** took effect 6 April 2026 (PS25/22), and the FCA's Perimeter Report (26 Mar 2026) explicitly flags unregulated AI tools offering financial advice as a gap.

**EU:** investment advice appears **nowhere in AI Act Annex III** — a market-research tool is not high-risk. But **Art. 50(2) transparency has been in force since 2 August 2026**: a system interacting directly with natural persons must disclose it is AI. Relevant only on EU placement.

---

**Sources:** see the research appendix in the session record. Primary anchors: [NI 31-103 s.8.25](https://www.bclaws.gov.bc.ca/civix/document/id/loo94/loo94/22_226a_2009) · [CSA 11-348](https://www.osc.ca/en/securities-law/instruments-rules-policies/1/11-348/csa-staff-notice-and-consultation-11-348-applicability-canadian-securities-laws-and-use-artificial) · [PERG 8.30B](https://handbook.fca.org.uk/handbook/perg8/perg8s41) · [FINRA 2026 GenAI](https://www.finra.org/rules-guidance/guidance/reports/2026-finra-annual-regulatory-oversight-report/gen-ai) · [IOSCO FR/02/2026](https://www.iosco.org/library/pubdocs/pdf/IOSCOPD823.pdf) · [SEC AI-washing charges](https://www.sec.gov/newsroom/press-releases/2024-36) · [IM Guidance 2017-02](https://www.sec.gov/investment/im-guidance-2017-02.pdf) · [Padilla et al. 2020](https://www.frontiersin.org/journals/computer-science/articles/10.3389/fcomp.2020.590232/full) · [van der Bles et al. PNAS 2020](https://www.pnas.org/doi/abs/10.1073/pnas.1913678117) · [Fernandes et al. CHI 2018](https://idl.uw.edu/papers/uncertainty-bus) · [Wintle et al. 2019](https://journals.plos.org/plosone/article?id=10.1371%2Fjournal.pone.0213522) · [Novy-Marx NBER w21329](https://www.nber.org/papers/w21329) · [Bailey & López de Prado DSR](https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf) · [Sarkar & Vafa ICML 2025](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4754678) · [FinanceBench](https://arxiv.org/abs/2311.11944) · [Nasdaq Display Policy](https://www.nasdaqtrader.com/content/AdministrationSupport/Policy/DISPLAYREQUIREMENTSPOLICY.pdf) · [Shumway 1997](https://www.tylergshumway.org/Shumway-DelistingBiasCRSP-1997.pdf)
