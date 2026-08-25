//! The weekly market brief — prose that may not outrun its own numbers.
//!
//! One paragraph per project per week, from the forecast table, the last three
//! briefs' verdicts, and recent project memories. ~1.5 k in, 300 out.
//!
//! ## The rule, and why it is a test rather than a hope
//!
//! **The brief may only restate direction, magnitude, interval and method.** No
//! causal claim, no recommendation, and no number that was not in its input.
//! That is not a style preference: these are other people's download counts and
//! pageviews, and a sentence like "downloads fell because the competitor
//! shipped" is a claim the data cannot support in either direction. The prompt
//! says so and [`validate`] enforces it, the way `growth::power`'s own test
//! forbids the string "caus*" in a verdict.
//!
//! ## Routing
//!
//! Best-fit and cost-conscious, in that order. The brief is a small, bounded,
//! low-complexity job — exactly the shape the Apple on-device model serves for
//! free, with the prompt never leaving the machine. It is used **when the input
//! actually fits the running model's probed context window**, which is a
//! runtime property and not a constant. Otherwise the existing cheap cloud
//! route. Whichever ran is recorded on the row, because a brief is only as
//! trustworthy as its provenance.

use serde::{Deserialize, Serialize};

/// Words that assert a cause. A market brief has downloads and pageviews; it
/// does not have a mechanism, and every one of these smuggles one in.
const CAUSAL: &[&str] = &[
    "because",
    "caused",
    "causing",
    "cause of",
    "due to",
    "driven by",
    "drove",
    "thanks to",
    "as a result of",
    "resulted in",
    "explains",
    "reflects",
    "reflecting",
    "attributable",
    "owing to",
    "led to",
];

/// Words that give advice. Reporting a direction is the job; telling the user
/// what to do about it is not, and never was.
const ADVISORY: &[&str] = &[
    "you should",
    "we should",
    "recommend",
    "recommendation",
    "advise",
    "consider ",
    "it would be wise",
    "worth doing",
    "act now",
    "opportunity to",
    "you must",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Brief {
    pub project_id: String,
    pub generated_at: String,
    pub summary: String,
    /// Which methods produced the numbers this prose restates, counted. A brief
    /// whose mix is entirely `seasonal_naive` is a brief about a baseline, and
    /// the card says so.
    pub method_mix: std::collections::BTreeMap<String, usize>,
    /// The engine that wrote it: `apple_foundation_models` or the cloud model's
    /// name. Never blank.
    pub model: String,
}

/// Everything wrong with a candidate brief, all at once — a caller that fixes
/// one violation and resubmits should not discover the next one on the next
/// round trip.
#[derive(Debug, Clone, PartialEq)]
pub enum Violation {
    Causal(String),
    Advisory(String),
    /// A number that appears in the prose and in none of the inputs.
    UngroundedNumber(String),
    Empty,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Causal(w) => write!(f, "asserts a cause (\"{w}\")"),
            Self::Advisory(w) => write!(f, "gives advice (\"{w}\")"),
            Self::UngroundedNumber(n) => write!(f, "states {n}, which is in no input"),
            Self::Empty => write!(f, "is empty"),
        }
    }
}

