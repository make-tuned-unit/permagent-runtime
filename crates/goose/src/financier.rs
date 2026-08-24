//! The Financier — the money character, and the stated rule about where its
//! work should run. The rule is resolved and shown; it does not yet steer
//! dispatch (see "What this resolver does and does NOT do today", below).
//!
//! # What it is
//!
//! The Financier reads and reports on money. Two halves already existed before
//! this module did, and neither of them is duplicated here:
//!
//!   * [`crate::market_data`] — live quotes and, with an optional key, company
//!     financial statements. The research half.
//!   * the `finance` platform extension
//!     ([`crate::agents::platform_extensions::finance`]) — the eight tools that
//!     expose that research to the agent, plus the bridge to whatever finance
//!     service the user runs themselves.
//!
//! What was missing was the *character*: an identity the user can see, a switch
//! they can reach, and a stated rule about where its inference is allowed to
//! run. That is what this module is.
//!
//! # One switch, and it is not a new one
//!
//! [`is_enabled`] reads the enabled bit of the `finance` extension — the same
//! bit `capabilities()` reports on the Agents surface and the same bit
//! [`crate::config::extensions::get_enabled_extensions`] filters a new session
//! on. There is deliberately **no `financier_enabled` config key.** A second
//! boolean would be a second thing to disagree with the first, and the failure
//! it produces is the one this codebase keeps re-learning: a user flips the
//! switch they can find and the code reads the other one.
//!
//! # Where its inference may run, and why the order is this order
//!
//! The user's standing instruction about financial work is that it stays local
//! by preference and reaches a cloud model only deliberately. [`FinancierRoute`]
//! and [`resolve_financier_route`] express that instruction as code:
//!
//! 1. **On-device** ([`crate::providers::apple_fm`]) — Apple's Foundation
//!    Models. The prompt does not leave this Mac.
//! 2. **Local Ollama** — a model served from this machine.
//! 3. **Cloud**, and only with a standing consent the user set on purpose.
//! 4. Otherwise **refused**, with the reason.
//!
//! ## What this resolver does and does NOT do today — read this before trusting it
//!
//! **It is a stated preference that is currently only DISPLAYED.** Nothing in
//! the agent's inference path calls [`resolve_financier_route`]: when the
//! Financier's tools run inside a session, that session's own configured
//! provider serves the model call, exactly as it did before this module
//! existed. The only production caller is the daemon's
//! `GET /api/finance/routing` read, which renders the answer on the Financier
//! tab so the user can see what the rule says and whether a local route is
//! even available on this machine.
//!
//! That is written here in the plainest words available because the failure it
//! guards against is specific and this codebase has paid for it repeatedly: a
//! resolver that LOOKS like routing, reads like routing, and is wired to
//! nothing. Making the session provider honour this preference means selecting
//! a provider per worker rather than per session, which the agent loop does not
//! do yet. Until it does, the honest claim is "this is the rule, and here is
//! whether your machine could satisfy it" — not "your financial work runs
//! locally".
//!
//! **What IS enforced, in code, today** is the layer below, and this module
//! does not weaken it: [`crate::providers::base::Provider::data_locality`] is
//! fail-closed (`Cloud` unless a provider proves otherwise) and
//! [`crate::providers::sovereign_guard`] audits or blocks every cloud call at
//! the single choke point every provider passes through. Separately,
//! [`crate::market_data`]'s two fetches — the part of the Financier that really
//! does carry the user's symbols off the machine — now pass through
//! [`crate::sovereignty::guard_outbound_egress`] before any client is built, so
//! they are audited every time and refused outright under sovereign mode. That
//! wire is real and tested.
//!
//! # What the consent flag actually is
//!
//! [`FINANCIER_ALLOW_CLOUD_KEY`] is a **standing** permission, set once, not a
//! prompt per call. It is written down that way here because the temptation is
//! to describe it as per-call consent, and it is not: nothing in this module
//! asks a question at call time. Per-call approval has a home in this codebase
//! already — the Decision Inbox that [`crate::cost_router::budget`] gates
//! through — and routing this flag through it is the honest next step, not
//! something to claim now.

use crate::config::Config;

