//! The Financier's read surface.
//!
//! Two endpoints, and both are deliberately reads:
//!
//!   GET /api/finance/routing        — where the Financier's inference would run right now
//!   GET /api/finance/quote?symbol=  — one live quote
//!
//! There is **no write route here.** The one setting this surface exposes —
//! the standing cloud permission,
//! [`permagent::financier::FINANCIER_ALLOW_CLOUD_KEY`] — is a plain config key,
//! so it is written with the existing `POST /config/upsert` that Settings uses
//! for every other flag. Adding a `POST /api/finance/...` twin would create a
//! second way to set one value, which is the exact drift this codebase keeps
//! paying for.
//!
//! ## Why the quote endpoint is not a cache
//!
//! It calls straight through to [`permagent::market_data::quote`], which does
//! not cache and stamps every reading with the time the exchange gave it. A
//! price is only a price at the moment it was read. Nothing is stored: this
//! daemon persists no quote, no symbol history and no watchlist, so a request
//! here leaves no record beyond the egress audit row the fetch itself writes.
//!
//! ## The failure mode that matters
//!
//! A quote that could not be fetched is reported as a failure, with the
//! reason, and never smoothed into a zero or an empty object. Wrong numbers
//! about money are worse than no numbers, and a `price: 0` rendering as "$0.00"
//! is a wrong number. The `Quote` type makes every field optional for the same
//! reason, and this route passes that shape through untouched.

use std::sync::Arc;

use axum::{
    extract::{Json, Query},
    http::StatusCode,
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};

use permagent::config::Config;
use permagent::financier::{self, FinancierRefusal, FinancierRoute};
use permagent::market_data;

use crate::state::AppState;

/// Where the Financier's inference would run, rendered for the UI.
///
/// `kind` is the machine-readable decision and `statement` the sentence shown
/// to the user; both come from the same resolved route, so the label and the
/// behaviour cannot disagree. `is_local` is derived from
/// [`FinancierRoute::locality`] rather than recomputed here — one mapping, one
/// place.
#[derive(Debug, Serialize)]
pub struct RoutingView {
    /// `on_device` | `local_ollama` | `cloud` | `refused`
    pub kind: &'static str,
    /// The provider/model that would serve it, when there is one to name.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub is_local: bool,
    pub statement: String,
    /// The standing cloud permission, and the key that holds it — the client
    /// never hard-codes a config key, the same rule the agent gate rows follow.
    pub cloud_allowed: bool,
    pub cloud_consent_key: &'static str,
    /// Whether the Financier itself is switched on (the `finance` extension's
    /// enabled bit). A routing answer is not an "it will run" promise if the
    /// capability is off, and the UI must be able to say which is which.
    pub enabled: bool,
}

fn routing_view(route: &FinancierRoute, cloud_allowed: bool, enabled: bool) -> RoutingView {
    let (kind, provider, model) = match route {
        FinancierRoute::OnDevice => ("on_device", None, None),
        FinancierRoute::LocalOllama { model } => (
            "local_ollama",
            Some("ollama".to_string()),
            Some(model.clone()),
        ),
        FinancierRoute::Cloud { provider, model } => {
            ("cloud", Some(provider.clone()), Some(model.clone()))
        }
        FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent)
        | FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider) => {
            ("refused", None, None)
        }
    };
    RoutingView {
        kind,
        provider,
        model,
        is_local: route.is_local(),
        statement: financier::route_statement(route),
        cloud_allowed,
        cloud_consent_key: financier::FINANCIER_ALLOW_CLOUD_KEY,
        enabled,
    }
}

async fn get_routing() -> Json<RoutingView> {
    let config = Config::global();
    let route = financier::resolve_live_financier_route(config).await;
    let cloud_allowed = financier::cloud_consent(config);
    Json(routing_view(&route, cloud_allowed, financier::is_enabled()))
}

#[derive(Debug, Deserialize)]
pub struct QuoteQuery {
    pub symbol: String,
}

/// One live quote, or the reason there is not one.
///
/// A refusal at the data boundary (sovereign mode on, or the egress audit
/// unwritable) arrives here as an `Err` from `market_data` carrying the
/// `[sovereign]` prefix, and is passed through verbatim: the user set that
/// boundary and is entitled to be told it fired, rather than seeing a generic
/// "could not load".
async fn get_quote(
    Query(params): Query<QuoteQuery>,
) -> Result<Json<market_data::Quote>, (StatusCode, String)> {
    match market_data::quote(&params.symbol).await {
        Ok(quote) => Ok(Json(quote)),
        // BAD_GATEWAY, not INTERNAL_SERVER_ERROR: the daemon is fine, the
        // upstream data source (or the user's own boundary) is what stopped
        // this. The distinction is what tells a reader whether to look at
        // their settings or at our logs.
        Err(reason) => Err((StatusCode::BAD_GATEWAY, reason)),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/finance/routing", get(get_routing))
        .route("/api/finance/quote", get(get_quote))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every route variant renders a distinct `kind`, and `is_local` tracks the
    /// sovereignty layer's own answer rather than a second opinion. A view that
    /// labelled a cloud route local would be the worst possible bug on this
    /// surface: it is the exact claim the user is relying on.
    #[test]
    fn the_view_never_calls_a_cloud_route_local() {
        let cloud = routing_view(
            &FinancierRoute::Cloud {
                provider: "some-provider".to_string(),
                model: "some-model".to_string(),
            },
            true,
            true,
        );
        assert_eq!(cloud.kind, "cloud");
        assert!(!cloud.is_local);
        assert_eq!(cloud.provider.as_deref(), Some("some-provider"));

        for route in [
            FinancierRoute::OnDevice,
            FinancierRoute::LocalOllama {
                model: "qwen3".to_string(),
            },
            FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent),
        ] {
            let view = routing_view(&route, false, true);
            assert!(view.is_local, "{route:?} must not be reported as egress");
        }
    }

    /// The client is handed the key name rather than knowing it. If the key is
    /// ever renamed, the toggle follows it instead of silently writing a key
    /// nothing reads — the failure PR #1052 built the `gate.config_key` field
    /// to prevent.
    #[test]
    fn the_view_carries_the_consent_key_rather_than_assuming_it() {
        let view = routing_view(&FinancierRoute::OnDevice, false, true);
        assert_eq!(view.cloud_consent_key, financier::FINANCIER_ALLOW_CLOUD_KEY);
    }

    /// "Where it would run" and "is it switched on" are separate facts, and the
    /// view reports both. Collapsing them would let the surface imply the
    /// Financier is working when its extension is off.
    #[test]
    fn enabled_is_reported_independently_of_the_route() {
        let off = routing_view(&FinancierRoute::OnDevice, false, false);
        assert!(!off.enabled);
        assert_eq!(off.kind, "on_device");
    }

    /// Each refusal still produces a sentence with the reason in it. An empty
    /// or generic statement is what turns "nothing happened" into a mystery.
    #[test]
    fn a_refusal_explains_itself() {
        let no_consent = routing_view(
            &FinancierRoute::Refused(FinancierRefusal::NoLocalRouteAndNoCloudConsent),
            false,
            true,
        );
        assert_eq!(no_consent.kind, "refused");
        assert!(no_consent.statement.contains("local"));

        let no_provider = routing_view(
            &FinancierRoute::Refused(FinancierRefusal::CloudConsentedButNoProvider),
            true,
            true,
        );
        assert!(no_provider.statement.contains("provider"));
    }
}
