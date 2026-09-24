//! The Council's weekly pass.
//!
//! Hourly tick; a session is due Sunday 22:00 local onward (Monday catch-up if
//! the machine slept). No-ops unless `council_enabled` is on. Re-reads the flag
//! every tick so a Settings flip needs no restart.

use crate::state::AppState;
use chrono::{DateTime, Local};
use permagent::council::preparation::{CODE_APPROVAL_REQUIRED, CODE_REFUSED};
use permagent::council::{self, debate, due, store};
use permagent::session::SessionManager;
use std::sync::{Arc, Mutex};
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
    // The occurrence reuses its attribution parent and Council-owned child;
    // only Council preparation begins the child's budget task, once.
    let manager = Arc::new(SessionManager::instance());
    let session_id = match council::preparation::weekly_parent(&manager).await {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(target: "permagentd::council", "weekly Council skipped; could not load attribution session: {e}");
            return;
        }
    };
    let caller = debate::LiveCaller::new(manager, session_id);
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
        Err(e) => match classify(&e, &LAST_OUTCOME) {
            Outcome::Repeat => {
                tracing::debug!(target: "permagentd::council", "weekly council still blocked: {e}")
            }
            Outcome::Waiting => tracing::info!(
                target: "permagentd::council",
                "weekly council is waiting on its Decision Inbox card: {e}"
            ),
            Outcome::Refused => {
                tracing::info!(target: "permagentd::council", "weekly council not run: {e}")
            }
            Outcome::Failed => {
                tracing::warn!(target: "permagentd::council", "weekly council failed: {e}")
            }
        },
    }
}

/// The last blocked outcome this process logged. The sweep retries hourly for
/// the whole week, so an unchanged block — a card still unanswered, an
/// occurrence already refused — would otherwise write the same line ~150 times.
static LAST_OUTCOME: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Identical to the last logged block: nothing new to say.
    Repeat,
    /// Budget admission raised (or found) a Decision Inbox card; the expected
    /// state until the user answers it, not a failure.
    Waiting,
    /// A deliberate refusal: cancelled, already run, hard limit.
    Refused,
    /// Anything else, including an approval the admission check rejected.
    Failed,
}

fn classify(error: &str, last: &Mutex<Option<String>>) -> Outcome {
    let mut last = last.lock().unwrap_or_else(|p| p.into_inner());
    if last.as_deref() == Some(error) {
        return Outcome::Repeat;
    }
    *last = Some(error.to_string());
    let code = error.split(':').next().unwrap_or_default();
    if code == CODE_APPROVAL_REQUIRED {
        Outcome::Waiting
    } else if code == CODE_REFUSED {
        Outcome::Refused
    } else {
        Outcome::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::council::preparation::CODE_UNKNOWN;

    #[test]
    fn an_unanswered_card_logs_once_as_waiting_then_goes_quiet() {
        let last = Mutex::new(None);
        let waiting =
            format!("{CODE_APPROVAL_REQUIRED}: approve the bounded convene (Decision Inbox: d1)");
        assert_eq!(classify(&waiting, &last), Outcome::Waiting);
        assert_eq!(classify(&waiting, &last), Outcome::Repeat);
        assert_eq!(classify(&waiting, &last), Outcome::Repeat);
        let refused = format!("{CODE_REFUSED}: already admitted or cancelled");
        assert_eq!(classify(&refused, &last), Outcome::Refused);
        assert_eq!(classify(&refused, &last), Outcome::Repeat);
    }

    /// A rejected approval or a storage fault is a real failure and says so,
    /// once, and again whenever it changes.
    #[test]
    fn failures_warn_and_a_changed_failure_warns_again() {
        let last = Mutex::new(None);
        let a = format!("{CODE_UNKNOWN}: unauthorized: approval actor may not fund this plan");
        let b = format!("{CODE_UNKNOWN}: database is locked");
        assert_eq!(classify(&a, &last), Outcome::Failed);
        assert_eq!(classify(&a, &last), Outcome::Repeat);
        assert_eq!(classify(&b, &last), Outcome::Failed);
        assert_eq!(classify("council_enabled is off", &last), Outcome::Failed);
    }
}
