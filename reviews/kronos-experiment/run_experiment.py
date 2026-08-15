"""Kill criterion: does Kronos path dispersion beat trailing RV and GARCH(1,1)
at predicting next-window realized volatility, out of sample?

METHOD NOTES (the things that decide whether this result means anything)

* Out of sample. Kronos pretraining cutoff is June 2024. Every forecast window
  here begins on or after 2024-08-01, so no target period was in training.
* Non-overlapping horizons. Step == horizon, so consecutive observations do not
  share days. Overlapping windows are serially dependent and would inflate
  significance — the standard way financial backtests fool themselves.
* No lookahead. GARCH and trailing RV are fitted only on returns strictly
  before the forecast window. Kronos normalises on its own lookback internally.
* Paths come from the SHIPPED api. predict_batch averages over sample_count;
  with sample_count=1 that is the identity, so N batched copies give N
  independent sampled paths without patching the library.
* Scale-free scoring. Kronos dispersion has no reason to be calibrated in
  level, so the primary tests are rank correlation and an encompassing
  regression, both of which are indifferent to a constant scale factor.
  Penalising it on raw QLIKE would be a strawman.
"""
import sys, os, time, warnings, json
warnings.filterwarnings("ignore")
sys.path.insert(0, "Kronos")

import numpy as np, pandas as pd, torch, yfinance as yf

TICKERS  = ["SPY", "AAPL", "MSFT", "NVDA", "AMD", "TSLA"]
LOOKBACK = 400          # < 512 max_context
HORIZON  = 5            # trading days
NPATHS   = 30
OOS_FROM = "2024-08-01" # after Kronos' June-2024 cutoff
OUT      = "results.csv"

def realized_vol(logrets):
    """Realized volatility over a window: sqrt of summed squared log returns."""
    return float(np.sqrt(np.sum(np.square(logrets))))

def garch_forecast(rets_pct, horizon):
    """GARCH(1,1) h-step cumulative vol, fitted only on the supplied history."""
    from arch import arch_model
    try:
        am = arch_model(rets_pct, vol="Garch", p=1, q=1, dist="normal", mean="Constant")
        res = am.fit(disp="off", show_warning=False)
        f = res.forecast(horizon=horizon, reindex=False)
        var_path = f.variance.values[-1]          # per-day variance, in pct^2
        return float(np.sqrt(var_path.sum())) / 100.0
    except Exception:
        return np.nan

def main():
    dev = "mps" if torch.backends.mps.is_available() else "cpu"
    from model import Kronos, KronosTokenizer, KronosPredictor
    tok = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base")
    mdl = Kronos.from_pretrained("NeoQuasar/Kronos-small")
    pred = KronosPredictor(mdl, tok, device=dev, max_context=512)
    print(f"[setup] Kronos-small on {dev}", flush=True)

    rows, t_start = [], time.time()

    for tk in TICKERS:
        df = yf.download(tk, start="2021-01-01", end="2026-08-14",
                         interval="1d", auto_adjust=False, progress=False)
        if isinstance(df.columns, pd.MultiIndex):
            df.columns = df.columns.get_level_values(0)
        df = df.dropna()
        close = df["Close"].values.astype(float)
        logret = np.diff(np.log(close), prepend=np.nan)

        oos_start = df.index.searchsorted(pd.Timestamp(OOS_FROM))
        first = max(LOOKBACK, oos_start)
        starts = list(range(first, len(df) - HORIZON, HORIZON))   # non-overlapping
        print(f"[{tk}] {len(df)} bars, {len(starts)} non-overlapping windows "
              f"from {df.index[first].date()}", flush=True)

        for n, i in enumerate(starts):
            w = df.iloc[i - LOOKBACK:i]
            ctx = pd.DataFrame({
                "open": w["Open"].values, "high": w["High"].values,
                "low": w["Low"].values, "close": w["Close"].values,
                "volume": w["Volume"].values.astype(float),
            }).astype(float)
            x_ts = pd.Series(w.index)
            y_ts = pd.Series(df.index[i:i + HORIZON])
            anchor = float(w["Close"].iloc[-1])

            try:
                out = pred.predict_batch(
                    [ctx] * NPATHS, [x_ts] * NPATHS, [y_ts] * NPATHS,
                    pred_len=HORIZON, T=1.0, top_p=0.9,
                    sample_count=1, verbose=False)
            except Exception as e:
                print(f"  [{tk} {n}] predict failed: {e}", flush=True)
                continue

            paths = np.array([o["close"].values for o in out], dtype=float)  # (N, H)
            if not np.isfinite(paths).all():
                continue

            # Kronos predictor of vol: dispersion of terminal log return across paths.
            term_lr = np.log(paths[:, -1] / anchor)
            k_disp  = float(np.std(term_lr, ddof=1))
            # Secondary: mean across paths of each path's own realized vol.
            path_lr = np.diff(np.log(np.column_stack([np.full(NPATHS, anchor), paths])), axis=1)
            k_pathvol = float(np.mean(np.sqrt(np.sum(np.square(path_lr), axis=1))))
            # Directional, recorded to test it too (expected to be worthless).
            k_dir = float(np.mean(term_lr))

            hist = logret[max(1, i - 500):i]
            hist = hist[np.isfinite(hist)]
            rv20 = float(np.std(hist[-20:], ddof=1) * np.sqrt(HORIZON))
            rv60 = float(np.std(hist[-60:], ddof=1) * np.sqrt(HORIZON))
            lam, ew = 0.94, np.var(hist[-100:])
            for r in hist[-100:]:
                ew = lam * ew + (1 - lam) * r * r
            ewma = float(np.sqrt(ew * HORIZON))
            garch = garch_forecast(pd.Series(hist[-750:]) * 100.0, HORIZON)

            fwd = logret[i:i + HORIZON]
            if not np.isfinite(fwd).all():
                continue
            rv_next = realized_vol(fwd)
            fwd_ret = float(np.log(close[i + HORIZON - 1] / anchor))

            rows.append(dict(ticker=tk, date=str(df.index[i].date()),
                             kronos_disp=k_disp, kronos_pathvol=k_pathvol,
                             kronos_dir=k_dir, rv20=rv20, rv60=rv60,
                             ewma=ewma, garch=garch,
                             realized_vol=rv_next, fwd_ret=fwd_ret, anchor=anchor))

            if n and n % 20 == 0:
                el = time.time() - t_start
                print(f"  [{tk}] {n}/{len(starts)}  ({len(rows)} rows, {el/60:.1f} min)",
                      flush=True)

        pd.DataFrame(rows).to_csv(OUT, index=False)   # checkpoint per ticker

    pd.DataFrame(rows).to_csv(OUT, index=False)
    print(f"[done] {len(rows)} forecasts in {(time.time()-t_start)/60:.1f} min -> {OUT}",
          flush=True)

if __name__ == "__main__":
    main()
