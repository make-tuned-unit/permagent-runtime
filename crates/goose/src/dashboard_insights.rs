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

/// One ranked candidate — the top pick, or a runner-up. Carries its own
/// reason so "why this one" travels with it rather than living only on the
/// winner (coordinator, Phase-1 presentation pass: "Today's Focus must add
/// value: the ONE thing + WHY it ranks first ... + the next 2 runners-up").
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TodayFocusItem {
    pub kind: FocusKind,
    pub ref_id: String,
    pub project_id: Option<String>,
    pub title: String,
    /// The ranking signals in plain words (R5 / HAX G11): what made this
    /// rank where it did, e.g. "the oldest of 3 decisions waiting, from 2d
    /// ago" or "stalled the longest of 2 held goals · shares its blocker
    /// with 1 other goal".
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayFocus {
    /// `None` ⇒ nothing qualifies; the card must show "Nothing urgent — pick
    /// anything", never force a pick (§3 honest-data rule).
    pub top: Option<TodayFocusItem>,
    /// Up to 2 next-ranked items — NOT a second copy of the Needs-You queue:
    /// this is the ranking's runner-up, with its own reason, so the card adds
    /// information the decision count elsewhere on Home does not.
    pub runners_up: Vec<TodayFocusItem>,
}

/// Deterministic ranking: open decisions first (each one blocks the user
/// directly), oldest first; then stalled goals, oldest-stalled first, each
/// annotated when it shares its blocker with other stalled goals (the
/// `find_stalled_patterns` clustering below — "blocks N goals" in plain
/// words). No LLM (§3: `today_focus` is fully deterministic). Never forces a
/// pick: an empty candidate list returns `top: None` (§3 honest-data rule).
pub fn compute_today_focus(decisions: &[DecisionFact], goals: &[GoalFact]) -> TodayFocus {
    // Cluster membership first, so a stalled goal's reason can say how many
    // OTHER goals share its blocker — the concrete signal HOME_TEMPLATES_
    // RESEARCH.md §4 asks for ("blocks N goals"), not just its own age.
    let clusters = find_stalled_patterns(goals);
    let mut shared_with: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for cluster in &clusters {
        for g in &cluster.goals {
            shared_with.insert(g.goal_id.as_str(), cluster.goals.len() - 1);
        }
    }

    let mut candidates: Vec<TodayFocusItem> = Vec::new();

    let mut sorted_decisions: Vec<&DecisionFact> = decisions.iter().collect();
    sorted_decisions.sort_by_key(|d| d.created_at);
    for (i, d) in sorted_decisions.iter().enumerate() {
        let age = human_age(Utc::now().signed_duration_since(d.created_at));
        let rank = if i == 0 {
            "the oldest".to_string()
        } else {
            format!("#{} oldest", i + 1)
        };
        candidates.push(TodayFocusItem {
            kind: FocusKind::Decision,
            ref_id: d.id.clone(),
            project_id: d.project_id.clone(),
            title: d.headline.clone(),
            reason: format!(
                "{rank} of {}, from {age}",
                plural(decisions.len(), "decision waiting", "decisions waiting"),
            ),
        });
    }

    let stalled: Vec<&GoalFact> = goals.iter().filter(|g| g.is_stalled()).collect();
    let mut sorted_stalled = stalled.clone();
    sorted_stalled.sort_by_key(|g| g.updated_at);
    for (i, g) in sorted_stalled.iter().enumerate() {
        let age = human_age(Utc::now().signed_duration_since(g.updated_at));
        let mut reason = if i == 0 {
            format!(
                "stalled the longest of {}, since {age}",
                plural(stalled.len(), "held goal", "held goals"),
            )
        } else {
            format!("stalled since {age}")
        };
        if let Some(&others) = shared_with.get(g.id.as_str()) {
            if others > 0 {
                reason.push_str(&format!(
                    " · shares its blocker with {}",
                    plural(others, "other goal", "other goals"),
                ));
            }
        }
        candidates.push(TodayFocusItem {
            kind: FocusKind::Goal,
            ref_id: g.id.clone(),
            project_id: Some(g.project_id.clone()),
            title: g.title.clone(),
            reason,
        });
    }

    if candidates.is_empty() {
        return TodayFocus {
            top: None,
            runners_up: Vec::new(),
        };
    }
    let top = candidates.remove(0);
    let runners_up = candidates.into_iter().take(2).collect();
    TodayFocus {
        top: Some(top),
        runners_up,
    }
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

/// "N word", pluralized properly — shared by every sentence-builder below.
/// Never "1 goal(s)".
pub fn plural(n: usize, singular: &str, plural_form: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural_form })
}

