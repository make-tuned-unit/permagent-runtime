//! The Council's weekly pass.
//!
//! Hourly tick; a session is due Sunday 22:00 local onward (Monday catch-up if
//! the machine slept). No-ops unless `council_enabled` is on. Re-reads the flag
//! every tick so a Settings flip needs no restart.

use crate::state::AppState;
use chrono::{DateTime, Local};
use permagent::council::{self, debate, due, store};
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
    match council::convene(&pool, store::Trigger::Weekly, None, &debate::LiveCaller).await {
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
