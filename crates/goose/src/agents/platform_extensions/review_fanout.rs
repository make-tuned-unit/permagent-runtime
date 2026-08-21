//! Parallel in-process review fan-out on goal Review (Prime async subagents).
//!
//! Opt-in: goal or project metadata `review_fanout: true`, or env
//! `PERMAGENT_REVIEW_FANOUT=1`. Default remains the single-path approve_review
//! detail. When enabled, two review workers (security + debugger) are spawned
//! via [`crate::agents::subagent_handler::spawn_subagent_work`] and their
//! summaries fold into the decision detail. Either failure degrades: the
//! other summary still lands.

use crate::agents::subagent_handler::spawn_subagent_work;
use crate::cards::Card;
use serde_json::Value;

pub const REVIEW_FANOUT_KEY: &str = "review_fanout";
pub const REVIEW_FANOUT_ENV: &str = "PERMAGENT_REVIEW_FANOUT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    Security,
    Debugger,
}

impl ReviewKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Debugger => "debugger",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldedReviews {
    pub security: Option<String>,
    pub debugger: Option<String>,
}

pub fn is_enabled(goal_meta: &Value, project_meta: &Value) -> bool {
    if flag(goal_meta) || flag(project_meta) {
        return true;
    }
    matches!(
        std::env::var(REVIEW_FANOUT_ENV).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn flag(meta: &Value) -> bool {
    meta.get(REVIEW_FANOUT_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Spawn two review workers and fold. Degrades if either join fails.
pub async fn run_parallel_reviews(card: &Card) -> FoldedReviews {
    let title = card.title.clone();
    let evidence = card
        .metadata_json
        .get("dispatch_evidence")
        .cloned()
        .unwrap_or(Value::Null);

    let sec_title = title.clone();
    let sec_ev = evidence.clone();
    let dbg_title = title;
    let dbg_ev = evidence;

    let security =
        spawn_subagent_work(async move { review_brief(ReviewKind::Security, &sec_title, &sec_ev) });
    let debugger =
        spawn_subagent_work(async move { review_brief(ReviewKind::Debugger, &dbg_title, &dbg_ev) });

    fold_handles(security, debugger).await
}

pub async fn fold_handles(
    security: tokio::task::JoinHandle<String>,
    debugger: tokio::task::JoinHandle<String>,
) -> FoldedReviews {
    let (sec, dbg) = tokio::join!(security, debugger);
    FoldedReviews {
        security: sec.ok(),
        debugger: dbg.ok(),
    }
}

pub fn append_to_detail(detail: &str, folded: &FoldedReviews) -> String {
    let mut out = detail.to_string();
    if folded.security.is_none() && folded.debugger.is_none() {
        return out;
    }
    out.push_str("\n\nParallel review fan-out:");
    match &folded.security {
        Some(s) => out.push_str(&format!("\n\n[security]\n{s}")),
        None => out.push_str("\n\n[security] (unavailable — degraded)"),
    }
    match &folded.debugger {
        Some(s) => out.push_str(&format!("\n\n[debugger]\n{s}")),
        None => out.push_str("\n\n[debugger] (unavailable — degraded)"),
    }
    out
}

fn review_brief(kind: ReviewKind, title: &str, evidence: &Value) -> String {
    let commits = evidence
        .get("commits")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let files = evidence
        .get("files_changed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    match kind {
        ReviewKind::Security => format!(
            "Security review of \"{title}\": {commits} commit(s), {files} file(s) changed. \
             Check for credential-shaped content, authz gaps, and untrusted input. \
             This brief is an in-process review worker (no extra LLM required)."
        ),
        ReviewKind::Debugger => format!(
            "Debugger review of \"{title}\": {commits} commit(s), {files} file(s) changed. \
             Look for missing tests, swallowed errors, and incomplete error paths. \
             This brief is an in-process review worker (no extra LLM required)."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::subagent_handler::spawn_subagent_work;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn default_is_off() {
        let _guard = env_lock::lock_env([(REVIEW_FANOUT_ENV, None::<&str>)]);
        assert!(!is_enabled(&json!({}), &json!({})));
        assert!(is_enabled(&json!({REVIEW_FANOUT_KEY: true}), &json!({})));
        assert!(is_enabled(&json!({}), &json!({REVIEW_FANOUT_KEY: true})));
    }

    #[tokio::test]
    async fn fan_out_schedules_two_spawns_before_fold() {
        let started = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = tokio::sync::watch::channel(false);

        let s1 = started.clone();
        let mut r1 = rx.clone();
        let h1 = spawn_subagent_work(async move {
            s1.fetch_add(1, Ordering::SeqCst);
            let _ = r1.changed().await;
            "sec".to_string()
        });
        let s2 = started.clone();
        let mut r2 = rx;
        let h2 = spawn_subagent_work(async move {
            s2.fetch_add(1, Ordering::SeqCst);
            let _ = r2.changed().await;
            "dbg".to_string()
        });

        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        while started.load(Ordering::SeqCst) < 2 {
            if tokio::time::Instant::now() > deadline {
                panic!("both review workers should have started before join");
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !h1.is_finished() && !h2.is_finished(),
            "both handles must still be outstanding before release"
        );
        tx.send(true).unwrap();
        let folded = fold_handles(h1, h2).await;
        assert_eq!(folded.security.as_deref(), Some("sec"));
        assert_eq!(folded.debugger.as_deref(), Some("dbg"));
    }

    #[test]
    fn fold_degrades_when_one_side_missing() {
        let detail = append_to_detail(
            "base",
            &FoldedReviews {
                security: Some("ok".into()),
                debugger: None,
            },
        );
        assert!(detail.contains("[security]"));
        assert!(detail.contains("unavailable"));
    }
}
