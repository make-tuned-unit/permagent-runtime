use crate::routes::reply::MessageEvent;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

const BROADCAST_CAPACITY: usize = 256;
const REPLAY_BUFFER_CAPACITY: usize = 512;

/// Error returned by [`SessionEventBus::subscribe`].
#[derive(Debug)]
pub enum SubscribeError {
    /// The client's `Last-Event-ID` has been evicted from the replay buffer,
    /// so events have been irrecoverably lost.
    ClientTooFarBehind,
}

#[derive(Clone, Debug)]
pub struct SessionEvent {
    /// Monotonic sequence number, written as SSE `id:` frame (not in JSON payload).
    pub seq: u64,
    /// None for Ping events, Some for events associated with a specific request.
    pub request_id: Option<String>,
    /// The event payload.
    pub event: MessageEvent,
}

pub struct SessionEventBus {
    tx: broadcast::Sender<SessionEvent>,
    buffer: Mutex<VecDeque<SessionEvent>>,
    next_seq: AtomicU64,
    active_requests: Mutex<HashMap<String, CancellationToken>>,
    /// Name of the tool currently in flight for this session's active request,
    /// or `None` when no tool is executing (model still generating, or idle).
    /// Read by `/api/henry/status` (#84) so the HUD can show "working: <tool>".
    current_tool: Mutex<Option<String>>,
}

