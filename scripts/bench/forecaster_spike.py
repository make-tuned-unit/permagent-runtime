"""TimesFM 2.5 runs-on-Mac spike for The Forecaster.

RUN 2026-08-24 on Apple Silicon (arm64, 16 GB). VERDICT: RUNS.

  python 3.12.13 (uv-managed) | torch 2.13.0 | timesfm 2.0.2 (pypi) | numpy 2.5.2
  checkpoint google/timesfm-2.5-200m-pytorch, 882 MB on disk; venv 600 MB

  DEVICE            cpu. torch.cuda.is_available()=False, mps IS available but
                    timesfm never asks: timesfm_2p5/timesfm_2p5_torch.py:72-77 is
                    `if cuda: cuda else: cpu`, no MPS branch. Confirms the
                    UNLAZY_TIMESFM_DEEPAGENTS_2026-08-24.md sec.3 source read.
  COLD START        51.6 s (includes the 882 MB HF download)
  WARM START        2.06 s total = 0.70 s import + 1.36 s from_pretrained
  compile()         0.00 s (lazy; cost is paid inside the first forecast)
  LATENCY  n=256  H=28   first 393 ms, warm median 615 ms, warm min 385 ms
           n=1024 H=21   first 518 ms, warm median 189 ms, warm min 172 ms
  BATCH    12 x n=512 H=21 -> 2135 ms total = 178 ms/series
  PEAK RSS          1.93 GB warm / 2.17 GB including the download
  torch threads     4
  OUTPUT SHAPES     point (N, H) float32; quantile (N, H, 10)

  BACKTEST (this is why the design gates TimesFM on a backtest, not on length)
    synthetic 256 pts, clean weekly seasonality, H=28 holdout:
      MAE timesfm 1.218 vs seasonal-naive 6.317 -> MASE 0.193   TIMESFM WINS
    NVDA 1024 daily closes (Yahoo, the same endpoint as market_data.rs:36), H=21:
      MAE timesfm 12.623 vs naive-last 11.434  -> MASE 1.104    BASELINE WINS

  A longer series is not a better series. Length is necessary, never sufficient.

Reproduce:
  uv venv --python 3.12 ./forecaster-venv
  uv pip install --python ./forecaster-venv/bin/python 'timesfm[torch]'
  ./forecaster-venv/bin/python scripts/bench/forecaster_spike.py

Note: `uv venv --python /opt/homebrew/bin/python3.12` FAILS here -- Homebrew's
CPython returns an empty platform.mac_ver(), exactly the breakage documented in
crates/goose/src/python_runtime.rs:3-6. Ask uv for a managed interpreter instead.
"""
import json, os, resource, sys, time, urllib.request
import numpy as np

def rss_gb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / (1024**3)  # macOS: bytes

MARKS = {}
def mark(name):
    MARKS[name] = (time.perf_counter(), rss_gb())
    print(f"[{name}] t={MARKS[name][0]:.2f}s peak_rss={MARKS[name][1]:.2f} GB", flush=True)

t_import0 = time.perf_counter()
import torch, timesfm
t_import = time.perf_counter() - t_import0
print(f"versions: python={sys.version.split()[0]} torch={torch.__version__} numpy={np.__version__}")
print(f"import timesfm+torch: {t_import:.2f}s")
print(f"cuda_available={torch.cuda.is_available()} mps_available={torch.backends.mps.is_available()}")

# ---- load ----
t0 = time.perf_counter()
model = timesfm.TimesFM_2p5_200M_torch.from_pretrained("google/timesfm-2.5-200m-pytorch")
t_load = time.perf_counter() - t0
print(f"LOAD from_pretrained: {t_load:.2f}s  rss={rss_gb():.2f} GB")
print(f"model device: {getattr(model, 'device', 'n/a')}")

t0 = time.perf_counter()
model.compile(timesfm.ForecastConfig(
    max_context=1024, max_horizon=256,
    normalize_inputs=True, use_continuous_quantile_head=True,
))
t_compile = time.perf_counter() - t0
print(f"COMPILE: {t_compile:.2f}s  rss={rss_gb():.2f} GB")

