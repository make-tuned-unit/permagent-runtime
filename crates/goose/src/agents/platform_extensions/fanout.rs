//! Bounded async subagent fan-out (Prime async subagents).
//!
//! `delegate` already spawns one background subagent and `load(taskId)` joins
//! it, so a caller *could* fan out by hand: N `delegate` calls, N `load` calls,
//! and its own bookkeeping. What it could not do is fan out **bounded** — every
//! child started the instant it was asked for, up to a flat registry cap of five
//! background tasks, on a 16 GB machine where three concurrent coding subagents
//! is already too many.
//!
//! This module is the primitive underneath `delegate_many`:
//!
//! - **A cap on work in flight**, not on work outstanding
//!   ([`fanout_concurrency`], default [`DEFAULT_FANOUT_CONCURRENCY`]). Ten
//!   children with a cap of two means eight of them are waiting, not running.
//! - **Results join in order.** Children finish in whatever order they finish;
//!   the caller reads child 0, child 1, child 2 — the order it asked in.
//! - **Cancellation propagates.** Every child runs on a token derived from the
//!   parent's, so cancelling the caller cancels the children; a child that had
//!   not started yet is reported cancelled instead of quietly costing money.
//! - **Per-child cost is attributed**, read back out of `cost_ledger` by
//!   `subagent_id` ([`subagent_cost`]) so the aggregate can say what each child
//!   spent rather than handing back one anonymous total.
//!
//! The runner is injected ([`run_bounded`] takes a closure), which is what lets
//! the tests drive N children with a fake provider — and what lets a review or
//! audit helper reuse the same bounded fan-out without going through the tool
//! layer at all.

use std::future::Future;
use std::sync::Arc;

use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

/// Config key for how many fan-out children may run at once.
pub const KEY_FANOUT_CONCURRENCY: &str = "PERMAGENT_FANOUT_CONCURRENCY";

/// Two. The dev machine this runs on has 16 GB, and a coding subagent is not a
/// cheap thread — it is a whole agent loop with its own context and its own
/// share of a rate-limited API key.
pub const DEFAULT_FANOUT_CONCURRENCY: usize = 2;

/// A ceiling on how many children one `delegate_many` call may ask for. Beyond
/// this the caller is not fanning out, it is queueing, and it should say so.
pub const MAX_FANOUT_CHILDREN: usize = 8;

/// Configured concurrency, clamped to at least 1 — a cap of zero is not a
/// slower fan-out, it is a deadlock.
pub fn fanout_concurrency() -> usize {
    Config::global()
        .get_param::<usize>(KEY_FANOUT_CONCURRENCY)
        .unwrap_or(DEFAULT_FANOUT_CONCURRENCY)
        .max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildStatus {
    Ok,
    Failed,
    /// The parent was cancelled. A child that never got a permit is cancelled
    /// too — and that is the cheap outcome, so it is reported, not hidden.
    Cancelled,
}

impl ChildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ChildStatus::Ok => "ok",
            ChildStatus::Failed => "failed",
            ChildStatus::Cancelled => "cancelled",
        }
    }
}

/// One child's result. `index` is its position in the caller's request, and the
/// aggregate is always sorted by it.
#[derive(Debug, Clone, Serialize)]
pub struct ChildOutcome {
    pub index: usize,
    pub label: String,
    pub status: ChildStatus,
    /// The subagent's own session id — the `subagent_id` its `cost_ledger` rows
    /// carry. `None` when the child never started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
    /// The `cost_router::delegate` receipt for this child: which model, and why
    /// that one. Per child, because each child routes on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_routing: Option<serde_json::Value>,
    pub text: String,
}

impl ChildOutcome {
    pub fn cancelled(index: usize, label: impl Into<String>) -> Self {
        Self {
            index,
            label: label.into(),
            status: ChildStatus::Cancelled,
            subagent_id: None,
            model_routing: None,
            text: "cancelled before it started".to_string(),
        }
    }

