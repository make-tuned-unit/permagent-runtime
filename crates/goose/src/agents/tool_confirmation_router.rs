use std::collections::HashMap;

use tokio::sync::{oneshot, Mutex};
use tracing::warn;

use crate::permission::PermissionConfirmation;

pub struct ToolConfirmationRouter {
    pending: Mutex<HashMap<String, oneshot::Sender<PermissionConfirmation>>>,
}

impl ToolConfirmationRouter {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub async fn register(
        &self,
        request_id: String,
    ) -> anyhow::Result<oneshot::Receiver<PermissionConfirmation>> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().await;
        pending.retain(|_, sender| !sender.is_closed());
        if pending.contains_key(&request_id) {
            anyhow::bail!("Confirmation waiter already registered for request {request_id}");
        }
        pending.insert(request_id, tx);
        Ok(rx)
    }

    /// True while at least one turn is parked on a live confirmation waiter.
    ///
    /// Used as a busy signal (e.g. by the AgentManager LRU eviction guard):
    /// evicting an agent whose router holds a live waiter would orphan the
    /// parked turn — the eventual Decision-Inbox answer would be delivered to
    /// a freshly recreated agent with no waiter. Prunes closed senders first
    /// so an aborted turn can't pin its session as busy forever.
    pub async fn has_live_waiter(&self) -> bool {
        let mut pending = self.pending.lock().await;
        pending.retain(|_, sender| !sender.is_closed());
        !pending.is_empty()
    }

    pub async fn deliver(&self, request_id: String, confirmation: PermissionConfirmation) -> bool {
        if let Some(tx) = self.pending.lock().await.remove(&request_id) {
            if tx.send(confirmation).is_err() {
                warn!(
                    request_id = %request_id,
                    "Confirmation receiver was dropped (task cancelled)"
                );
                false
            } else {
                true
            }
        } else {
            warn!(
                request_id = %request_id,
                "No task waiting for confirmation"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::permission_confirmation::PrincipalType;
    use crate::permission::Permission;

    fn test_confirmation() -> PermissionConfirmation {
        PermissionConfirmation {
            principal_type: PrincipalType::Tool,
            permission: Permission::AllowOnce,
        }
    }

    #[tokio::test]
    async fn test_register_then_deliver() {
        let router = ToolConfirmationRouter::new();
        let rx = router.register("req_1".to_string()).await.unwrap();
        assert!(
            router
                .deliver("req_1".to_string(), test_confirmation())
                .await
        );
        let confirmation = rx.await.unwrap();
        assert_eq!(confirmation.permission, Permission::AllowOnce);
    }

    #[tokio::test]
    async fn test_deliver_unknown_request() {
        let router = ToolConfirmationRouter::new();
        assert!(
            !router
                .deliver("unknown".to_string(), test_confirmation())
                .await
        );
    }

    #[tokio::test]
    async fn test_cancelled_receiver() {
        let router = ToolConfirmationRouter::new();
        let rx = router.register("req_1".to_string()).await.unwrap();
        drop(rx); // simulate task cancellation
        assert!(
            !router
                .deliver("req_1".to_string(), test_confirmation())
                .await
        );
    }

    #[tokio::test]
    async fn test_stale_entries_pruned_on_register() {
        let router = ToolConfirmationRouter::new();
        let rx = router.register("req_1".to_string()).await.unwrap();
        drop(rx); // simulate task cancellation — entry is now stale

        assert_eq!(router.pending.lock().await.len(), 1);

        let _rx2 = router.register("req_2".to_string()).await.unwrap();
        assert_eq!(router.pending.lock().await.len(), 1); // only req_2 remains
        assert!(router.pending.lock().await.contains_key("req_2"));
    }

    #[tokio::test]
    async fn test_has_live_waiter_reflects_registration_and_delivery() {
        let router = ToolConfirmationRouter::new();
        assert!(!router.has_live_waiter().await, "empty router is not busy");

        let rx = router.register("req_1".to_string()).await.unwrap();
        assert!(router.has_live_waiter().await, "registered waiter is live");

        assert!(
            router
                .deliver("req_1".to_string(), test_confirmation())
                .await
        );
        assert!(
            !router.has_live_waiter().await,
            "delivered waiter no longer counts as busy"
        );
        drop(rx);
    }

    #[tokio::test]
    async fn test_has_live_waiter_prunes_closed_senders() {
        let router = ToolConfirmationRouter::new();
        let rx = router.register("req_1".to_string()).await.unwrap();
        drop(rx); // turn aborted — the waiter is dead

        assert!(
            !router.has_live_waiter().await,
            "a dropped receiver must not pin the router as busy"
        );
        assert_eq!(
            router.pending.lock().await.len(),
            0,
            "stale entry is pruned by the busy probe"
        );
    }

    #[tokio::test]
    async fn test_concurrent_requests_out_of_order() {
        use std::sync::Arc;

        let router = Arc::new(ToolConfirmationRouter::new());

        // Register two requests
        let rx1 = router.register("req_1".to_string()).await.unwrap();
        let rx2 = router.register("req_2".to_string()).await.unwrap();

        // Deliver in reverse order
        assert!(
            router
                .deliver(
                    "req_2".to_string(),
                    PermissionConfirmation {
                        principal_type: PrincipalType::Tool,
                        permission: Permission::DenyOnce,
                    }
                )
                .await
        );
        assert_eq!(router.pending.lock().await.len(), 1);
        assert!(
            router
                .deliver("req_1".to_string(), test_confirmation())
                .await
        );
        assert_eq!(router.pending.lock().await.len(), 0);

        let c1 = rx1.await.unwrap();
        assert_eq!(c1.permission, Permission::AllowOnce);
        let c2 = rx2.await.unwrap();
        assert_eq!(c2.permission, Permission::DenyOnce);
    }

    #[tokio::test]
    async fn test_duplicate_live_request_is_rejected_without_dropping_first() {
        let router = ToolConfirmationRouter::new();
        let first_rx = router.register("req_1".to_string()).await.unwrap();

        let duplicate = router.register("req_1".to_string()).await;
        assert!(duplicate.is_err(), "duplicate live waiter must be rejected");

        assert!(
            router
                .deliver("req_1".to_string(), test_confirmation())
                .await,
            "the original waiter must remain deliverable"
        );
        assert_eq!(
            first_rx.await.unwrap().permission,
            Permission::AllowOnce,
            "delivery must reach the original waiter"
        );
    }
}
