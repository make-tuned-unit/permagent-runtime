//! Inbox curation: deterministic ranking of open decisions.
//!
//! Approved formula (Phase 0 §4):
//!
//! ```text
//! score = 0.5 * blocked_norm + 0.3 * priority_norm + 0.2 * age_norm
//!
//! blocked_norm  = min(transitive_blocked_goals / 5, 1.0)
//! priority_norm = {low: 0.25, normal: 0.5, high: 0.75, critical: 1.0}
//! age_norm      = min(age_hours / 72, 1.0)
//! ```
//!
//! Tie-break: older `created_at` first, then decision id ascending.
//! Malformed items get a score floor of 0.30. At most 10 items surface.
//!
//! Pure functions over decision-shaped structs — Part B feeds real rows
//! from Lane L1's decisions table. Zero cloud tokens, no local model.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_SURFACED: usize = 10;
pub const MALFORMED_SCORE_FLOOR: f64 = 0.30;
const BLOCKED_CAP: f64 = 5.0;
const AGE_CAP_HOURS: f64 = 72.0;

/// Priority of the goal/card a decision is blocking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl Priority {
    pub fn norm(&self) -> f64 {
        match self {
            Priority::Low => 0.25,
            Priority::Normal => 0.5,
            Priority::High => 0.75,
            Priority::Critical => 1.0,
        }
    }
}

/// Decision-shaped ranking input. Part B populates these from L1's
/// decisions table joined against goal cards.
#[derive(Debug, Clone)]
pub struct OpenDecision {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub priority: Priority,
    /// Transitive count of goals whose `depends_on` chain passes through
    /// the blocked goal (see [`transitive_dependents`]).
    pub blocked_goals: usize,
    pub malformed: bool,
}

/// A ranked decision with its score components, so the UI can explain
/// each rank ("blocks 3 goals, open 2 days").
#[derive(Debug, Clone)]
pub struct RankedDecision {
    pub id: String,
    pub score: f64,
    pub blocked_norm: f64,
    pub priority_norm: f64,
    pub age_norm: f64,
}

/// Score a single open decision at time `now`.
pub fn score_decision(d: &OpenDecision, now: DateTime<Utc>) -> RankedDecision {
    let blocked_norm = ((d.blocked_goals as f64) / BLOCKED_CAP).min(1.0);
    let priority_norm = d.priority.norm();
    let age_hours = (now - d.created_at).num_seconds().max(0) as f64 / 3600.0;
    let age_norm = (age_hours / AGE_CAP_HOURS).min(1.0);

    let mut score = 0.5 * blocked_norm + 0.3 * priority_norm + 0.2 * age_norm;
    if d.malformed {
        score = score.max(MALFORMED_SCORE_FLOOR);
    }

    RankedDecision {
        id: d.id.clone(),
        score,
        blocked_norm,
        priority_norm,
        age_norm,
    }
}

/// Rank open decisions: highest score first, fully deterministic
/// (tie-break on older `created_at`, then id ascending), capped at
/// [`MAX_SURFACED`] items.
pub fn rank_decisions(decisions: &[OpenDecision], now: DateTime<Utc>) -> Vec<RankedDecision> {
    let mut scored: Vec<(RankedDecision, DateTime<Utc>)> = decisions
        .iter()
        .map(|d| (score_decision(d, now), d.created_at))
        .collect();

    scored.sort_by(|(a, a_created), (b, b_created)| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a_created.cmp(b_created))
            .then_with(|| a.id.cmp(&b.id))
    });

    scored
        .into_iter()
        .take(MAX_SURFACED)
        .map(|(r, _)| r)
        .collect()
}

