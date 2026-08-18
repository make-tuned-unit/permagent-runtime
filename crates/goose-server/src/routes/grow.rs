//! Grow tab — the deterministic "growth inbox" (#634).
//!
//! `GET /api/projects/:project_id/growth-inbox` runs a **NO-LLM** ranker over
//! the project's real signals (social_post cards + shipped/active goals + post
//! dates) and returns the 2-3 growth moves worth making this week, plus a
//! "keep doing" wins list. It fills the Grow tab's Analytics lens.
//!
//! ## The algorithm (adapted from the Apache-2.0 `gtm-superintelligence`
//! `inbox.py` — reimplemented, not copied)
//!
//! A deterministic ranker over structured focus areas ("levers"). Each lever
//! carries an `evidence_count` (how much real activity backs it — the
//! *frequency*) and a `score` 0-100 (how strong the project is on that lever).
//! Levers are ranked by `frequency × weakness` = `evidence_count × (100 −
//! score)`: something that matters a lot (lots of evidence) *and* is weak (low
//! score) rises to the top. Weak levers become **moves**, strong levers become
//! **wins** ("keep doing"), and priority is set by frequency. Pure integer
//! math, no model call — cheap enough to run on every read, so the inbox is
//! always fresh and there is no scheduler to own.
//!
//! ## Honest signals only
//!
//! Everything here is derived from data that exists today. Deliberately NOT
//! invented (no source without an events pipeline): reach, visitors, signups,
//! retention — the Analytics lens keeps its honest "awaiting analytics" tiles
//! for those. A `social_post` card is counted as one post regardless of
//! draft/scheduled/published, because no status field distinguishes them yet.
//! Project age is available but intentionally not ranked (any age-only lever
//! would fire on empty projects, which should instead show the "not enough
//! signal" empty state).

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use permagent::{cards, projects};
use serde::Serialize;
use std::sync::Arc;

// ── Response types ─────────────────────────────────────────────────────────

/// One growth move — a concrete, ranked next action for this week.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthMove {
    /// Short imperative headline ("Post again — it's been 12 days").
    pub title: String,
    /// One sentence of grounding that cites the real signal behind the move.
    pub why: String,
    /// `"high"` | `"medium"` | `"low"`, set by frequency (evidence volume).
    pub priority: &'static str,
    /// How many real signals back this move (the "frequency").
    pub evidence_count: u32,
}

/// One win — something the project is already doing well ("keep doing").
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthWin {
    pub title: String,
    pub why: String,
}

/// The signals the ranker saw, surfaced verbatim so the UI can be honest about
/// what "this week's moves" were computed from — no hidden inputs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthSignalSummary {
    pub posts: u32,
    pub shipped: u32,
    pub active_goals: u32,
    pub days_since_last_post: Option<i64>,
}

/// The growth inbox payload returned by the route.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthInbox {
    pub moves: Vec<GrowthMove>,
    pub wins: Vec<GrowthWin>,
    pub signal: GrowthSignalSummary,
}

// ── Signals (the pure input to the ranker) ──────────────────────────────────

/// The structured, real-signal snapshot the deterministic ranker consumes.
/// Extracted from the project's cards up front so the ranker itself never
/// touches the DB (and is trivially unit-testable).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrowthSignals {
    /// social_post cards on the project.
    pub posts: u32,
    /// Goal cards sitting in a lifecycle column bound to `complete` (shipped).
    pub shipped: u32,
    /// Goal cards in an active column (`ready` / `in_progress`).
    pub active_goals: u32,
    /// Whole days since the most recent post. `None` if nothing posted.
    pub days_since_last_post: Option<i64>,
}

// ── Ranker ──────────────────────────────────────────────────────────────────

/// Score at/above which a lever is a strength (a win); below it, the lever's
/// move surfaces. 0-100, higher = stronger.
const STRONG_SCORE: u32 = 70;
/// Cap on surfaced moves — "your 2-3 growth moves this week".
const MAX_MOVES: usize = 3;
/// Cap on the wins strip so it stays a glanceable "keep doing" row.
const MAX_WINS: usize = 3;