impl SessionEventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            buffer: Mutex::new(VecDeque::with_capacity(REPLAY_BUFFER_CAPACITY)),
            next_seq: AtomicU64::new(1),
            active_requests: Mutex::new(HashMap::new()),
            current_tool: Mutex::new(None),
        }
    }

    /// Publish an event to the bus. Assigns a monotonic sequence number.
    ///
    /// The sequence ID is assigned under the buffer lock so that concurrent
    /// callers cannot reorder events (i.e. seq=2 published before seq=1).
    pub async fn publish(&self, request_id: Option<String>, event: MessageEvent) -> u64 {
        let session_event = {
            let mut buf = self.buffer.lock().await;
            let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
            let session_event = SessionEvent {
                seq,
                request_id,
                event,
            };
            buf.push_back(session_event.clone());
            while buf.len() > REPLAY_BUFFER_CAPACITY {
                buf.pop_front();
            }
            session_event
        };

        // Send on broadcast channel (ignore error if no subscribers)
        let _ = self.tx.send(session_event.clone());

        session_event.seq
    }

    /// Subscribe to live events. If `last_event_id` is provided, replay buffered
    /// events with seq > last_event_id. Returns (replay_events, replay_max_seq, live_receiver).
    ///
    /// Returns `Err(SubscribeError::ClientTooFarBehind)` when `last_event_id`
    /// refers to an event that has already been evicted from the replay buffer,
    /// meaning the client has irrecoverably missed events.
    ///
    /// The live receiver is created *before* snapshotting the buffer so that
    /// no event can fall into the gap between the two steps. The caller must
    /// skip live events with `seq <= replay_max_seq` to deduplicate.
    pub async fn subscribe(
        &self,
        last_event_id: Option<u64>,
    ) -> Result<(Vec<SessionEvent>, u64, broadcast::Receiver<SessionEvent>), SubscribeError> {
        // Subscribe first so that any event published while we hold the
        // buffer lock is guaranteed to appear in `rx` (possibly duplicating
        // a replay entry). The caller deduplicates via replay_max_seq.
        let rx = self.tx.subscribe();

        let (replay, replay_max_seq) = {
            let buf = self.buffer.lock().await;
            let buf_max = buf.back().map(|e| e.seq).unwrap_or(0);
            let buf_min = buf.front().map(|e| e.seq).unwrap_or(0);
            let last_id = last_event_id.unwrap_or(0);

            // If the client sent a Last-Event-ID that has been evicted from
            // the buffer, they have irrecoverably missed events.
            if last_id > 0 && buf_min > 0 && last_id < buf_min {
                return Err(SubscribeError::ClientTooFarBehind);
            }

            // Clamp to the actual buffer max so a stale Last-Event-ID
            // (e.g. from before a server restart) doesn't suppress live events.
            let events: Vec<_> = buf.iter().filter(|e| e.seq > last_id).cloned().collect();
            let max_seq = events.last().map(|e| e.seq).unwrap_or(last_id.min(buf_max));
            (events, max_seq)
        };

        Ok((replay, replay_max_seq, rx))
    }

    /// Return the IDs of all currently active (in-flight) requests.
    pub async fn active_request_ids(&self) -> Vec<String> {
        let requests = self.active_requests.lock().await;
        requests.keys().cloned().collect()
    }

    /// Record the name of the tool now executing for this session (#84). Set
    /// when a `ToolRequest` is observed in the reply stream; cleared on the
    /// matching `ToolResponse` and on request cleanup/cancel.
    pub async fn set_current_tool(&self, name: Option<String>) {
        *self.current_tool.lock().await = name;
    }

    /// Return the name of the tool currently in flight, if any.
    pub async fn current_tool(&self) -> Option<String> {
        self.current_tool.lock().await.clone()
    }

    #[cfg(test)]
    pub async fn register_request(&self, request_id: String) -> CancellationToken {
        let token = CancellationToken::new();
        let mut requests = self.active_requests.lock().await;
        requests.insert(request_id, token.clone());
        token
    }

    /// Atomically check no requests are active and register one. Returns Err if busy.
    pub async fn try_register_request(
        &self,
        request_id: String,
    ) -> Result<CancellationToken, String> {
        let mut requests = self.active_requests.lock().await;
        requests.retain(|_id, token| !token.is_cancelled());
        if !requests.is_empty() {
            return Err("Session already has an active request".into());
        }
        let token = CancellationToken::new();
        requests.insert(request_id, token.clone());
        Ok(token)
    }

    /// Cancel a specific request by request_id.
    pub async fn cancel_request(&self, request_id: &str) -> bool {
        let requests = self.active_requests.lock().await;
        if let Some(token) = requests.get(request_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Cancel all active requests (e.g. when deleting a session).
    pub async fn cancel_all_requests(&self) {
        let requests = self.active_requests.lock().await;
        for token in requests.values() {
            token.cancel();
        }
        drop(requests);
        self.set_current_tool(None).await;
    }

    /// Remove the cancellation token for a completed request.
    pub async fn cleanup_request(&self, request_id: &str) {
        let mut requests = self.active_requests.lock().await;
        requests.remove(request_id);
        drop(requests);
        // The request is done — never leave a stale in-flight tool name behind.
        self.set_current_tool(None).await;
    }
}

impl Default for SessionEventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Frees the session's single request slot if the reply task dies without
/// reaching its normal terminal path, and — because subscribers otherwise have
/// no way to learn the turn is dead — publishes a terminal `Error` frame so the
/// chat UI settles instead of showing "Agent is responding…" forever (C1).
///
/// Contract: every reply-task exit that publishes its own terminal frame
/// (`Finish`, or an explicit `Error` on an early return) must call `disarm()`
/// (plus `cleanup_request`) before returning; an ARMED drop therefore always
/// means "died un-finished" (panic or a missed exit path), and the drop frame
/// is never a duplicate of a real terminal.
pub struct RequestGuard {
    bus: std::sync::Arc<SessionEventBus>,
    request_id: String,
    disarmed: bool,
}

impl RequestGuard {
    pub fn new(bus: std::sync::Arc<SessionEventBus>, request_id: String) -> Self {
        Self {
            bus,
            request_id,
            disarmed: false,
        }
    }

    pub fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let bus = self.bus.clone();
            let request_id = self.request_id.clone();
            // `try_current` (not a bare `tokio::spawn`) keeps this Drop
            // panic-free even when it runs off-runtime (e.g. runtime teardown)
            // — a panicking Drop during an unwind would abort the process.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    // Free the slot BEFORE announcing the death, so a client
                    // that reacts to the Error by re-sending never races a
                    // stale slot into a 400 "already has an active request".
                    bus.cleanup_request(&request_id).await;
                    bus.publish(
                        Some(request_id),
                        MessageEvent::Error {
                            error: "The agent stopped unexpectedly before finishing this reply."
                                .to_string(),
                        },
                    )
                    .await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::conversation::message::TokenState;

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let bus = SessionEventBus::new();

        // Publish some events
        bus.publish(Some("req-1".to_string()), MessageEvent::Ping)
            .await;
        bus.publish(
            Some("req-1".to_string()),
            MessageEvent::Finish {
                reason: "stop".to_string(),
                token_state: TokenState::default(),
            },
        )
        .await;

        // Subscribe with replay
        let (replay, replay_max_seq, _rx) = bus.subscribe(Some(0)).await.unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].seq, 1);
        assert_eq!(replay[1].seq, 2);
        assert_eq!(replay_max_seq, 2);
    }

    #[tokio::test]
    async fn test_subscribe_with_last_event_id() {
        let bus = SessionEventBus::new();

        bus.publish(None, MessageEvent::Ping).await;
        bus.publish(None, MessageEvent::Ping).await;
        bus.publish(None, MessageEvent::Ping).await;

        // Only get events after seq 2
        let (replay, replay_max_seq, _rx) = bus.subscribe(Some(2)).await.unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 3);
        assert_eq!(replay_max_seq, 3);
    }

    #[tokio::test]
    async fn test_subscribe_without_last_event_id_replays_all() {
        let bus = SessionEventBus::new();

        bus.publish(None, MessageEvent::Ping).await;
        bus.publish(None, MessageEvent::Ping).await;

        // First connect (no Last-Event-ID) should replay all buffered events
        let (replay, replay_max_seq, _rx) = bus.subscribe(None).await.unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay_max_seq, 2);
    }

    #[tokio::test]
    async fn test_subscribe_with_stale_last_event_id() {
        let bus = SessionEventBus::new();

        // Buffer has seq 1..3, but client sends Last-Event-ID: 9999
        bus.publish(None, MessageEvent::Ping).await;
        bus.publish(None, MessageEvent::Ping).await;
        bus.publish(None, MessageEvent::Ping).await;

        let (replay, replay_max_seq, _rx) = bus.subscribe(Some(9999)).await.unwrap();
        // No replay events (all are below 9999)
        assert_eq!(replay.len(), 0);
        // replay_max_seq should be clamped to buf_max (3), not 9999
        assert_eq!(replay_max_seq, 3);
    }

    #[tokio::test]
    async fn test_cancel_request() {
        let bus = SessionEventBus::new();

        let token = bus.register_request("req-1".to_string()).await;
        assert!(!token.is_cancelled());

        let cancelled = bus.cancel_request("req-1").await;
        assert!(cancelled);
        assert!(token.is_cancelled());

        // Non-existent request
        let cancelled = bus.cancel_request("req-999").await;
        assert!(!cancelled);
    }

    #[tokio::test]
    async fn test_cleanup_request() {
        let bus = SessionEventBus::new();

        bus.register_request("req-1".to_string()).await;
        bus.cleanup_request("req-1").await;

        // Should return false since it was cleaned up
        let cancelled = bus.cancel_request("req-1").await;
        assert!(!cancelled);
    }

    #[tokio::test]
    async fn test_request_guard_armed_drop_publishes_terminal_error_and_frees_slot() {
        let bus = std::sync::Arc::new(SessionEventBus::new());
        bus.register_request("req-dead".to_string()).await;

        // Dropped WITHOUT disarm — simulates a reply task that died (panic or
        // missed exit path) before publishing its terminal frame.
        drop(RequestGuard::new(bus.clone(), "req-dead".to_string()));

        // The drop handler runs on a spawned task; poll until it lands.
        let mut slot_freed = false;
        let mut error_published = false;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            slot_freed = bus.active_request_ids().await.is_empty();
            let (replay, _, _rx) = bus.subscribe(None).await.unwrap();
            error_published = replay.iter().any(|e| {
                e.request_id.as_deref() == Some("req-dead")
                    && matches!(&e.event, MessageEvent::Error { .. })
            });
            if slot_freed && error_published {
                break;
            }
        }
        assert!(slot_freed, "armed drop must free the request slot");
        assert!(
            error_published,
            "armed drop must publish a terminal Error frame so the UI settles"
        );
    }

    #[tokio::test]
    async fn test_request_guard_disarmed_drop_publishes_nothing() {
        let bus = std::sync::Arc::new(SessionEventBus::new());
        let mut guard = RequestGuard::new(bus.clone(), "req-ok".to_string());
        guard.disarm();
        drop(guard);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (replay, _, _rx) = bus.subscribe(None).await.unwrap();
        assert!(
            replay.is_empty(),
            "disarmed drop must not publish a spurious Error after a real terminal"
        );
    }

    #[tokio::test]
    async fn test_current_tool_set_clear_and_cleanup() {
        let bus = SessionEventBus::new();

        // Starts with no tool in flight.
        assert_eq!(bus.current_tool().await, None);

        // Set on tool start, cleared on tool complete (#84).
        bus.set_current_tool(Some("read_file".to_string())).await;
        assert_eq!(bus.current_tool().await, Some("read_file".to_string()));
        bus.set_current_tool(None).await;
        assert_eq!(bus.current_tool().await, None);

        // cleanup_request must never leave a stale tool name behind.
        bus.register_request("req-1".to_string()).await;
        bus.set_current_tool(Some("search_memory".to_string()))
            .await;
        bus.cleanup_request("req-1").await;
        assert_eq!(bus.current_tool().await, None);
    }
}