// ── overnight_digest ─────────────────────────────────────────────────────────

/// A finished session, for the "finished" bucket below — the same rows
/// `dashboard.rs`'s `RecentSession` already computes (non-active sessions),
/// narrowed to what the digest needs.
#[derive(Debug, Clone)]
pub struct SessionFact {
    pub id: String,
    pub title: String,
    pub ended_at: DateTime<Utc>,
}

/// One named line in a digest bucket — a real title, not just a count, so
/// "3 goals stalled overnight" is followed by which 3 (coordinator, Phase-1
/// presentation pass: "each a line with counts and 1-2 named examples,
/// clickable").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestItem {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OvernightDigest {
    /// Sessions that wrapped up in the window.
    pub finished: Vec<DigestItem>,
    /// Goals newly stalled in the window (were fine, now have a hold note).
    pub stalled_new: Vec<DigestItem>,
    pub new_decisions: Vec<DigestItem>,
    /// Real, not fabricated (§3 / P12): 0 whenever the daemon has no
    /// timestamped memory-delta tracking for the window, which is today's
    /// honest state (`dashboard.rs`'s `get_dashboard` already documents this
    /// gap) — the card must show "0", never invent a plausible-looking count.
    pub new_memories: i64,
    /// `true` when nothing happened in the window — the card shows "Quiet
    /// night" rather than four blank rows (§3 honest-data rule).
    pub quiet: bool,
}