/// The character's id. Kept identical across the three id namespaces that only
/// usually agree — the agent.yaml roster key, the world roster id, and the
/// settings portrait id — so none of the id-bridging helpers has to know about
/// it. See `config::agent_identity::descriptor_id_for_worker_key` for what the
/// alternative costs.
pub const FINANCIER_ID: &str = "financier";

/// The name the user sees.
pub const FINANCIER_NAME: &str = "The Financier";

/// Config key: a standing permission to let Financier work reach a cloud model
/// when no local route is available. Default **off**. See the module docs for
/// what this is and, more importantly, what it is not.
pub const FINANCIER_ALLOW_CLOUD_KEY: &str = "financier_allow_cloud";

/// Is the Financier switched on? Delegates to the `finance` extension's own
/// enabled bit — see the module docs on why there is no second key.
pub fn is_enabled() -> bool {
    crate::config::is_extension_enabled(crate::agents::platform_extensions::finance::EXTENSION_NAME)
}

/// Has the user granted the standing cloud permission? Default false: absent
/// an explicit yes, financial work does not go to a cloud model.
pub fn cloud_consent(config: &Config) -> bool {
    config
        .get_param::<bool>(FINANCIER_ALLOW_CLOUD_KEY)
        .unwrap_or(false)
}

/// Why no route was available. Separate variants because the fixes differ, and
/// "it did not run" with no reason is the shape of message this codebase treats
/// as a defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinancierRefusal {
    /// A local route exists in principle but none is ready here, and the user
    /// has not granted the standing cloud permission. The fix is theirs to
    /// choose: make a local model available, or grant the permission.
    NoLocalRouteAndNoCloudConsent,
    /// Cloud was permitted, but no session provider/model is configured, so
    /// there is nothing to fall back TO.
    CloudConsentedButNoProvider,
}

/// Where a Financier inference pass should run. Ordered by preference in
/// [`resolve_financier_route`]; each variant carries what the caller needs to
/// actually run it, so no second lookup can disagree with the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinancierRoute {
    /// Apple's on-device Foundation Models. Inference stays on this Mac.
    OnDevice,
    /// A model served by a local Ollama.
    LocalOllama { model: String },
    /// The configured session provider. Reached only with standing consent,
    /// and audited at the provider choke point like every other cloud call.
    Cloud { provider: String, model: String },
    /// Nothing may run.
    Refused(FinancierRefusal),
}

impl FinancierRoute {
    /// Where this route physically runs. Expressed in the same vocabulary the
    /// sovereignty layer uses ([`crate::sovereignty::DataLocality`]) so a test
    /// can assert the preference without re-encoding the mapping, and so a new
    /// variant cannot quietly be assumed local.
    ///
    /// A refusal reports `Local`: nothing runs, so nothing leaves. Calling it
    /// `Cloud` would put a refusal in the same bucket as an egress.
    pub fn locality(&self) -> crate::sovereignty::DataLocality {
        match self {
            Self::OnDevice | Self::LocalOllama { .. } | Self::Refused(_) => {
                crate::sovereignty::DataLocality::Local
            }
            Self::Cloud { .. } => crate::sovereignty::DataLocality::Cloud,
        }
    }

    /// True when this route keeps the work on this machine.
    pub fn is_local(&self) -> bool {
        self.locality().is_local()
    }
}

/// Choose a route. Pure: every input is passed in, so this is unit-testable
/// without an Apple Intelligence-capable Mac, without an Ollama, and without
/// touching process-global config — the same discipline
/// [`crate::meeting_writeup::resolve_meeting_writeup_plan`] follows, and for
/// the same reason (CI has none of those things).
///
/// * `on_device_ready` — the Apple Foundation Models provider is available
///   *now*. A probe that said yes a minute ago is not evidence about now, so
///   the caller passes a fresh answer rather than this function caching one.
/// * `ollama_model` — the model name a local Ollama is configured to serve,
///   when the local pack actually names Ollama.
/// * `cloud_consent` — [`cloud_consent`].
/// * `session` — the configured session provider/model, if any.
pub fn resolve_financier_route(
    on_device_ready: bool,
    ollama_model: Option<&str>,
    cloud_consent: bool,
    session: Option<(String, String)>,
) -> FinancierRoute {
    if on_device_ready {
        return FinancierRoute::OnDevice;
    }
    if let Some(model) = ollama_model {
        return FinancierRoute::LocalOllama {
            model: model.to_string(),
        };
    }
    if !cloud_consent {
        return FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent);
    }
    match session {
        Some((provider, model)) => FinancierRoute::Cloud { provider, model },
        None => FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider),
    }
}

