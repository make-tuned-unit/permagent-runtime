//! Agent-to-agent messaging between goal workers (Prime A2A).
//!
//! Payload shape: `{ from_goal, to_goal, body, ts }`. Messages are addressed by
//! GOAL ID: the target goal is resolved to its card, its live state, and — when
//! one exists — the live worker session steering it. The message is written to
//! the target's `a2a_inbox` metadata, the RLM control plane, and that worker.
//!
//! ## Refusals say which kind of "no" this is
//!
//! Delivery requires an InProgress target, but "no" is not one answer. A goal
//! that is Complete or Cancelled is [`A2aRefusal::Terminal`]: no retry will ever
//! work, and a sender that keeps trying is burning turns on a goal that has
//! stopped existing for this purpose. A goal that is Triage / Ready / Review /
//! Failed is [`A2aRefusal::NotRunning`]: the same message may well deliver in a
//! minute. Both used to come back as one undifferentiated string.
//!
//! ## Every delivery is audited
//!
//! A delivered message emits a `a2a_message` event, which the durable activity
//! journal records: who sent it, who received it, how long it was, and the
//! SHA-256 of the body — and NOT the body. One agent's instructions to another
//! is exactly the content an audit trail must prove the existence of without
//! republishing: the hash settles "was this the message" for anyone holding the
//! original, and the length settles "was something substantive sent", while the
//! text itself stays in the recipient's own inbox where it was addressed.

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};

use crate::cards;
use crate::events;
use crate::goal_state::GoalState;
use crate::rlm;

pub const A2A_INBOX_KEY: &str = "a2a_inbox";
pub const A2A_SENT_KEY: &str = "a2a_sent";

/// Why a message was not delivered. Typed, because the caller's next move
/// differs: a terminal target is never worth retrying and a not-yet-running one
/// usually is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2aRefusal {
    /// The target is Complete or Cancelled — terminal. Permanent.
    Terminal { goal: String, state: String },
    /// The target exists and is not terminal, but is not running right now
    /// (Triage / Ready / Review / Failed, or a column with no state binding).
    NotRunning { goal: String, state: String },
    /// No card with that id.
    NotFound { goal: String },
    /// The id resolved to a card that is not a goal.
    NotAGoal { goal: String, card_type: String },
    /// `from_goal == to_goal`.
    SelfAddressed { goal: String },
    /// Nothing to deliver.
    EmptyBody,
    /// The store refused mid-delivery. Carries the underlying error.
    Storage { detail: String },
}

impl A2aRefusal {
    /// A stable machine reason, for logs and for a caller that branches.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Terminal { .. } => "target_terminal",
            Self::NotRunning { .. } => "target_not_running",
            Self::NotFound { .. } => "target_not_found",
            Self::NotAGoal { .. } => "not_a_goal",
            Self::SelfAddressed { .. } => "self_addressed",
            Self::EmptyBody => "empty_body",
            Self::Storage { .. } => "storage_error",
        }
    }

    /// True when no retry can ever succeed.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Terminal { .. } | Self::NotAGoal { .. } | Self::SelfAddressed { .. }
        )
    }
}

impl std::fmt::Display for A2aRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Terminal { goal, state } => write!(
                f,
                "goal '{goal}' is {state} — a terminal goal cannot receive agent-to-agent \
                 messages, and retrying will not change that. If this work still needs doing, \
                 open a new goal."
            ),
            Self::NotRunning { goal, state } => write!(
                f,
                "goal '{goal}' is {state}, not InProgress — there is no worker to deliver to \
                 yet. This is not permanent: send again once it is running."
            ),
            Self::NotFound { goal } => write!(f, "goal '{goal}' not found"),
            Self::NotAGoal { goal, card_type } => write!(
                f,
                "card '{goal}' is type '{card_type}', not 'goal' — A2A is only between goals"
            ),
            Self::SelfAddressed { goal } => {
                write!(f, "goal '{goal}' cannot message itself")
            }
            Self::EmptyBody => write!(f, "A2A body is empty"),
            Self::Storage { detail } => write!(f, "A2A store write failed: {detail}"),
        }
    }
}

impl From<A2aRefusal> for String {
    fn from(r: A2aRefusal) -> String {
        r.to_string()
    }
}

/// Human label for a state binding, for the refusal text.
fn state_label(binding: &str) -> String {
    match GoalState::from_binding(binding) {
        Some(s) => format!("{s:?}"),
        None => binding.to_string(),
    }
}

