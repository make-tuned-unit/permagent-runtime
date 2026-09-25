//! Deterministic cross-project synthesis for the Home dashboard's Phase-1
//! insight cards: `today_focus`, `overnight_digest`, `fleet_kpis` and
//! `stalled_pattern` (see `HOME_TEMPLATES_RESEARCH.md` §3, and the
//! `home-templates` lane brief).
//!
//! Every function here is a pure transform over plain facts — no SQL, no
//! HTTP, no `AppState` — so the deterministic rules (which is the whole
//! point: P12 forbids fabrication, and a deterministic rule is the only kind
//! that can be *proven* not to fabricate) are unit-testable without a
//! database. The goose-server route handler (`routes/dashboard.rs`) is the
//! thin glue: fetch real rows via `cards::list_active_goals`,
//! `decisions::list_open_decisions`/`inbox_summary` and
//! `projects::list_projects`, map them into the `*Fact` shapes below, and
//! call through.
//!
//! P12, restated as the rule every function here follows: never fabricate a
//! synthesis. An empty or ambiguous input renders "not enough data yet" (or
//! the narrower honest equivalent), never a confident-looking guess, a forced
//! pick, or a division by zero dressed up as a ratio.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Minimal goal shape the synthesis functions need — narrower than
/// `cards::ActiveGoal` on purpose, so this module has no dependency on the
/// cards/board schema and can be unit-tested with plain literals.
#[derive(Debug, Clone)]
pub struct GoalFact {
    pub id: String,
    pub title: String,
    pub project_id: String,
    /// state_binding: "ready" | "in_progress" (mirrors `cards::ActiveGoal`).
    pub state: String,
    pub hold_note: Option<String>,
    pub routing_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GoalFact {
    /// A goal Henry has parked with an explanation is "stalled" — the same
    /// reading `InFlightCard.tsx` already gives `hold_note` on the frontend
    /// (`status = hold_note ? 'Held' : ...`). One definition, reused here.
    pub fn is_stalled(&self) -> bool {
        self.hold_note.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct DecisionFact {
    pub id: String,
    pub headline: String,
    pub project_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ProjectFact {
    pub id: String,
    pub name: String,
}

// ── today_focus ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FocusKind {
    Decision,
    Goal,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayFocus {
    /// `None` ⇒ nothing qualifies; the card must show "Nothing urgent — pick
    /// anything", never force a pick (§3 honest-data rule).
    pub kind: Option<FocusKind>,
    pub ref_id: Option<String>,
    pub project_id: Option<String>,
    pub title: Option<String>,
    /// One sentence explaining WHY this was picked (R5 / HAX G11 — "explain
    /// why the system did what it did").
    pub reason: Option<String>,
}

impl TodayFocus {
    fn none() -> Self {
        Self {
            kind: None,
            ref_id: None,
            project_id: None,
            title: None,
            reason: None,
        }
    }
}

/// Deterministic ranking: blocking-count (a stalled goal sharing its blocker
/// with others outranks a lone one) > overdue-age > due-today, tie-broken by
/// decision age. No LLM (§3: `today_focus` is fully deterministic).
///
/// Simplified to what Phase 1 has real data for: the OLDEST open decision
/// wins (a decision blocks the user directly, and age is the honest proxy
/// for "most overdue"), falling back to the oldest stalled goal when there
/// are no open decisions at all.
pub fn compute_today_focus(decisions: &[DecisionFact], goals: &[GoalFact]) -> TodayFocus {
    if let Some(oldest) = decisions.iter().min_by_key(|d| d.created_at) {
        let age = Utc::now().signed_duration_since(oldest.created_at);
        let age_txt = human_age(age);
        return TodayFocus {
            kind: Some(FocusKind::Decision),
            ref_id: Some(oldest.id.clone()),
            project_id: oldest.project_id.clone(),
            title: Some(oldest.headline.clone()),
            reason: Some(if decisions.len() > 1 {
                format!(
                    "The oldest of {} decisions waiting on you, from {age_txt}",
                    decisions.len()
                )
            } else {
                format!("The one decision waiting on you, from {age_txt}")
            }),
        };
    }
    let stalled: Vec<&GoalFact> = goals.iter().filter(|g| g.is_stalled()).collect();
    if let Some(oldest) = stalled.iter().min_by_key(|g| g.updated_at) {
        let age = Utc::now().signed_duration_since(oldest.updated_at);
        return TodayFocus {
            kind: Some(FocusKind::Goal),
            ref_id: Some(oldest.id.clone()),
            project_id: Some(oldest.project_id.clone()),
            title: Some(oldest.title.clone()),
            reason: Some(format!(
                "Stalled the longest of {} held goal(s), since {}",
                stalled.len(),
                human_age(age)
            )),
        };
    }
    TodayFocus::none()
}

fn human_age(d: chrono::Duration) -> String {
    let mins = d.num_minutes();
    if mins < 60 {
        return "just now".to_string();
    }
    let hours = d.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", d.num_days())
}

// ── overnight_digest ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OvernightDigest {
    pub new_decisions: usize,
    pub stalled_since: usize,
    /// `true` when nothing happened in the window — the card shows "Quiet
    /// night" rather than four blank rows (§3 honest-data rule).
    pub quiet: bool,
}

/// Bucket events since `since` into new-decision / newly-stalled counts.
/// Sentence-template, not LLM (§3).
pub fn compute_overnight_digest(
    decisions: &[DecisionFact],
    goals: &[GoalFact],
    since: DateTime<Utc>,
) -> OvernightDigest {
    let new_decisions = decisions.iter().filter(|d| d.created_at >= since).count();
    let stalled_since = goals
        .iter()
        .filter(|g| g.is_stalled() && g.updated_at >= since)
        .count();
    OvernightDigest {
        new_decisions,
        stalled_since,
        quiet: new_decisions == 0 && stalled_since == 0,
    }
}

// ── fleet_kpis ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetKpis {
    pub project_count: usize,
    pub active_goal_count: usize,
    pub stalled_goal_count: usize,
    pub pending_decision_count: usize,
    pub sessions_today: usize,
}

/// Cross-project counts. No division, so there is no zero-project edge case
/// to fabricate around — every field here is a plain count that is honestly
/// `0` when there is nothing (§3: "any metric with zero underlying projects
/// shows '—' not '0%'" — the '—' vs '0' framing is a *rendering* choice the
/// card component makes from `project_count == 0`, kept out of this struct).
pub fn compute_fleet_kpis(
    projects: &[ProjectFact],
    goals: &[GoalFact],
    decisions: &[DecisionFact],
    sessions_today: usize,
) -> FleetKpis {
    FleetKpis {
        project_count: projects.len(),
        active_goal_count: goals.len(),
        stalled_goal_count: goals.iter().filter(|g| g.is_stalled()).count(),
        pending_decision_count: decisions.len(),
        sessions_today,
    }
}

// ── stalled_pattern ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StalledCluster {
    /// The shared keyword the cluster was found by (lowercased).
    pub keyword: String,
    /// One entry per goal in the cluster, each carrying its OWN project id —
    /// not two parallel lists. A UI drilling into "goal N" needs its actual
    /// project, and a goal_ids[i]/project_ids[i] pairing silently breaks the
    /// moment the two lists are deduped independently (project_ids is
    /// deduped; goal_ids is not), which is exactly the bug this shape avoids.
    pub goals: Vec<StalledGoalRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StalledGoalRef {
    pub goal_id: String,
    pub project_id: String,
}

impl StalledCluster {
    pub fn project_count(&self) -> usize {
        let mut ids: Vec<&str> = self.goals.iter().map(|g| g.project_id.as_str()).collect();
        ids.sort();
        ids.dedup();
        ids.len()
    }
}

const STOPWORDS: &[&str] = &[
    "the", "and", "with", "from", "that", "this", "have", "been", "will", "into", "still", "over",
    "your", "while", "until", "than", "then", "they", "them", "were", "does", "isn't", "wasn't",
];

/// Significant words (len >= 5, lowercased, alnum-only, stripped of
/// stopwords) in a blocker note — the vocabulary shared-blocker clustering
/// matches on.
fn significant_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 5 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Deterministic clustering: group stalled goals by a shared significant
/// word in their hold/routing note. Needs >= 2 goals to report a "pattern"
/// (§3: "0-1 stalled goals sharing a cause -> card hides itself, doesn't
/// force a pattern") — a lone stalled goal is just `today_focus` material,
/// not a cross-project pattern.
///
/// Greedy union by shared word rather than full clustering: deterministic,
/// stable across runs (same input -> same output, ordered), and easy to
/// reason about without a similarity-graph library for what is, in practice,
/// a handful of goals.
pub fn find_stalled_patterns(goals: &[GoalFact]) -> Vec<StalledCluster> {
    let stalled: Vec<&GoalFact> = goals.iter().filter(|g| g.is_stalled()).collect();
    if stalled.len() < 2 {
        return Vec::new();
    }

    // word -> goal indices (into `stalled`) that mention it.
    let mut by_word: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, g) in stalled.iter().enumerate() {
        let note = g
            .hold_note
            .as_deref()
            .or(g.routing_note.as_deref())
            .unwrap_or("");
        for w in significant_words(note) {
            by_word.entry(w).or_default().push(i);
        }
    }

    let mut clusters: Vec<StalledCluster> = by_word
        .into_iter()
        .filter(|(_, idxs)| {
            // >= 2 DISTINCT goals sharing the word (a repeated word within one
            // goal's own note must not count as a cross-goal pattern).
            let mut uniq = idxs.clone();
            uniq.dedup();
            uniq.len() >= 2
        })
        .map(|(word, idxs)| {
            let mut idxs = idxs;
            idxs.dedup();
            let goals = idxs
                .iter()
                .map(|&i| StalledGoalRef {
                    goal_id: stalled[i].id.clone(),
                    project_id: stalled[i].project_id.clone(),
                })
                .collect();
            StalledCluster {
                keyword: word,
                goals,
            }
        })
        .collect();

    // Largest, most cross-project cluster first; keyword as a stable
    // tie-break so output order never depends on BTreeMap iteration alone.
    clusters.sort_by(|a, b| {
        b.project_count()
            .cmp(&a.project_count())
            .then(b.goals.len().cmp(&a.goals.len()))
            .then(a.keyword.cmp(&b.keyword))
    });
    clusters
}

// ── Layout packing (for the agent's preview/compose tools) ─────────────────

/// One card's resolved cell on the grid — the Rust mirror of
/// `useLayout.ts`'s `PackedCell`, for the `preview_dashboard_layout` /
/// `compose_dashboard_layout` agent tools (`platform_extensions/dashboard.rs`)
/// to pack a template or an ad-hoc card list the same way the frontend would,
/// so the preview the agent describes matches what the UI then renders.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackedCard {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// First-fit ("dense") packing, in input order — the same rule as CSS
/// `grid-auto-flow: dense`, and the same algorithm as `useLayout.ts`'s
/// `packDense`: each card goes to the topmost, then leftmost, cell range it
/// fits in. Kept as a straight line-for-line port rather than a "smarter"
/// reimplementation, specifically so the two never quietly diverge.
pub fn pack_dense(cards: &[(String, i32, i32)], cols: i32) -> Vec<PackedCard> {
    let cols = cols.max(1);
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let is_free = |occupied: &[Vec<bool>], x: i32, y: i32, w: i32, h: i32| -> bool {
        for r in y..y + h {
            if let Some(row) = occupied.get(r as usize) {
                for c in x..x + w {
                    if *row.get(c as usize).unwrap_or(&false) {
                        return false;
                    }
                }
            }
        }
        true
    };
    let mark = |occupied: &mut Vec<Vec<bool>>, x: i32, y: i32, w: i32, h: i32| {
        for r in y..y + h {
            let ru = r as usize;
            if occupied.len() <= ru {
                occupied.resize(ru + 1, vec![false; cols as usize]);
            }
            for c in x..x + w {
                occupied[ru][c as usize] = true;
            }
        }
    };

    let mut out = Vec::with_capacity(cards.len());
    for (id, raw_w, raw_h) in cards {
        let w = (*raw_w).clamp(1, cols);
        let h = (*raw_h).max(1);
        let mut y = 0;
        'search: loop {
            let mut x = 0;
            while x + w <= cols {
                if is_free(&occupied, x, y, w, h) {
                    mark(&mut occupied, x, y, w, h);
                    out.push(PackedCard {
                        id: id.clone(),
                        x,
                        y,
                        w,
                        h,
                    });
                    break 'search;
                }
                x += 1;
            }
            y += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(
        id: &str,
        project_id: &str,
        hold_note: Option<&str>,
        updated_secs_ago: i64,
    ) -> GoalFact {
        GoalFact {
            id: id.to_string(),
            title: format!("Goal {id}"),
            project_id: project_id.to_string(),
            state: "in_progress".to_string(),
            hold_note: hold_note.map(str::to_string),
            routing_note: None,
            created_at: Utc::now() - chrono::Duration::days(1),
            updated_at: Utc::now() - chrono::Duration::seconds(updated_secs_ago),
        }
    }

    fn decision(id: &str, headline: &str, hours_ago: i64) -> DecisionFact {
        DecisionFact {
            id: id.to_string(),
            headline: headline.to_string(),
            project_id: None,
            created_at: Utc::now() - chrono::Duration::hours(hours_ago),
        }
    }

    // ── today_focus ──────────────────────────────────────────────────────

    #[test]
    fn today_focus_is_none_with_nothing_to_pick() {
        let out = compute_today_focus(&[], &[]);
        assert_eq!(out.kind, None);
        assert!(
            out.reason.is_none(),
            "must not force a pick — §3 honest-data rule"
        );
    }

    #[test]
    fn today_focus_picks_the_oldest_open_decision() {
        let decisions = vec![
            decision("d1", "Recent one", 1),
            decision("d2", "Older one", 48),
        ];
        let out = compute_today_focus(&decisions, &[]);
        assert_eq!(out.kind, Some(FocusKind::Decision));
        assert_eq!(out.ref_id.as_deref(), Some("d2"));
        assert!(out.reason.unwrap().contains("2 decisions"));
    }

    #[test]
    fn today_focus_falls_back_to_the_oldest_stalled_goal_with_no_decisions() {
        let goals = vec![
            goal("g1", "p1", Some("blocked recently"), 60),
            goal("g2", "p1", Some("blocked longer"), 3600),
        ];
        let out = compute_today_focus(&[], &goals);
        assert_eq!(out.kind, Some(FocusKind::Goal));
        assert_eq!(out.ref_id.as_deref(), Some("g2"));
    }

    #[test]
    fn today_focus_never_picks_a_non_stalled_goal() {
        let goals = vec![goal("g1", "p1", None, 60)];
        let out = compute_today_focus(&[], &goals);
        assert_eq!(out.kind, None);
    }

    // ── overnight_digest ─────────────────────────────────────────────────

    #[test]
    fn overnight_digest_is_quiet_when_nothing_happened() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let out = compute_overnight_digest(&[], &[], since);
        assert!(out.quiet);
        assert_eq!(out.new_decisions, 0);
    }

    #[test]
    fn overnight_digest_counts_only_events_inside_the_window() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let decisions = vec![
            decision("d1", "inside window", 2),
            decision("d2", "before window", 20),
        ];
        let out = compute_overnight_digest(&decisions, &[], since);
        assert_eq!(out.new_decisions, 1);
        assert!(!out.quiet);
    }

