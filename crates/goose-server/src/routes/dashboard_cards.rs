//! Dashboard card-type registration + manifest-card data endpoints (issues
//! #182 / #181).
//!
//! ## The registration mechanism (#182)
//!
//! `GET /api/dashboard/card-types` serves the list of **manifest cards** —
//! declarative card definitions the command-center renders with its first-party
//! `ManifestCard` component. A manifest is pure data: it names a data endpoint
//! and one of a constrained set of layouts. No card-specific code ships to the
//! frontend, which keeps the dashboard's extension surface a data boundary
//! rather than a code boundary.
//!
//! Today the manifests are the daemon's own built-ins (see
//! [`builtin_card_manifests`]). The same list is the seam a future skill pack
//! extends: an installed pack contributes a manifest (type, layout, data
//! endpoint) and it appears in the Add-card picker automatically. See
//! `docs/architecture/DASHBOARD_CARD_EXTENSIBILITY.md`.
//!
//! ## The card data endpoints (#181)
//!
//! Each built-in manifest points at a real endpoint in this module that returns
//! the normalized [`CardData`] shape. There are no placeholder cards — every
//! card is backed by a live source (system stats via `sysinfo`, calendar via
//! the macOS AppleScript bridge, weather via the Open-Meteo API).

use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Manifest (registration) types ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardSize {
    pub w: i32,
    pub h: i32,
}

/// Optional inline setup affordance a manifest card exposes in its empty state
/// (e.g. the weather card asking for a location). The frontend PUTs
/// `{ "query": "…" }` to `endpoint`, then refetches the card's data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardConfigure {
    pub endpoint: String,
    pub label: String,
    pub placeholder: String,
}

/// A declarative dashboard-card definition. Serialized camelCase to match the
/// command-center `CardManifest` type; the `card_type` field is emitted as the
/// `type` key (a JS reserved word) via [`manifest_to_json`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardManifest {
    /// Registry key / persisted card `type`.
    pub card_type: String,
    pub name: String,
    pub description: String,
    pub default_size: CardSize,
    /// One of `"stat-grid"`, `"list"`, `"key-value"`.
    pub layout: String,
    /// Endpoint the ManifestCard polls for this card's data.
    pub data_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_seconds: Option<u32>,
    /// Provenance shown in the picker — `"built-in"` or a skill pack's name.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configure: Option<CardConfigure>,
}

// ── Data payload returned by every manifest-card data endpoint ───────────────

/// One datum in a card. Interpreted by layout: `stat-grid`/`key-value` use
/// `label`+`value`(+`delta`/`accent`); `list` uses `label` (title), `sub`
/// (subtitle) and `value` (trailing meta).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CardCell {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accent: bool,
}

/// Normalized response every manifest-card data endpoint returns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CardData {
    #[serde(default)]
    pub cells: Vec<CardCell>,
    /// A subtle empty / permission / error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `Some(false)` ⇒ the card needs setup before it has data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured: Option<bool>,
}

impl CardData {
    #[allow(dead_code)] // used by the #181 data endpoints
    pub fn note(msg: impl Into<String>) -> Self {
        CardData {
            cells: Vec::new(),
            note: Some(msg.into()),
            configured: None,
        }
    }
}

// ── Registration endpoint ────────────────────────────────────────────────────

/// The daemon's built-in card manifests. Populated in #181 with the
/// system-stats, calendar and weather cards. A skill pack's manifests would be
/// appended to this list once the pack registry exists.
pub fn builtin_card_manifests() -> Vec<CardManifest> {
    Vec::new()
}

async fn get_card_types() -> Json<Vec<serde_json::Value>> {
    let values = builtin_card_manifests()
        .iter()
        .map(manifest_to_json)
        .collect();
    Json(values)
}

/// Serialize a manifest with `card_type` rendered as the `type` key.
fn manifest_to_json(m: &CardManifest) -> serde_json::Value {
    let mut v = serde_json::to_value(m).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = v.as_object_mut() {
        if let Some(t) = obj.remove("cardType") {
            obj.insert("type".to_string(), t);
        }
    }
    v
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/dashboard/card-types", get(get_card_types))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_type_serializes_as_type_key() {
        let m = CardManifest {
            card_type: "system_stats".to_string(),
            name: "System".to_string(),
            description: "d".to_string(),
            default_size: CardSize { w: 5, h: 4 },
            layout: "stat-grid".to_string(),
            data_endpoint: "/api/dashboard/system-stats".to_string(),
            refresh_seconds: Some(30),
            source: "built-in".to_string(),
            configure: None,
        };
        let v = manifest_to_json(&m);
        assert_eq!(v["type"], "system_stats");
        assert!(v.get("cardType").is_none());
        assert_eq!(v["dataEndpoint"], "/api/dashboard/system-stats");
        assert_eq!(v["defaultSize"]["w"], 5);
        assert_eq!(v["refreshSeconds"], 30);
    }

    #[test]
    fn card_data_omits_empty_optionals() {
        let d = CardData {
            cells: vec![CardCell {
                label: "CPU".to_string(),
                value: "12%".to_string(),
                ..Default::default()
            }],
            note: None,
            configured: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""label":"CPU""#));
        assert!(!json.contains("note"));
        assert!(!json.contains("configured"));
        assert!(!json.contains("accent"));
        assert!(!json.contains("sub"));
    }

    #[test]
    fn builtin_manifests_have_unique_types_and_valid_layouts() {
        let manifests = builtin_card_manifests();
        let mut seen = std::collections::HashSet::new();
        for m in &manifests {
            assert!(seen.insert(m.card_type.clone()), "dup type {}", m.card_type);
            assert!(
                matches!(m.layout.as_str(), "stat-grid" | "list" | "key-value"),
                "invalid layout {} on {}",
                m.layout,
                m.card_type
            );
            assert!(m.data_endpoint.starts_with("/api/dashboard/"));
        }
    }
}