/// Bucket events since `since` into named finished/stalled/decision lines.
/// Sentence-template, not LLM (§3).
pub fn compute_overnight_digest(
    decisions: &[DecisionFact],
    goals: &[GoalFact],
    sessions: &[SessionFact],
    new_memories: i64,
    since: DateTime<Utc>,
) -> OvernightDigest {
    let finished: Vec<DigestItem> = sessions
        .iter()
        .filter(|s| s.ended_at >= since)
        .map(|s| DigestItem {
            id: s.id.clone(),
            title: s.title.clone(),
            project_id: None,
        })
        .collect();
    let stalled_new: Vec<DigestItem> = goals
        .iter()
        .filter(|g| g.is_stalled() && g.updated_at >= since)
        .map(|g| DigestItem {
            id: g.id.clone(),
            title: g.title.clone(),
            project_id: Some(g.project_id.clone()),
        })
        .collect();
    let new_decisions: Vec<DigestItem> = decisions
        .iter()
        .filter(|d| d.created_at >= since)
        .map(|d| DigestItem {
            id: d.id.clone(),
            title: d.headline.clone(),
            project_id: d.project_id.clone(),
        })
        .collect();
    let quiet = finished.is_empty()
        && stalled_new.is_empty()
        && new_decisions.is_empty()
        && new_memories == 0;
    OvernightDigest {
        finished,
        stalled_new,
        new_decisions,
        new_memories,
        quiet,
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
    /// The shared PHRASE the cluster was found by, e.g. "daemon restarts" —
    /// a normalized two-word bigram, never a bare single word (coordinator,
    /// Phase-1 presentation pass: "a keyword is not an insight").
    pub phrase: String,
    /// The whole insight as a sentence, ready to render verbatim: "3 goals in
    /// 3 projects are stuck on the same thing: daemon restarts."
    pub sentence: String,
    /// "Fixing this unblocks N goals." — omitted only if somehow the cluster
    /// ever had fewer than 2 goals (it can't; kept `Option` so a future
    /// caller can't assume otherwise).
    pub next_step: Option<String>,
    /// One entry per goal in the cluster, each carrying its OWN project id
    /// AND title — not two parallel lists, and not an id the UI has to
    /// re-fetch to show a name for.
    pub goals: Vec<StalledGoalRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StalledGoalRef {
    pub goal_id: String,
    pub project_id: String,
    pub title: String,
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
    "the",
    "and",
    "with",
    "from",
    "that",
    "this",
    "have",
    "been",
    "will",
    "into",
    "over",
    "your",
    "while",
    "until",
    "than",
    "then",
    "they",
    "them",
    "were",
    "does",
    "isn't",
    "wasn't",
    "keeps",
    "during",
    "after",
    "before",
    "again",
    "session",
    "unrelated",
];

/// Words that name the STATE of being blocked rather than its CAUSE.
/// Clustering on these alone ("2 goals share the word 'blocked'") is exactly
/// the "keyword, not an insight" failure the coordinator flagged — every
/// stalled goal's note says some version of this, so it carries no signal.
const GENERIC_TERMS: &[&str] = &[
    "blocked", "blocking", "stalled", "stall", "stuck", "waiting", "still", "issue", "issues",
    "problem", "problems", "error", "errors", "failing", "failed", "broken",
];

/// Significant words (len >= 4, lowercased, alnum-only, stripped of
/// stopwords and generic status-words) in a blocker note — the vocabulary
/// shared-blocker clustering builds bigrams from.
fn significant_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| {
            w.len() >= 4 && !STOPWORDS.contains(&w.as_str()) && !GENERIC_TERMS.contains(&w.as_str())
        })
        .collect()
}

/// Consecutive pairs from an already-filtered significant-word sequence —
/// "daemon restarts during the build gate" (after stopword/generic removal:
/// `[daemon, restarts, build, gate]`) yields `["daemon restarts", "restarts
/// build", "build gate"]`. Order-sensitive by design: two notes phrasing the
/// same cause in a different word order will not match, which is the
/// deliberate trade-off for "phrase overlap, not a similarity score" — a
/// bigram match is a much stronger, more literal signal than one shared word.
fn bigrams(words: &[String]) -> Vec<String> {
    words
        .windows(2)
        .map(|w| format!("{} {}", w[0], w[1]))
        .collect()
}

