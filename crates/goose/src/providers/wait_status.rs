//! What the agent is waiting on, published so a UI can say so.
//!
//! Before this, a provider rate limit was silent: the retry loop slept inside
//! `with_retry_config` and the TUI showed nothing at all. A user watching a
//! GLM-5.3 session on 2026-08-25 saw four minutes of blank terminal across a
//! 429 and a long reasoning stream, and reasonably read it as a hang.
//!
//! This is a single process-wide slot, not a queue: there is one thing the user
//! is waiting on at a time, and a stale entry is worse than none. Publishers
//! must clear what they set.

use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Why the agent is not talking to the model right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    /// The provider returned 429 and we are honouring a backoff.
    RateLimited,
    /// The provider returned a 5xx and we are retrying.
    ServerError,
    /// Anything else the retry layer decided to sit out.
    Transient,
}

impl WaitReason {
    /// Words for a status line — lowercase, no trailing punctuation.
    pub fn as_str(&self) -> &'static str {
        match self {
            WaitReason::RateLimited => "rate limit",
            WaitReason::ServerError => "server error",
            WaitReason::Transient => "connection problem",
        }
    }
}

/// One in-progress wait.
#[derive(Debug, Clone)]
pub struct ProviderWait {
    /// Provider id, e.g. `zai`.
    pub provider: String,
    pub reason: WaitReason,
    /// When the wait started and how long it is meant to last. Kept as a start
    /// plus a duration rather than a deadline so a renderer can show both
    /// "retrying in N s" and how long it has already waited.
    pub started: Instant,
    pub delay: Duration,
    pub attempt: usize,
    pub max_attempts: usize,
}

impl ProviderWait {
    /// Whole seconds left, saturating at zero.
    pub fn seconds_remaining(&self) -> u64 {
        self.delay.saturating_sub(self.started.elapsed()).as_secs()
    }

    /// The one-line form a status area shows, e.g.
    /// `Z.AI rate limit — retrying in 12 s (attempt 2 of 4)`.
    pub fn status_line(&self) -> String {
        format!(
            "{} {} — retrying in {} s (attempt {} of {})",
            display_provider(&self.provider),
            self.reason.as_str(),
            self.seconds_remaining(),
            self.attempt,
            self.max_attempts,
        )
    }
}

/// Provider ids the user knows by a different name.
fn display_provider(id: &str) -> String {
    match id {
        "zai" => "Z.AI".to_string(),
        "openai" => "OpenAI".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "" => "The model provider".to_string(),
        other => other.to_string(),
    }
}

/// `Z.AI rate limit` — the headline a UI puts in front of a rate-limit
/// message, with the provider named the way the user knows it.
pub fn rate_limit_headline(provider_id: &str) -> String {
    format!("{} rate limit", display_provider(provider_id))
}

type Slot = (
    watch::Sender<Option<ProviderWait>>,
    watch::Receiver<Option<ProviderWait>>,
);

fn slot() -> &'static Slot {
    static SLOT: OnceLock<Slot> = OnceLock::new();
    SLOT.get_or_init(|| watch::channel(None))
}

/// Announce a wait. Overwrites any previous one.
pub fn publish(wait: ProviderWait) {
    let _ = slot().0.send(Some(wait));
}

/// Announce that nothing is being waited on.
pub fn clear() {
    let _ = slot().0.send(None);
}

/// The current wait, if any. Cheap enough to poll once per render frame.
pub fn current() -> Option<ProviderWait> {
    slot().1.borrow().clone()
}

/// Changes to the current wait, for a consumer that would rather be woken.
pub fn subscribe() -> watch::Receiver<Option<ProviderWait>> {
    slot().0.subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_of(delay_secs: u64) -> ProviderWait {
        ProviderWait {
            provider: "zai".to_string(),
            reason: WaitReason::RateLimited,
            started: Instant::now(),
            delay: Duration::from_secs(delay_secs),
            attempt: 2,
            max_attempts: 4,
        }
    }

    #[test]
    fn status_line_names_the_provider_the_user_configured() {
        let line = wait_of(12).status_line();
        assert!(line.starts_with("Z.AI rate limit — retrying in "), "{line}");
        assert!(line.ends_with("(attempt 2 of 4)"), "{line}");
    }

    #[test]
    fn an_elapsed_wait_reports_zero_rather_than_underflowing() {
        let mut wait = wait_of(1);
        wait.started = Instant::now() - Duration::from_secs(30);
        assert_eq!(wait.seconds_remaining(), 0);
    }

    #[test]
    fn publish_then_clear_round_trips() {
        publish(wait_of(5));
        assert!(current().is_some());
        clear();
        assert!(current().is_none());
    }

    #[test]
    fn the_rate_limit_headline_names_the_provider() {
        assert_eq!(rate_limit_headline("zai"), "Z.AI rate limit");
        assert_eq!(rate_limit_headline(""), "The model provider rate limit");
    }

    #[test]
    fn an_unknown_provider_id_is_shown_as_given() {
        assert_eq!(display_provider("openrouter"), "openrouter");
        assert_eq!(display_provider(""), "The model provider");
    }
}
