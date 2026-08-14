//! Funnels: where visitors drop out, and what a step is worth.
//!
//! The collector already stores everything this needs — `kind='event'` rows
//! carry a `name`, rows carry a first-party `session_id` and a `visitor_hash`,
//! and `d` (properties) can carry a value. What was missing is the question anyone
//! actually asks of analytics: of the people who saw the pricing page, how many
//! started checkout, and how many paid?
//!
//! Two rules make the answer trustworthy rather than merely plausible:
//!
//!   ORDER MATTERS. A session counts at step N only if it hit steps 1..N in
//!   sequence, each at or after the previous one. Counting each step
//!   independently is the classic funnel lie: a visitor who lands on /thanks
//!   from a bookmark would otherwise "convert" without ever seeing pricing, and
//!   a funnel that reports more conversions than entries destroys trust in the
//!   whole tool.
//!
//!   IDENTITIES, NOT EVENTS. Reloading the checkout page three times is one
//!   person, not three. Rows carrying no id for the chosen identity cannot be
//!   sequenced at all, so they are excluded rather than guessed at — see
//!   `Funnel::excluded_no_identity`.
//!
//! WHICH identity is a real choice, and a funnel whose denominator is undefined
//! is not measurable, so it is a parameter rather than an assumption:
//!
//!   `Identity::Session` (default) — the first-party `session_id` from
//!   sessionStorage. One visit, one journey. This is what a funnel means: steps
//!   completed in one sitting. Its cost is coverage — rows predating the
//!   session-id snippet, or from a relay that does not send one, carry none and
//!   are excluded (and counted).
//!
//!   `Identity::Visitor` — `visitor_hash`, which is present on every row but is
//!   sha256(site_key, UA, Accept-Language, UTC day). It COLLAPSES everyone
//!   sharing a browser build and language into one identity and ROTATES at
//!   midnight UTC. Higher coverage, coarser truth: it counts device signatures,
//!   not people, and can stitch two strangers' steps into one "journey".
//!
//! Both are exposed because the honest answer differs by dataset; the UI names
//! which one produced the numbers on screen.

use serde::{Deserialize, Serialize};

/// One step in a funnel: a pageview of a path, or a named custom event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Step {
    /// A pageview whose path matches exactly.
    Path { value: String },
    /// A `kind='event'` row whose name matches exactly.
    Event { value: String },
}

impl Step {
    /// Parse the compact wire form used by the query string: `path:/pricing`
    /// or `event:signup`. A bare value is treated as a path, which is what
    /// people type first.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.split_once(':') {
            Some(("event", v)) if !v.trim().is_empty() => Some(Step::Event {
                value: v.trim().to_string(),
            }),
            Some(("path", v)) if !v.trim().is_empty() => Some(Step::Path {
                value: v.trim().to_string(),
            }),
            _ => Some(Step::Path {
                value: raw.to_string(),
            }),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Step::Path { value } | Step::Event { value } => value,
        }
    }
}

/// Who a step is credited to. Chosen per query, never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Identity {
    /// First-party `session_id` — one visit, one journey. The default.
    #[default]
    Session,
    /// `visitor_hash` — present on every row, but a daily-rotating device
    /// signature rather than a person.
    Visitor,
}

impl Identity {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "visitor" | "visitor_hash" => Identity::Visitor,
            _ => Identity::Session,
        }
    }
}

