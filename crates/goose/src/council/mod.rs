//! The Council — a weekly multi-provider debate about the state of the work.
//!
//! Identity and the debate loop live here. The Sunday sweep lives in the
//! daemon (`council_sweep`). Henry live-queries with `council_status` /
//! `council_report` and convenes on demand via the `deliberate` platform
//! extension. Default OFF: convene spends every connected chat provider.

use std::sync::OnceLock;

use sqlx::{Pool, Sqlite};

use crate::agents::self_knowledge::{
    FeatureCategory, FeatureDescriptor, StateSource, SurfaceRef, TeachingStep,
};
use crate::config::Config;

pub mod brief;
pub mod debate;
pub mod deliver;
pub mod due;
pub mod membership;
pub mod store;

pub const ENABLED_KEY: &str = "council_enabled";
pub const AGENT_ID: &str = "council";
pub const AGENT_NAME: &str = "The Council";

pub fn is_enabled() -> bool {
    Config::global()
        .get_param::<bool>(ENABLED_KEY)
        .unwrap_or(false)
}

pub const SELF_KNOWLEDGE_FEATURE: FeatureDescriptor = FeatureDescriptor {
    id: AGENT_ID,
    display_name: "The Council",
    category: FeatureCategory::Worker,
    what_it_does:
        "Briefs every connected chat-completion provider on the current state of the work — \
         projects, boards, due cards, activity, analytics, Watcher insights, Forecaster \
         direction, open decisions — then runs a two-round debate and chairs a weekly report. \
         Live-query with council_status (on or off, seated models, last headline, open inbox \
         actions) and council_report (the full digest including per-model dissent). You \
         convene it with council_convene; a Sunday-night sweep runs the same session when \
         council_enabled is on. Actions land as Decision Inbox proposals (council_action): \
         approve files a board card, reject dismisses. It only ever proposes",
    why_it_matters:
        "One model is a take. Several models, looking at the same brief and then at each other, \
         surface the project that needs attention, the pattern you are missing, and the \
         analytics that actually moved — as a digest you can act on, not a second inbox",
    state_source: StateSource::Queryable,
    teaching: &[
        TeachingStep {
            title: "The user flips the switch",
            body: "The Council is off until the user turns on council_enabled under \
                   Settings → Features. A scanner of every connected chat model is \
                   switched on by them, never by you. Coding CLIs are not seats; \
                   membership checkboxes on that same Features row exclude a provider. \
                   Tell them plainly that each session spends API credits on every \
                   seated model.",
            open_surface: Some(SurfaceRef {
                tab: "Settings",
                section: Some("features"),
            }),
            confirm: None,
        },
        TeachingStep {
            title: "Live-query; do not convene to peek",
            body: "Ask how it is doing with council_status — that is the cheap live \
                   query (on or off, who sits, last headline, open Decision Inbox \
                   actions). Read the last digest with council_report (omit the \
                   session id for latest, or pass one). Never council_convene just \
                   to check status: that spends every seated provider.",
            open_surface: None,
            confirm: None,
        },
        TeachingStep {
            title: "Convene or wait for Sunday night",
            body: "Once council_enabled is on, council_convene runs a session now \
                   (optional question is added to the portfolio brief). A Sunday \
                   22:00 local sweep runs the same session; Monday still catches a \
                   missed Sunday. You chair the synthesis; you do not impersonate \
                   the other models.",
            open_surface: None,
            confirm: None,
        },
        TeachingStep {
            title: "Actions land in the Inbox; the report lands on Dashboard",
            body: "Up to five recommendations file as Decision Inbox council_action \
                   cards — approve files a board card on the named project, reject \
                   dismisses. The weekly report is the Council card on Dashboard. \
                   You do not act on the report yourself.",
            open_surface: Some(SurfaceRef {
                tab: "Dashboard",
                section: None,
            }),
            confirm: None,
        },
    ],
};

/// Process-wide lock so Henry and the Sunday sweep cannot overlap.
fn session_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, Clone)]
pub struct Convened {
    pub session_id: String,
    pub status: store::SessionStatus,
    pub headline: String,
    pub markdown: String,
    pub n_members: usize,
    pub n_ok: usize,
    pub n_actions: usize,
    pub error: Option<String>,
}

