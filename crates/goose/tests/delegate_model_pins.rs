//! Bench-style regression: in a run where the operator PINNED the models, no
//! delegate goes to an unpinned provider.
//!
//! This is the model-defaults bench (2026-08-25) turned into a test. That bench
//! ran `permagent run --recipe permagent-coding` with every
//! `PERMAGENT_PACK_{EDIT,HARD,MECHANICAL,LOCAL}_{PROVIDER,MODEL}` set to the
//! candidate model and measured the subagents running on `anthropic/claude-fable-5`
//! regardless: a `gpt-5.4-mini` run billed three Fable calls at $1.51 against
//! $0.31 of its own spend, and a `glm-5.3` run billed six at $2.20 — 82% of that
//! task's total.
//!
//! It drives the REAL knowledge base and the REAL recommender over a realistic
//! provider surface, so it fails if a future knowledge-base row, floor or
//! ranking change re-opens the escape hatch. No network, no provider keys, no DB
//! — the decision layer is pure by construction.

use permagent::cost_router::delegate::{decide_delegate_model, DelegateSource, EscalationRefusal};
use permagent::cost_router::derived::derive_role_map;
use permagent::cost_router::packs::{configured_pack_pin, pack_role_for_workflow_role};
use permagent::cost_router::role_map::RoleModel;
use permagent::cost_router::{
    budget::BudgetBand, AvailableModel, DerivedRoleMap, ModelPack, WorkflowRole,
};

/// The operator's own model for the run. The bench used two; both are cheaper
/// than every Anthropic row the derived map can reach.
const CANDIDATES: &[(&str, &str)] = &[("openai", "gpt-5.6-mini"), ("zai", "glm-5.2")];

/// What the recommender sees when the operator holds an `ANTHROPIC_API_KEY` —
/// which is enough, on its own, to put the entire Anthropic line in the pool.
/// This is the surface that produced the leak.
fn available_with_anthropic_key(session: (&str, &str)) -> Vec<AvailableModel> {
    let mut v: Vec<AvailableModel> = [
        "claude-fable-5",
        "claude-opus-4-8",
        "claude-sonnet-5",
        "claude-haiku-4-5-20251001",
    ]
    .iter()
    .map(|m| AvailableModel::new("anthropic", m))
    .collect();
    v.push(AvailableModel::new(session.0, session.1));
    v
}

/// Every `PERMAGENT_PACK_*` key set to one model, as the bench set them.
fn all_packs_pinned_to(provider: &str, model: &str) -> Vec<(String, String)> {
    use permagent::cost_router::packs::{
        KEY_EDIT_MODEL, KEY_EDIT_PROVIDER, KEY_HARD_MODEL, KEY_HARD_PROVIDER, KEY_LOCAL_MODEL,
        KEY_LOCAL_PROVIDER, KEY_MECHANICAL_MODEL, KEY_MECHANICAL_PROVIDER,
    };
    [
        KEY_EDIT_PROVIDER,
        KEY_HARD_PROVIDER,
        KEY_MECHANICAL_PROVIDER,
        KEY_LOCAL_PROVIDER,
    ]
    .iter()
    .map(|k| (k.to_string(), provider.to_string()))
    .chain(
        [
            KEY_EDIT_MODEL,
            KEY_HARD_MODEL,
            KEY_MECHANICAL_MODEL,
            KEY_LOCAL_MODEL,
        ]
        .iter()
        .map(|k| (k.to_string(), model.to_string())),
    )
    .collect()
}

fn pin_for(env: &[(String, String)], role: WorkflowRole) -> Option<ModelPack> {
    let pack_role = pack_role_for_workflow_role(role)?;
    configured_pack_pin(pack_role, |k| {
        env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    })
}

fn route(
    role: WorkflowRole,
    env: &[(String, String)],
    derived: &DerivedRoleMap,
    session: (&str, &str),
    allow_escalation: bool,
) -> permagent::cost_router::DelegateRouting {
    decide_delegate_model(
        Some(role),
        // No hand-configured `PERMAGENT_ROLE_*` mapping — the bench set packs.
        None,
        pin_for(env, role).map(|p| RoleModel {
            provider: p.provider,
            model: p.model,
        }),
        derived.get(role).map(|(rm, _)| rm.clone()),
        allow_escalation,
        Some(BudgetBand::Ok),
        &|p| p == "anthropic" || p == session.0,
        Some(session),
    )
}