/// One row the funnel is computed from: an identity touching a known step at a
/// point in time. `at` only has to be monotonic and comparable.
#[derive(Debug, Clone)]
pub struct Touch {
    /// The session id or visitor hash, per the funnel's `Identity`.
    pub identity: String,
    pub step_index: usize,
    pub at: String,
    /// Epoch milliseconds for `at`, when it could be parsed. Ordering uses the
    /// string (RFC3339 sorts correctly); only the time BETWEEN steps needs a
    /// number, and an unparseable timestamp must weaken that one figure rather
    /// than silently reorder the funnel.
    pub at_ms: Option<i64>,
    /// Value carried by this touch, when the step is a purchase-like event.
    pub value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub label: String,
    /// Identities that reached this step having passed every earlier one in
    /// order. Named `sessions` for wire compatibility; the unit is whatever
    /// `Funnel::identity` says it is.
    pub sessions: u64,
    /// Identities lost between the previous step and this one.
    pub dropped: u64,
    /// Share of the PREVIOUS step that continued here, 0..1. `None` on step 1.
    pub step_rate: Option<f64>,
    /// Share of the FIRST step that reached here, 0..1. `None` on step 1.
    pub overall_rate: Option<f64>,
    /// MEDIAN seconds from the previous step to this one, across identities
    /// that made both. `None` on step 1, or when no pair could be timed.
    ///
    /// Median, not mean, and the difference is not cosmetic: one visitor who
    /// leaves the checkout tab open overnight adds ~43,000 seconds to the mean
    /// of a ten-session funnel and moves it by an hour and a bit. The median is
    /// unmoved. "How long does this normally take" is a question about the
    /// typical journey, and only the median answers it.
    pub median_seconds_from_prev: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Funnel {
    pub steps: Vec<StepResult>,
    /// What one unit in `StepResult::sessions` IS — `session` or `visitor`.
    /// Serialized so the UI can name the denominator instead of implying one.
    pub identity: Identity,
    /// Identities completing every step, over those entering the first.
    pub conversion_rate: f64,
    /// Total value across identities that completed the funnel.
    pub value: f64,
    /// The single largest leak: the step index (1-based) losing the most
    /// sessions. This is the "what do I fix first" answer, and computing it
    /// here keeps every client from re-deriving it differently.
    pub biggest_drop_step: Option<usize>,
    /// Matching rows that carried no id for the chosen identity and so could
    /// not be sequenced. Surfaced rather than hidden: if this is large the
    /// funnel is not representative, and a silently-filtered denominator is how
    /// analytics tools lie.
    pub excluded_no_identity: u64,
    /// Rows the bot filter removed before any of the above. Bot traffic is
    /// excluded by default — on an SEO-heavy site crawlers are a large share of
    /// all requests, and counting them inflates every step.
    pub excluded_bots: u64,
}

/// Rows removed before sequencing, reported alongside the figures they shaped.
#[derive(Debug, Clone, Copy, Default)]
pub struct Excluded {
    pub no_identity: u64,
    pub bots: u64,
}

/// Median of a sample. Empty → `None`; even count → mean of the two middles.
fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    })
}