/// The audited fingerprint of a message body: enough to prove WHICH message,
/// never enough to read it.
pub fn body_fingerprint(body: &str) -> (String, usize) {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    (hex::encode(hasher.finalize()), body.chars().count())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct A2aMessage {
    pub from_goal: String,
    pub to_goal: String,
    pub body: String,
    pub ts: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A2aDelivery {
    pub steered: bool,
    pub message: A2aMessage,
}

/// Deliver one agent-to-agent message, addressed by goal id.
///
/// Resolution is: goal id → card → live state → (when one exists) the live
/// worker session steering that goal. Every refusal is typed
/// ([`A2aRefusal`]) so the caller can tell a permanent "no" from a "not yet",
/// and every delivery is audited as an `a2a_message` activity event carrying
/// the body's hash and length — never its text.
pub async fn send_goal_a2a(
    pool: &Pool<Sqlite>,
    from_goal: &str,
    to_goal: &str,
    body: &str,
) -> Result<A2aDelivery, A2aRefusal> {
    let body = body.trim();
    if body.is_empty() {
        return Err(A2aRefusal::EmptyBody);
    }
    if from_goal == to_goal {
        return Err(A2aRefusal::SelfAddressed {
            goal: from_goal.to_string(),
        });
    }

    let from = resolve_goal(pool, from_goal).await?;
    let to = resolve_goal(pool, to_goal).await?;

    // ── The target's live state decides whether this can be delivered ──
    let to_col = cards::get_column(pool, &to.column_id)
        .await
        .map_err(|e| A2aRefusal::Storage { detail: e })?
        .ok_or_else(|| A2aRefusal::NotFound {
            goal: to_goal.to_string(),
        })?;
    let binding = to_col.state_binding.as_deref().unwrap_or(&to_col.name);
    if binding != GoalState::InProgress.binding() {
        let state = state_label(binding);
        let refusal = match GoalState::from_binding(binding) {
            // Complete and Cancelled are the terminal pair — say so explicitly
            // rather than lumping them in with "not running yet".
            Some(GoalState::Complete) | Some(GoalState::Cancelled) => A2aRefusal::Terminal {
                goal: to_goal.to_string(),
                state,
            },
            _ => A2aRefusal::NotRunning {
                goal: to_goal.to_string(),
                state,
            },
        };
        tracing::info!(
            target: "permagentd::a2a",
            from_goal,
            to_goal,
            reason = refusal.reason(),
            permanent = refusal.is_permanent(),
            "A2A refused"
        );
        return Err(refusal);
    }

    let msg = A2aMessage {
        from_goal: from_goal.to_string(),
        to_goal: to_goal.to_string(),
        body: body.to_string(),
        ts: Utc::now().to_rfc3339(),
    };
    let value = serde_json::to_value(&msg).map_err(|e| A2aRefusal::Storage {
        detail: e.to_string(),
    })?;

    append_meta_array(pool, to_goal, A2A_INBOX_KEY, value.clone()).await?;
    append_meta_array(pool, from_goal, A2A_SENT_KEY, value.clone()).await?;

    // Durable RLM write-through. This is the ONLY way A2A touches the control
    // plane: `write_a2a_feedback` appends into a bounded, version-checked ring
    // in `rlm_context`, so two senders racing cannot clobber each other. The
    // old path read-modify-wrote the whole `metadata_json` blob with no version
    // guard, which silently lost any concurrent write to `attempt_count`,
    // `last_error` or `worktree_path`.
    rlm::write_a2a_feedback(pool, to_goal, &value)
        .await
        .map_err(|e| A2aRefusal::Storage {
            detail: e.to_string(),
        })?;

    // ── Goal id → live worker ──
    let steered = if let Some(handle) =
        crate::agents::platform_extensions::orchestrator::steer_handle_for(to_goal)
    {
        match handle.steer(&format!("A2A from {from_goal}: {body}")).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::a2a",
                    to_goal,
                    "steer failed after A2A persist: {e}"
                );
                false
            }
        }
    } else {
        false
    };

    // ── Audit ──
    // The sender's own worker is the honest actor; a goal nobody is assigned to
    // falls back to "system", which the journal resolves fail-closed anyway.
    let (body_sha256, body_len) = body_fingerprint(body);
    let actor = from
        .assigned_to
        .as_deref()
        .filter(|a| !a.is_empty())
        .unwrap_or("system");
    events::emit(events::a2a_message(
        from_goal,
        to_goal,
        &body_sha256,
        body_len,
        steered,
        actor,
    ));

    Ok(A2aDelivery {
        steered,
        message: msg,
    })
}