/// The provider a delegate would actually be dispatched to: the routed one, or
/// the session's when the decision is to inherit.
fn dispatched_provider(
    r: &permagent::cost_router::DelegateRouting,
    session: (&str, &str),
) -> String {
    r.role_model
        .as_ref()
        .map(|rm| rm.provider.clone())
        .unwrap_or_else(|| session.0.to_string())
}

/// The knowledge base still holds the row that caused this. If it stops being
/// the top-ranked pick the test below would pass vacuously, so assert it first.
#[test]
fn the_derived_map_still_wants_to_escalate_to_anthropics_priciest_row() {
    let session = CANDIDATES[0];
    let derived = derive_role_map(&available_with_anthropic_key(session));
    let edit = derived
        .get(WorkflowRole::Edit)
        .expect("EDIT derives from an Anthropic-keyed surface");
    assert_eq!(
        (edit.0.provider.as_str(), edit.0.model.as_str()),
        ("anthropic", "claude-fable-5"),
        "if this changes, revisit the escalation gates rather than deleting the test",
    );
}

/// The regression itself: with the packs pinned, NOTHING lands off the pin.
#[test]
fn a_pinned_run_never_dispatches_to_an_unpinned_provider() {
    for &session in CANDIDATES {
        let env = all_packs_pinned_to(session.0, session.1);
        let derived = derive_role_map(&available_with_anthropic_key(session));

        for role in WorkflowRole::all() {
            // The knob is deliberately exercised BOTH ways: a pin outranks
            // escalation, so even an operator who opted into escalation gets the
            // model they pinned.
            for allow in [false, true] {
                let r = route(role, &env, &derived, session, allow);
                let provider = dispatched_provider(&r, session);
                assert_eq!(
                    provider, session.0,
                    "role {role:?} (escalation={allow}) went to '{provider}' — the operator \
                     pinned '{}' and named no other provider; receipt: {}",
                    session.0, r.receipt,
                );
                assert_ne!(
                    r.source,
                    DelegateSource::Escalated,
                    "role {role:?} escalated past a pin: {}",
                    r.receipt
                );
            }
        }
    }
}

/// And with NO pins at all: still nothing leaves the session's provider, because
/// escalation is off by default. This is the half that would have saved the
/// bench's money without the operator setting anything.
#[test]
fn an_unpinned_run_stays_on_the_session_model_by_default() {
    for &session in CANDIDATES {
        let derived = derive_role_map(&available_with_anthropic_key(session));
        for role in WorkflowRole::all() {
            let r = route(role, &[], &derived, session, false);
            assert_eq!(
                dispatched_provider(&r, session),
                session.0,
                "role {role:?} left the session provider with no pin and no opt-in: {}",
                r.receipt
            );
            assert_eq!(r.source, DelegateSource::Session);
        }
    }
}

/// The opt-in still works — and says so on the record. Without this the knob
/// would be a synonym for "off" and the escape hatch a lie.
#[test]
fn the_opt_in_restores_escalation_and_leaves_a_receipt() {
    let session = CANDIDATES[0];
    let derived = derive_role_map(&available_with_anthropic_key(session));
    let r = route(WorkflowRole::Edit, &[], &derived, session, true);
    assert_eq!(r.source, DelegateSource::Escalated);
    assert_eq!(dispatched_provider(&r, session), "anthropic");
    assert!(r.receipt.contains("claude-fable-5"), "{}", r.receipt);
    assert!(
        r.receipt.contains("PERMAGENT_DELEGATE_ALLOW_ESCALATION"),
        "the receipt must name what authorized the spend: {}",
        r.receipt
    );
}

/// Escalation stays inside the operator's configured providers even when it is
/// allowed: an Anthropic row the operator holds no key for is not reachable.
#[test]
fn escalation_cannot_reach_an_unconfigured_provider() {
    let session = CANDIDATES[0];
    let derived = derive_role_map(&available_with_anthropic_key(session));
    let r = decide_delegate_model(
        Some(WorkflowRole::Edit),
        None,
        None,
        derived.get(WorkflowRole::Edit).map(|(rm, _)| rm.clone()),
        true,
        Some(BudgetBand::Ok),
        // Only the session's own provider is configured here.
        &|p| p == session.0,
        Some(session),
    );
    assert_eq!(r.source, DelegateSource::Session);
    assert_eq!(
        r.refused.map(|(_, why)| why),
        Some(EscalationRefusal::UnconfiguredProvider)
    );
}