    pub fn failed(index: usize, label: impl Into<String>, why: impl Into<String>) -> Self {
        Self {
            index,
            label: label.into(),
            status: ChildStatus::Failed,
            subagent_id: None,
            model_routing: None,
            text: why.into(),
        }
    }
}

/// Run `items` as children, at most `concurrency` in flight, joined IN ORDER.
///
/// `run_child` is injected: production hands it the real subagent runner, tests
/// hand it a fake, and an in-process review helper can hand it something that
/// never touches a provider at all.
///
/// Every child is handed a token derived from `cancel`, so a parent cancel
/// reaches children that are already running. A child still waiting for a permit
/// when the parent is cancelled never starts.
pub async fn run_bounded<T, F, Fut>(
    items: Vec<T>,
    concurrency: usize,
    cancel: CancellationToken,
    run_child: F,
) -> Vec<ChildOutcome>
where
    T: Send + 'static,
    F: Fn(usize, T, CancellationToken) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ChildOutcome> + Send + 'static,
{
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::with_capacity(items.len());

    for (index, item) in items.into_iter().enumerate() {
        let permits = Arc::clone(&permits);
        let cancel = cancel.clone();
        let run_child = run_child.clone();
        handles.push(tokio::spawn(async move {
            // Wait for a slot, but never past a cancel: a queued child that the
            // caller no longer wants must not start spending on its turn.
            let permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                p = permits.acquire_owned() => p.ok(),
            };
            let Some(permit) = permit else {
                return ChildOutcome::cancelled(index, format!("child {index}"));
            };
            let outcome = run_child(index, item, cancel.child_token()).await;
            drop(permit);
            outcome
        }));
    }

    let mut outcomes: Vec<ChildOutcome> = Vec::with_capacity(handles.len());
    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(outcome) => outcomes.push(outcome),
            // A panicked or aborted child is a failure with a name, not a gap in
            // the results: the caller asked for N answers and gets N.
            Err(e) => outcomes.push(ChildOutcome::failed(
                index,
                format!("child {index}"),
                format!("subagent task did not complete: {e}"),
            )),
        }
    }
    outcomes.sort_by_key(|o| o.index);
    outcomes
}

/// What one subagent spent, read from the ledger by its own id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct SubagentCost {
    pub calls: i64,
    pub cost_usd: f64,
    pub tokens: i64,
}

/// Per-child spend, keyed on `cost_ledger.subagent_id`.
///
/// That column has been written for every subagent call since 2026-08-25 and
/// read by nothing — an attribution that exists only in the schema answers no
/// question. A fan-out is exactly the caller that needs it: it spends on N
/// models at once, and "the fan-out cost $2.20" is not an answer, it is a
/// starting point.
pub async fn subagent_cost(pool: &Pool<Sqlite>, subagent_id: &str) -> SubagentCost {
    let row = sqlx::query(
        "SELECT COUNT(*) AS calls,
                COALESCE(SUM(cost_usd), 0.0) AS cost,
                COALESCE(SUM(input_tokens + output_tokens), 0) AS tokens
           FROM cost_ledger
          WHERE subagent_id = ?",
    )
    .bind(subagent_id)
    .fetch_one(pool)
    .await;

    match row {
        Ok(r) => SubagentCost {
            calls: r.try_get("calls").unwrap_or(0),
            cost_usd: r.try_get("cost").unwrap_or(0.0),
            tokens: r.try_get("tokens").unwrap_or(0),
        },
        Err(e) => {
            tracing::warn!(
                target: "permagent::fanout",
                subagent_id = %subagent_id,
                "could not read the subagent's ledger rows: {e}"
            );
            SubagentCost::default()
        }
    }
}