/// Pull every number out of a string, normalized so "1,200" and "1200" and
/// "1200.0" compare equal.
pub fn numbers_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if cur.is_empty() {
            return;
        }
        let cleaned: String = cur.chars().filter(|c| *c != ',').collect();
        let normalized = cleaned
            .trim_end_matches('.')
            .trim_start_matches('0')
            .to_string();
        let normalized = if normalized.is_empty() || normalized.starts_with('.') {
            format!("0{normalized}")
        } else {
            normalized
        };
        // Trailing ".0" is the same number.
        let normalized = normalized
            .strip_suffix(".0")
            .map(str::to_string)
            .unwrap_or(normalized);
        out.push(normalized);
        cur.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || ((c == '.' || c == ',') && !cur.is_empty()) {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Check a candidate brief against the rule.
///
/// `grounded` is every number the model was given. A brief may round — "up 12%"
/// from 11.8 is honest prose — so a number within 1 of a grounded one passes;
/// anything else is invention.
pub fn validate(summary: &str, grounded: &[f64]) -> Result<(), Vec<Violation>> {
    let mut violations = Vec::new();
    let text = summary.trim();
    if text.is_empty() {
        return Err(vec![Violation::Empty]);
    }
    let lower = text.to_ascii_lowercase();
    for w in CAUSAL {
        if lower.contains(w) {
            violations.push(Violation::Causal((*w).to_string()));
        }
    }
    for w in ADVISORY {
        if lower.contains(w) {
            violations.push(Violation::Advisory((*w).to_string()));
        }
    }
    for token in numbers_in(text) {
        let Ok(value) = token.parse::<f64>() else {
            continue;
        };
        // Small integers are ordinary prose ("all 3 series", "the next 7 days")
        // and pinning them would forbid the sentence rather than the invention.
        if value <= 100.0 && value.fract() == 0.0 {
            continue;
        }
        let grounded_here = grounded.iter().any(|g| {
            (g - value).abs() <= 1.0 || (g.abs() > 0.0 && ((g - value) / g).abs() <= 0.01)
        });
        if !grounded_here {
            violations.push(Violation::UngroundedNumber(token));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// The system prompt. Kept as a const so the test below reads the same string
/// the model does — a rule that lives only in a comment is not a rule.
pub const SYSTEM_PROMPT: &str = "\
You summarise market-direction forecasts for one project, in one short paragraph.

You are given a table of series, each with a direction, a magnitude, an 80% interval, \
and the METHOD that produced it. You may restate those four things and nothing else.

You must NOT:
- say why anything moved, or suggest any cause;
- recommend an action, or say what the user should do;
- state any number that is not in the table you were given.

Say the method out loud when a forecast came from a baseline rather than a model. \
If every series is a baseline, say that the week's numbers are baselines. \
If a series is too short or its collector is stale, say so plainly instead of \
describing a direction for it.

These are other people's public numbers — downloads, pageviews, mentions. They say \
where a category is heading. They say nothing about whether this project will ship or \
whether any goal will succeed.";

/// Assemble the model's input from the project's rows, and collect every number
/// the prose is allowed to use.
///
/// The grounding set is built from the same values that go into the prompt, so
/// [`validate`] cannot disagree with what the model was shown.
pub fn compose(
    project_name: &str,
    rows: &[(
        crate::forecaster::store::SeriesSummary,
        Option<crate::forecaster::Forecast>,
    )],
) -> (String, Vec<f64>) {
    use std::fmt::Write as _;
    let mut prompt = format!("Project: {project_name}\n\nSeries:\n");
    let mut grounded = Vec::new();
    for (summary, forecast) in rows {
        let _ = write!(
            prompt,
            "- {} ({}, {})",
            summary.subject,
            summary.source_label,
            summary.cadence.as_str()
        );
        match forecast {
            Some(f) => {
                let last = f.point.last().copied().unwrap_or(f64::NAN);
                let lo = f.p10.last().copied().unwrap_or(f64::NAN);
                let hi = f.p90.last().copied().unwrap_or(f64::NAN);
                grounded.extend([last, lo, hi]);
                grounded.extend(f.point.iter().copied());
                let _ = writeln!(
                    prompt,
                    ": {} steps ahead {last:.0}, 80% range {lo:.0} to {hi:.0}, method {} ({})",
                    f.horizon,
                    f.method.as_str(),
                    f.method_label
                );
            }
            None => {
                grounded.push(summary.points as f64);
                grounded.push(summary.cadence.min_points() as f64);
                let _ = writeln!(
                    prompt,
                    ": no forecast — {} of {} points, or the collector is stale",
                    summary.points,
                    summary.cadence.min_points()
                );
            }
        }
    }
    prompt.push_str(
        "\nWrite one short paragraph. Restate direction, magnitude, interval and method only.\n",
    );
    (prompt, grounded)
}

/// Count which methods actually produced the numbers the prose restates.
pub fn method_mix(
    rows: &[(
        crate::forecaster::store::SeriesSummary,
        Option<crate::forecaster::Forecast>,
    )],
) -> std::collections::BTreeMap<String, usize> {
    let mut mix = std::collections::BTreeMap::new();
    for (_, f) in rows {
        let key = match f {
            Some(f) => f.method.as_str().to_string(),
            None => "refused".to_string(),
        };
        *mix.entry(key).or_insert(0) += 1;
    }
    mix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brief_states_no_causal_claim() {
        let grounded = [1200.0, 1310.0];
        // The prompt itself has to carry the rule.
        assert!(SYSTEM_PROMPT.contains("suggest any cause"));
        assert!(SYSTEM_PROMPT.contains("recommend an action"));

        let bad = "npm downloads are up 9% because the competitor shipped a release.";
        let violations = validate(bad, &grounded).unwrap_err();
        assert!(
            violations.iter().any(|v| matches!(v, Violation::Causal(_))),
            "{violations:?}"
        );

        let good = "npm downloads are up 9% over the next 7 days, 80% range 1200 to 1310, \
                    by seasonal naive — a baseline, not a model.";
        assert_eq!(validate(good, &grounded), Ok(()));
    }

    #[test]
    fn a_recommendation_is_refused_however_politely_it_is_phrased() {
        for bad in [
            "Downloads are flat; you should watch this one.",
            "Pageviews are down 4%. Consider a response.",
            "It would be wise to look at the adjacent category.",
        ] {
            let v = validate(bad, &[]).unwrap_err();
            assert!(
                v.iter().any(|x| matches!(x, Violation::Advisory(_))),
                "{bad:?} -> {v:?}"
            );
        }
    }

    #[test]
    fn a_number_absent_from_the_input_is_refused() {
        let grounded = [1200.0, 1310.0];
        let invented = "npm downloads reach 4820 next week.";
        let v = validate(invented, &grounded).unwrap_err();
        assert!(
            v.iter()
                .any(|x| matches!(x, Violation::UngroundedNumber(n) if n == "4820")),
            "{v:?}"
        );
        // Rounding a grounded number is honest prose, not invention.
        assert_eq!(validate("about 1310 downloads", &grounded), Ok(()));
        assert_eq!(validate("roughly 1,200 downloads", &grounded), Ok(()));
        // And ordinary small counts in a sentence are not numbers under test.
        assert_eq!(
            validate("all 3 series over the next 7 days", &grounded),
            Ok(())
        );
    }

    #[test]
    fn an_empty_brief_is_a_violation_not_a_pass() {
        assert_eq!(validate("   ", &[]), Err(vec![Violation::Empty]));
    }

    #[test]
    fn numbers_are_normalized_before_they_are_compared() {
        assert_eq!(
            numbers_in("1,200 and 1200.0 and 007"),
            vec!["1200", "1200", "7"]
        );
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;
    use crate::forecaster::forecast::{Forecast, Method};
    use crate::forecaster::series::SeriesStatus;
    use crate::forecaster::store::{SeriesSummary, Verdict};
    use crate::forecaster::{Cadence, SourceKind};

    fn summary(points: usize) -> SeriesSummary {
        SeriesSummary {
            series_id: "s1".into(),
            project_id: "p1".into(),
            intel_id: None,
            source_kind: SourceKind::Npm,
            source_label: SourceKind::Npm.label().into(),
            subject: "langchain".into(),
            subject_group: None,
            cadence: Cadence::Daily,
            label: "langchain".into(),
            status: SeriesStatus::Active,
            points,
            span_days: points as i64,
            first_ts: None,
            last_ts: None,
            last_collected_at: None,
            last_error: None,
            snapshot_only: false,
            official_source: true,
            verdict: Verdict::Forecastable,
        }
    }

    fn forecast() -> Forecast {
        Forecast {
            series_id: "s1".into(),
            made_at: "2026-08-24T00:00:00.000Z".into(),
            horizon: 7,
            point: vec![1200.0; 7],
            p10: vec![1100.0; 7],
            p90: vec![1310.0; 7],
            method: Method::SeasonalNaive,
            method_label: Method::SeasonalNaive.label().into(),
            mase_vs_baseline: Some(1.0),
            folds: 8,
            fold_wins: 0,
            selection: "baseline".into(),
        }
    }

    /// Everything the model may say a number about is in the grounding set, so
    /// a faithful brief always validates and an invented one never does.
    #[test]
    fn the_grounding_set_is_exactly_what_the_model_was_shown() {
        let rows = vec![(summary(200), Some(forecast()))];
        let (prompt, grounded) = compose("Acme", &rows);
        assert!(prompt.contains("method seasonal_naive"));
        assert!(prompt.contains("80% range 1100 to 1310"));
        assert_eq!(
            validate("downloads hold near 1200, range 1100 to 1310", &grounded),
            Ok(())
        );
        assert!(validate("downloads reach 9999", &grounded).is_err());
    }

    #[test]
    fn a_refused_series_reaches_the_prompt_as_a_refusal_not_a_gap() {
        let rows = vec![(summary(42), None)];
        let (prompt, grounded) = compose("Acme", &rows);
        assert!(
            prompt.contains("no forecast — 42 of 180 points"),
            "{prompt}"
        );
        // And the two numbers in that sentence are groundable.
        assert!(grounded.contains(&180.0));
        assert_eq!(method_mix(&rows).get("refused"), Some(&1));
    }

    #[test]
    fn a_week_of_baselines_is_visible_in_the_method_mix() {
        let rows = vec![
            (summary(200), Some(forecast())),
            (summary(200), Some(forecast())),
        ];
        let mix = method_mix(&rows);
        assert_eq!(mix.get("seasonal_naive"), Some(&2));
        assert!(!mix.contains_key("timesfm-2.5-200m"));
    }
}