pub async fn convene(
    pool: &Pool<Sqlite>,
    trigger: store::Trigger,
    extra_question: Option<&str>,
    caller: &dyn debate::MemberCaller,
) -> Result<Convened, String> {
    if !is_enabled() {
        return Err("council_enabled is off — turn it on under Settings → Features".to_string());
    }
    let _guard = session_lock()
        .try_lock()
        .map_err(|_| "a council session is already running".to_string())?;
    if store::has_running(pool).await? {
        return Err("a council session is already running".to_string());
    }

    let members = membership::resolve_members().await;
    if members.is_empty() {
        return Err(
            "no connected chat providers sit on the Council (connect a key, or un-exclude a seat)"
                .to_string(),
        );
    }

    let snapshot = brief::assemble(pool, extra_question).await?;
    let brief_value = serde_json::to_value(&snapshot).unwrap_or(serde_json::json!({}));
    let session_id = store::insert_session(pool, trigger, extra_question, &brief_value).await?;

    let round1 = debate::run_round1_parallel(caller, &members, &snapshot.markdown).await;
    for r in &round1 {
        let _ = store::insert_position(
            pool,
            store::NewPosition {
                session_id: &session_id,
                round: 1,
                provider: &r.member.provider,
                model: &r.member.model,
                status: &r.status,
                raw_text: r.raw.as_deref(),
                parsed: r.parsed.as_ref(),
                error: r.error.as_deref(),
            },
        )
        .await;
    }

    if !debate::any_ok(&round1) {
        let err = "every council member failed or timed out";
        let _ = store::finish_session(
            pool,
            &session_id,
            store::SessionStatus::Failed,
            None,
            None,
            Some(err),
        )
        .await;
        return Ok(Convened {
            session_id,
            status: store::SessionStatus::Failed,
            headline: String::new(),
            markdown: String::new(),
            n_members: members.len(),
            n_ok: 0,
            n_actions: 0,
            error: Some(err.to_string()),
        });
    }

    let round2 = debate::run_round2_parallel(caller, &round1).await;
    for r in &round2 {
        let _ = store::insert_position(
            pool,
            store::NewPosition {
                session_id: &session_id,
                round: 2,
                provider: &r.member.provider,
                model: &r.member.model,
                status: &r.status,
                raw_text: r.raw.as_deref(),
                parsed: r.parsed.as_ref(),
                error: r.error.as_deref(),
            },
        )
        .await;
    }

    let (chair_provider, chair_model) = membership::chair_route();
    let chair = membership::Member {
        provider: chair_provider.clone(),
        display_name: "Chair".to_string(),
        model: chair_model.clone(),
    };
    let report = match debate::run_chair(caller, &chair, &snapshot.markdown, &round1, &round2).await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = store::finish_session(
                pool,
                &session_id,
                store::SessionStatus::Failed,
                Some(&chair_provider),
                Some(&chair_model),
                Some(&e),
            )
            .await;
            return Ok(Convened {
                session_id,
                status: store::SessionStatus::Failed,
                headline: String::new(),
                markdown: String::new(),
                n_members: members.len(),
                n_ok: round1.iter().filter(|r| r.status == "ok").count(),
                n_actions: 0,
                error: Some(e),
            });
        }
    };

    let actions: Vec<serde_json::Value> = report
        .actions
        .iter()
        .map(|a| serde_json::to_value(a).unwrap_or(serde_json::Value::Null))
        .collect();
    let _ = store::insert_report(
        pool,
        store::NewReport {
            session_id: &session_id,
            headline: &report.headline,
            markdown: &report.markdown,
            consensus: &report.consensus,
            dissent: &report.dissent,
            actions: &actions,
            chair_provider: Some(&chair_provider),
            chair_model: Some(&chair_model),
        },
    )
    .await;

    let n_ok = round1.iter().filter(|r| r.status == "ok").count();
    let status = if n_ok == members.len() {
        store::SessionStatus::Complete
    } else {
        store::SessionStatus::Partial
    };
    let _ = store::finish_session(
        pool,
        &session_id,
        status,
        Some(&chair_provider),
        Some(&chair_model),
        None,
    )
    .await;

    let action_ids = deliver::file_actions(pool, &session_id, &report)
        .await
        .unwrap_or_default();
    deliver::file_briefing(pool, &session_id, &report.headline, action_ids.len()).await;
    deliver::emit_nudge(&report.headline, action_ids.len());

    Ok(Convened {
        session_id,
        status,
        headline: report.headline,
        markdown: report.markdown,
        n_members: members.len(),
        n_ok,
        n_actions: action_ids.len(),
        error: None,
    })
}

