//! Turning a stored series into a forecast — with the label that produced it.
//!
//! `Method` is not an `Option<String>` and not a default. It is a mandatory
//! enum on every forecast this module emits, for the same reason
//! `SpendForecast.method` is literally the string *"this is a trailing average,
//! not a model"*: a number whose provenance is unstated will be read as better
//! than it is.
//!
//! ## How a method earns its label
//!
//! Seasonal naive is the floor. Any other method — ETS here, TimesFM later —
//! is served **only** if it clears [`crate::forecaster::backtest::gate`]
//! against seasonal naive on the same rolling-origin folds: a 10% margin on the
//! median MASE *and* three quarters of the folds. Otherwise seasonal naive is
//! served, and labelled as seasonal naive. One rule, applied to every
//! candidate, so "the model was used" and "the model won" cannot come apart.
//!
//! ## When there is no forecast
//!
//! [`Refusal`] is a first-class outcome. A series under its minimum returns
//! `InsufficientHistory` carrying both numbers so the card can render "42 of
//! 180" instead of an empty chart; a series whose collector stopped returns
//! `CollectorStale`; a series that is long and fresh but on which nothing can
//! be scored returns `NoMethodBeatsBaseline`. That is `growth::power::judge`'s
//! guard order applied to a new domain.

use serde::{Deserialize, Serialize};

use super::backtest::{self, FoldScores, GateOutcome};
use super::baseline::{self, seasonal_naive};
use super::series::{Cadence, SeriesStatus};
use super::store::{self, Point, Verdict};

/// What produced the numbers. Closed, and never inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    SeasonalNaive,
    Ets,
    /// The 200M TimesFM 2.5 checkpoint, run locally on CPU.
    Timesfm,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SeasonalNaive => "seasonal_naive",
            Self::Ets => "ets",
            Self::Timesfm => "timesfm-2.5-200m",
        }
    }

    /// The label a human reads on the card. Says what it is, not what we wish
    /// it were.
    pub fn label(self) -> &'static str {
        match self {
            Self::SeasonalNaive => "seasonal naive — last week repeated, not a model",
            Self::Ets => "Holt-Winters ETS — level, damped trend, season",
            Self::Timesfm => "TimesFM 2.5 (200M, local CPU)",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "seasonal_naive" => Some(Self::SeasonalNaive),
            "ets" => Some(Self::Ets),
            "timesfm-2.5-200m" => Some(Self::Timesfm),
            _ => None,
        }
    }
}

/// Why there is no forecast. Carries the numbers the UI needs to explain it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    /// Shorter than the minimum any method may speak at.
    InsufficientHistory { points: usize, needed: usize },
    /// The collector has stopped. Stale numbers read as a flat market.
    CollectorStale {
        #[serde(rename = "lastCollectedAt")]
        last_collected_at: Option<String>,
    },
    /// Approved but not collecting, or never approved.
    NotBound,
    /// Long enough and fresh, and still nothing can be scored — a perfectly
    /// flat window makes MASE undefined, and an undefined number is not a good
    /// one.
    NoMethodBeatsBaseline { detail: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientHistory { points, needed } => write!(
                f,
                "{points} of {needed} points — too short for any method to speak"
            ),
            Self::CollectorStale { last_collected_at } => match last_collected_at {
                Some(t) => write!(f, "the collector last ran {t}"),
                None => write!(f, "this series has never been collected"),
            },
            Self::NotBound => write!(f, "this series is not approved, so nothing is collected"),
            Self::NoMethodBeatsBaseline { detail } => write!(f, "{detail}"),
        }
    }
}

/// A forecast, and how it was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forecast {
    pub series_id: String,
    pub made_at: String,
    pub horizon: usize,
    pub point: Vec<f64>,
    pub p10: Vec<f64>,
    pub p90: Vec<f64>,
    /// Mandatory. Never defaulted, never inferred from what we tried to run.
    pub method: Method,
    pub method_label: String,
    /// The candidate's median MASE as a multiple of seasonal naive's, when a
    /// backtest could be scored. `None` means "not scored", never "1.0".
    pub mase_vs_baseline: Option<f64>,
    pub folds: usize,
    pub fold_wins: usize,
    /// Why this method rather than another, in one sentence.
    pub selection: String,
}

