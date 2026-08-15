"""Score the kill criterion.

Question: does Kronos path dispersion beat trailing realized vol and GARCH(1,1)
at predicting next-window realized volatility, out of sample?

Two tests, because "beat" has two honest readings:
  1. STANDALONE — is its rank correlation with realized vol as good or better?
  2. INCREMENTAL — does it add information a GARCH user does not already have?
     A predictor can lose standalone and still earn its place by adding signal.
     Losing both is the clean kill.

Scale-free by construction. Kronos dispersion has no reason to be calibrated in
level, so Spearman (rank) and the encompassing regression (fits a coefficient)
are used rather than raw QLIKE, which would punish a constant scale factor and
be a strawman. Calibrated QLIKE is reported as a secondary read.
"""
import numpy as np, pandas as pd
from scipy import stats
import statsmodels.api as sm

df = pd.read_csv("results.csv")
df = df.replace([np.inf, -np.inf], np.nan)
need = ["kronos_disp", "kronos_pathvol", "rv20", "rv60", "ewma", "garch", "realized_vol"]
df = df.dropna(subset=need)
df = df[(df[need] > 0).all(axis=1)]

print("=" * 74)
print(f"N = {len(df)} non-overlapping forecasts   tickers: {df.ticker.nunique()}"
      f"   {df.date.min()} .. {df.date.max()}")
print("=" * 74)

PREDS = {
    "Kronos dispersion": "kronos_disp",
    "Kronos path-vol":   "kronos_pathvol",
    "Trailing RV(20)":   "rv20",
    "Trailing RV(60)":   "rv60",
    "EWMA(0.94)":        "ewma",
    "GARCH(1,1)":        "garch",
}

# ---- 1. standalone rank skill -------------------------------------------
print("\n1. STANDALONE — Spearman rank correlation with next-window realized vol")
print(f"   {'predictor':<20} {'rho':>7} {'p':>10}   95% CI")
res = {}
n = len(df)
for name, col in PREDS.items():
    rho, p = stats.spearmanr(df[col], df.realized_vol)
    # Fisher z CI for a rank correlation
    z, se = np.arctanh(rho), 1 / np.sqrt(n - 3)
    lo, hi = np.tanh(z - 1.96 * se), np.tanh(z + 1.96 * se)
    res[name] = rho
    star = "  <-- Kronos" if "Kronos" in name else ""
    print(f"   {name:<20} {rho:>7.3f} {p:>10.2e}   [{lo:.3f}, {hi:.3f}]{star}")

best_base = max(res["Trailing RV(20)"], res["Trailing RV(60)"],
                res["EWMA(0.94)"], res["GARCH(1,1)"])
print(f"\n   best baseline rho = {best_base:.3f}"
      f"   |   Kronos dispersion rho = {res['Kronos dispersion']:.3f}")

# Steiger test: are two correlated correlations different?
def steiger(r_jk, r_jh, r_kh, n):
    rbar = (r_jk + r_jh) / 2
    detR = 1 - r_jk**2 - r_jh**2 - r_kh**2 + 2 * r_jk * r_jh * r_kh
    t = (r_jk - r_jh) * np.sqrt(((n - 1) * (1 + r_kh)) /
        ((2 * (n - 1) / (n - 3)) * detR + rbar**2 * (1 - r_kh)**3))
    return t, 2 * (1 - stats.t.cdf(abs(t), n - 3))

r_kh = stats.spearmanr(df.kronos_disp, df.garch).correlation
t, p = steiger(res["Kronos dispersion"], res["GARCH(1,1)"], r_kh, n)
print(f"   Steiger test, Kronos vs GARCH: t={t:.2f}, p={p:.3g}"
      f"   (corr between the two predictors = {r_kh:.3f})")

# ---- 2. incremental information -----------------------------------------
print("\n2. INCREMENTAL — does Kronos add anything beyond GARCH?")
print("   log(realized) ~ log(GARCH) + log(Kronos dispersion), HC3 errors")
y = np.log(df.realized_vol.values)
X = sm.add_constant(np.column_stack([np.log(df.garch.values),
                                     np.log(df.kronos_disp.values)]))
m = sm.OLS(y, X).fit(cov_type="HC3")
for nm, b, se, p in zip(["const", "log(GARCH)", "log(Kronos)"],
                        m.params, m.bse, m.pvalues):
    print(f"     {nm:<14} beta={b:>7.3f}  se={se:.3f}  p={p:.3g}")