pub async fn format_report(
    pool: &Pool<Sqlite>,
    session_id: Option<&str>,
) -> Result<String, String> {
    let (session, report) = match session_id {
        Some(id) => {
            let session = store::get_session(pool, id)
                .await?
                .ok_or_else(|| format!("no council session {id}"))?;
            let report = store::get_report_for_session(pool, id).await?;
            (session, report)
        }
        None => store::latest_finished(pool)
            .await?
            .ok_or_else(|| "no council report yet".to_string())?,
    };
    let positions = store::list_positions(pool, &session.id)
        .await
        .unwrap_or_default();
    let mut out = format!(
        "# Council session {}\nStatus: {:?}\nTrigger: {}\nChair: {} / {}\n\n",
        session.id,
        session.status,
        session.trigger,
        session.chair_provider.as_deref().unwrap_or("—"),
        session.chair_model.as_deref().unwrap_or("—"),
    );
    if let Some(r) = report {
        out.push_str(&format!("## {}\n\n{}\n\n", r.headline, r.markdown));
        if !r.consensus.is_empty() {
            out.push_str("### Consensus\n");
            for c in &r.consensus {
                out.push_str(&format!("- {c}\n"));
            }
            out.push('\n');
        }
        if !r.dissent.is_empty() {
            out.push_str("### Dissent\n");
            for d in &r.dissent {
                out.push_str(&format!("- {d}\n"));
            }
            out.push('\n');
        }
    }
    let round2 = positions.iter().any(|p| p.round == 2);
    out.push_str("## Per-model takes\n");
    for p in positions {
        if p.round == 1 || round2 {
            out.push_str(&format!(
                "\n### {} / {} (round {}, {})\n{}\n",
                p.provider,
                p.model,
                p.round,
                p.status,
                p.raw_text
                    .as_deref()
                    .unwrap_or(p.error.as_deref().unwrap_or(""))
            ));
        }
    }
    Ok(out)
}

/// Cheap live query: on/off, who sits, last headline, open inbox actions.
/// Works while the flag is off — convene is the call that spends.
pub async fn format_status(pool: Option<&Pool<Sqlite>>) -> String {
    let seats = membership::resolve_seats().await;
    let seated: Vec<String> = seats
        .iter()
        .filter(|s| s.eligible())
        .map(|s| format!("{} / {} ({})", s.display_name, s.model, s.provider))
        .collect();
    let mut last_session = None;
    let mut last_headline = None;
    let mut last_status = None;
    let mut last_started = None;
    let mut running = false;
    let mut open_actions = 0i64;
    let db_reachable = pool.is_some();
    if let Some(pool) = pool {
        running = store::has_running(pool).await.unwrap_or(false);
        open_actions = store::open_council_action_count(pool).await.unwrap_or(0);
        if let Ok(Some((session, report))) = store::latest_finished(pool).await {
            last_session = Some(session.id);
            last_status = Some(session.status.as_str().to_string());
            last_started = Some(session.started_at);
            last_headline = report.map(|r| r.headline);
        }
    }
    render_status(&StatusView {
        enabled: is_enabled(),
        seated,
        running,
        last_session,
        last_headline,
        last_status,
        last_started,
        open_actions,
        db_reachable,
    })
}

#[derive(Debug)]
struct StatusView {
    enabled: bool,
    seated: Vec<String>,
    running: bool,
    last_session: Option<String>,
    last_headline: Option<String>,
    last_status: Option<String>,
    last_started: Option<String>,
    open_actions: i64,
    db_reachable: bool,
}