/// TimesFM's contribution, computed elsewhere.
///
/// The model runs on the M1 over SSH, so this module never calls it — it is
/// handed the results and scores them exactly as it scores the local methods.
/// `fold_forecasts` must be in [`backtest::fold_origins`] order and the same
/// length; anything else is refused rather than compared, because the sign test
/// is paired.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteCandidate {
    pub fold_forecasts: Vec<Vec<f64>>,
    pub final_forecast: Vec<f64>,
    pub final_p10: Vec<f64>,
    pub final_p90: Vec<f64>,
}

/// The training prefix the backtest starts from, sized to leave the folds the
/// gate needs. Shared so the sweep can build the remote batch against the same
/// origins this module will score.
pub fn min_train_for(len: usize, horizon: usize, m: usize) -> usize {
    len.saturating_sub(horizon * backtest::MIN_FOLDS)
        .max(2 * m + 1)
}

/// Pick a method by backtest and produce the forecast it earns.
///
/// Pure: takes values, returns a forecast. The database is the caller's
/// problem, as it is in `growth::power`.
pub fn choose_and_forecast(
    series_id: &str,
    y: &[f64],
    horizon: usize,
    cadence: Cadence,
    non_negative: bool,
    made_at: String,
) -> Result<Forecast, Refusal> {
    choose_and_forecast_with(series_id, y, horizon, cadence, non_negative, made_at, None)
}