    // ── fleet_kpis ───────────────────────────────────────────────────────

    #[test]
    fn fleet_kpis_are_plain_counts_with_no_projects() {
        let out = compute_fleet_kpis(&[], &[], &[], 0);
        assert_eq!(out.project_count, 0);
        assert_eq!(out.active_goal_count, 0);
    }

    #[test]
    fn fleet_kpis_counts_stalled_separately_from_active() {
        let goals = vec![
            goal("g1", "p1", None, 60),
            goal("g2", "p1", Some("blocked"), 60),
        ];
        let out = compute_fleet_kpis(
            &[ProjectFact {
                id: "p1".into(),
                name: "P1".into(),
            }],
            &goals,
            &[],
            3,
        );
        assert_eq!(out.active_goal_count, 2);
        assert_eq!(out.stalled_goal_count, 1);
        assert_eq!(out.sessions_today, 3);
    }

    // ── stalled_pattern ──────────────────────────────────────────────────

    #[test]
    fn no_pattern_with_fewer_than_two_stalled_goals() {
        let goals = vec![goal("g1", "p1", Some("blocked on the daemon restart"), 60)];
        assert!(find_stalled_patterns(&goals).is_empty());
    }

    #[test]
    fn no_pattern_when_stalled_goals_share_no_keyword() {
        let goals = vec![
            goal("g1", "p1", Some("blocked on the daemon restart"), 60),
            goal("g2", "p2", Some("waiting on API keys"), 60),
        ];
        assert!(find_stalled_patterns(&goals).is_empty());
    }

