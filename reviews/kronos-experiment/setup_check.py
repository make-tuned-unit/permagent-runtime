"""Smoke test: model loads, data downloads, and we can get genuine per-path output.

The path extraction is the load-bearing trick, so it is validated here rather
than assumed. `predict_batch` averages over sample_count internally; with
sample_count=1 that average is the identity, so N batched copies of the same
context yield N independent sampled paths through the SHIPPED api, with no
patch to the library that could be subtly wrong.
"""
import sys, warnings
warnings.filterwarnings("ignore")
sys.path.insert(0, "Kronos")

import numpy as np, pandas as pd, torch

print(f"torch {torch.__version__}  mps={torch.backends.mps.is_available()}")

# ---- data ---------------------------------------------------------------
import yfinance as yf
df = yf.download("AAPL", start="2023-01-01", end="2026-08-14",
                 interval="1d", auto_adjust=False, progress=False)
if isinstance(df.columns, pd.MultiIndex):
    df.columns = df.columns.get_level_values(0)
print(f"AAPL rows={len(df)}  {df.index[0].date()} .. {df.index[-1].date()}")
if len(df) < 600:
    print("FAIL: not enough data"); sys.exit(1)

# ---- model --------------------------------------------------------------
from model import Kronos, KronosTokenizer, KronosPredictor
tok = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base")
mdl = Kronos.from_pretrained("NeoQuasar/Kronos-small")
dev = "mps" if torch.backends.mps.is_available() else "cpu"
pred = KronosPredictor(mdl, tok, device=dev, max_context=512)
print(f"model loaded on {dev}")

# ---- one window ---------------------------------------------------------
LOOKBACK, HORIZON, NPATHS = 400, 5, 16
w = df.iloc[-(LOOKBACK + HORIZON):-HORIZON]
ctx = pd.DataFrame({
    "open": w["Open"].values, "high": w["High"].values,
    "low": w["Low"].values, "close": w["Close"].values,
    "volume": w["Volume"].values.astype(float),
}).astype(float)
x_ts = pd.Series(w.index)
y_ts = pd.Series(df.index[-HORIZON:])

import time
t0 = time.time()
out = pred.predict_batch(
    [ctx] * NPATHS, [x_ts] * NPATHS, [y_ts] * NPATHS,
    pred_len=HORIZON, T=1.0, top_p=0.9, sample_count=1, verbose=False)
el = time.time() - t0

paths = np.array([o["close"].values for o in out])   # (NPATHS, HORIZON)
print(f"paths shape {paths.shape}   {el:.1f}s for {NPATHS} paths ({el/NPATHS:.2f}s/path)")

# The whole experiment depends on these paths being genuinely DIFFERENT.
# If sampling were collapsed (T too low, or a bug), dispersion would be ~0 and
# every downstream number would be meaningless-but-plausible.
last = paths[:, -1]
spread = last.std() / last.mean()
print(f"terminal close: mean={last.mean():.2f} std={last.std():.4f} cv={spread:.5f}")
print(f"distinct terminal values: {len(np.unique(np.round(last,4)))}/{NPATHS}")
if len(np.unique(np.round(last, 4))) < 3:
    print("FAIL: paths are not distinct — sampling is collapsed, dispersion is not real")
    sys.exit(1)

anchor = ctx["close"].iloc[-1]
realized = df["Close"].iloc[-HORIZON:].values
print(f"anchor={anchor:.2f}  realized_final={realized[-1]:.2f}  path_mean_final={last.mean():.2f}")
print("OK")