def run(label, series, horizon):
    x = np.asarray(series, dtype=np.float32)
    lat = []
    for i in range(4):
        t = time.perf_counter()
        point, quant = model.forecast(horizon=horizon, inputs=[x])
        lat.append(time.perf_counter() - t)
        if i == 0:
            print(f"  {label}: point.shape={np.shape(point)} quantile.shape={np.shape(quant)}")
    print(f"  {label}: n={len(x)} H={horizon} first={lat[0]*1000:.0f}ms "
          f"warm_median={np.median(lat[1:])*1000:.0f}ms warm_min={min(lat[1:])*1000:.0f}ms")
    return np.asarray(point)[0], np.asarray(quant)[0], lat

# ---- (a) synthetic 256 points ----
print("\n=== (a) synthetic 256-point series ===")
rng = np.random.default_rng(7)
t = np.arange(256)
syn = 100 + 0.35*t + 6*np.sin(2*np.pi*t/7) + rng.normal(0, 1.5, 256)
p_syn, q_syn, lat_syn = run("synthetic", syn, 28)
print(f"  last obs={syn[-1]:.2f}  forecast[0]={p_syn[0]:.2f} forecast[-1]={p_syn[-1]:.2f}")
print(f"  q[0] (h=1, 10 cols) = {np.round(q_syn[0], 2).tolist()}")
# seasonal-naive baseline (period 7) backtest on last 28
def seasonal_naive(y, h, m=7):
    return np.array([y[-m + (i % m)] for i in range(h)])
hold = 28
ctx, truth = syn[:-hold], syn[-hold:]
p_bt, _ = model.forecast(horizon=hold, inputs=[ctx.astype(np.float32)])
p_bt = np.asarray(p_bt)[0]
sn = seasonal_naive(ctx, hold)
mae_tfm, mae_sn = np.mean(np.abs(p_bt-truth)), np.mean(np.abs(sn-truth))
print(f"  BACKTEST hold={hold}: MAE timesfm={mae_tfm:.3f}  seasonal_naive={mae_sn:.3f}  "
      f"MASE_vs_sn={mae_tfm/mae_sn:.3f} ({'timesfm wins' if mae_tfm<mae_sn else 'baseline wins'})")

# ---- (b) real series: Yahoo daily closes (same endpoint as crates/goose/src/market_data.rs:36) ----
print("\n=== (b) real series: Yahoo daily closes ===")
SYM = os.environ.get("SPIKE_SYMBOL", "NVDA")
url = (f"https://query1.finance.yahoo.com/v8/finance/chart/{SYM}"
       f"?range=5y&interval=1d")
req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"})
t0 = time.perf_counter()
body = json.loads(urllib.request.urlopen(req, timeout=20).read())
t_fetch = time.perf_counter() - t0
res = body["chart"]["result"][0]
closes = [c for c in res["indicators"]["quote"][0]["close"] if c is not None]
print(f"  fetched {SYM}: {len(closes)} daily closes in {t_fetch:.2f}s "
      f"(last={closes[-1]:.2f})")
real = np.array(closes[-1024:], dtype=np.float32)
p_real, q_real, lat_real = run(f"{SYM}", real, 21)
print(f"  last obs={real[-1]:.2f}  forecast h=1..5 = {np.round(p_real[:5],2).tolist()}")
print(f"  q[20] (h=21) p10..p90 = {np.round(q_real[20], 2).tolist()}")

hold = 21
ctx, truth = real[:-hold], real[-hold:]
p_bt, _ = model.forecast(horizon=hold, inputs=[ctx])
p_bt = np.asarray(p_bt)[0]
sn = np.full(hold, ctx[-1])  # random-walk / naive is the right baseline for prices
mae_tfm, mae_sn = np.mean(np.abs(p_bt-truth)), np.mean(np.abs(sn-truth))
print(f"  BACKTEST hold={hold}: MAE timesfm={mae_tfm:.3f}  naive(last)={mae_sn:.3f}  "
      f"MASE_vs_naive={mae_tfm/mae_sn:.3f} ({'timesfm wins' if mae_tfm<mae_sn else 'baseline wins'})")

# ---- batching ----
print("\n=== batch throughput (12 series at once) ===")
batch = [real[-512:] for _ in range(12)]
t = time.perf_counter(); model.forecast(horizon=21, inputs=batch); t_batch = time.perf_counter()-t
print(f"  12 series x H=21: {t_batch*1000:.0f}ms total  -> {t_batch/12*1000:.0f}ms/series")

print(f"\n=== PEAK RSS: {rss_gb():.2f} GB ===")
print(f"=== load={t_load:.2f}s compile={t_compile:.2f}s ===")