    #[test]
    fn finds_a_shared_blocker_across_projects() {
        let goals = vec![
            goal(
                "g1",
                "p1",
                Some("blocked on daemon restarts during the build gate"),
                60,
            ),
            goal(
                "g2",
                "p2",
                Some("stalled: daemon restarts keep killing the session"),
                120,
            ),
            goal("g3", "p3", Some("unrelated network timeout"), 30),
        ];
        let clusters = find_stalled_patterns(&goals);
        assert!(!clusters.is_empty());
        let top = &clusters[0];
        let ids: Vec<&str> = top.goals.iter().map(|g| g.goal_id.as_str()).collect();
        assert!(ids.contains(&"g1"));
        assert!(ids.contains(&"g2"));
        assert!(!ids.contains(&"g3"));
        assert_eq!(top.project_count(), 2);
        // Each goal carries its OWN project, not a positionally-paired list.
        let g1 = top.goals.iter().find(|g| g.goal_id == "g1").unwrap();
        assert_eq!(g1.project_id, "p1");
        let g2 = top.goals.iter().find(|g| g.goal_id == "g2").unwrap();
        assert_eq!(g2.project_id, "p2");
    }

    #[test]
    fn a_repeated_word_within_one_goals_own_note_is_not_a_pattern() {
        let goals = vec![
            goal("g1", "p1", Some("restart restart restart"), 60),
            goal("g2", "p2", Some("unrelated timeout issue"), 60),
        ];
        assert!(find_stalled_patterns(&goals).is_empty());
    }