/// The same choice, with TimesFM offered as a third candidate.
///
/// It is scored by the identical gate the local methods face — a 10% margin on
/// the median MASE and three quarters of the folds, against seasonal naive.
/// Being a foundation model earns it nothing; the spike's NVDA result (MASE
/// 1.104, worse than naive-last) is exactly why.
#[allow(clippy::too_many_arguments)]
pub fn choose_and_forecast_with(
    series_id: &str,
    y: &[f64],
    horizon: usize,
    cadence: Cadence,
    non_negative: bool,
    made_at: String,
    remote: Option<RemoteCandidate>,
) -> Result<Forecast, Refusal> {
    let m = cadence.seasonal_period();
    let needed = cadence.min_points();
    if y.len() < needed {
        return Err(Refusal::InsufficientHistory {
            points: y.len(),
            needed,
        });
    }
    // Leave the folds the gate needs: enough training prefix that at least
    // MIN_FOLDS origins fit after it.
    let min_train = min_train_for(y.len(), horizon, m);

    let base_scores = backtest::rolling_origin(y, horizon, m, min_train, |train| {
        let f = seasonal_naive(train, horizon, m);
        (f.len() == horizon).then_some(f)
    });
    let ets_scores = backtest::rolling_origin(y, horizon, m, min_train, |train| {
        baseline::ets(train, horizon, m)
    });

    // Score TimesFM on the very same folds, if it was run.
    let remote_scores = remote.as_ref().and_then(|r| {
        let origins = backtest::fold_origins(y.len(), horizon, min_train);
        if r.fold_forecasts.len() != origins.len() || r.final_forecast.len() != horizon {
            // Mismatched folds are refused, never realigned: a quietly
            // truncated comparison is worse than no comparison.
            tracing::warn!(
                target: "permagent::forecaster",
                "remote returned {} folds for {} origins; not comparable",
                r.fold_forecasts.len(),
                origins.len()
            );
            return None;
        }
        let mut mases = Vec::with_capacity(origins.len());
        for (i, origin) in origins.iter().enumerate() {
            let train = &y[..*origin];
            let actual = &y[*origin..*origin + horizon];
            let f = &r.fold_forecasts[i];
            if f.len() != horizon {
                return None;
            }
            match baseline::mase(actual, f, train, m) {
                Some(s) => mases.push(s),
                // One unscoreable fold breaks the pairing, so the whole
                // candidate is withdrawn rather than compared on a subset.
                None => return None,
            }
        }
        Some(FoldScores { mases })
    });

    let remote_outcome = remote_scores
        .as_ref()
        .map(|rs| backtest::gate(rs, &base_scores));
    let ets_outcome = backtest::gate(&ets_scores, &base_scores);

    // Both cleared the floor? The lower median wins. Neither being promoted is
    // the common case and the baseline is served, labelled as itself.
    if let (
        Some(GateOutcome::Promoted {
            median_ratio,
            wins,
            folds,
        }),
        Some(r),
        Some(cand),
    ) = (
        remote_outcome.clone(),
        remote.as_ref(),
        remote_scores.as_ref(),
    ) {
        let ets_ratio = match &ets_outcome {
            GateOutcome::Promoted { median_ratio, .. } => Some(*median_ratio),
            _ => None,
        };
        let _ = cand;
        if ets_ratio.is_none_or(|e| median_ratio <= e) {
            let (p10, p90) = if r.final_p10.len() == horizon && r.final_p90.len() == horizon {
                // The model's own calibrated quantiles, when it gave them.
                (r.final_p10.clone(), r.final_p90.clone())
            } else {
                (r.final_forecast.clone(), r.final_forecast.clone())
            };
            let (p10, p90) = if non_negative {
                (
                    p10.iter().map(|v| v.max(0.0)).collect(),
                    p90.iter().map(|v| v.max(0.0)).collect(),
                )
            } else {
                (p10, p90)
            };
            return Ok(Forecast {
                series_id: series_id.to_string(),
                made_at,
                horizon,
                point: r.final_forecast.clone(),
                p10,
                p90,
                method: Method::Timesfm,
                method_label: Method::Timesfm.label().to_string(),
                mase_vs_baseline: Some(median_ratio),
                folds,
                fold_wins: wins,
                selection: format!(
                    "TimesFM cleared the gate: median MASE {median_ratio:.3}x seasonal naive over \
                     {folds} folds, winning {wins}"
                ),
            });
        }
    }

    let outcome = ets_outcome;
    let (method, point, residuals, mase, folds, wins, selection) = match &outcome {
        GateOutcome::Promoted {
            median_ratio,
            wins,
            folds,
        } => {
            let fit = baseline::fit_ets(y, m);
            match fit {
                Some(fit) => {
                    let p = baseline::ets_forecast(&fit, y.len(), horizon);
                    let r = fit.residuals.clone();
                    (
                        Method::Ets,
                        p,
                        r,
                        Some(*median_ratio),
                        *folds,
                        *wins,
                        format!(
                            "ETS cleared the gate: median MASE {median_ratio:.3}x seasonal naive \
                             over {folds} folds, winning {wins}"
                        ),
                    )
                }
                // Scored on the folds but cannot fit the whole series: fall
                // back and say so, rather than label a naive forecast as ETS.
                None => naive_result(
                    y,
                    horizon,
                    m,
                    &base_scores,
                    "ETS could not fit the full series",
                ),
            }
        }
        GateOutcome::Rejected { reason, .. } => {
            // Name the model's rejection too, when it was tried. A week of
            // baseline forecasts that never mentions TimesFM reads as a week
            // TimesFM agreed with the baseline.
            let remote_note = match &remote_outcome {
                Some(GateOutcome::Rejected { reason, .. }) => {
                    format!("; TimesFM did not clear it either: {reason}")
                }
                Some(GateOutcome::NoVerdict { folds, needed }) => {
                    format!("; TimesFM scored only {folds} of {needed} folds")
                }
                _ => String::new(),
            };
            naive_result(
                y,
                horizon,
                m,
                &base_scores,
                &format!("ETS did not clear the gate: {reason}{remote_note}"),
            )
        }
        GateOutcome::NoVerdict { folds, needed } => naive_result(
            y,
            horizon,
            m,
            &base_scores,
            &format!("{folds} of {needed} folds — not enough to certify any method over the floor"),
        ),
    };

    if point.len() != horizon {
        return Err(Refusal::NoMethodBeatsBaseline {
            detail: "no method could produce a forecast for this series".into(),
        });
    }
    let (p10, p90) = baseline::residual_interval(&point, &residuals, non_negative);
    Ok(Forecast {
        series_id: series_id.to_string(),
        made_at,
        horizon,
        point,
        p10,
        p90,
        method,
        method_label: method.label().to_string(),
        mase_vs_baseline: mase,
        folds,
        fold_wins: wins,
        selection,
    })
}