print(f"     R^2 = {m.rsquared:.4f}")

m0 = sm.OLS(y, sm.add_constant(np.log(df.garch.values))).fit(cov_type="HC3")
print(f"     GARCH alone R^2 = {m0.rsquared:.4f}"
      f"   ->  Kronos adds {m.rsquared - m0.rsquared:+.4f}")

mk = sm.OLS(y, sm.add_constant(np.log(df.kronos_disp.values))).fit(cov_type="HC3")
print(f"     Kronos alone R^2 = {mk.rsquared:.4f}")

# ---- 3. calibrated QLIKE + Diebold-Mariano ------------------------------
print("\n3. CALIBRATED QLIKE (scale fitted per predictor, so level bias is not penalised)")
print("   lower is better; DM tests Kronos against each baseline")
rv2 = df.realized_vol.values ** 2

def qlike(fvar, rvar):
    r = rvar / fvar
    return r - np.log(r) - 1

losses = {}
for name, col in PREDS.items():
    f = df[col].values ** 2
    k = np.mean(rv2 / f)          # single scale factor, fitted in-sample
    losses[name] = qlike(f * k, rv2)
    print(f"   {name:<20} {np.mean(losses[name]):.4f}")

print()
for base in ["Trailing RV(20)", "EWMA(0.94)", "GARCH(1,1)"]:
    d = losses["Kronos dispersion"] - losses[base]
    dm = d.mean() / (d.std(ddof=1) / np.sqrt(len(d)))
    p = 2 * (1 - stats.norm.cdf(abs(dm)))
    verdict = "Kronos better" if d.mean() < 0 else "Kronos WORSE"
    sig = "significant" if p < 0.05 else "not significant"
    print(f"   Kronos vs {base:<18} dQLIKE={d.mean():+.4f}  DM={dm:+.2f}  p={p:.3g}"
          f"  -> {verdict}, {sig}")

# ---- 4. the directional claim, tested so it is on the record ------------
print("\n4. DIRECTIONAL (recorded to test the claim we expect to fail)")
hit = np.mean(np.sign(df.kronos_dir.values) == np.sign(df.fwd_ret.values))
se = np.sqrt(hit * (1 - hit) / len(df))
p = stats.binomtest(int(hit * len(df)), len(df), 0.5).pvalue
print(f"   directional accuracy = {hit*100:.1f}% +/- {se*196:.1f}pp (95%), "
      f"vs 50% chance p={p:.3g}")
rho_d, p_d = stats.spearmanr(df.kronos_dir, df.fwd_ret)
print(f"   Spearman(predicted return, realized return) = {rho_d:.3f} (p={p_d:.3g})")

# ---- 5. per ticker, to check the pooled result is not one name ----------
print("\n5. PER TICKER — Spearman with realized vol (guards against one name driving it)")
print(f"   {'ticker':<8} {'n':>4} {'Kronos':>8} {'GARCH':>8} {'RV20':>8}")
for tk, g in df.groupby("ticker"):
    if len(g) < 10:
        continue
    rk = stats.spearmanr(g.kronos_disp, g.realized_vol).correlation
    rg = stats.spearmanr(g.garch, g.realized_vol).correlation
    rr = stats.spearmanr(g.rv20, g.realized_vol).correlation
    print(f"   {tk:<8} {len(g):>4} {rk:>8.3f} {rg:>8.3f} {rr:>8.3f}")

# ---- verdict -------------------------------------------------------------
print("\n" + "=" * 74)
beats_standalone = res["Kronos dispersion"] >= best_base
adds_incremental = m.pvalues[2] < 0.05 and m.params[2] > 0
print(f"KILL CRITERION")
print(f"  beats best baseline standalone : {'YES' if beats_standalone else 'NO'}"
      f"  ({res['Kronos dispersion']:.3f} vs {best_base:.3f})")
print(f"  adds incremental info vs GARCH : {'YES' if adds_incremental else 'NO'}"
      f"  (beta={m.params[2]:+.3f}, p={m.pvalues[2]:.3g})")
print(f"  VERDICT: {'PASSES — build the sidecar' if (beats_standalone or adds_incremental) else 'FAILS — do not build'}")
print("=" * 74)