    // ── pack_dense ───────────────────────────────────────────────────────

    fn c(id: &str, w: i32, h: i32) -> (String, i32, i32) {
        (id.to_string(), w, h)
    }

    #[test]
    fn packs_cards_left_to_right_top_to_bottom() {
        let out = pack_dense(&[c("a", 6, 2), c("b", 6, 2)], 12);
        assert_eq!(
            out[0],
            PackedCard {
                id: "a".into(),
                x: 0,
                y: 0,
                w: 6,
                h: 2
            }
        );
        assert_eq!(
            out[1],
            PackedCard {
                id: "b".into(),
                x: 6,
                y: 0,
                w: 6,
                h: 2
            }
        );
    }

    #[test]
    fn back_fills_a_hole_with_a_later_card_that_fits() {
        // Mirrors useLayout.test.ts's "back-fills a hole with a later card
        // that fits (grid-auto-flow: dense)" — the two packers must agree.
        let out = pack_dense(&[c("a", 7, 4), c("b", 12, 2), c("c", 5, 2)], 12);
        let by_id = |id: &str| out.iter().find(|p| p.id == id).unwrap();
        assert_eq!(
            by_id("c"),
            &PackedCard {
                id: "c".into(),
                x: 7,
                y: 0,
                w: 5,
                h: 2
            }
        );
        assert_eq!(
            by_id("b"),
            &PackedCard {
                id: "b".into(),
                x: 0,
                y: 4,
                w: 12,
                h: 2
            }
        );
    }

    #[test]
    fn clamps_an_oversized_width_to_the_grid() {
        let out = pack_dense(&[c("a", 20, 1)], 12);
        assert_eq!(out[0].w, 12);
    }

    #[test]
    fn a_zero_or_negative_height_becomes_one_row() {
        let out = pack_dense(&[c("a", 4, 0)], 12);
        assert_eq!(out[0].h, 1);
    }
}
