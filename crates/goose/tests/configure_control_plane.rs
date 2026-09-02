//! R2 — the agent's first user-facing settings write, end to end.
//!
//! Its own integration binary, and therefore its own PROCESS: these tests pin
//! `PERMAGENT_PATH_ROOT` and the process-global `Config`, which is a
//! `OnceLock`. Sharing a binary with tests that read the global config would
//! make the result depend on which test ran first — the same constraint
//! `extension_persistence.rs` and `liveness_wire.rs` document.
//!
//! The pin runs in a `#[ctor]` rather than inside each test, because
//! `pin_config_to_temp_root_for_tests` WIPES and re-creates the root: two tests
//! calling it in parallel would each delete the other's fixture. One call,
//! before any test starts, lets several named tests share the binary and still
//! fail independently.

use permagent::agents::platform_extensions::configure_app;
use permagent::agents::self_knowledge::{worker_gate, FeatureFlags};
use permagent::config::Config;
use permagent::decisions::{self, DecisionAnswer};
use permagent::events::{self, PermagentEvent, PermagentEventType};
use sqlx::{Pool, Sqlite};

#[ctor::ctor(unsafe)]
fn pin_config_for_this_binary() {
    permagent::config::base::pin_config_to_temp_root_for_tests();
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<PermagentEvent>) -> Vec<PermagentEvent> {
    let mut out = Vec::new();
    while let Ok(e) = rx.try_recv() {
        out.push(e);
    }
    out
}

/// Does any `config_changed` frame name `key`?
fn names_key(frames: &[PermagentEvent], key: &str) -> bool {
    frames
        .iter()
        .filter(|e| e.event_type == PermagentEventType::ConfigChanged)
        .any(|e| {
            e.payload
                .get("keys")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.iter().any(|k| k.as_str() == Some(key)))
        })
}

async fn test_pool() -> Pool<Sqlite> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    permagent::session::spectral_schema::init_spectral_db(&pool)
        .await
        .unwrap();
    pool
}

/// **The end-to-end gate.** `configure_set` flips a real feature gate, and the
/// flip is visible everywhere the flip has to be visible:
///
/// 1. the config key actually changed, re-read the way a restarted daemon reads
///    it (`Config::get_param`, straight off config.yaml);
/// 2. a `config_changed` frame naming that key reached the bus — which is what
///    `livenessSync` consumes to bump `configRev` and refresh an open Settings
///    pane, so the human's view of the switch cannot go stale behind the agent;
/// 3. the value `FeatureFlags::from_live_config` reads — the loader
///    `FeaturesPanel` and the self-knowledge brief and every worker gate agree
///    on — is the flipped one, not a cached false.
///
/// FAILS BEFORE: there was no `configure` extension and no `configure_set`, so
/// the agent had zero user-facing config writes; this file does not compile
/// against the base branch. What it defends going forward is that the write
/// keeps landing on `Config::set_param` — the writer the Settings pane calls,
/// the one that announces. A "helpful" refactor that wrote config.yaml directly,
/// or that cached the flag, turns step 2 or step 3 red.
#[tokio::test]
async fn a_configure_set_flip_reaches_the_key_the_bus_and_the_worker_gate() {
    let key = permagent::strix::STRIX_ENABLED_KEY;

    // Start from a known false, written the same way, so the assertion is about
    // the flip and not about an ambient default.
    Config::global().set_param(key, false).unwrap();
    assert_eq!(Config::global().get_param::<bool>(key).ok(), Some(false));
    assert!(
        !FeatureFlags::from_live_config().strix_enabled,
        "fixture did not land"
    );

    let mut rx = events::subscribe();
    let report = configure_app::configure_set_impl(
        key,
        &serde_json::json!(true),
        "the user asked me to switch the Guard on",
    )
    .await
    .expect("a feature gate is on the direct-write allowlist");

    // 1. The key really changed, read back off disk.
    assert_eq!(
        Config::global().get_param::<bool>(key).ok(),
        Some(true),
        "configure_set reported success without changing the key"
    );

    // 2. …and it announced itself, from the writer.
    let frames = drain(&mut rx);
    assert!(
        names_key(&frames, key),
        "no config_changed frame named {key}; an open Settings pane would still show the old \
         value. Frames seen: {:?}",
        frames.iter().map(|f| &f.event_type).collect::<Vec<_>>()
    );

    // 3. …and the value every consumer of the gate reads is the flipped one.
    let flags = FeatureFlags::from_live_config();
    assert!(
        flags.strix_enabled,
        "FeatureFlags — what FeaturesPanel and the brief read — did not see the flip"
    );
    let gate = worker_gate(permagent::strix::STRIX_FEATURE_ID).expect("the Guard has a gate");
    assert_eq!(gate.key, key, "the gate table and the allowlist disagree");
    assert!(
        gate.is_on(flags),
        "the worker gate still reads off after the agent switched it on"
    );

    // The report has to describe what happened, durably, without overclaiming.
    assert!(report.starts_with("Saved:"), "got: {report}");
    assert!(report.contains(key), "got: {report}");
    assert!(report.contains("survives a restart"), "got: {report}");

    // And back off again, so the binary leaves the fixture as it found it.
    configure_app::configure_set_impl(key, &serde_json::json!(false), "putting it back")
        .await
        .unwrap();
    assert_eq!(Config::global().get_param::<bool>(key).ok(), Some(false));
}