fn render_status(view: &StatusView) -> String {
    let mut out = String::new();
    if view.enabled {
        out.push_str("The Council is ON (council_enabled).\n");
    } else {
        out.push_str(
            "The Council is OFF (council_enabled=false). Flip Settings → Features to turn it on. \
             council_status and council_report still work; council_convene refuses until the flag is on.\n",
        );
    }
    if view.seated.is_empty() {
        out.push_str(
            "Seated: none — connect a chat-completion provider key, or un-exclude a seat \
             under Settings → Features. Coding CLIs are not seats.\n",
        );
    } else {
        out.push_str(&format!(
            "Seated: {} chat-completion provider(s)\n",
            view.seated.len()
        ));
        for s in &view.seated {
            out.push_str(&format!("- {s}\n"));
        }
    }
    if !view.db_reachable {
        out.push_str("Session store: unreachable — last report and open actions are unknown.\n");
    } else if view.running {
        out.push_str("Session: a debate is running now.\n");
    } else {
        out.push_str("Session: idle.\n");
    }
    match (
        &view.last_session,
        &view.last_headline,
        &view.last_status,
        &view.last_started,
    ) {
        (Some(id), headline, status, started) => {
            let h = headline.as_deref().unwrap_or("(no headline)");
            out.push_str(&format!(
                "Last report: \"{h}\" ({}, session {id}, started {})\n",
                status.as_deref().unwrap_or("unknown"),
                started.as_deref().unwrap_or("unknown")
            ));
        }
        _ if view.db_reachable => out.push_str("Last report: none yet.\n"),
        _ => {}
    }
    if view.db_reachable {
        out.push_str(&format!(
            "Open Decision Inbox actions: {}\n",
            view.open_actions
        ));
    }
    out.push_str(
        "\nLive query: this is council_status. Call council_report (no args) for the full digest \
         including per-model dissent; pass a session id to read an older one. Call council_convene \
         to run a new debate (spends every seated provider; optional question is added to the \
         brief). Do not convene just to check status. Do not impersonate the other models. Do not \
         act on the report — actions are Decision Inbox proposals the user approves or rejects.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        assert!(!is_enabled());
    }

    #[test]
    fn status_render_names_the_live_query_and_does_not_spend() {
        let off = render_status(&StatusView {
            enabled: false,
            seated: vec!["Anthropic / haiku (anthropic)".into()],
            running: false,
            last_session: None,
            last_headline: None,
            last_status: None,
            last_started: None,
            open_actions: 0,
            db_reachable: true,
        });
        assert!(off.contains("OFF"));
        assert!(off.contains("council_status"));
        assert!(off.contains("council_report"));
        assert!(off.contains("council_convene refuses"));
        assert!(off.contains("Do not convene just to check status"));
        assert!(off.contains("Anthropic / haiku"));

        let on = render_status(&StatusView {
            enabled: true,
            seated: vec!["OpenAI / gpt-4.1 (openai)".into()],
            running: false,
            last_session: Some("sess-1".into()),
            last_headline: Some("Ship the card".into()),
            last_status: Some("complete".into()),
            last_started: Some("2026-08-23".into()),
            open_actions: 2,
            db_reachable: true,
        });
        assert!(on.contains("The Council is ON"));
        assert!(on.contains("Ship the card"));
        assert!(on.contains("sess-1"));
        assert!(on.contains("Open Decision Inbox actions: 2"));
        assert!(!on.contains("convene refuses"));
    }

    #[test]
    fn teaching_walks_enable_live_query_and_dashboard_not_home() {
        let steps = SELF_KNOWLEDGE_FEATURE.teaching;
        assert!(steps.len() >= 4);
        let tabs: Vec<&str> = steps
            .iter()
            .filter_map(|s| s.open_surface.map(|t| t.tab))
            .collect();
        assert!(tabs.contains(&"Settings"));
        assert!(tabs.contains(&"Dashboard"));
        assert!(
            !tabs.contains(&"Home"),
            "Home is not a catalog tab — Dashboard is"
        );
        let lesson = steps
            .iter()
            .map(|s| format!("{} {}", s.title, s.body))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(lesson.contains("council_status"));
        assert!(lesson.contains("council_report"));
        assert!(lesson.contains("council_convene"));
        assert!(lesson.contains("council_enabled"));
        assert!(lesson.contains("Settings → Features"));
    }
}