/// Resolve a goal id to its card, refusing anything that is not a goal.
async fn resolve_goal(pool: &Pool<Sqlite>, goal_id: &str) -> Result<cards::Card, A2aRefusal> {
    let card = cards::get_card(pool, goal_id)
        .await
        .map_err(|e| A2aRefusal::Storage { detail: e })?
        .ok_or_else(|| A2aRefusal::NotFound {
            goal: goal_id.to_string(),
        })?;
    if card.card_type != "goal" {
        return Err(A2aRefusal::NotAGoal {
            goal: goal_id.to_string(),
            card_type: card.card_type.clone(),
        });
    }
    Ok(card)
}

async fn append_meta_array(
    pool: &Pool<Sqlite>,
    card_id: &str,
    key: &str,
    item: Value,
) -> Result<(), A2aRefusal> {
    let card = cards::get_card(pool, card_id)
        .await
        .map_err(|e| A2aRefusal::Storage { detail: e })?
        .ok_or_else(|| A2aRefusal::NotFound {
            goal: card_id.to_string(),
        })?;
    let mut meta = card.metadata_json.as_object().cloned().unwrap_or_default();
    let mut arr = meta
        .remove(key)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    arr.push(item);
    const CAP: usize = 20;
    if arr.len() > CAP {
        arr.drain(0..arr.len() - CAP);
    }
    meta.insert(key.to_string(), json!(arr));
    sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
        .bind(
            serde_json::to_string(&Value::Object(meta)).map_err(|e| A2aRefusal::Storage {
                detail: e.to_string(),
            })?,
        )
        .bind(card_id)
        .execute(pool)
        .await
        .map_err(|e| A2aRefusal::Storage {
            detail: e.to_string(),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        crate::session::spectral_schema::apply_rlm_context_schema(&pool)
            .await
            .unwrap();
        pool
    }

    async fn goal(pool: &Pool<Sqlite>, state: &str) -> cards::Card {
        cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = cards::get_goal_column(pool, PERSONAL_PROJECT_ID, state)
            .await
            .unwrap()
            .unwrap();
        cards::create_card(
            pool,
            cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: format!("goal-{state}"),
                description: Some("t".into()),
                card_type: Some("goal".into()),
                column_id: Some(col.id),
                created_by: None,
                metadata_json: Some(json!({"goal_state": state, "attempt_count": 1})),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_goal_id_resolves_to_its_inbox_its_rlm_slice_and_its_worker() {
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;
        let to = goal(&pool, "in_progress").await;

        let ok = send_goal_a2a(&pool, &from.id, &to.id, "watch the race")
            .await
            .unwrap();
        assert_eq!(ok.message.from_goal, from.id);
        assert_eq!(ok.message.to_goal, to.id);
        assert!(
            !ok.steered,
            "no live CLI worker is registered for this goal id in a unit test"
        );

        let updated = cards::get_card(&pool, &to.id).await.unwrap().unwrap();
        let inbox = updated
            .metadata_json
            .get(A2A_INBOX_KEY)
            .and_then(|v| v.as_array())
            .expect("inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0]["body"], "watch the race");
        // A2A state is durable in `rlm_context`, not in the card's metadata blob.
        let cell = rlm::get(&pool, rlm::Scope::Goal, &to.id, rlm::A2A_FEEDBACK_KEY)
            .await
            .unwrap()
            .expect("a2a feedback is stored in the RLM control plane");
        let ring = cell
            .value
            .as_array()
            .expect("the feedback ring is an array");
        assert_eq!(ring.len(), 1);
        assert_eq!(ring[0]["body"], "watch the race");

        let sent = cards::get_card(&pool, &from.id).await.unwrap().unwrap();
        assert_eq!(
            sent.metadata_json[A2A_SENT_KEY].as_array().map(|a| a.len()),
            Some(1),
            "the sender keeps its own copy"
        );
    }

    #[tokio::test]
    async fn a_terminal_target_is_refused_with_a_reason_that_says_do_not_retry() {
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;

        for state in ["complete", "cancelled"] {
            let target = goal(&pool, state).await;
            let refusal = send_goal_a2a(&pool, &from.id, &target.id, "too late")
                .await
                .unwrap_err();
            assert!(
                matches!(refusal, A2aRefusal::Terminal { .. }),
                "{state} must refuse as Terminal, got {refusal:?}"
            );
            assert_eq!(refusal.reason(), "target_terminal");
            assert!(refusal.is_permanent(), "{state} is permanent");
            let text = refusal.to_string();
            assert!(text.contains("terminal"), "{text}");
            assert!(
                text.contains("retrying will not change that"),
                "the refusal must say a retry is pointless: {text}"
            );

            let card = cards::get_card(&pool, &target.id).await.unwrap().unwrap();
            assert!(
                card.metadata_json.get(A2A_INBOX_KEY).is_none(),
                "a refused message must not land in the target's inbox"
            );
        }
    }

    #[tokio::test]
    async fn a_target_that_is_merely_not_running_is_a_different_no() {
        // Not-yet-running is retryable, and a caller that cannot tell the two
        // apart either gives up on live work or hammers a dead goal.
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;

        for state in ["triage", "ready", "review", "failed"] {
            let target = goal(&pool, state).await;
            let refusal = send_goal_a2a(&pool, &from.id, &target.id, "hold on")
                .await
                .unwrap_err();
            assert!(
                matches!(refusal, A2aRefusal::NotRunning { .. }),
                "{state} must refuse as NotRunning, got {refusal:?}"
            );
            assert_eq!(refusal.reason(), "target_not_running");
            assert!(!refusal.is_permanent(), "{state} may run later");
            assert!(refusal.to_string().contains("not permanent"), "{refusal}");
        }
    }

    #[tokio::test]
    async fn a_delivery_is_audited_by_hash_and_length_never_by_body() {
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;
        let to = goal(&pool, "in_progress").await;

        const SECRET: &str = "rotate the staging key, it is in the shared vault";
        let mut bus = crate::events::subscribe();

        send_goal_a2a(&pool, &from.id, &to.id, SECRET)
            .await
            .unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let e = bus.recv().await.expect("bus open");
                // The bus is global and these tests run in parallel, so match
                // on this delivery's own recipient rather than the kind alone.
                if e.event_type == crate::events::PermagentEventType::A2aMessage
                    && e.payload["to_goal"] == serde_json::json!(to.id)
                {
                    return e;
                }
            }
        })
        .await
        .expect("an A2A delivery must emit its audit event");

        // It says who, to whom, and how much.
        assert_eq!(event.payload["from_goal"], serde_json::json!(from.id));
        assert_eq!(event.payload["to_goal"], serde_json::json!(to.id));
        let (expected_hash, expected_len) = body_fingerprint(SECRET);
        assert_eq!(
            event.payload["body_sha256"],
            serde_json::json!(expected_hash)
        );
        assert_eq!(event.payload["body_len"], serde_json::json!(expected_len));

        // And nowhere does it say what.
        let wire = serde_json::to_string(&event).unwrap();
        assert!(
            !wire.contains("rotate the staging key"),
            "the audit event must not republish the body: {wire}"
        );
        assert!(!wire.contains("shared vault"), "{wire}");

        // The journal row it becomes carries the same discipline.
        let entry = crate::activity_journal::entry_from_event(&event)
            .expect("a2a_message must be journal-worthy");
        assert_eq!(entry.kind, "a2a_message");
        assert!(crate::activity_journal::KNOWN_KINDS.contains(&entry.kind.as_str()));
        assert_eq!(entry.ref_kind.as_deref(), Some("goal"));
        assert_eq!(
            entry.ref_id.as_deref(),
            Some(to.id.as_str()),
            "the row points at the recipient goal"
        );
        let detail = entry.detail.unwrap_or_default();
        assert!(
            detail.contains(&format!("{expected_len} chars")),
            "{detail}"
        );
        let digest_prefix: String = expected_hash.chars().take(12).collect();
        assert!(detail.contains(&digest_prefix), "{detail}");
        assert!(!detail.contains("rotate the staging key"), "{detail}");
    }

    #[tokio::test]
    async fn the_shapes_that_are_not_messages_at_all() {
        let pool = pool().await;
        let from = goal(&pool, "in_progress").await;
        let to = goal(&pool, "in_progress").await;

        assert_eq!(
            send_goal_a2a(&pool, &from.id, &to.id, "   ")
                .await
                .unwrap_err(),
            A2aRefusal::EmptyBody
        );
        assert!(matches!(
            send_goal_a2a(&pool, &from.id, &from.id, "hello there")
                .await
                .unwrap_err(),
            A2aRefusal::SelfAddressed { .. }
        ));
        assert!(matches!(
            send_goal_a2a(&pool, &from.id, "no-such-goal", "hello there")
                .await
                .unwrap_err(),
            A2aRefusal::NotFound { .. }
        ));
    }

    #[test]
    fn the_fingerprint_is_a_hash_and_a_character_count() {
        let (hash, len) = body_fingerprint("abc");
        assert_eq!(
            hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA-256 of \"abc\", so an auditor holding the original can verify it"
        );
        assert_eq!(len, 3);
        // Characters, not bytes: a length that changes with the encoding is not
        // a useful "was something substantive sent" signal.
        assert_eq!(body_fingerprint("héllo").1, 5);
    }

    #[test]
    fn the_event_type_serialises_as_a2a_message() {
        assert_eq!(
            serde_json::to_value(crate::events::PermagentEventType::A2aMessage).unwrap(),
            serde_json::json!("a2a_message")
        );
    }
}