/// **The proposal gate.** A sensitive key changes ONLY on approval.
///
/// Three phases on one key so the "only after approval" claim is a comparison
/// and not two unrelated assertions: the value is checked after filing (must be
/// unchanged), after approval (must be the proposed one), and a second card is
/// rejected (must change nothing).
///
/// FAILS BEFORE: `config_change_proposal` was not a decision kind, so
/// `create_decision` stored it as `malformed` and there was no effect arm to
/// apply anything.
#[tokio::test]
async fn a_proposal_changes_nothing_until_approval_and_nothing_on_rejection() {
    let pool = test_pool().await;
    let key = permagent::sovereignty::SOVEREIGN_CAPTURE_PROMPTS_KEY;
    Config::global().set_param(key, false).unwrap();

    let payload = decisions::ConfigChangeProposalPayload {
        key_class: "sovereignty".to_string(),
        key: key.to_string(),
        value: serde_json::json!(true),
        current_value: Some("false".to_string()),
        rationale: "the user wants full prompt capture in the egress audit".to_string(),
    };
    let decision = decisions::create_decision(
        &pool,
        decisions::NewDecision {
            kind: configure_app::CONFIG_CHANGE_PROPOSAL_KIND.to_string(),
            headline: Some("Capture full prompts in the egress audit".to_string()),
            detail: Some("proposed by the agent".to_string()),
            payload: serde_json::to_value(&payload).unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        decision.kind,
        configure_app::CONFIG_CHANGE_PROPOSAL_KIND,
        "the proposal was stored as {:?} — the kind is not registered",
        decision.kind
    );

    // PHASE 1 — filed, not applied.
    assert_eq!(
        Config::global().get_param::<bool>(key).ok(),
        Some(false),
        "filing the card already changed the setting; the user never got to decide"
    );

    // PHASE 2 — approval applies it, server-side, through the same writer.
    let (answered, proof) = decisions::answer_decision(
        &pool,
        &decision.id,
        &DecisionAnswer {
            answer: "approve".to_string(),
            ..Default::default()
        },
        decisions::ACTOR_JESSE,
    )
    .await
    .expect("approve is a valid answer for this kind");

    let mut rx = events::subscribe();
    let (effect, _) = permagent::decisions_effects::apply_decision_effect(
        &pool,
        &answered,
        proof,
        configure_app::CONFIG_CHANGE_PROPOSAL_KIND,
    )
    .await
    .expect("the approved effect applies");

    assert_eq!(
        Config::global().get_param::<bool>(key).ok(),
        Some(true),
        "approving the card did not apply the write"
    );
    assert!(
        names_key(&drain(&mut rx), key),
        "the approved write did not announce itself — it did not go through Config::set_param"
    );
    let effect = effect.unwrap_or_default();
    assert!(
        effect.contains(key),
        "the effect must say what changed: {effect}"
    );

    // PHASE 3 — a rejection writes nothing.
    Config::global().set_param(key, false).unwrap();
    let second = decisions::create_decision(
        &pool,
        decisions::NewDecision {
            kind: configure_app::CONFIG_CHANGE_PROPOSAL_KIND.to_string(),
            headline: Some("Capture full prompts in the egress audit".to_string()),
            detail: Some("proposed by the agent".to_string()),
            payload: serde_json::to_value(&payload).unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (answered, proof) = decisions::answer_decision(
        &pool,
        &second.id,
        &DecisionAnswer {
            answer: "reject".to_string(),
            ..Default::default()
        },
        decisions::ACTOR_JESSE,
    )
    .await
    .unwrap();
    permagent::decisions_effects::apply_decision_effect(
        &pool,
        &answered,
        proof,
        configure_app::CONFIG_CHANGE_PROPOSAL_KIND,
    )
    .await
    .expect("a rejection is a valid, no-op effect");
    assert_eq!(
        Config::global().get_param::<bool>(key).ok(),
        Some(false),
        "rejecting the card wrote the change anyway"
    );
}

/// A malformed proposal must never become a live card. The gate is
/// `validate_config_change_payload`, which delegates to the SAME allowlist the
/// tool and the effect use — so a card that could not be applied cannot be
/// filed in the first place.
#[tokio::test]
async fn a_proposal_naming_a_key_outside_its_class_is_stored_as_malformed() {
    let pool = test_pool().await;
    let payload = serde_json::json!({
        "key_class": "budget",
        "key": "GOOSE_MODE",
        "value": "auto",
        "rationale": "smuggling autonomy in through the budget class",
    });
    let decision = decisions::create_decision(
        &pool,
        decisions::NewDecision {
            kind: configure_app::CONFIG_CHANGE_PROPOSAL_KIND.to_string(),
            headline: Some("Raise the budget".to_string()),
            detail: Some("…".to_string()),
            payload,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        decision.kind, "malformed",
        "a proposal whose key does not belong to its class became a live card"
    );
    assert_eq!(
        Config::global().get_param::<String>("GOOSE_MODE").ok(),
        None,
        "filing the malformed proposal wrote GOOSE_MODE"
    );
}