/// Deterministic clustering: group stalled goals that share a two-word
/// PHRASE in their hold/routing note (not a single word — see
/// `GENERIC_TERMS`'s doc). Needs >= 2 goals to report a "pattern" (§3: "0-1
/// stalled goals sharing a cause -> card hides itself, doesn't force a
/// pattern") — a lone stalled goal is just `today_focus` material, not a
/// cross-project pattern.
///
/// Greedy union by shared bigram rather than full clustering: deterministic,
/// stable across runs (same input -> same output, ordered), and easy to
/// reason about without a similarity-graph library for what is, in practice,
/// a handful of goals.
pub fn find_stalled_patterns(goals: &[GoalFact]) -> Vec<StalledCluster> {
    let stalled: Vec<&GoalFact> = goals.iter().filter(|g| g.is_stalled()).collect();
    if stalled.len() < 2 {
        return Vec::new();
    }

    // phrase -> goal indices (into `stalled`) whose note contains it.
    let mut by_phrase: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, g) in stalled.iter().enumerate() {
        let note = g
            .hold_note
            .as_deref()
            .or(g.routing_note.as_deref())
            .unwrap_or("");
        for bg in bigrams(&significant_words(note)) {
            by_phrase.entry(bg).or_default().push(i);
        }
    }

    let mut clusters: Vec<StalledCluster> = by_phrase
        .into_iter()
        .filter_map(|(phrase, idxs)| {
            // >= 2 DISTINCT goals sharing the phrase (a phrase repeated
            // within one goal's own note must not count as a cross-goal
            // pattern).
            let mut idxs = idxs;
            idxs.dedup();
            if idxs.len() < 2 {
                return None;
            }
            let goal_refs: Vec<StalledGoalRef> = idxs
                .iter()
                .map(|&i| StalledGoalRef {
                    goal_id: stalled[i].id.clone(),
                    project_id: stalled[i].project_id.clone(),
                    title: stalled[i].title.clone(),
                })
                .collect();
            let mut project_ids: Vec<&str> =
                goal_refs.iter().map(|g| g.project_id.as_str()).collect();
            project_ids.sort();
            project_ids.dedup();
            let sentence = format!(
                "{} in {} are stuck on the same thing: {phrase}.",
                plural(goal_refs.len(), "goal", "goals"),
                plural(project_ids.len(), "project", "projects"),
            );
            let next_step = Some(format!(
                "Fixing this unblocks {}.",
                plural(goal_refs.len(), "goal", "goals"),
            ));
            Some(StalledCluster {
                phrase,
                sentence,
                next_step,
                goals: goal_refs,
            })
        })
        .collect();

    // Largest, most cross-project cluster first; phrase as a stable
    // tie-break so output order never depends on BTreeMap iteration alone.
    clusters.sort_by(|a, b| {
        b.project_count()
            .cmp(&a.project_count())
            .then(b.goals.len().cmp(&a.goals.len()))
            .then(a.phrase.cmp(&b.phrase))
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

    fn session(id: &str, title: &str, hours_ago: i64) -> SessionFact {
        SessionFact {
            id: id.to_string(),
            title: title.to_string(),
            ended_at: Utc::now() - chrono::Duration::hours(hours_ago),
        }
    }

    // ── today_focus ──────────────────────────────────────────────────────

    #[test]
    fn today_focus_is_none_with_nothing_to_pick() {
        let out = compute_today_focus(&[], &[]);
        assert!(
            out.top.is_none(),
            "must not force a pick — §3 honest-data rule"
        );
        assert!(out.runners_up.is_empty());
    }

    #[test]
    fn today_focus_picks_the_oldest_open_decision() {
        let decisions = vec![
            decision("d1", "Recent one", 1),
            decision("d2", "Older one", 48),
        ];
        let out = compute_today_focus(&decisions, &[]);
        let top = out.top.expect("a decision qualifies");
        assert_eq!(top.kind, FocusKind::Decision);
        assert_eq!(top.ref_id, "d2");
        assert!(top.reason.contains("2 decisions waiting"), "{}", top.reason);
        assert_eq!(out.runners_up.len(), 1);
        assert_eq!(out.runners_up[0].ref_id, "d1");
    }

    #[test]
    fn today_focus_falls_back_to_the_oldest_stalled_goal_with_no_decisions() {
        let goals = vec![
            goal("g1", "p1", Some("blocked recently"), 60),
            goal("g2", "p1", Some("blocked longer"), 3600),
        ];
        let out = compute_today_focus(&[], &goals);
        let top = out.top.expect("a stalled goal qualifies");
        assert_eq!(top.kind, FocusKind::Goal);
        assert_eq!(top.ref_id, "g2");
    }

    #[test]
    fn today_focus_never_picks_a_non_stalled_goal() {
        let goals = vec![goal("g1", "p1", None, 60)];
        let out = compute_today_focus(&[], &goals);
        assert!(out.top.is_none());
    }

    #[test]
    fn today_focus_names_the_shared_blocker_in_its_reason() {
        // Same shape as the realistic fixture: 2 goals share "daemon
        // restarts" — the top pick's reason should say so in plain words.
        let goals = vec![
            goal(
                "g1",
                "p1",
                Some("blocked on daemon restarts during the build gate"),
                3600,
            ),
            goal(
                "g2",
                "p2",
                Some("stalled: daemon restarts keep killing the session"),
                60,
            ),
        ];
        let out = compute_today_focus(&[], &goals);
        let top = out.top.expect("a stalled goal qualifies");
        assert_eq!(
            top.ref_id, "g1",
            "the longer-stalled of the pair ranks first"
        );
        assert!(
            top.reason.contains("shares its blocker with 1 other goal"),
            "{}",
            top.reason
        );
    }

    // ── overnight_digest ─────────────────────────────────────────────────

    #[test]
    fn overnight_digest_is_quiet_when_nothing_happened() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let out = compute_overnight_digest(&[], &[], &[], 0, since);
        assert!(out.quiet);
        assert!(out.new_decisions.is_empty());
        assert!(out.finished.is_empty());
    }

    #[test]
    fn overnight_digest_names_events_inside_the_window_only() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let decisions = vec![
            decision("d1", "inside window", 2),
            decision("d2", "before window", 20),
        ];
        let out = compute_overnight_digest(&decisions, &[], &[], 0, since);
        assert_eq!(out.new_decisions.len(), 1);
        assert_eq!(out.new_decisions[0].title, "inside window");
        assert!(!out.quiet);
    }

    #[test]
    fn overnight_digest_names_finished_sessions_and_newly_stalled_goals() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let sessions = vec![session("s1", "Ship the release notes", 2)];
        let goals = vec![goal("g1", "p1", Some("blocked on daemon restarts"), 60)];
        let out = compute_overnight_digest(&[], &goals, &sessions, 0, since);
        assert_eq!(out.finished.len(), 1);
        assert_eq!(out.finished[0].title, "Ship the release notes");
        assert_eq!(out.stalled_new.len(), 1);
        assert_eq!(out.stalled_new[0].title, "Goal g1");
        assert!(!out.quiet);
    }

    #[test]
    fn overnight_digest_never_fabricates_a_memory_count() {
        let since = Utc::now() - chrono::Duration::hours(8);
        let out = compute_overnight_digest(&[], &[], &[], 0, since);
        assert_eq!(
            out.new_memories, 0,
            "0 is honest when there is no delta source"
        );
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
    fn no_pattern_when_stalled_goals_share_no_phrase() {
        let goals = vec![
            goal("g1", "p1", Some("blocked on the daemon restart"), 60),
            goal("g2", "p2", Some("waiting on API keys"), 60),
        ];
        assert!(find_stalled_patterns(&goals).is_empty());
    }

    #[test]
    fn finds_a_shared_blocker_across_projects_realistic_fixture() {
        // Mirrors the harness's realistic fixture: 3 projects' goals share
        // "daemon restarts" verbatim or near-verbatim; a 4th is unrelated.
        let goals = vec![
            goal(
                "g1",
                "p1",
                Some("stalled: blocked on daemon restarts during the build gate"),
                60,
            ),
            goal(
                "g2",
                "p2",
                Some("stalled — daemon restarts keep killing the session"),
                120,
            ),
            goal(
                "g3",
                "p3",
                Some("still blocked: daemon restarts during rebuild"),
                90,
            ),
            goal("g4", "p4", Some("unrelated network timeout"), 30),
        ];
        let clusters = find_stalled_patterns(&goals);
        assert!(!clusters.is_empty());
        let top = &clusters[0];
        assert_eq!(top.phrase, "daemon restarts");
        let ids: Vec<&str> = top.goals.iter().map(|g| g.goal_id.as_str()).collect();
        assert!(ids.contains(&"g1"));
        assert!(ids.contains(&"g2"));
        assert!(ids.contains(&"g3"));
        assert!(!ids.contains(&"g4"));
        assert_eq!(top.project_count(), 3);
        assert_eq!(
            top.sentence,
            "3 goals in 3 projects are stuck on the same thing: daemon restarts."
        );
        assert_eq!(
            top.next_step.as_deref(),
            Some("Fixing this unblocks 3 goals.")
        );
        // Each goal carries its OWN project AND title — no lookup needed.
        let g1 = top.goals.iter().find(|g| g.goal_id == "g1").unwrap();
        assert_eq!(g1.project_id, "p1");
        assert_eq!(g1.title, "Goal g1");
        let g2 = top.goals.iter().find(|g| g.goal_id == "g2").unwrap();
        assert_eq!(g2.project_id, "p2");
    }

    #[test]
    fn a_repeated_phrase_within_one_goals_own_note_is_not_a_pattern() {
        let goals = vec![
            goal(
                "g1",
                "p1",
                Some("daemon restart daemon restart daemon restart"),
                60,
            ),
            goal("g2", "p2", Some("unrelated timeout issue"), 60),
        ];
        assert!(find_stalled_patterns(&goals).is_empty());
    }

    // ── adversarial clustering cases (coordinator, Phase-1 presentation pass) ─

    #[test]
    fn a_shared_generic_status_word_alone_is_not_a_pattern() {
        // Every stalled goal's note says some version of "blocked" / "stuck" /
        // "issue" — clustering on that alone is a keyword, not an insight.
        let goals = vec![
            goal("g1", "p1", Some("blocked on the API rate limit"), 60),
            goal("g2", "p2", Some("blocked on a missing config file"), 60),
        ];
        assert!(
            find_stalled_patterns(&goals).is_empty(),
            "must not cluster on a generic status word alone"
        );
    }

    #[test]
    fn a_single_shared_specific_word_without_a_bigram_is_not_a_pattern() {
        // "daemon" alone is specific, but the brief explicitly asks for
        // n-gram (phrase) overlap, not single words — a lone shared word,
        // with no shared two-word phrase, must not cluster.
        let goals = vec![
            goal("g1", "p1", Some("blocked: daemon crashed on startup"), 60),
            goal("g2", "p2", Some("waiting on the daemon health check"), 60),
        ];
        assert!(
            find_stalled_patterns(&goals).is_empty(),
            "one shared word with no shared bigram must not cluster"
        );
    }

    #[test]
    fn a_bigram_that_only_partially_overlaps_does_not_join_the_cluster() {
        // g3 shares ONE word ("restarts") with the g1/g2 cluster's phrase but
        // not the full bigram ("network restarts" != "daemon restarts") —
        // it must be left out, not folded in on partial credit.
        let goals = vec![
            goal(
                "g1",
                "p1",
                Some("blocked on daemon restarts during the build"),
                60,
            ),
            goal(
                "g2",
                "p2",
                Some("stalled: daemon restarts keep killing sessions"),
                90,
            ),
            goal(
                "g3",
                "p3",
                Some("waiting on network restarts to finish"),
                30,
            ),
        ];
        let clusters = find_stalled_patterns(&goals);
        assert_eq!(clusters.len(), 1);
        let ids: Vec<&str> = clusters[0]
            .goals
            .iter()
            .map(|g| g.goal_id.as_str())
            .collect();
        assert_eq!(ids, vec!["g1", "g2"]);
    }

    #[test]
    fn two_independent_clusters_both_surface_largest_first() {
        let goals = vec![
            goal("g1", "p1", Some("blocked on daemon restarts nightly"), 60),
            goal(
                "g2",
                "p2",
                Some("stalled: daemon restarts again tonight"),
                60,
            ),
            goal("g3", "p3", Some("waiting on API rate limits daily"), 60),
            goal("g4", "p4", Some("stuck on API rate limits too"), 60),
            goal("g5", "p5", Some("also hit API rate limits here"), 60),
        ];
        let clusters = find_stalled_patterns(&goals);
        assert_eq!(clusters.len(), 2);
        // The 3-goal "API rate limits" cluster outranks the 2-goal one.
        assert_eq!(clusters[0].phrase, "rate limits");
        assert_eq!(clusters[0].goals.len(), 3);
        assert_eq!(clusters[1].phrase, "daemon restarts");
        assert_eq!(clusters[1].goals.len(), 2);
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
