use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use chrono::{NaiveTime, Utc};
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
                    truncate(&s.name, 40)
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
                    truncate(&s.name, 40)
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

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/dashboard/layout", get(get_layout).put(put_layout))
        .with_state(state)
}

#[cfg(test)]
mod tests {
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
}