/// A single evaluated focus area (growth lever). Internal to the ranker: it
/// carries both the move copy (used when weak) and the win copy (when strong),
/// so the same evaluation drives either outcome.
struct Lever {
    /// Stable key, used only as the final deterministic tie-break.
    key: &'static str,
    evidence_count: u32,
    /// 0-100 strength; `100 − score` is the weakness the ranker multiplies.
    score: u32,
    move_title: String,
    move_why: String,
    win_title: String,
    win_why: String,
}

impl Lever {
    /// `frequency × weakness` — the gtm-si ranking value. `saturating_sub`
    /// guards the weakness against any future score > 100.
    fn rank_value(&self) -> u32 {
        self.evidence_count
            .saturating_mul(100u32.saturating_sub(self.score))
    }
}

/// Priority label from frequency (evidence volume). Absolute thresholds, small
/// by design: early projects accumulate signal slowly, and a project with more
/// real activity behind a lever has more at stake, so its move ranks higher.
fn priority_for(evidence_count: u32) -> &'static str {
    match evidence_count {
        0..=1 => "low",
        2..=3 => "medium",
        _ => "high",
    }
}

/// Evaluate the growth levers for a project from its real signals. A lever is
/// omitted entirely when it has no supporting evidence, so nothing surfaces
/// that the data can't back.
fn levers(s: &GrowthSignals) -> Vec<Lever> {
    let mut out = Vec::new();

    // L1 — Publishing cadence. About the *rhythm* of an active publisher, so it
    // only applies once there's at least one post. Evidence is the size of the
    // content opportunity (posts made + wins shipped worth talking about);
    // score is the recency of the last post.
    if s.posts > 0 {
        let days = s.days_since_last_post.unwrap_or(0);
        let score = match days {
            d if d <= 7 => 85,
            d if d <= 14 => 60,
            d if d <= 30 => 35,
            _ => 15,
        };
        out.push(Lever {
            key: "cadence",
            evidence_count: s.posts + s.shipped,
            score,
            move_title: format!("Post again — it's been {days} days"),
            move_why:
                // No persona name here: this is a pure function with no access
                // to the configured identity, and every user's agent has a
                // different name. Dropping the self-reference is also better
                // copy — the agent is the one saying this.
                "Momentum comes from cadence. Draft the next post before the gap widens."
                    .to_string(),
            win_title: "Posting consistently".to_string(),
            win_why: format!("Last post {days} days ago — keep the rhythm going."),
        });
    }

    // L2 — Announce shipped work. Each shipped goal is a story; the classic
    // builder gap is shipping in silence. Evidence is the shipped count; score
    // is how well your posts cover what you've shipped (posts as a % of
    // shipped, capped at 100).
    if s.shipped > 0 {
        let score = (s.posts.saturating_mul(100) / s.shipped).min(100);
        out.push(Lever {
            key: "announce",
            evidence_count: s.shipped,
            score,
            move_title: "Announce your shipped work".to_string(),
            move_why: format!(
                "You've shipped {shipped} but published {posts} — each shipped goal is a story. \
                 Turn the latest into a post.",
                shipped = s.shipped,
                posts = s.posts
            ),
            win_title: "Shipping in public".to_string(),
            win_why: format!(
                "{posts} posts for {shipped} shipped — you're telling the story as you build.",
                posts = s.posts,
                shipped = s.shipped
            ),
        });
    }

    // L3 — Keep momentum on the board. Traction (posts + shipped) with nothing
    // active means the pipeline has stalled; a goal in flight is positive
    // momentum the agent is already driving. Only meaningful once there's traction
    // worth steering. (Goals are general project goals, not growth-typed — this
    // reads "is work moving?", not "is this a growth goal?".)
    let traction = s.posts + s.shipped;
    if traction > 0 {
        let score = if s.active_goals > 0 { 90 } else { 20 };
        out.push(Lever {
            key: "momentum",
            evidence_count: traction,
            score,
            move_title: "Line up your next goal".to_string(),
            move_why: format!(
                "You've got traction ({posts} posts, {shipped} shipped) but nothing active on the \
                 board. Set a goal to drive the next move.",
                posts = s.posts,
                shipped = s.shipped
            ),
            win_title: "Momentum on the board".to_string(),
            win_why: format!(
                "{active} goal(s) in flight — the work is moving.",
                active = s.active_goals
            ),
        });
    }

    out
}

