use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use chrono::{DateTime, NaiveTime, Utc};
use permagent::config::paths::Paths;
use permagent::session::session_manager::SessionType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
struct AgentStatus {
    name: String,
    state: String,
    active_count: usize,
    summary: String,
}

#[derive(Serialize)]
struct DashboardStats {
    sessions_today: usize,
    sessions_total: usize,
    memory_count: usize,
    memory_delta_today: usize,
}

#[derive(Serialize)]
struct InFlightSession {
    id: String,
    title: String,
    started_at: String,
    state: String,
    progress: f64,
}

#[derive(Serialize)]
struct RecentSession {
    id: String,
    title: String,
    state: String,
    ended_at: String,
}

#[derive(Serialize)]
struct DashboardResponse {
    agent: AgentStatus,
    stats: DashboardStats,
    in_flight: Vec<InFlightSession>,
    recent: Vec<RecentSession>,
}

async fn get_dashboard(State(state): State<Arc<AppState>>) -> Json<DashboardResponse> {
    let persona = state.persona.read().await;
    let name = persona.first_name.clone();
    drop(persona);

    let now = Utc::now();
    let today_midnight = now.date_naive().and_time(NaiveTime::MIN).and_utc();

    // Fetch all sessions — lean projection (SessionSummary). This view only
    // reads scalar fields/counts, never the heavy JSON blobs. See #341/#371.
    let sessions = state
        .session_manager()
        .list_session_summaries()
        .await
        .unwrap_or_default();

    let sessions_total = sessions.len();
    let sessions_today = sessions
        .iter()
        .filter(|s| s.created_at >= today_midnight)
        .count();

    // Determine active sessions (updated within last 2 minutes, not system type)
    let two_min_ago = now - chrono::Duration::seconds(120);
    let active: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.updated_at >= two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .collect();
    let active_count = active.len();

    let agent_state = if active_count > 0 { "thinking" } else { "idle" };
    let summary = match active_count {
        0 => format!("{} is ready", name),
        1 => format!("{} is working on 1 thing for you", name),
        n => format!("{} is working on {} things for you", name, n),
    };

    // Memory stats from brain
    let (memory_count, memory_delta_today) = if let Some(brain) = state.brain.as_ref() {
        match brain.recall(&name, spectral::Visibility::Private).await {
            Ok(r) => {
                let ent_count = r.graph.neighborhood.entities.len();
                let mem_count = r.memory_hits.len();
                (ent_count + mem_count, 0) // delta requires timestamp tracking we don't have
            }
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    // In-flight: active sessions (updated within 2 min)
    let in_flight: Vec<InFlightSession> = active
        .iter()
        .take(3)
        .map(|s| {
            let elapsed_min = (now - s.updated_at).num_minutes().max(0) as f64;
            let progress = (elapsed_min / 5.0).min(0.95);
            InFlightSession {
                id: s.id.clone(),
                title: if s.name.is_empty() || s.name == "New Chat" {
                    format!("Session {}", s.id)
                } else {
                    truncate(&s.name, SESSION_TITLE_MAX)
                },
                started_at: s.created_at.to_rfc3339(),
                state: "thinking".to_string(),
                progress,
            }
        })
        .collect();

    // Recent: last 4 non-active sessions
    let recent: Vec<RecentSession> = sessions
        .iter()
        .filter(|s| {
            s.updated_at < two_min_ago
                && matches!(s.session_type, SessionType::User | SessionType::Scheduled)
        })
        .take(4)
        .map(|s| {
            // Non-active sessions (updated_at > 2 min ago) are completed.
            // We have no explicit pause/stop events so all recent sessions
            // are treated as completed to avoid false "paused" labels.
            let state = "completed";
            RecentSession {
                id: s.id.clone(),
                title: if s.name.is_empty() || s.name == "New Chat" {
                    format!("Session {}", s.id)
                } else {
                    truncate(&s.name, SESSION_TITLE_MAX)
                },
                state: state.to_string(),
                ended_at: s.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Json(DashboardResponse {
        agent: AgentStatus {
            name,
            state: agent_state.to_string(),
            active_count,
            summary,
        },
        stats: DashboardStats {
            sessions_today,
            sessions_total,
            memory_count,
            memory_delta_today,
        },
        in_flight,
        recent,
    })
}

/// Longest session title Home is sent, in bytes.
///
/// Was 40, which cut real titles mid-word ("Fix event_cta_click funnel leak:
/// 3,836 i…") while the Recent card had half its width empty — the card can
/// ellipsize to whatever width it has and shows the full title on hover, so
/// the server only needs a sanity cap against pathological names.
const SESSION_TITLE_MAX: usize = 200;

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", s.get(..end).unwrap_or(s))
    }
}

// ─── Dashboard Layout Persistence ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardSize {
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardCard {
    pub id: String,
    #[serde(rename = "type")]
    pub card_type: String,
    pub position: CardPosition,
    pub size: CardSize,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardLayout {
    pub cards: Vec<DashboardCard>,
}

/// Config key for the orchestrator platform extension. Mirrors
/// `permagent::agents::platform_extensions::orchestrator::EXTENSION_NAME`;
/// kept as a local literal to avoid a cross-crate visibility dependency.
/// The Decision Inbox feature is dormant until this extension is enabled, so
/// its Home card must appear iff `is_extension_enabled(ORCHESTRATOR_EXTENSION)`.
const ORCHESTRATOR_EXTENSION: &str = "orchestrator";

impl DashboardLayout {
    /// The default layout matching the current static Dashboard render.
    /// Grid uses 12 columns. Row height is abstract (1 unit ~ one card row).
    ///
    /// When the orchestrator is enabled the Decision Inbox card takes the
    /// top-right slot (matching the command-center frontend `DEFAULT_LAYOUT`)
    /// and the remaining cards shift down a row. When dormant the card is
    /// absent entirely — dormancy is uniform across daemon and surface.
    pub fn default_layout(orchestrator_enabled: bool) -> Self {
        let hero = DashboardCard {
            id: "hero".to_string(),
            card_type: "hero".to_string(),
            position: CardPosition { x: 0, y: 0 },
            size: CardSize { w: 7, h: 4 },
            visible: true,
        };

        if orchestrator_enabled {
            // 5-card layout: decisions top-right, everything else shifts down.
            Self {
                cards: vec![
                    hero,
                    Self::decisions_card(),
                    DashboardCard {
                        id: "stats".to_string(),
                        card_type: "stats".to_string(),
                        position: CardPosition { x: 0, y: 4 },
                        size: CardSize { w: 5, h: 4 },
                        visible: true,
                    },
                    DashboardCard {
                        id: "in_flight".to_string(),
                        card_type: "in_flight".to_string(),
                        position: CardPosition { x: 0, y: 8 },
                        size: CardSize { w: 12, h: 3 },
                        visible: true,
                    },
                    DashboardCard {
                        id: "recent".to_string(),
                        card_type: "recent".to_string(),
                        position: CardPosition { x: 0, y: 11 },
                        size: CardSize { w: 12, h: 4 },
                        visible: true,
                    },
                ],
            }
        } else {
            // 4-card dormant layout (no Decision Inbox card).
            Self {
                cards: vec![
                    hero,
                    DashboardCard {
                        id: "stats".to_string(),
                        card_type: "stats".to_string(),
                        position: CardPosition { x: 7, y: 0 },
                        size: CardSize { w: 5, h: 4 },
                        visible: true,
                    },
                    DashboardCard {
                        id: "in_flight".to_string(),
                        card_type: "in_flight".to_string(),
                        position: CardPosition { x: 0, y: 4 },
                        size: CardSize { w: 12, h: 3 },
                        visible: true,
                    },
                    DashboardCard {
                        id: "recent".to_string(),
                        card_type: "recent".to_string(),
                        position: CardPosition { x: 0, y: 7 },
                        size: CardSize { w: 12, h: 4 },
                        visible: true,
                    },
                ],
            }
        }
    }

    /// The Decision Inbox card descriptor (top-right slot, 5x4).
    fn decisions_card() -> DashboardCard {
        DashboardCard {
            id: "decisions".to_string(),
            card_type: "decisions".to_string(),
            position: CardPosition { x: 7, y: 0 },
            size: CardSize { w: 5, h: 4 },
            visible: true,
        }
    }
}

/// Reconcile a (possibly persisted) layout's Decision Inbox card against the
/// orchestrator's enabled state so dormancy is uniform regardless of any
/// stale `dashboard.json`:
///  - disabled  -> strip every decisions card (a dormant feature never surfaces)
///  - enabled and missing -> append one below all existing cards (surfaces
///    without overlapping a layout the user has already arranged; fresh
///    installs get the proper top-right slot via `default_layout`)
///  - enabled and present -> leave as-is
fn enforce_decisions_visibility(cards: &mut Vec<DashboardCard>, orchestrator_enabled: bool) {
    let is_decisions = |c: &DashboardCard| c.card_type == "decisions" || c.id == "decisions";
    if !orchestrator_enabled {
        cards.retain(|c| !is_decisions(c));
        return;
    }
    if cards.iter().any(is_decisions) {
        return;
    }
    let next_y = cards
        .iter()
        .map(|c| c.position.y + c.size.h)
        .max()
        .unwrap_or(0);
    let mut card = DashboardLayout::decisions_card();
    card.position = CardPosition { x: 0, y: next_y };
    cards.push(card);
}

fn layout_path() -> std::path::PathBuf {
    Paths::in_data_dir("dashboard.json")
}

async fn get_layout() -> Json<DashboardLayout> {
    // Decision Inbox dormancy: the Home card is present iff the orchestrator
    // platform extension is enabled. Read once and apply to whatever layout we
    // serve — default or persisted.
    let orchestrator_enabled =
        permagent::config::extensions::is_extension_enabled(ORCHESTRATOR_EXTENSION);
    let path = layout_path();
    let mut layout = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => serde_json::from_str::<DashboardLayout>(&contents)
            .unwrap_or_else(|_| DashboardLayout::default_layout(orchestrator_enabled)),
        Err(_) => DashboardLayout::default_layout(orchestrator_enabled),
    };
    enforce_decisions_visibility(&mut layout.cards, orchestrator_enabled);
    Json(layout)
}

#[derive(Deserialize)]
struct PutLayoutBody {
    cards: Vec<DashboardCard>,
}

async fn put_layout(
    Json(body): Json<PutLayoutBody>,
) -> Result<Json<DashboardLayout>, crate::routes::errors::ErrorResponse> {
    // Validate: at least one card
    if body.cards.is_empty() {
        return Err(crate::routes::errors::ErrorResponse::bad_request(
            "Layout must contain at least one card",
        ));
    }

    // Validate: no duplicate ids
    let mut seen_ids = std::collections::HashSet::new();
    for card in &body.cards {
        if !seen_ids.insert(&card.id) {
            return Err(crate::routes::errors::ErrorResponse::bad_request(format!(
                "Duplicate card id: {}",
                card.id
            )));
        }
    }

    // Snapshot whatever is live before it is overwritten — the one-level
    // undo `revert_layout` reads back (dagK home-templates lane; P9).
    snapshot_current_layout_before_write().await;

    let layout = DashboardLayout { cards: body.cards };
    let path = layout_path();

    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    // Atomic write: write to tmp then rename
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&layout)
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    tokio::fs::write(&tmp_path, json.as_bytes())
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    Ok(Json(layout))
}

// ─── Layout snapshot + revert (dagK home-templates lane) ────────────────────
//
// A layout change proposed by the agent (`apply_dashboard_layout`) or made by
// hand in Customize should have one level of undo (P9,
// HOME_TEMPLATES_RESEARCH.md — an adaptive UI a user cannot cleanly back out
// of is the trust-breaking failure mode). Every `PUT` snapshots whatever was
// live BEFORE the write into `dashboard.previous.json`; revert restores it,
// snapshotting the (about to be replaced) current layout in turn, so a second
// revert flips back — undo/redo of exactly one step, never a deeper history.

fn previous_layout_path() -> std::path::PathBuf {
    Paths::in_data_dir("dashboard.previous.json")
}

/// Copy whatever is currently live into the snapshot slot. Best-effort and
/// silent on a missing/corrupt current file — the FIRST-ever `PUT` has
/// nothing to snapshot, which is exactly why `revert_layout` below reports
/// "nothing to revert to" rather than restoring garbage.
async fn snapshot_current_layout_before_write() {
    let path = layout_path();
    let Ok(contents) = tokio::fs::read_to_string(&path).await else {
        return;
    };
    let prev_path = previous_layout_path();
    if let Some(parent) = prev_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&prev_path, contents).await;
}

async fn revert_layout() -> Result<Json<DashboardLayout>, crate::routes::errors::ErrorResponse> {
    let prev_path = previous_layout_path();
    let contents = tokio::fs::read_to_string(&prev_path).await.map_err(|_| {
        crate::routes::errors::ErrorResponse::bad_request(
            "No previous dashboard layout to revert to",
        )
    })?;
    let layout: DashboardLayout = serde_json::from_str(&contents).map_err(|e| {
        crate::routes::errors::ErrorResponse::internal(format!(
            "Saved previous layout is corrupt: {e}"
        ))
    })?;

    // Snapshot the layout revert is about to replace, so calling revert AGAIN
    // flips back to it — one level of undo in either direction.
    snapshot_current_layout_before_write().await;

    let path = layout_path();
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let tmp_path = path.with_extension("json.tmp");
    tokio::fs::write(&tmp_path, contents.as_bytes())
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    Ok(Json(layout))
}

// ─── Dashboard templates (dagK home-templates lane) ─────────────────────────
//
// The five layouts from HOME_TEMPLATES_RESEARCH.md §2, as data — the daemon
// is the source of truth the agent's `list_dashboard_templates`/
// `preview_dashboard_layout` tools read; the frontend keeps a hand-synced
// mirror (`dashboard/templates.ts`) for instant rendering and the harness,
// the same two-copies-in-sync pattern `DEFAULT_LAYOUT`/`default_layout`
// already use between this file and `useLayout.ts`.

/// Card types with a real card behind them today. Everything else a template
/// names is a real card from the research doc that Phase 2 has not built
/// yet — marked `comingSoon` so a template degrades gracefully (omitted from
/// what actually renders, named in the preview only) instead of showing a
/// broken tile.
const IMPLEMENTED_CARD_TYPES: &[&str] = &[
    "decisions",
    "stats",
    "in_flight",
    "recent",
    "timeline",
    "todos",
    "growth_results",
    "council",
    "calendar",
    "weather",
    "system_stats",
    "henry_status",
    "job_health",
    "today_focus",
    "overnight_digest",
    "fleet_kpis",
    "stalled_pattern",
    "project_health_grid",
    // Phase 2 (dagK home-templates lane) — all manifest cards.
    "finance_summary",
    "finance_watchlist",
    "cost_vs_outcome",
    "people_attention",
    "open_commitments",
    "recent_meetings",
    "week_headline",
    "week_small_multiples",
    "memory_growth",
    "on_this_week",
];

#[derive(Debug, Clone, Serialize)]
pub struct TemplateCard {
    #[serde(rename = "type")]
    pub card_type: String,
    pub id: String,
    pub position: CardPosition,
    pub size: CardSize,
    /// `true` when Phase 2 hasn't built this card yet (see
    /// `IMPLEMENTED_CARD_TYPES`).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub coming_soon: bool,
}

fn tcard(card_type: &str, x: i32, y: i32, w: i32, h: i32) -> TemplateCard {
    TemplateCard {
        card_type: card_type.to_string(),
        id: card_type.to_string(),
        position: CardPosition { x, y },
        size: CardSize { w, h },
        coming_soon: !IMPLEMENTED_CARD_TYPES.contains(&card_type),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardTemplate {
    pub id: String,
    pub name: String,
    pub question: String,
    pub cards: Vec<TemplateCard>,
}

pub fn dashboard_templates() -> Vec<DashboardTemplate> {
    vec![
        DashboardTemplate {
            id: "morning_briefing".to_string(),
            name: "Morning Briefing".to_string(),
            question: "What does today need from me?".to_string(),
            cards: vec![
                // h:2 — a manifest 'compact' card is icon+label, a hero value
                // and an inline supporting row (ManifestCard.tsx); h:1
                // clipped the hero value's glyphs against the tile edge.
                tcard("henry_status", 0, 0, 12, 2),
                tcard("decisions", 0, 2, 5, 4),
                tcard("today_focus", 5, 2, 7, 4),
                tcard("in_flight", 0, 6, 12, 3),
                tcard("calendar", 0, 9, 5, 4),
                tcard("todos", 5, 9, 4, 4),
                tcard("overnight_digest", 9, 9, 3, 4),
                tcard("weather", 0, 13, 3, 2),
                tcard("system_stats", 3, 13, 3, 2),
            ],
        },
        DashboardTemplate {
            id: "operator_command_center".to_string(),
            name: "Operator Command Center".to_string(),
            question: "What's the state of all my projects, and where's the risk?".to_string(),
            cards: vec![
                tcard("fleet_kpis", 0, 0, 12, 2),
                tcard("stalled_pattern", 0, 2, 12, 3),
                tcard("project_health_grid", 0, 5, 12, 5),
                tcard("in_flight", 0, 10, 8, 3),
                tcard("job_health", 8, 10, 4, 3),
                tcard("growth_results", 0, 13, 12, 6),
            ],
        },
        DashboardTemplate {
            id: "weekly_story".to_string(),
            name: "Weekly Story".to_string(),
            question: "What actually happened this week, and how does it compare?".to_string(),
            cards: vec![
                tcard("week_headline", 0, 0, 12, 2),
                tcard("timeline", 0, 2, 12, 6),
                tcard("week_small_multiples", 0, 8, 12, 3),
                tcard("council", 0, 11, 12, 5),
                tcard("memory_growth", 0, 16, 6, 3),
                tcard("on_this_week", 6, 16, 6, 3),
            ],
        },
        DashboardTemplate {
            id: "relationships_commitments".to_string(),
            name: "Relationships & Commitments".to_string(),
            question: "Who am I neglecting, and what did I promise?".to_string(),
            cards: vec![
                tcard("people_attention", 0, 0, 6, 5),
                tcard("open_commitments", 6, 0, 6, 5),
                tcard("recent_meetings", 0, 5, 6, 4),
                tcard("calendar", 6, 5, 6, 4),
            ],
        },
        DashboardTemplate {
            id: "growth_and_money".to_string(),
            name: "Growth & Money".to_string(),
            question: "Is the work paying off?".to_string(),
            cards: vec![
                tcard("growth_results", 0, 0, 12, 6),
                tcard("finance_summary", 0, 6, 8, 5),
                tcard("finance_watchlist", 8, 6, 4, 5),
                tcard("cost_vs_outcome", 0, 11, 12, 3),
            ],
        },
    ]
}

async fn get_templates() -> Json<Vec<DashboardTemplate>> {
    Json(dashboard_templates())
}

// ─── Dashboard insights (dagK home-templates lane; Phase-1 synthesis) ───────
//
// One aggregation endpoint backs all of `today_focus`, `overnight_digest`,
// `fleet_kpis` and `stalled_pattern` (project_health_grid rides along too) —
// converged into a single call rather than one per card (R7: the client
// would otherwise fan out 4+ requests for cards that all read the same
// underlying goals/decisions/projects). All computation is the deterministic,
// unit-tested `permagent::dashboard_insights` module; this handler only
// fetches rows and maps them into that module's plain fact types.

fn parse_rfc3339_or_now(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

async fn get_insights(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, crate::routes::errors::ErrorResponse> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    let goals_raw = permagent::cards::list_active_goals(&pool)
        .await
        .unwrap_or_default();
    let goals: Vec<permagent::dashboard_insights::GoalFact> = goals_raw
        .iter()
        .map(|g| permagent::dashboard_insights::GoalFact {
            id: g.id.clone(),
            title: g.title.clone(),
            project_id: g.project_id.clone(),
            state: g.state.clone(),
            hold_note: g.hold_note.clone(),
            routing_note: g.routing_note.clone(),
            created_at: parse_rfc3339_or_now(&g.created_at),
            updated_at: parse_rfc3339_or_now(&g.updated_at),
        })
        .collect();

    let decisions_raw = permagent::decisions::list_open_decisions(&pool)
        .await
        .unwrap_or_default();
    let decisions: Vec<permagent::dashboard_insights::DecisionFact> = decisions_raw
        .iter()
        .map(|d| permagent::dashboard_insights::DecisionFact {
            id: d.decision.id.clone(),
            headline: d.decision.headline.clone(),
            project_id: d.decision.project_id.clone(),
            created_at: parse_rfc3339_or_now(&d.decision.created_at),
        })
        .collect();

    let projects_raw = permagent::projects::list_projects(&pool, None)
        .await
        .unwrap_or_default();
    let projects: Vec<permagent::dashboard_insights::ProjectFact> = projects_raw
        .iter()
        .map(|p| permagent::dashboard_insights::ProjectFact {
            id: p.id.clone(),
            name: p.name.clone(),
        })
        .collect();

    let sessions = state
        .session_manager()
        .list_session_summaries()
        .await
        .unwrap_or_default();
    let now = Utc::now();
    let today_midnight = now.date_naive().and_time(NaiveTime::MIN).and_utc();
    let sessions_today = sessions
        .iter()
        .filter(|s| s.created_at >= today_midnight)
        .count();

    // "Finished" for the overnight digest: non-active sessions (same 2-minute
    // active threshold `get_dashboard` uses) that wrapped up since midnight —
    // named rows, not just a count (coordinator, Phase-1 presentation pass).
    let two_min_ago = now - chrono::Duration::seconds(120);
    let finished_sessions: Vec<permagent::dashboard_insights::SessionFact> = sessions
        .iter()
        .filter(|s| {
            s.updated_at < two_min_ago
                && s.updated_at >= today_midnight
                && matches!(
                    s.session_type,
                    permagent::session::session_manager::SessionType::User
                        | permagent::session::session_manager::SessionType::Scheduled
                )
        })
        .map(|s| permagent::dashboard_insights::SessionFact {
            id: s.id.clone(),
            title: if s.name.is_empty() || s.name == "New Chat" {
                format!("Session {}", s.id)
            } else {
                s.name.clone()
            },
            ended_at: s.updated_at,
        })
        .collect();

    // Memory delta: honestly 0 — the daemon has no timestamped memory-delta
    // tracking yet (the same gap `get_dashboard`'s `memory_delta_today`
    // already documents). Never fabricated (P12).
    let new_memories: i64 = 0;

    let today_focus = permagent::dashboard_insights::compute_today_focus(&decisions, &goals);
    let overnight_digest = permagent::dashboard_insights::compute_overnight_digest(
        &decisions,
        &goals,
        &finished_sessions,
        new_memories,
        today_midnight,
    );
    let fleet_kpis = permagent::dashboard_insights::compute_fleet_kpis(
        &projects,
        &goals,
        &decisions,
        sessions_today,
    );
    let stalled_patterns = permagent::dashboard_insights::find_stalled_patterns(&goals);

    // project_health_grid: one row per project, with a plain-words reason for
    // a non-"active" status (coordinator, Phase-1 presentation pass) and a
    // real 14-day daily activity count where the activity journal has one —
    // "no history yet" otherwise, never a fabricated flat sparkline (P12).
    let daily_activity = permagent::activity_journal::daily_goal_activity_by_project(&pool, 14)
        .await
        .unwrap_or_default();
    let grid: Vec<serde_json::Value> = projects
        .iter()
        .map(|p| {
            let project_goals: Vec<_> = goals.iter().filter(|g| g.project_id == p.id).collect();
            let held = project_goals.iter().filter(|g| g.is_stalled()).count();
            let last_touched = project_goals.iter().map(|g| g.updated_at).max();
            let status = if project_goals.is_empty() {
                "dormant"
            } else if held > 0 {
                "stalled"
            } else {
                "active"
            };
            let reason = match status {
                "stalled" => project_goals
                    .iter()
                    .find(|g| g.is_stalled())
                    .and_then(|g| g.hold_note.clone()),
                "dormant" => Some("No active goals right now".to_string()),
                _ => None,
            };
            let sparkline: Vec<serde_json::Value> = daily_activity
                .iter()
                .filter(|d| d.project_id == p.id)
                .map(|d| serde_json::json!({ "date": d.date, "count": d.count }))
                .collect();
            serde_json::json!({
                "projectId": p.id,
                "projectName": p.name,
                "activeGoals": project_goals.len(),
                "heldGoals": held,
                "status": status,
                "reason": reason,
                "lastTouched": last_touched.map(|t| t.to_rfc3339()),
                "sparkline": sparkline,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "todayFocus": today_focus,
        "overnightDigest": overnight_digest,
        "fleetKpis": fleet_kpis,
        "stalledPatterns": stalled_patterns,
        "projectHealthGrid": grid,
    })))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/dashboard/layout", get(get_layout).put(put_layout))
        .route(
            "/api/dashboard/layout/revert",
            get(revert_layout).post(revert_layout),
        )
        .route("/api/dashboard/templates", get(get_templates))
        .route("/api/dashboard/insights", get(get_insights))
        .with_state(state)
}

#[cfg(test)]
mod tests {

    #[test]
    fn session_titles_are_not_cut_at_forty_chars() {
        let title = "Fix event_cta_click funnel leak: 3,836 impressions, 0 clicks";
        assert!(title.len() > 40);
        assert_eq!(super::truncate(title, super::SESSION_TITLE_MAX), title);
    }

    #[test]
    fn truncate_still_caps_pathological_titles_on_a_char_boundary() {
        let title = "é".repeat(300);
        let out = super::truncate(&title, super::SESSION_TITLE_MAX);
        assert!(out.len() <= super::SESSION_TITLE_MAX + 3);
        assert!(out.ends_with("..."));
    }
    use super::*;

    #[test]
    fn default_layout_has_four_cards() {
        // Dormant (orchestrator disabled): no Decision Inbox card.
        let layout = DashboardLayout::default_layout(false);
        assert_eq!(layout.cards.len(), 4);
        assert_eq!(layout.cards[0].id, "hero");
        assert_eq!(layout.cards[1].id, "stats");
        assert_eq!(layout.cards[2].id, "in_flight");
        assert_eq!(layout.cards[3].id, "recent");
        assert!(!layout.cards.iter().any(|c| c.card_type == "decisions"));
    }

    #[test]
    fn default_layout_enabled_includes_decisions_top_right() {
        let layout = DashboardLayout::default_layout(true);
        assert_eq!(layout.cards.len(), 5);
        let decisions = layout
            .cards
            .iter()
            .find(|c| c.card_type == "decisions")
            .expect("decisions card present when orchestrator enabled");
        // Top-right slot, matching the command-center frontend DEFAULT_LAYOUT.
        assert_eq!(decisions.position, CardPosition { x: 7, y: 0 });
        assert_eq!(decisions.size, CardSize { w: 5, h: 4 });
        // Hero still anchors top-left; remaining cards shifted down a row.
        assert_eq!(layout.cards[0].id, "hero");
        assert_eq!(layout.cards[0].position, CardPosition { x: 0, y: 0 });
    }

    #[test]
    fn enforce_strips_decisions_when_disabled() {
        let mut cards = DashboardLayout::default_layout(true).cards;
        assert!(cards.iter().any(|c| c.card_type == "decisions"));
        enforce_decisions_visibility(&mut cards, false);
        assert!(!cards.iter().any(|c| c.card_type == "decisions"));
        assert_eq!(cards.len(), 4);
    }

    #[test]
    fn enforce_appends_decisions_when_enabled_and_missing() {
        // A persisted dormant layout that predates the feature.
        let mut cards = DashboardLayout::default_layout(false).cards;
        let max_bottom = cards.iter().map(|c| c.position.y + c.size.h).max().unwrap();
        enforce_decisions_visibility(&mut cards, true);
        let decisions: Vec<_> = cards
            .iter()
            .filter(|c| c.card_type == "decisions")
            .collect();
        assert_eq!(decisions.len(), 1, "exactly one decisions card");
        // Appended below existing cards — never overlaps the user's arrangement.
        assert!(decisions[0].position.y >= max_bottom);
    }

    #[test]
    fn enforce_is_noop_when_enabled_and_present() {
        let mut cards = DashboardLayout::default_layout(true).cards;
        let before = cards.len();
        enforce_decisions_visibility(&mut cards, true);
        assert_eq!(cards.len(), before);
        assert_eq!(
            cards.iter().filter(|c| c.card_type == "decisions").count(),
            1
        );
    }

    #[test]
    fn default_layout_uses_12_column_grid() {
        let layout = DashboardLayout::default_layout(false);
        // Hero(7) + Stats(5) = 12 columns in first row
        assert_eq!(layout.cards[0].size.w + layout.cards[1].size.w, 12);
        // Top row cards have same height
        assert_eq!(layout.cards[0].size.h, layout.cards[1].size.h);
        // In-flight and Recent span full width
        assert_eq!(layout.cards[2].size.w, 12);
        assert_eq!(layout.cards[3].size.w, 12);
        // In-flight starts after top row
        assert_eq!(layout.cards[2].position.y, layout.cards[0].size.h);
    }

    #[test]
    fn layout_serialization_roundtrip() {
        let layout = DashboardLayout::default_layout(false);
        let json = serde_json::to_string_pretty(&layout).unwrap();
        let deserialized: DashboardLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(layout, deserialized);
    }

    #[test]
    fn card_type_field_serializes_as_type() {
        let card = DashboardCard {
            id: "test".to_string(),
            card_type: "hero".to_string(),
            position: CardPosition { x: 0, y: 0 },
            size: CardSize { w: 6, h: 3 },
            visible: true,
        };
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains(r#""type":"hero""#));
        assert!(!json.contains("card_type"));
    }

    #[test]
    fn malformed_json_returns_default_layout() {
        // Simulates what get_layout does when file content is invalid
        let bad_json = "{ not valid json }";
        let result = serde_json::from_str::<DashboardLayout>(bad_json)
            .unwrap_or_else(|_| DashboardLayout::default_layout(false));
        assert_eq!(result, DashboardLayout::default_layout(false));
    }

    // Sets PERMAGENT_PATH_ROOT → #[serial] + env_lock guard (standing rule;
    // the findings.rs/agent.rs pattern). Unserialized, these raced every other
    // test that resolves paths through the data dir — the trigger of the #792
    // macos-15 flake. The guard also restores the previous value on drop
    // instead of blindly removing it.
    #[tokio::test]
    #[serial_test::serial]
    async fn layout_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        // Point Paths at temp dir
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        let layout = DashboardLayout {
            cards: vec![DashboardCard {
                id: "hero".to_string(),
                card_type: "hero".to_string(),
                position: CardPosition { x: 0, y: 0 },
                size: CardSize { w: 12, h: 4 },
                visible: true,
            }],
        };

        let path = layout_path();
        let json = serde_json::to_string_pretty(&layout).unwrap();
        tokio::fs::write(&path, json.as_bytes()).await.unwrap();

        let read_back = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: DashboardLayout = serde_json::from_str(&read_back).unwrap();
        assert_eq!(parsed, layout);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        let path = layout_path();
        // File doesn't exist
        let result = match tokio::fs::read_to_string(&path).await {
            Ok(contents) => serde_json::from_str::<DashboardLayout>(&contents)
                .unwrap_or_else(|_| DashboardLayout::default_layout(false)),
            Err(_) => DashboardLayout::default_layout(false),
        };
        assert_eq!(result, DashboardLayout::default_layout(false));
    }

    // ── Layout snapshot + revert (dagK home-templates lane) ─────────────────

    fn one_card_layout(card_type: &str) -> DashboardLayout {
        DashboardLayout {
            cards: vec![DashboardCard {
                id: card_type.to_string(),
                card_type: card_type.to_string(),
                position: CardPosition { x: 0, y: 0 },
                size: CardSize { w: 12, h: 4 },
                visible: true,
            }],
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn revert_with_no_prior_write_is_an_explicit_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);
        // No dashboard.json and no dashboard.previous.json exist yet.
        assert!(revert_layout().await.is_err());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn put_snapshots_the_layout_it_replaces_and_revert_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        // Seed the "before" layout directly (as if an earlier PUT wrote it).
        let before = one_card_layout("timeline");
        let path = layout_path();
        tokio::fs::write(&path, serde_json::to_string_pretty(&before).unwrap())
            .await
            .unwrap();

        // A PUT to a different layout must snapshot "timeline" as previous.
        let after = one_card_layout("council");
        put_layout(Json(PutLayoutBody {
            cards: after.cards.clone(),
        }))
        .await
        .expect("put succeeds");

        let live = serde_json::from_str::<DashboardLayout>(
            &tokio::fs::read_to_string(&path).await.unwrap(),
        )
        .unwrap();
        assert_eq!(live, after);

        let reverted = revert_layout().await.expect("revert succeeds").0;
        assert_eq!(reverted, before, "revert restores the pre-PUT layout");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn reverting_twice_flips_back_to_the_reverted_from_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        let a = one_card_layout("recent");
        let b = one_card_layout("growth_results");
        tokio::fs::write(layout_path(), serde_json::to_string_pretty(&a).unwrap())
            .await
            .unwrap();
        put_layout(Json(PutLayoutBody {
            cards: b.cards.clone(),
        }))
        .await
        .unwrap();

        let first_revert = revert_layout().await.unwrap().0;
        assert_eq!(first_revert, a);
        let second_revert = revert_layout().await.unwrap().0;
        assert_eq!(second_revert, b, "a second revert is a redo, not a no-op");
    }

    // ── Templates (dagK home-templates lane) ─────────────────────────────────

    #[test]
    fn there_are_exactly_five_templates_matching_the_research_doc() {
        let templates = dashboard_templates();
        assert_eq!(templates.len(), 5);
        let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "morning_briefing",
                "operator_command_center",
                "weekly_story",
                "relationships_commitments",
                "growth_and_money",
            ]
        );
    }

    #[test]
    fn every_template_has_a_name_and_a_question_and_at_least_one_card() {
        for t in dashboard_templates() {
            assert!(!t.name.is_empty(), "{} has no name", t.id);
            assert!(!t.question.is_empty(), "{} has no question", t.id);
            assert!(!t.cards.is_empty(), "{} has no cards", t.id);
        }
    }

    #[test]
    fn phase_1_templates_have_no_coming_soon_cards() {
        // Morning Briefing and Operator Command Center are the two templates
        // this lane fully builds (LANE brief, build order #2/#3) — every card
        // they name must already be implemented.
        for id in ["morning_briefing", "operator_command_center"] {
            let t = dashboard_templates()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap();
            assert!(
                t.cards.iter().all(|c| !c.coming_soon),
                "{id} names a card that isn't built yet: {:?}",
                t.cards
                    .iter()
                    .filter(|c| c.coming_soon)
                    .map(|c| &c.card_type)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn all_five_templates_are_fully_built_after_phase_2() {
        // Phase 2 (dagK home-templates lane) shipped every remaining card —
        // no template should still mark anything `comingSoon`.
        for t in dashboard_templates() {
            assert!(
                t.cards.iter().all(|c| !c.coming_soon),
                "{} still names an unbuilt card: {:?}",
                t.id,
                t.cards
                    .iter()
                    .filter(|c| c.coming_soon)
                    .map(|c| &c.card_type)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn no_template_card_is_positioned_or_sized_outside_the_12_column_grid() {
        for t in dashboard_templates() {
            for c in &t.cards {
                assert!(
                    c.size.w >= 1 && c.size.w <= 12,
                    "{}/{} bad width",
                    t.id,
                    c.card_type
                );
                assert!(c.size.h >= 1, "{}/{} bad height", t.id, c.card_type);
                assert!(
                    c.position.x + c.size.w <= 12,
                    "{}/{} overflows the grid",
                    t.id,
                    c.card_type
                );
            }
        }
    }

    #[test]
    fn get_templates_serializes_type_not_card_type() {
        let json = serde_json::to_value(dashboard_templates()).unwrap();
        let first_card = &json[0]["cards"][0];
        assert!(first_card.get("type").is_some());
        assert!(first_card.get("cardType").is_none());
    }

    // ── Insights (dagK home-templates lane) ──────────────────────────────────
    // The deterministic computation itself is unit-tested exhaustively in
    // `permagent::dashboard_insights`; these prove only the glue (parsing,
    // field mapping) this route owns.

    #[test]
    fn parse_rfc3339_or_now_falls_back_instead_of_panicking() {
        let bad = parse_rfc3339_or_now("not a timestamp");
        // No panic, and the fallback is "now" (recent), not epoch/zero, so a
        // malformed timestamp never reads as "ancient" in age-based ranking.
        assert!((Utc::now() - bad).num_seconds().abs() < 5);
    }

    #[test]
    fn parse_rfc3339_or_now_parses_a_real_timestamp() {
        let dt = parse_rfc3339_or_now("2026-01-01T00:00:00Z");
        assert_eq!(dt.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    }
}
