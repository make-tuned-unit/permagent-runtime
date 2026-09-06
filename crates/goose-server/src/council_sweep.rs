//! The Council's weekly pass.
//!
//! Hourly tick; a session is due Sunday 22:00 local onward (Monday catch-up if
//! the machine slept). No-ops unless `council_enabled` is on. Re-reads the flag
//! every tick so a Settings flip needs no restart.

use crate::state::AppState;
use chrono::{DateTime, Local};
use permagent::config::GooseMode;
use permagent::council::{self, debate, due, store};
use permagent::session::{SessionManager, SessionType};
use std::sync::Arc;
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(600);
const TICK: Duration = Duration::from_secs(3600);

pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            run_once(&state).await;
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn run_once(state: &AppState) {
    if !council::is_enabled() {
        tracing::debug!(target: "permagentd::council", "weekly pass idle (council_enabled=false)");
        return;
    }
    let pool = match state.session_manager().pool_clone().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::debug!(target: "permagentd::council", "weekly pass skipped: {e}");
            return;
        }
    };
    let last = match store::last_success_started_at(&pool).await {
        Ok(ts) => ts
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Local)),
        Err(e) => {
            tracing::warn!(target: "permagentd::council", "could not read last session: {e}");
            return;
        }
    };
    if !due::should_run(Local::now(), last) {
        return;
    }
    tracing::info!(target: "permagentd::council", "convening weekly council");
    // Weekly Council work still needs a durable harness task identity. Create
    // a scheduled session in the existing Spectral session store; this is the
    // attribution parent, while the Council store remains the report store.
    let manager = Arc::new(SessionManager::instance());
    let session = match manager
        .create_session(
            std::env::current_dir().unwrap_or_default(),
            "Council weekly".to_string(),
            SessionType::Scheduled,
            GooseMode::Auto,
        )
        .await
    {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!(target: "permagentd::council", "weekly Council skipped; could not create attribution session: {e}");
            return;
        }
    };
    if let Err(e) = manager.begin_budget_task(&session.id).await {
        tracing::warn!(target: "permagentd::council", "weekly Council skipped; could not create budget task: {e}");
        return;
    }
    let caller = debate::LiveCaller::new(manager, session.id);
    match council::convene(&pool, store::Trigger::Weekly, None, &caller).await {
        Ok(c) => tracing::info!(
            target: "permagentd::council",
            session = %c.session_id,
            status = ?c.status,
            members = c.n_members,
            ok = c.n_ok,
            actions = c.n_actions,
            "weekly council finished"
        ),
        Err(e) => tracing::warn!(target: "permagentd::council", "weekly council failed: {e}"),
    }
}