/// Seasonal naive, plus its own one-step residuals so the interval is measured
/// rather than assumed.
fn naive_result(
    y: &[f64],
    horizon: usize,
    m: usize,
    base: &FoldScores,
    why: &str,
) -> (
    Method,
    Vec<f64>,
    Vec<f64>,
    Option<f64>,
    usize,
    usize,
    String,
) {
    let point = seasonal_naive(y, horizon, m);
    let residuals: Vec<f64> = if y.len() > m {
        y.windows(m + 1).map(|w| w[m] - w[0]).collect()
    } else {
        Vec::new()
    };
    (
        Method::SeasonalNaive,
        point,
        residuals,
        base.median(),
        base.mases.len(),
        0,
        why.to_string(),
    )
}

/// The database-facing entry point: read a series, check its verdict, forecast
/// it or refuse.
pub async fn forecast_series(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    series_id: &str,
    horizon: Option<usize>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Forecast, Refusal> {
    let series = match store::get(pool, series_id).await {
        Ok(Some(s)) => s,
        _ => return Err(Refusal::NotBound),
    };
    let points: Vec<Point> = store::load_points(pool, series_id)
        .await
        .unwrap_or_default();
    // Guard order first, exactly as the registry reports it — the tool and the
    // card must never disagree about why a series is silent.
    match store::verdict_for(&series, points.len(), now) {
        Verdict::NotBound => return Err(Refusal::NotBound),
        Verdict::CollectorStale { last_collected_at } => {
            return Err(Refusal::CollectorStale { last_collected_at })
        }
        Verdict::InsufficientHistory { points, needed } => {
            return Err(Refusal::InsufficientHistory { points, needed })
        }
        Verdict::Forecastable => {}
    }
    debug_assert_eq!(series.status, SeriesStatus::Active);
    let horizon = horizon
        .filter(|h| *h > 0 && *h <= 60)
        .unwrap_or_else(|| series.cadence.default_horizon());
    let y: Vec<f64> = points.iter().map(|p| p.value).collect();
    // Downloads, pageviews and mention counts cannot be negative; a price can
    // be anything, so only counts get the floor.
    let non_negative = !matches!(series.source_kind, super::SourceKind::EquityClose);
    choose_and_forecast(
        series_id,
        &y,
        horizon,
        series.cadence,
        non_negative,
        now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}

/// Persist a forecast. The `method` column is NOT NULL, so a forecast whose
/// label was lost cannot be stored at all.
pub async fn record(pool: &sqlx::Pool<sqlx::Sqlite>, f: &Forecast) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO forecaster_forecasts
         (id, series_id, made_at, horizon, method, point_json, quantiles_json,
          mase_vs_baseline, folds, fold_wins)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&f.series_id)
    .bind(&f.made_at)
    .bind(f.horizon as i64)
    .bind(f.method.as_str())
    .bind(serde_json::to_string(&f.point).unwrap_or_else(|_| "[]".into()))
    .bind(
        serde_json::to_string(&serde_json::json!({ "p10": f.p10, "p90": f.p90 }))
            .unwrap_or_else(|_| "{}".into()),
    )
    .bind(f.mase_vs_baseline)
    .bind(f.folds as i64)
    .bind(f.fold_wins as i64)
    .execute(pool)
    .await
    .map_err(|e| format!("record forecast: {e}"))?;
    Ok(())
}
