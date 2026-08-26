//! Process-wide per-provider concurrency limits for inference requests.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::base::MessageStream;
use super::wait_status;

const DEFAULT_MAX_INFLIGHT: usize = 4;

tokio::task_local! {
    static BACKGROUND_REQUEST: bool;
}

/// Run work at background priority. Background callers cannot consume the
/// permit reserved for an interactive caller.
pub async fn background<F: Future>(future: F) -> F::Output {
    BACKGROUND_REQUEST.scope(true, future).await
}

fn is_background() -> bool {
    BACKGROUND_REQUEST.try_with(|value| *value).unwrap_or(false)
}

struct ProviderSlots {
    all: Arc<Semaphore>,
    background: Arc<Semaphore>,
}

impl ProviderSlots {
    fn new(cap: usize) -> Self {
        Self {
            all: Arc::new(Semaphore::new(cap)),
            // One slot is exclusively available to foreground work.
            background: Arc::new(Semaphore::new(cap.saturating_sub(1))),
        }
    }

    async fn acquire(&self, provider: &str) -> InflightPermit {
        let background = is_background();
        let background_permit = if background {
            Some(acquire_announced(self.background.clone(), provider).await)
        } else {
            None
        };
        let permit = acquire_announced(self.all.clone(), provider).await;
        InflightPermit {
            _permit: permit,
            _background_permit: background_permit,
        }
    }
}

/// RAII guard that releases provider capacity when dropped.
///
/// [`stream`](super::base::Provider::stream) returns as soon as the HTTP
/// body is open, so the caller must keep this alive for the *stream's*
/// lifetime via [`hold_stream`] — otherwise the cap only covers request
/// setup, not the in-flight generation that actually trips 429s.
pub struct InflightPermit {
    _permit: OwnedSemaphorePermit,
    _background_permit: Option<OwnedSemaphorePermit>,
}

/// Pin `permit` to `stream` so the slot is held until the stream ends or
/// is dropped, not merely until the `stream()` future resolves.
pub fn hold_stream(stream: MessageStream, permit: InflightPermit) -> MessageStream {
    Box::pin(stream.map(move |item| {
        let _ = &permit;
        item
    }))
}

async fn acquire_announced(semaphore: Arc<Semaphore>, provider: &str) -> OwnedSemaphorePermit {
    if let Ok(permit) = semaphore.clone().try_acquire_owned() {
        return permit;
    }

    tracing::debug!(provider, "waiting for a free provider request slot");
    wait_status::publish(wait_status::ProviderWait {
        provider: provider.to_string(),
        reason: wait_status::WaitReason::ConcurrencyLimit,
        started: Instant::now(),
        delay: Duration::ZERO,
        attempt: 0,
        max_attempts: 0,
    });
    let permit = semaphore
        .acquire_owned()
        .await
        .expect("provider in-flight semaphore is never closed");
    wait_status::clear();
    permit
}

fn configured_cap_with(provider: &str, read: impl Fn(&str) -> Option<String>) -> usize {
    let provider_key: String = provider
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let override_key = format!("PERMAGENT_PROVIDER_MAX_INFLIGHT_{provider_key}");
    read(&override_key)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .or_else(|| {
            read("PERMAGENT_PROVIDER_MAX_INFLIGHT")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(DEFAULT_MAX_INFLIGHT)
}

fn registry() -> &'static Mutex<HashMap<String, Arc<ProviderSlots>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<ProviderSlots>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn slots_for(
    registry: &mut HashMap<String, Arc<ProviderSlots>>,
    provider: &str,
    cap: impl FnOnce() -> usize,
) -> Arc<ProviderSlots> {
    registry
        .entry(provider.to_string())
        .or_insert_with(|| Arc::new(ProviderSlots::new(cap())))
        .clone()
}

/// Acquire this process's request slot for `provider`.
pub async fn acquire(provider: &str) -> InflightPermit {
    let slots = {
        let mut registry = registry().lock().expect("provider slot registry poisoned");
        slots_for(&mut registry, provider, || {
            configured_cap_with(provider, |key| std::env::var(key).ok())
        })
    };
    slots.acquire(provider).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{mpsc, Notify};

    struct FakeProvider {
        slots: Arc<ProviderSlots>,
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl FakeProvider {
        async fn request(&self, id: &str) {
            let _permit = self.slots.acquire(id).await;
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(10)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cap_two_never_has_more_than_two_fake_requests_in_flight() {
        let slots = Arc::new(ProviderSlots::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(FakeProvider {
            slots,
            active,
            peak: peak.clone(),
        });
        let calls = (0..20).map(|_| {
            let provider = provider.clone();
            tokio::spawn(async move { provider.request("fake").await })
        });
        for call in calls {
            call.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn provider_ids_have_independent_pools() {
        let mut registry = HashMap::new();
        let first = slots_for(&mut registry, "first", || 1);
        let second = slots_for(&mut registry, "second", || 1);
        let held = first.all.clone().acquire_owned().await.unwrap();
        let other =
            tokio::time::timeout(Duration::from_millis(100), second.acquire("other-provider"))
                .await;
        assert!(
            other.is_ok(),
            "another provider must not share the saturated pool"
        );
        drop(held);
    }

    #[tokio::test]
    async fn foreground_uses_the_reserved_permit_before_waiting_background() {
        let slots = Arc::new(ProviderSlots::new(2));
        let release = Arc::new(Notify::new());
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();

        let first_slots = slots.clone();
        let first_release = release.clone();
        let first_tx = entered_tx.clone();
        let first = tokio::spawn(background(async move {
            let _permit = first_slots.acquire("fake").await;
            first_tx.send("background-1").unwrap();
            first_release.notified().await;
        }));
        assert_eq!(entered_rx.recv().await, Some("background-1"));

        let second_slots = slots.clone();
        let second_tx = entered_tx.clone();
        let second = tokio::spawn(background(async move {
            let _permit = second_slots.acquire("fake").await;
            second_tx.send("background-2").unwrap();
        }));
        tokio::task::yield_now().await;

        let foreground_slots = slots.clone();
        let foreground = tokio::spawn(async move {
            let _permit = foreground_slots.acquire("fake").await;
            entered_tx.send("foreground").unwrap();
        });
        assert_eq!(entered_rx.recv().await, Some("foreground"));
        release.notify_one();
        assert_eq!(entered_rx.recv().await, Some("background-2"));
        first.await.unwrap();
        second.await.unwrap();
        foreground.await.unwrap();
    }

    #[test]
    fn environment_default_override_and_invalid_fallback_are_parsed() {
        let values = HashMap::from([
            (
                "PERMAGENT_PROVIDER_MAX_INFLIGHT".to_string(),
                "3".to_string(),
            ),
            (
                "PERMAGENT_PROVIDER_MAX_INFLIGHT_ZAI".to_string(),
                "2".to_string(),
            ),
        ]);
        assert_eq!(
            configured_cap_with("openai", |key| values.get(key).cloned()),
            3
        );
        assert_eq!(
            configured_cap_with("zai", |key| values.get(key).cloned()),
            2
        );
        assert_eq!(configured_cap_with("missing", |_| None), 4);
        assert_eq!(
            configured_cap_with("invalid", |key| {
                (key == "PERMAGENT_PROVIDER_MAX_INFLIGHT").then(|| "nope".to_string())
            }),
            4
        );
    }
}