/// Compute the funnel. `touches` may arrive in any order.
pub fn compute(
    step_labels: &[Step],
    touches: &[Touch],
    excluded: Excluded,
    identity: Identity,
) -> Funnel {
    let n = step_labels.len();
    if n == 0 {
        return Funnel {
            steps: Vec::new(),
            identity,
            conversion_rate: 0.0,
            value: 0.0,
            biggest_drop_step: None,
            excluded_no_identity: excluded.no_identity,
            excluded_bots: excluded.bots,
        };
    }

    // Group by identity, then walk each one's touches in time order and advance
    // a cursor through the steps. The cursor is what enforces ORDER: a step only
    // counts once every earlier step has already been satisfied.
    let mut by_identity: std::collections::HashMap<&str, Vec<&Touch>> =
        std::collections::HashMap::new();
    for t in touches {
        by_identity.entry(t.identity.as_str()).or_default().push(t);
    }

    let mut reached = vec![0u64; n];
    let mut completed_value = 0.0f64;
    // Per step, the gaps from the previous step across everyone who made both.
    let mut gaps: Vec<Vec<f64>> = vec![Vec::new(); n];

    for (_, mut list) in by_identity {
        // Ties broken by step index so a pageview and an event recorded in the
        // same millisecond still sequence correctly rather than by hash order.
        list.sort_by(|a, b| a.at.cmp(&b.at).then(a.step_index.cmp(&b.step_index)));

        let mut cursor = 0usize;
        let mut value_this_identity = 0.0f64;
        // When each step was first satisfied, for the time-between figures.
        let mut satisfied_at: Vec<Option<i64>> = vec![None; n];
        for t in list {
            if cursor < n && t.step_index == cursor {
                reached[cursor] += 1;
                satisfied_at[cursor] = t.at_ms;
                if let Some(v) = t.value {
                    value_this_identity += v;
                }
                cursor += 1;
            } else if cursor > 0 && t.step_index == cursor - 1 {
                // Repeat of the step already credited — same person, one count.
                if let Some(v) = t.value {
                    value_this_identity += v;
                }
            }
        }
        for i in 1..cursor {
            // Only pairs this identity actually completed IN ORDER contribute,
            // so the figure answers "how long did the people who advanced take"
            // rather than mixing in journeys that never got there.
            if let (Some(prev), Some(here)) = (satisfied_at[i - 1], satisfied_at[i]) {
                gaps[i].push(((here - prev).max(0) as f64) / 1000.0);
            }
        }
        if cursor == n {
            completed_value += value_this_identity;
        }
    }

    let entered = reached[0] as f64;
    let mut steps = Vec::with_capacity(n);
    let mut biggest_drop_step = None;
    let mut biggest_drop = 0u64;

    for i in 0..n {
        let prev = if i == 0 { reached[0] } else { reached[i - 1] };
        let dropped = prev.saturating_sub(reached[i]);
        if i > 0 && dropped > biggest_drop {
            biggest_drop = dropped;
            biggest_drop_step = Some(i + 1);
        }
        steps.push(StepResult {
            label: step_labels[i].label().to_string(),
            sessions: reached[i],
            dropped: if i == 0 { 0 } else { dropped },
            step_rate: if i == 0 {
                None
            } else if prev == 0 {
                Some(0.0)
            } else {
                Some(reached[i] as f64 / prev as f64)
            },
            overall_rate: if i == 0 {
                None
            } else if entered == 0.0 {
                Some(0.0)
            } else {
                Some(reached[i] as f64 / entered)
            },
            median_seconds_from_prev: if i == 0 {
                None
            } else {
                median(std::mem::take(&mut gaps[i]))
            },
        });
    }

    let conversion_rate = if entered == 0.0 {
        0.0
    } else {
        reached[n - 1] as f64 / entered
    };

    Funnel {
        steps,
        identity,
        conversion_rate,
        value: completed_value,
        biggest_drop_step,
        excluded_no_identity: excluded.no_identity,
        excluded_bots: excluded.bots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(session: &str, step: usize, at: &str) -> Touch {
        Touch {
            identity: session.into(),
            step_index: step,
            at: at.into(),
            at_ms: None,
            value: None,
        }
    }

    /// A touch at a real instant, for the time-between-steps figures.
    fn timed(session: &str, step: usize, at_ms: i64) -> Touch {
        Touch {
            identity: session.into(),
            step_index: step,
            at: format!("{at_ms:020}"),
            at_ms: Some(at_ms),
            value: None,
        }
    }

    fn run(steps: &[Step], touches: &[Touch]) -> Funnel {
        compute(steps, touches, Excluded::default(), Identity::Session)
    }

    fn steps() -> Vec<Step> {
        vec![
            Step::Path {
                value: "/pricing".into(),
            },
            Step::Event {
                value: "checkout_started".into(),
            },
            Step::Event {
                value: "purchase".into(),
            },
        ]
    }

    #[test]
    fn parses_both_wire_forms_and_defaults_to_path() {
        assert_eq!(
            Step::parse("event:signup"),
            Some(Step::Event {
                value: "signup".into()
            })
        );
        assert_eq!(
            Step::parse("path:/a"),
            Some(Step::Path { value: "/a".into() })
        );
        assert_eq!(Step::parse("/a"), Some(Step::Path { value: "/a".into() }));
        assert_eq!(Step::parse("   "), None);
    }

    #[test]
    fn counts_a_clean_conversion() {
        let t = vec![
            touch("s1", 0, "T1"),
            touch("s1", 1, "T2"),
            touch("s1", 2, "T3"),
        ];
        let f = run(&steps(), &t);
        assert_eq!(
            f.steps.iter().map(|s| s.sessions).collect::<Vec<_>>(),
            vec![1, 1, 1]
        );
        assert_eq!(f.conversion_rate, 1.0);
    }

    #[test]
    fn a_step_reached_out_of_order_does_not_count() {
        // Landed straight on the purchase confirmation — a bookmark, or a
        // refund email. Counting it would report a conversion that never
        // travelled the funnel.
        let t = vec![touch("s1", 2, "T1")];
        let f = run(&steps(), &t);
        assert_eq!(f.steps[2].sessions, 0);
        assert_eq!(f.conversion_rate, 0.0);
    }

    #[test]
    fn later_steps_can_never_exceed_earlier_ones() {
        // The property that makes a funnel believable at a glance.
        let t = vec![
            touch("s1", 0, "T1"),
            touch("s1", 1, "T2"),
            touch("s2", 0, "T1"),
            touch("s3", 1, "T1"),
            touch("s3", 2, "T2"), // never saw step 1
        ];
        let f = run(&steps(), &t);
        for w in f.steps.windows(2) {
            assert!(w[1].sessions <= w[0].sessions, "{:?}", f.steps);
        }
    }

    #[test]
    fn repeated_steps_are_one_session_not_three() {
        let t = vec![
            touch("s1", 0, "T1"),
            touch("s1", 0, "T2"),
            touch("s1", 0, "T3"),
            touch("s1", 1, "T4"),
        ];
        let f = run(&steps(), &t);
        assert_eq!(f.steps[0].sessions, 1);
        assert_eq!(f.steps[1].sessions, 1);
    }

    #[test]
    fn reports_drop_off_and_names_the_worst_leak() {
        //   10 see pricing, 5 start checkout, 4 buy → worst leak is step 2.
        let mut t = Vec::new();
        for i in 0..10 {
            t.push(touch(&format!("s{i}"), 0, "T1"));
        }
        for i in 0..5 {
            t.push(touch(&format!("s{i}"), 1, "T2"));
        }
        for i in 0..4 {
            t.push(touch(&format!("s{i}"), 2, "T3"));
        }
        let f = run(&steps(), &t);
        assert_eq!(f.steps[1].dropped, 5);
        assert_eq!(f.steps[1].step_rate, Some(0.5));
        assert_eq!(f.steps[2].dropped, 1);
        assert_eq!(f.steps[2].overall_rate, Some(0.4));
        assert_eq!(f.biggest_drop_step, Some(2));
        assert_eq!(f.conversion_rate, 0.4);
    }

    #[test]
    fn value_counts_only_sessions_that_completed() {
        let mut done = touch("s1", 2, "T3");
        done.value = Some(99.0);
        let mut abandoned = touch("s2", 1, "T2");
        abandoned.value = Some(50.0);
        let t = vec![
            touch("s1", 0, "T1"),
            touch("s1", 1, "T2"),
            done,
            touch("s2", 0, "T1"),
            abandoned,
        ];
        let f = run(&steps(), &t);
        assert_eq!(f.value, 99.0);
    }

    #[test]
    fn an_empty_window_is_zero_not_a_divide_by_zero() {
        let f = run(&steps(), &[]);
        assert_eq!(f.conversion_rate, 0.0);
        assert_eq!(f.steps[0].sessions, 0);
        assert_eq!(f.steps[1].step_rate, Some(0.0));
    }

    #[test]
    fn unsequenceable_rows_are_surfaced_not_silently_dropped() {
        let f = compute(
            &steps(),
            &[touch("s1", 0, "T1")],
            Excluded {
                no_identity: 42,
                bots: 9,
            },
            Identity::Session,
        );
        assert_eq!(f.excluded_no_identity, 42);
        // Bot rows are excluded by default, so the count has to travel with the
        // figures it shaped or a filtered number reads as a quiet day.
        assert_eq!(f.excluded_bots, 9);
    }

    #[test]
    fn the_denominator_names_itself() {
        // A funnel whose unit is undefined is not measurable. The response says
        // which identity produced the counts.
        let f = run(&steps(), &[]);
        assert_eq!(f.identity, Identity::Session);
        let v = compute(&steps(), &[], Excluded::default(), Identity::Visitor);
        assert_eq!(v.identity, Identity::Visitor);
        assert_eq!(Identity::parse("visitor"), Identity::Visitor);
        assert_eq!(Identity::parse("SESSION"), Identity::Session);
        // Anything unrecognised falls back to the conservative default rather
        // than erroring out mid-query.
        assert_eq!(Identity::parse("nonsense"), Identity::Session);
    }

    #[test]
    fn time_between_steps_is_the_median_not_the_mean() {
        // Four sessions take 10s, 20s, 30s from step 1 to 2 — and one leaves the
        // tab open overnight. Mean: ~10,815s (three hours), a number no visitor
        // experienced. Median: 25s.
        let day_ms = 86_400_000;
        let mut t = Vec::new();
        for (i, gap) in [10_000i64, 20_000, 30_000, day_ms].iter().enumerate() {
            let s = format!("s{i}");
            t.push(timed(&s, 0, 1_000_000));
            t.push(timed(&s, 1, 1_000_000 + gap));
        }
        let f = run(&steps(), &t);
        assert_eq!(f.steps[0].median_seconds_from_prev, None); // nothing precedes step 1
        assert_eq!(f.steps[1].median_seconds_from_prev, Some(25.0));
        // Nobody reached step 3, so there is no gap to report — an honest gap,
        // not a reassuring zero.
        assert_eq!(f.steps[2].median_seconds_from_prev, None);
    }

    #[test]
    fn timing_uses_the_first_time_a_step_was_satisfied() {
        // Repeat views of step 1 must not restart the clock: the journey began
        // at the first one.
        let t = vec![
            timed("s1", 0, 0),
            timed("s1", 0, 5_000),
            timed("s1", 1, 9_000),
        ];
        let f = run(&steps(), &t);
        assert_eq!(f.steps[1].median_seconds_from_prev, Some(9.0));
    }

    #[test]
    fn an_untimeable_row_weakens_only_the_timing() {
        // `at_ms` is None (an unparseable stored timestamp). The step still
        // counts; only the median it could not contribute to goes missing.
        let t = vec![touch("s1", 0, "T1"), touch("s1", 1, "T2")];
        let f = run(&steps(), &t);
        assert_eq!(f.steps[1].sessions, 1);
        assert_eq!(f.steps[1].median_seconds_from_prev, None);
    }

    #[test]
    fn median_of_an_even_sample_is_the_midpoint() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![4.0]), Some(4.0));
        assert_eq!(median(vec![10.0, 20.0]), Some(15.0));
        assert_eq!(median(vec![30.0, 10.0, 20.0]), Some(20.0));
    }
}
