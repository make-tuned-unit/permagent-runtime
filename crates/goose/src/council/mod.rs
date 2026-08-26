//! The Council — a weekly multi-provider debate about the state of the work.
//!
//! Identity and the debate loop live here. The Sunday sweep lives in the
//! daemon (`council_sweep`). Henry convenes on demand via the `deliberate`
//! platform extension. Default OFF: it spends every connected chat provider.

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
         Henry convenes it with council_convene; a Sunday-night sweep runs the same session \
         when council_enabled is on. Actions land as Decision Inbox proposals (council_action): \
         approve files a board card, reject dismisses. It only ever proposes",
    why_it_matters:
        "One model is a take. Several models, looking at the same brief and then at each other, \
         surface the project that needs attention, the pattern you are missing, and the \
         analytics that actually moved — as a digest you can act on, not a second inbox",
    state_source: StateSource::Queryable,
    teaching: &[
        TeachingStep {
            title: "He briefs; he does not impersonate",
            body: "The Council briefs every connected chat-completion model on the same \
                   portfolio snapshot. You convene and chair; you do not pretend to be \
                   those other models.",
            open_surface: Some(SurfaceRef {
                tab: "Settings",
                section: Some("features"),
            }),
            confirm: None,
        },
        TeachingStep {
            title: "Weekly on Sunday night",
            body: "When council_enabled is on, a Sunday 22:00 local sweep runs the same \
                   session. You can also council_convene on demand. Actions land as \
                   Decision Inbox proposals — approve files a board card, reject dismisses.",
            open_surface: Some(SurfaceRef {
                tab: "Home",
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
            &session_id,
            1,
            &r.member.provider,
            &r.member.model,
            &r.status,
            r.raw.as_deref(),
            r.parsed.as_ref(),
            r.error.as_deref(),
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
            &session_id,
            2,
            &r.member.provider,
            &r.member.model,
            &r.status,
            r.raw.as_deref(),
            r.parsed.as_ref(),
            r.error.as_deref(),
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

    let _ = store::insert_report(
        pool,
        &session_id,
        &report.headline,
        &report.markdown,
        &report.consensus,
        &report.dissent,
        &report
            .actions
            .iter()
            .map(|a| serde_json::to_value(a).unwrap_or(serde_json::Value::Null))
            .collect::<Vec<_>>(),
        Some(&chair_provider),
        Some(&chair_model),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        assert!(!is_enabled());
    }
}