/// Rank the project's growth levers into this week's moves + wins. Pure and
/// deterministic — the same signals always produce the same inbox (the whole
/// point: cheap enough to run on every read, and trustworthy).
pub fn rank_growth_inbox(signals: GrowthSignals) -> GrowthInbox {
    let levers = levers(&signals);

    // Moves: weak levers, ranked by `frequency × weakness` (desc). Deterministic
    // tie-break — higher evidence first, then key (alphabetical) — so equal
    // ranks never reorder run-to-run.
    let mut move_levers: Vec<&Lever> = levers
        .iter()
        .filter(|l| l.evidence_count > 0 && l.score < STRONG_SCORE)
        .collect();
    move_levers.sort_by(|a, b| {
        b.rank_value()
            .cmp(&a.rank_value())
            .then(b.evidence_count.cmp(&a.evidence_count))
            .then(a.key.cmp(b.key))
    });
    let moves: Vec<GrowthMove> = move_levers
        .into_iter()
        .take(MAX_MOVES)
        .map(|l| GrowthMove {
            title: l.move_title.clone(),
            why: l.move_why.clone(),
            priority: priority_for(l.evidence_count),
            evidence_count: l.evidence_count,
        })
        .collect();

    // Wins: strong levers, best score first (same deterministic tie-break).
    let mut win_levers: Vec<&Lever> = levers
        .iter()
        .filter(|l| l.evidence_count > 0 && l.score >= STRONG_SCORE)
        .collect();
    win_levers.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.evidence_count.cmp(&a.evidence_count))
            .then(a.key.cmp(b.key))
    });
    let wins: Vec<GrowthWin> = win_levers
        .into_iter()
        .take(MAX_WINS)
        .map(|l| GrowthWin {
            title: l.win_title.clone(),
            why: l.win_why.clone(),
        })
        .collect();

    GrowthInbox {
        moves,
        wins,
        signal: GrowthSignalSummary {
            posts: signals.posts,
            shipped: signals.shipped,
            active_goals: signals.active_goals,
            days_since_last_post: signals.days_since_last_post,
        },
    }
}

// ── Signal extraction (pure; the DB read stays in the handler) ───────────────

/// Parse a card timestamp: RFC3339 first, then SQLite's space-separated
/// `CURRENT_TIMESTAMP` (`YYYY-MM-DD HH:MM:SS`, UTC). Mirrors the tolerant parse
/// used across the brain routes — a fresh card carries the space form; one that
/// has been update-triggered carries RFC3339.
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    s.parse::<DateTime<Utc>>().ok().or_else(|| {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|dt| dt.and_utc())
    })
}

/// Build [`GrowthSignals`] from already-fetched project data. Pure — no DB and
/// no clock of its own (`now` is passed in) — so the card→signal mapping is
/// unit-testable. A goal is classified by the `state_binding` of the column it
/// sits in (`complete` = shipped; `ready`/`in_progress` = active).
fn signals_from_cards(
    posts: &[cards::Card],
    goals: &[cards::Card],
    columns: &[cards::BoardColumn],
    now: DateTime<Utc>,
) -> GrowthSignals {
    use std::collections::HashMap;
    let binding: HashMap<&str, &str> = columns
        .iter()
        .filter_map(|c| c.state_binding.as_deref().map(|b| (c.id.as_str(), b)))
        .collect();

    let mut shipped = 0u32;
    let mut active_goals = 0u32;
    for g in goals {
        match binding.get(g.column_id.as_str()).copied() {
            Some("complete") => shipped += 1,
            Some("ready") | Some("in_progress") => active_goals += 1,
            _ => {}
        }
    }

    let days_since_last_post = posts
        .iter()
        .filter_map(|c| parse_ts(&c.created_at))
        .max()
        .map(|ts| (now - ts).num_days().max(0));

    GrowthSignals {
        posts: posts.len() as u32,
        shipped,
        active_goals,
        days_since_last_post,
    }
}