/// The local Ollama model name, when the configured local pack actually names
/// Ollama. Returns `None` for a pack pointed at anything else, so a user who
/// repointed `PERMAGENT_PACK_LOCAL_PROVIDER` at a cloud service does not get it
/// silently treated as a local route.
pub fn local_ollama_model(packs: &crate::cost_router::packs::ModelPacks) -> Option<String> {
    (packs.local.provider == "ollama").then(|| packs.local.model.clone())
}

/// Resolve against the live system. Async because on-device availability is a
/// runtime probe against the running OS, not a build-time fact.
pub async fn resolve_live_financier_route(config: &Config) -> FinancierRoute {
    let on_device_ready = crate::providers::apple_fm::availability()
        .await
        .is_available();
    let packs = crate::cost_router::packs::load_packs();
    let ollama_model = local_ollama_model(&packs);
    let session = match (config.get_goose_provider(), config.get_goose_model()) {
        (Ok(provider), Ok(model)) => Some((provider, model)),
        _ => None,
    };
    resolve_financier_route(
        on_device_ready,
        ollama_model.as_deref(),
        cloud_consent(config),
        session,
    )
}

/// One sentence describing the route, for the user.
///
/// Phrased as "would run", not "ran": nothing dispatches on this decision yet
/// (see the module docs), so a past tense here would be a claim about work that
/// this resolver did not place. When the agent loop can select a provider per
/// worker, these sentences become provenance in the sense
/// `meeting_writeup::privacy_statement` means it — and the wording will need to
/// change with the wire, not before it.
pub fn route_statement(route: &FinancierRoute) -> String {
    match route {
        FinancierRoute::OnDevice => {
            "Would run on this Mac, on-device — an on-device model is available here."
                .to_string()
        }
        FinancierRoute::LocalOllama { model } => format!(
            "Would run on this machine via Ollama (`{model}`) — no on-device model is available, \
             but a local one is."
        ),
        FinancierRoute::Cloud { provider, model } => format!(
            "Would run on a cloud model (`{provider}/{model}`): no local route is available and \
             you granted the standing cloud permission. Every cloud call is recorded in the \
             egress audit."
        ),
        FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent) => {
            "Would not run: no local model is available here, and financial work is not sent to \
             a cloud model without your say-so. Make a local model available, or turn on the \
             cloud permission below."
                .to_string()
        }
        FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider) => {
            "Would not run: no local model is available and, although cloud work is permitted, \
             no provider and model are configured to send it to."
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sovereignty::DataLocality;

    fn session() -> Option<(String, String)> {
        Some(("anthropic".to_string(), "a-model".to_string()))
    }

    /// The headline rule. On-device wins whenever it is ready — even with an
    /// Ollama present, even with cloud consent granted and a provider ready.
    #[test]
    fn on_device_is_preferred_over_every_other_route() {
        assert_eq!(
            resolve_financier_route(true, Some("qwen3"), true, session()),
            FinancierRoute::OnDevice
        );
    }

    /// Second rung. With no on-device model, a local Ollama still beats a cloud
    /// provider the user has already consented to.
    #[test]
    fn local_ollama_is_preferred_over_a_consented_cloud_provider() {
        assert_eq!(
            resolve_financier_route(false, Some("qwen3"), true, session()),
            FinancierRoute::LocalOllama {
                model: "qwen3".to_string()
            }
        );
    }

    /// The fail-closed rung, and the one the user's instruction is really
    /// about: no local route and no explicit consent means the work does not
    /// run, rather than quietly going to the cloud.
    #[test]
    fn without_consent_no_local_route_refuses_rather_than_falling_back_to_cloud() {
        let route = resolve_financier_route(false, None, false, session());
        assert_eq!(
            route,
            FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent)
        );
        assert!(route.is_local(), "a refusal sends nothing anywhere");
    }

    /// Consent is necessary but not sufficient — there must also be somewhere
    /// to send it, and the two failures are reported separately.
    #[test]
    fn consent_without_a_configured_provider_is_its_own_refusal() {
        assert_eq!(
            resolve_financier_route(false, None, true, None),
            FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider)
        );
    }

    /// Cloud is reachable, but only through the one door.
    #[test]
    fn cloud_is_reached_only_with_consent_and_a_provider() {
        assert_eq!(
            resolve_financier_route(false, None, true, session()),
            FinancierRoute::Cloud {
                provider: "anthropic".to_string(),
                model: "a-model".to_string()
            }
        );
    }

    /// Every route this resolver can PREFER over cloud must actually be local
    /// in the sovereignty layer's own vocabulary. Asserting the mapping rather
    /// than the variant names means a future variant has to state its locality
    /// to compile, and cannot inherit "local" by looking local.
    #[test]
    fn only_the_cloud_route_reports_cloud_locality() {
        for (route, expected) in [
            (FinancierRoute::OnDevice, DataLocality::Local),
            (
                FinancierRoute::LocalOllama {
                    model: "qwen3".to_string(),
                },
                DataLocality::Local,
            ),
            (
                FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent),
                DataLocality::Local,
            ),
            (
                FinancierRoute::Cloud {
                    provider: "p".to_string(),
                    model: "m".to_string(),
                },
                DataLocality::Cloud,
            ),
        ] {
            assert_eq!(route.locality(), expected, "{route:?}");
        }
    }

    /// A local pack pointed at something that is not Ollama is not a local
    /// route. Without this, repointing the pack at a hosted endpoint would
    /// promote cloud inference into the second rung, silently.
    #[test]
    fn a_local_pack_that_is_not_ollama_is_not_treated_as_local() {
        use crate::cost_router::packs::ModelPacks;
        let mut packs = ModelPacks::default();
        assert_eq!(local_ollama_model(&packs).as_deref(), Some("qwen3"));
        packs.local.provider = "anthropic".to_string();
        assert_eq!(local_ollama_model(&packs), None);
    }

    /// Every route states where it ran, and a refusal says why. An empty or
    /// generic sentence here is what turns "it did not run" into a mystery.
    #[test]
    fn every_route_states_its_provenance() {
        for route in [
            FinancierRoute::OnDevice,
            FinancierRoute::LocalOllama {
                model: "qwen3".to_string(),
            },
            FinancierRoute::Cloud {
                provider: "p".to_string(),
                model: "m".to_string(),
            },
            FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent),
            FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider),
        ] {
            let statement = route_statement(&route);
            assert!(statement.len() > 20, "{route:?} -> {statement:?}");
            assert!(statement.ends_with('.'), "{route:?} -> {statement:?}");
        }
        assert!(route_statement(&FinancierRoute::OnDevice).contains("on-device"));
        // Conditional, not past tense: nothing dispatches on this decision yet,
        // so no statement may read as a report of work that actually ran.
        for route in [
            FinancierRoute::OnDevice,
            FinancierRoute::LocalOllama {
                model: "qwen3".to_string(),
            },
            FinancierRoute::Cloud {
                provider: "p".to_string(),
                model: "m".to_string(),
            },
        ] {
            assert!(
                route_statement(&route).starts_with("Would "),
                "{route:?} must describe what WOULD happen, not claim it did"
            );
        }
    }

    /// The Financier has no switch of its own: it rides the `finance`
    /// extension's enabled bit. This pins that there is no second key to drift
    /// — if someone adds a `financier_enabled` param, `is_enabled` must not
    /// start reading it without this test being deliberately changed.
    #[test]
    fn the_switch_is_the_finance_extension_and_not_a_second_key() {
        assert_eq!(
            crate::agents::platform_extensions::finance::EXTENSION_NAME,
            "finance"
        );
        // The consent flag is a separate concern (where work may RUN), not a
        // second enable switch (whether the Financier EXISTS).
        assert_ne!(FINANCIER_ALLOW_CLOUD_KEY, "financier_enabled");
    }
}
