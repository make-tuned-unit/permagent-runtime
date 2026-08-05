//! Funnels: where visitors drop out, and what a step is worth.
//!
//! The collector already stores everything this needs — `kind='event'` rows
//! carry a `name`, every row carries a first-party `session_id`, and `d`
//! (properties) can carry a value. What was missing is the question anyone
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
//!   SESSIONS, NOT EVENTS. Reloading the checkout page three times is one
//!   person, not three. Rows without a session id cannot be sequenced at all,
//!   so they are excluded rather than guessed at — see `Funnel::excluded`.

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

/// One row the funnel is computed from: a session touching a known step at a
/// point in time. `at` only has to be monotonic and comparable.
#[derive(Debug, Clone)]
pub struct Touch {
    pub session_id: String,
    pub step_index: usize,
    pub at: String,
    /// Value carried by this touch, when the step is a purchase-like event.
    pub value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub label: String,
    /// Sessions that reached this step having passed every earlier one in order.
    pub sessions: u64,
    /// Sessions lost between the previous step and this one.
    pub dropped: u64,
    /// Share of the PREVIOUS step that continued here, 0..1. `None` on step 1.
    pub step_rate: Option<f64>,
    /// Share of the FIRST step that reached here, 0..1. `None` on step 1.
    pub overall_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Funnel {
    pub steps: Vec<StepResult>,
    /// Sessions completing every step, over sessions entering the first.
    pub conversion_rate: f64,
    /// Total value across sessions that completed the funnel.
    pub value: f64,
    /// The single largest leak: the step index (1-based) losing the most
    /// sessions. This is the "what do I fix first" answer, and computing it
    /// here keeps every client from re-deriving it differently.
    pub biggest_drop_step: Option<usize>,
    /// Rows that carried no session id and so could not be sequenced. Surfaced
    /// rather than hidden: if this is large the funnel is not representative,
    /// and a silently-filtered denominator is how analytics tools lie.
    pub excluded_sessionless: u64,
}

/// Compute the funnel. `touches` may arrive in any order.
pub fn compute(step_labels: &[Step], touches: &[Touch], excluded_sessionless: u64) -> Funnel {
    let n = step_labels.len();
    if n == 0 {
        return Funnel {
            steps: Vec::new(),
            conversion_rate: 0.0,
            value: 0.0,
            biggest_drop_step: None,
            excluded_sessionless,
        };
    }

    // Group by session, then walk each session's touches in time order and
    // advance a cursor through the steps. The cursor is what enforces ORDER:
    // a step only counts once every earlier step has already been satisfied.
    let mut by_session: std::collections::HashMap<&str, Vec<&Touch>> =
        std::collections::HashMap::new();
    for t in touches {
        by_session.entry(t.session_id.as_str()).or_default().push(t);
    }

    let mut reached = vec![0u64; n];
    let mut completed_value = 0.0f64;

    for (_, mut list) in by_session {
        // Ties broken by step index so a pageview and an event recorded in the
        // same millisecond still sequence correctly rather than by hash order.
        list.sort_by(|a, b| a.at.cmp(&b.at).then(a.step_index.cmp(&b.step_index)));

        let mut cursor = 0usize;
        let mut value_this_session = 0.0f64;
        for t in list {
            if cursor < n && t.step_index == cursor {
                reached[cursor] += 1;
                if let Some(v) = t.value {
                    value_this_session += v;
                }
                cursor += 1;
            } else if cursor > 0 && t.step_index == cursor - 1 {
                // Repeat of the step already credited — same person, one count.
                if let Some(v) = t.value {
                    value_this_session += v;
                }
            }
        }
        if cursor == n {
            completed_value += value_this_session;
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
        });
    }

    let conversion_rate = if entered == 0.0 {
        0.0
    } else {
        reached[n - 1] as f64 / entered
    };

    Funnel {
        steps,
        conversion_rate,
        value: completed_value,
        biggest_drop_step,
        excluded_sessionless,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(session: &str, step: usize, at: &str) -> Touch {
        Touch {
            session_id: session.into(),
            step_index: step,
            at: at.into(),
            value: None,
        }
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
        let f = compute(&steps(), &t, 0);
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
        let f = compute(&steps(), &t, 0);
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
        let f = compute(&steps(), &t, 0);
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
        let f = compute(&steps(), &t, 0);
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
        let f = compute(&steps(), &t, 0);
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
        let f = compute(&steps(), &t, 0);
        assert_eq!(f.value, 99.0);
    }

    #[test]
    fn an_empty_window_is_zero_not_a_divide_by_zero() {
        let f = compute(&steps(), &[], 0);
        assert_eq!(f.conversion_rate, 0.0);
        assert_eq!(f.steps[0].sessions, 0);
        assert_eq!(f.steps[1].step_rate, Some(0.0));
    }

    #[test]
    fn sessionless_rows_are_surfaced_not_silently_dropped() {
        let f = compute(&steps(), &[touch("s1", 0, "T1")], 42);
        assert_eq!(f.excluded_sessionless, 42);
    }
}