/// The aggregate a caller reads: every child in the order it was asked for,
/// each naming its own model, its own spend, and its own outcome.
pub fn render_outcomes(outcomes: &[ChildOutcome], costs: &[SubagentCost]) -> String {
    let mut out = String::new();
    let total: f64 = costs.iter().map(|c| c.cost_usd).sum();
    out.push_str(&format!(
        "Fan-out of {} child(ren) complete ({} ok, {} failed, {} cancelled). \
         Total billed to the children: ${total:.4}\n",
        outcomes.len(),
        outcomes
            .iter()
            .filter(|o| o.status == ChildStatus::Ok)
            .count(),
        outcomes
            .iter()
            .filter(|o| o.status == ChildStatus::Failed)
            .count(),
        outcomes
            .iter()
            .filter(|o| o.status == ChildStatus::Cancelled)
            .count(),
    ));
    for (i, o) in outcomes.iter().enumerate() {
        let cost = costs.get(i).copied().unwrap_or_default();
        out.push_str(&format!(
            "\n── [{}] {} — {} (${:.4} over {} call(s)){}\n{}\n",
            o.index,
            o.label,
            o.status.as_str(),
            cost.cost_usd,
            cost.calls,
            o.subagent_id
                .as_deref()
                .map(|id| format!(" · subagent {id}"))
                .unwrap_or_default(),
            o.text,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A fake provider: it "runs" for a tick, records how many children were in
    /// flight while it did, and answers with its own index.
    #[derive(Clone, Default)]
    struct FakeProvider {
        in_flight: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        started: Arc<AtomicUsize>,
    }

    impl FakeProvider {
        async fn run(&self, index: usize, label: String, cancel: CancellationToken) -> ChildOutcome {
            self.started.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            let cancelled = tokio::select! {
                _ = cancel.cancelled() => true,
                _ = tokio::time::sleep(Duration::from_millis(40)) => false,
            };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            ChildOutcome {
                index,
                label,
                status: if cancelled {
                    ChildStatus::Cancelled
                } else {
                    ChildStatus::Ok
                },
                subagent_id: Some(format!("sub-{index}")),
                model_routing: None,
                text: format!("answer from child {index}"),
            }
        }
    }

    fn labels(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("task {i}")).collect()
    }

    #[tokio::test]
    async fn children_run_concurrently_but_never_past_the_cap() {
        let fake = FakeProvider::default();
        let f = fake.clone();
        let outcomes = run_bounded(labels(6), 2, CancellationToken::new(), move |i, label, tok| {
            let f = f.clone();
            async move { f.run(i, label, tok).await }
        })
        .await;

        assert_eq!(outcomes.len(), 6);
        assert!(
            outcomes.iter().all(|o| o.status == ChildStatus::Ok),
            "every child should have finished"
        );
        assert_eq!(
            fake.peak.load(Ordering::SeqCst),
            2,
            "the cap is a cap: never more than 2 children in flight at once"
        );
        assert!(
            fake.peak.load(Ordering::SeqCst) > 1,
            "and it is a fan-out: more than one child ran at a time"
        );
    }

    #[tokio::test]
    async fn a_cap_of_one_serialises() {
        let fake = FakeProvider::default();
        let f = fake.clone();
        let outcomes = run_bounded(labels(3), 1, CancellationToken::new(), move |i, label, tok| {
            let f = f.clone();
            async move { f.run(i, label, tok).await }
        })
        .await;
        assert_eq!(outcomes.len(), 3);
        assert_eq!(fake.peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn results_join_in_the_order_they_were_asked_for() {
        // Later children finish FIRST — the aggregate must still read 0,1,2,3.
        let outcomes = run_bounded(
            vec![80u64, 60, 40, 1],
            4,
            CancellationToken::new(),
            move |i, delay, _tok| async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                ChildOutcome {
                    index: i,
                    label: format!("task {i}"),
                    status: ChildStatus::Ok,
                    subagent_id: Some(format!("sub-{i}")),
                    model_routing: None,
                    text: format!("answer from child {i}"),
                }
            },
        )
        .await;

        let order: Vec<usize> = outcomes.iter().map(|o| o.index).collect();
        assert_eq!(order, vec![0, 1, 2, 3]);
        assert!(outcomes[0].text.contains("child 0"));
        assert!(outcomes[3].text.contains("child 3"));
    }

    #[tokio::test]
    async fn a_parent_cancel_reaches_every_child() {
        let fake = FakeProvider::default();
        let f = fake.clone();
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_for_task.cancel();
        });

        let outcomes = run_bounded(labels(6), 2, cancel, move |i, label, tok| {
            let f = f.clone();
            async move { f.run(i, label, tok).await }
        })
        .await;

        assert_eq!(outcomes.len(), 6, "a cancelled fan-out still answers for all");
        assert!(
            outcomes.iter().all(|o| o.status == ChildStatus::Cancelled),
            "running children see the cancel, queued children never start: {:?}",
            outcomes.iter().map(|o| o.status).collect::<Vec<_>>()
        );
        assert!(
            fake.started.load(Ordering::SeqCst) <= 2,
            "a cancel must stop the queue, not merely mark it: {} children started",
            fake.started.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn a_panicking_child_is_a_named_failure_not_a_missing_answer() {
        let outcomes = run_bounded(
            vec![0usize, 1, 2],
            2,
            CancellationToken::new(),
            move |i, _item, _tok| async move {
                if i == 1 {
                    panic!("child 1 exploded");
                }
                ChildOutcome {
                    index: i,
                    label: format!("task {i}"),
                    status: ChildStatus::Ok,
                    subagent_id: None,
                    model_routing: None,
                    text: "fine".to_string(),
                }
            },
        )
        .await;

        assert_eq!(outcomes.len(), 3);
        assert_eq!(outcomes[1].status, ChildStatus::Failed);
        assert!(outcomes[1].text.contains("did not complete"));
        assert_eq!(outcomes[0].status, ChildStatus::Ok);
        assert_eq!(outcomes[2].status, ChildStatus::Ok);
    }

    async fn ledger_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn cost_is_attributed_per_child_not_as_one_total() {
        let pool = ledger_pool().await;
        for (sub, cost) in [("sub-0", 0.25_f64), ("sub-0", 0.75), ("sub-1", 0.10)] {
            sqlx::query(
                "INSERT INTO cost_ledger
                   (call_id, ts, session_id, subagent_id, provider, model,
                    input_tokens, output_tokens, cost_usd)
                 VALUES (?, '2026-08-25T00:00:00Z', ?, ?, 'anthropic', 'fake', 10, 5, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(sub)
            .bind(sub)
            .bind(cost)
            .execute(&pool)
            .await
            .unwrap();
        }

        let zero = subagent_cost(&pool, "sub-0").await;
        assert_eq!(zero.calls, 2);
        assert!((zero.cost_usd - 1.0).abs() < 1e-9, "{zero:?}");
        assert_eq!(zero.tokens, 30);

        let one = subagent_cost(&pool, "sub-1").await;
        assert_eq!(one.calls, 1);
        assert!((one.cost_usd - 0.10).abs() < 1e-9, "{one:?}");

        let none = subagent_cost(&pool, "sub-never-ran").await;
        assert_eq!(none, SubagentCost::default());

        let rendered = render_outcomes(
            &[
                ChildOutcome {
                    index: 0,
                    label: "security".into(),
                    status: ChildStatus::Ok,
                    subagent_id: Some("sub-0".into()),
                    model_routing: None,
                    text: "looks fine".into(),
                },
                ChildOutcome {
                    index: 1,
                    label: "debugger".into(),
                    status: ChildStatus::Ok,
                    subagent_id: Some("sub-1".into()),
                    model_routing: None,
                    text: "no swallowed errors".into(),
                },
            ],
            &[zero, one],
        );
        assert!(rendered.contains("subagent sub-0"), "{rendered}");
        assert!(rendered.contains("$1.0000"), "{rendered}");
        assert!(rendered.contains("$0.1000"), "{rendered}");
        assert!(rendered.contains("$1.1000"), "the total is still shown: {rendered}");
    }
}