// ── Route ────────────────────────────────────────────────────────────────────

async fn growth_inbox_handler(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<GrowthInbox>, StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 404 on an unknown project (accepts id or slug, like the sibling routes).
    let project = projects::get_project_by_id_or_slug(&pool, &project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let posts = cards::list_cards(&pool, &project.id, Some("social_post"), None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let goals = cards::list_cards(&pool, &project.id, Some("goal"), None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let columns = cards::list_columns(&pool, &project.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let signals = signals_from_cards(&posts, &goals, &columns, Utc::now());
    Ok(Json(rank_growth_inbox(signals)))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/projects/{project_id}/growth-inbox",
            get(growth_inbox_handler),
        )
        .with_state(state)
}

// ── Tests ────────────────────────────────────────────────────────────────────
//
// The ranker and the card→signal mapping are pure, so these are plain unit
// tests — no AppState, no DB, no #[serial] needed.

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(posts: u32, shipped: u32, active: u32, days: Option<i64>) -> GrowthSignals {
        GrowthSignals {
            posts,
            shipped,
            active_goals: active,
            days_since_last_post: days,
        }
    }

    #[test]
    fn thin_signal_yields_empty_inbox() {
        // Brand-new project: nothing published, nothing shipped → the "not
        // enough signal yet" empty state, not a fabricated move.
        let inbox = rank_growth_inbox(sig(0, 0, 0, None));
        assert!(inbox.moves.is_empty(), "no signal → no moves");
        assert!(inbox.wins.is_empty(), "no signal → no wins");
    }

    #[test]
    fn a_lone_active_goal_is_not_enough_signal() {
        // A goal set but nothing posted or shipped is still the empty state:
        // there's no growth activity to rank yet.
        let inbox = rank_growth_inbox(sig(0, 0, 2, None));
        assert!(inbox.moves.is_empty());
        assert!(inbox.wins.is_empty());
    }

    #[test]
    fn silent_builder_is_told_to_publish_and_earns_no_win() {
        // Shipped 4, never posted → should be pushed to announce/publish, and
        // must NOT earn any "keep doing" win.
        let inbox = rank_growth_inbox(sig(0, 4, 0, None));
        assert!(!inbox.moves.is_empty(), "shipped-but-silent → moves");
        assert!(inbox.wins.is_empty(), "silent → no wins");
        // Announce is the strongest lever (0% post coverage of 4 shipped).
        assert!(inbox.moves[0].title.contains("Announce"));
        // Evidence of 4 → "high" priority.
        assert_eq!(inbox.moves[0].priority, "high");
    }

    #[test]
    fn stale_poster_is_nudged_with_the_day_count() {
        let inbox = rank_growth_inbox(sig(3, 0, 1, Some(21)));
        assert!(
            inbox.moves.iter().any(|m| m.title.contains("21 days")),
            "a 21-day-stale poster should be nudged by day count: {:?}",
            inbox.moves
        );
    }

    #[test]
    fn healthy_project_is_all_wins_no_moves() {
        // Recent post, posts cover shipped, an active goal → strengths only.
        let inbox = rank_growth_inbox(sig(5, 3, 2, Some(3)));
        assert!(inbox.moves.is_empty(), "healthy → no urgent moves");
        assert!(!inbox.wins.is_empty(), "healthy → wins to keep doing");
    }

    #[test]
    fn moves_are_capped_and_ranked_by_frequency_times_weakness() {
        // Everything weak: many shipped, silent, no active goal.
        //   announce: evidence 9, score 0  → rank 900 (ranked first)
        //   momentum: evidence 9, score 20 → rank 720 (ranked second)
        //   cadence:  absent (0 posts)
        let inbox = rank_growth_inbox(sig(0, 9, 0, None));
        assert!(inbox.moves.len() <= MAX_MOVES);
        assert_eq!(inbox.moves.len(), 2);
        assert!(inbox.moves[0].title.contains("Announce"));
        assert!(inbox.moves[1].title.contains("next goal"));
    }

    #[test]
    fn recency_bands_flip_a_poster_between_move_and_win() {
        // 5-day-old post → strong (win, no move); 20-day-old → weak (move).
        assert!(rank_growth_inbox(sig(2, 0, 1, Some(5))).moves.is_empty());
        assert!(!rank_growth_inbox(sig(2, 0, 1, Some(20))).moves.is_empty());
    }

    #[test]
    fn output_is_deterministic() {
        let s = sig(2, 5, 0, Some(40));
        assert_eq!(rank_growth_inbox(s), rank_growth_inbox(s));
    }

    // ── card → signal mapping ──

    fn col(id: &str, binding: &str) -> cards::BoardColumn {
        cards::BoardColumn {
            id: id.to_string(),
            project_id: "p".to_string(),
            name: id.to_string(),
            position: 0,
            column_kind: "custom".to_string(),
            state_binding: Some(binding.to_string()),
            wip_limit: None,
            created_at: "2026-07-01 00:00:00".to_string(),
        }
    }

    fn card(id: &str, card_type: &str, column_id: &str, created_at: &str) -> cards::Card {
        cards::Card {
            id: id.to_string(),
            project_id: "p".to_string(),
            card_type: card_type.to_string(),
            title: id.to_string(),
            description: String::new(),
            column_id: column_id.to_string(),
            position: 0,
            created_by: "user".to_string(),
            assigned_to: None,
            metadata_json: serde_json::json!({}),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            archived_at: None,
        }
    }

    #[test]
    fn signals_from_cards_counts_shipped_active_and_recency() {
        let now = "2026-07-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let columns = [
            col("c_done", "complete"),
            col("c_prog", "in_progress"),
            col("c_triage", "triage"),
        ];
        let goals = [
            card("g1", "goal", "c_done", "2026-07-01 00:00:00"),
            card("g2", "goal", "c_done", "2026-07-01 00:00:00"),
            card("g3", "goal", "c_prog", "2026-07-01 00:00:00"),
            card("g4", "goal", "c_triage", "2026-07-01 00:00:00"), // neither
        ];
        let posts = [
            card("p1", "social_post", "c_prog", "2026-07-10 00:00:00"), // 4d ago
            card("p2", "social_post", "c_prog", "2026-07-01 00:00:00"), // older
        ];
        let s = signals_from_cards(&posts, &goals, &columns, now);
        assert_eq!(s.posts, 2);
        assert_eq!(s.shipped, 2);
        assert_eq!(s.active_goals, 1);
        assert_eq!(s.days_since_last_post, Some(4), "most recent post wins");
    }

    #[test]
    fn signals_from_cards_handles_rfc3339_timestamps() {
        // A post whose created_at was rewritten by the update trigger (RFC3339).
        let now = "2026-07-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let posts = [card("p1", "social_post", "c", "2026-07-12T09:30:00Z")];
        let s = signals_from_cards(&posts, &[], &[], now);
        assert_eq!(s.posts, 1);
        assert_eq!(s.days_since_last_post, Some(1));
    }

    #[test]
    fn signals_from_cards_no_posts_has_no_recency() {
        let now = "2026-07-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let s = signals_from_cards(&[], &[], &[], now);
        assert_eq!(s.posts, 0);
        assert_eq!(s.days_since_last_post, None);
    }
}