/// Count the goals whose dependency chain passes through `goal_id`.
///
/// `edges` are `(goal, depends_on_goal)` pairs — exactly the shape stored
/// by `create_roadmap` in card metadata (`depends_on` arrays of card ids).
/// Cycle-safe via a visited set.
pub fn transitive_dependents(goal_id: &str, edges: &[(String, String)]) -> usize {
    // Reverse adjacency: dependency → goals that depend on it.
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for (goal, dep) in edges {
        dependents
            .entry(dep.as_str())
            .or_default()
            .push(goal.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(goal_id);
    visited.insert(goal_id);

    while let Some(current) = queue.pop_front() {
        if let Some(children) = dependents.get(current) {
            for &child in children {
                if visited.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }

    visited.len() - 1 // exclude the goal itself
}

/// Map a goal card's metadata `priority` string to a [`Priority`]
/// (unknown/absent → Normal).
pub fn priority_from_metadata(metadata: &serde_json::Value) -> Priority {
    match metadata.get("priority").and_then(|v| v.as_str()) {
        Some("low") => Priority::Low,
        Some("high") => Priority::High,
        Some("critical") => Priority::Critical,
        _ => Priority::Normal,
    }
}

/// Part B: deterministic re-rank pass over OPEN decisions — scores each one
/// and persists the score into L1's `decisions.rank` column (GET
/// /api/decisions orders by `rank DESC NULLS LAST`). Returns the number of
/// rows updated.
///
/// Call sites (outside this module; coordinator inserts): L1's list handler
/// before listing, or the orchestrator heartbeat. Pure SQL + arithmetic —
/// zero cloud tokens, no local model.
pub async fn rerank_open_decisions(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<usize, String> {
    let now = Utc::now();
    let open = crate::decisions::list_open_decisions(pool).await?;
    if open.is_empty() {
        return Ok(0);
    }

    // Dependency edges, loaded once per project referenced.
    let mut edges_by_project: HashMap<String, Vec<(String, String)>> = HashMap::new();

    let mut updated = 0usize;
    for item in &open {
        let d = &item.decision;

        let (priority, blocked_goals) = match d.goal_id.as_deref() {
            Some(goal_id) => match crate::cards::get_card(pool, goal_id).await? {
                Some(card) => {
                    let edges = match edges_by_project.get(&card.project_id) {
                        Some(e) => e,
                        None => {
                            let loaded = load_goal_dependency_edges(pool, &card.project_id).await?;
                            edges_by_project
                                .entry(card.project_id.clone())
                                .or_insert(loaded)
                        }
                    };
                    (
                        priority_from_metadata(&card.metadata_json),
                        transitive_dependents(goal_id, edges),
                    )
                }
                None => (Priority::Normal, 0),
            },
            None => (Priority::Normal, 0),
        };

        let created_at = DateTime::parse_from_rfc3339(&d.created_at)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or(now);

        let scored = score_decision(
            &OpenDecision {
                id: d.id.clone(),
                created_at,
                priority,
                blocked_goals,
                malformed: d.kind == "malformed",
            },
            now,
        );

        let result = sqlx::query("UPDATE decisions SET rank = ? WHERE id = ? AND status = 'open'")
            .bind(scored.score)
            .bind(&d.id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        updated += result.rows_affected() as usize;
    }

    Ok(updated)
}

/// Load `(goal, depends_on_goal)` edges for a project from existing card
/// metadata — the SQL-over-existing-tables half of curation. Part B joins
/// these against decision rows.
pub async fn load_goal_dependency_edges(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let cards = crate::cards::list_cards(pool, project_id, Some("goal"), None).await?;
    let mut edges = Vec::new();
    for card in cards {
        if let Some(deps) = card
            .metadata_json
            .get("depends_on")
            .and_then(|v| v.as_array())
        {
            for dep in deps {
                if let Some(dep_id) = dep.as_str() {
                    if !dep_id.is_empty() {
                        edges.push((card.id.clone(), dep_id.to_string()));
                    }
                }
            }
        }
    }
    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_750_000_000, 0).unwrap()
    }

    fn decision(
        id: &str,
        age_hours: i64,
        priority: Priority,
        blocked_goals: usize,
        malformed: bool,
    ) -> OpenDecision {
        OpenDecision {
            id: id.to_string(),
            created_at: now() - Duration::hours(age_hours),
            priority,
            blocked_goals,
            malformed,
        }
    }

    #[test]
    fn score_table() {
        // (decision, expected_score) — hand-computed against the formula.
        let cases: Vec<(OpenDecision, f64)> = vec![
            // zero everything except priority: 0.3 * 0.5
            (decision("a", 0, Priority::Normal, 0, false), 0.15),
            // blocked capped at 5: 0.5*1.0 + 0.3*0.5 + 0.2*0
            (decision("b", 0, Priority::Normal, 9, false), 0.65),
            // age capped at 72h: 0.5*0 + 0.3*0.25 + 0.2*1.0
            (decision("c", 100, Priority::Low, 0, false), 0.275),
            // critical, 3 blocked, 36h: 0.5*0.6 + 0.3*1.0 + 0.2*0.5
            (decision("d", 36, Priority::Critical, 3, false), 0.70),
            // everything maxed
            (decision("e", 72, Priority::Critical, 5, false), 1.0),
        ];
        for (d, expected) in cases {
            let r = score_decision(&d, now());
            assert!(
                (r.score - expected).abs() < 1e-9,
                "decision {}: expected {}, got {}",
                d.id,
                expected,
                r.score
            );
        }
    }

    #[test]
    fn malformed_floor_applies_only_upward() {
        // Fresh malformed item with nothing else: floor lifts it to 0.30.
        let low = decision("m1", 0, Priority::Low, 0, true);
        assert!((score_decision(&low, now()).score - MALFORMED_SCORE_FLOOR).abs() < 1e-9);

        // A malformed item that scores above the floor keeps its real score.
        let high = decision("m2", 72, Priority::Critical, 5, true);
        assert!((score_decision(&high, now()).score - 1.0).abs() < 1e-9);

        // Floor never outranks a decision blocking real work.
        let blocking = decision("w", 0, Priority::Normal, 5, false); // 0.65
        assert!(score_decision(&blocking, now()).score > MALFORMED_SCORE_FLOOR);
    }

    #[test]
    fn rank_orders_desc_with_deterministic_tie_breaks() {
        let ds = vec![
            decision("z-newer", 1, Priority::Normal, 0, false), // tie on score with y-older? no: age differs
            decision("b", 0, Priority::Critical, 5, false),     // 0.80
            decision("a", 0, Priority::Normal, 0, false),       // 0.15
        ];
        let ranked = rank_decisions(&ds, now());
        assert_eq!(ranked[0].id, "b");
        assert_eq!(ranked.last().unwrap().id, "a");

        // Exact ties: same created_at, same inputs → id ascending.
        let t1 = decision("id-b", 10, Priority::High, 2, false);
        let mut t2 = t1.clone();
        t2.id = "id-a".to_string();
        let ranked = rank_decisions(&[t1, t2], now());
        assert_eq!(ranked[0].id, "id-a");
        assert_eq!(ranked[1].id, "id-b");

        // Same score, different created_at → older first (created_at beats id).
        // Both are past the 72h age cap so a 1-hour difference doesn't change
        // the score, only the tie-break.
        let early = decision("z-early", 100, Priority::High, 2, false);
        let late = decision("a-late", 99, Priority::High, 2, false);
        let ranked = rank_decisions(&[late, early], now());
        assert_eq!(ranked[0].id, "z-early", "older created_at wins over id");
    }

    #[test]
    fn rank_caps_at_max_surfaced() {
        let ds: Vec<OpenDecision> = (0..25)
            .map(|i| decision(&format!("d{:02}", i), i, Priority::Normal, 0, false))
            .collect();
        let ranked = rank_decisions(&ds, now());
        assert_eq!(ranked.len(), MAX_SURFACED);
        // Oldest (highest age_norm) first.
        assert_eq!(ranked[0].id, "d24");
    }

    #[test]
    fn rank_is_deterministic_across_runs() {
        let ds: Vec<OpenDecision> = (0..12)
            .map(|i| {
                decision(
                    &format!("d{}", i),
                    i % 4,
                    Priority::Normal,
                    (i % 3) as usize,
                    i % 5 == 0,
                )
            })
            .collect();
        let a: Vec<String> = rank_decisions(&ds, now())
            .into_iter()
            .map(|r| r.id)
            .collect();
        let b: Vec<String> = rank_decisions(&ds, now())
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(a, b);
    }

    fn edges(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// (goal, dependency edges, expected transitive dependent count)
    type DependentCase<'a> = (&'a str, &'a Vec<(String, String)>, usize);

    #[test]
    fn transitive_dependents_table() {
        // chain: c depends on b depends on a → blocking a blocks {b, c}
        let chain = edges(&[("b", "a"), ("c", "b")]);
        // diamond: b,c depend on a; d depends on b and c → a blocks {b,c,d}
        let diamond = edges(&[("b", "a"), ("c", "a"), ("d", "b"), ("d", "c")]);
        // cycle (defensive): a↔b must not loop forever
        let cycle = edges(&[("a", "b"), ("b", "a")]);

        let cases: Vec<DependentCase> = vec![
            ("a", &chain, 2),
            ("b", &chain, 1),
            ("c", &chain, 0),
            ("a", &diamond, 3),
            ("b", &diamond, 1),
            ("a", &cycle, 1),
            ("unknown", &chain, 0),
        ];
        for (goal, es, expected) in cases {
            assert_eq!(
                transitive_dependents(goal, es),
                expected,
                "goal {} in {:?}",
                goal,
                es
            );
        }
    }

    #[tokio::test]
    async fn load_goal_dependency_edges_reads_card_metadata() {
        use crate::projects::PERSONAL_PROJECT_ID;
        use crate::session::spectral_schema::init_spectral_db;

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        crate::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .unwrap();

        let mk = |deps: serde_json::Value| crate::cards::CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "g".to_string(),
            description: None,
            card_type: Some("goal".to_string()),
            column_id: Some(col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({ "depends_on": deps })),
        };

        let root = crate::cards::create_card(&pool, mk(serde_json::json!([])))
            .await
            .unwrap();
        let child = crate::cards::create_card(&pool, mk(serde_json::json!([root.id])))
            .await
            .unwrap();

        let edges = load_goal_dependency_edges(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        assert_eq!(edges, vec![(child.id.clone(), root.id.clone())]);
        assert_eq!(transitive_dependents(&root.id, &edges), 1);
    }

    #[test]
    fn priority_from_metadata_table() {
        let cases = [
            (serde_json::json!({"priority": "low"}), Priority::Low),
            (serde_json::json!({"priority": "high"}), Priority::High),
            (
                serde_json::json!({"priority": "critical"}),
                Priority::Critical,
            ),
            (serde_json::json!({"priority": "weird"}), Priority::Normal),
            (serde_json::json!({}), Priority::Normal),
        ];
        for (meta, expected) in cases {
            assert_eq!(priority_from_metadata(&meta), expected, "meta {}", meta);
        }
    }

    #[tokio::test]
    async fn rerank_persists_scores_that_order_the_inbox() {
        use crate::projects::PERSONAL_PROJECT_ID;
        use crate::session::spectral_schema::init_spectral_db;

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        crate::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .unwrap();

        let mk_goal = |priority: &str, deps: serde_json::Value| crate::cards::CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "g".to_string(),
            description: None,
            card_type: Some("goal".to_string()),
            column_id: Some(col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({
                "priority": priority,
                "depends_on": deps
            })),
        };

        // critical goal with a dependent vs low-priority leaf goal.
        let hot = crate::cards::create_card(&pool, mk_goal("critical", serde_json::json!([])))
            .await
            .unwrap();
        let _dependent =
            crate::cards::create_card(&pool, mk_goal("normal", serde_json::json!([hot.id])))
                .await
                .unwrap();
        let cold = crate::cards::create_card(&pool, mk_goal("low", serde_json::json!([])))
            .await
            .unwrap();

        let mk_decision = |goal_id: &str| crate::decisions::NewDecision {
            kind: "unblock".to_string(),
            goal_id: Some(goal_id.to_string()),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("A goal is stuck and needs your direction".to_string()),
            detail: Some("stuck".to_string()),
            payload: serde_json::json!({"reason": "stuck"}),
            ..Default::default()
        };
        // Create the LOW-priority decision first so creation order cannot
        // accidentally produce the expected ranking.
        let d_cold = crate::decisions::create_decision(&pool, mk_decision(&cold.id))
            .await
            .unwrap();
        let d_hot = crate::decisions::create_decision(&pool, mk_decision(&hot.id))
            .await
            .unwrap();
        assert!(d_cold.rank.is_none() && d_hot.rank.is_none());

        let updated = rerank_open_decisions(&pool).await.unwrap();
        assert_eq!(updated, 2);

        let open = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(
            open[0].decision.id, d_hot.id,
            "critical + blocking decision must rank first"
        );
        assert!(open[0].decision.rank.unwrap() > open[1].decision.rank.unwrap());

        // Deterministic: a second pass yields identical ordering.
        rerank_open_decisions(&pool).await.unwrap();
        let again = crate::decisions::list_open_decisions(&pool).await.unwrap();
        assert_eq!(again[0].decision.id, open[0].decision.id);
    }
}
