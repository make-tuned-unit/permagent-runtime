//! Chime-in consent — the privacy surface for ambient recognition.
//!
//! Config schema + plumbing ONLY (no UX yet). Design follows the smart-TV ACR
//! lesson: ambient recognition must be opt-in per scope, visibly on or off,
//! and excludable per source — and unlike ACR it is local-only and auditable
//! by construction (every signal lands in `recognition_events` in
//! `~/.permagent/spectral/permagent.db`, never off the machine).
//!
//! Shape (YAML under the `recognition_consent` key in config.yaml, readable
//! and writable through the existing generic `/config` routes — no new
//! endpoint needed):
//!
//! ```yaml
//! recognition_consent:
//!   active: true                  # master switch — the visible "recognition active" state
//!   excluded_sources: [browser]   # global per-source exclusions (source surfaces)
//!   wings:
//!     permagent:
//!       ambient: true             # per-wing OPT-IN (absent wing = not consented)
//!       excluded_sources: []      # per-wing source exclusions
//! ```
//!
//! Defaults are all-off: `active` is false and the wing map is empty, so
//! ambient recognition is fully opt-in. Wings are project slugs — the same
//! labels `activity::ingestion` stamps on ambient memories and
//! [`crate::wing_rules`] generates classifier rules for.
//!
//! This module always compiles (it is pure config plumbing with no side
//! effects); the only production consumer is the `spectral-recognition`-gated
//! sink seam, so a default build's behavior is unchanged.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::Config;

/// Config key under which the consent block lives.
pub const RECOGNITION_CONSENT_KEY: &str = "recognition_consent";

/// Per-wing opt-in scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WingScope {
    /// Opt-in for ambient (stream-mode) recognition in this wing.
    #[serde(default)]
    pub ambient: bool,
    /// Source surfaces excluded within this wing (e.g. "browser", "terminal").
    #[serde(default)]
    pub excluded_sources: Vec<String>,
}

/// The consent block. `Default` is all-off: recognition inactive, no wings
/// consented — ambient recognition is strictly opt-in.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecognitionConsentConfig {
    /// Master switch — the user-visible "recognition active" state.
    #[serde(default)]
    pub active: bool,
    /// Source surfaces excluded across ALL wings.
    #[serde(default)]
    pub excluded_sources: Vec<String>,
    /// Per-wing opt-in scopes, keyed by wing slug. A wing absent from this
    /// map is NOT consented.
    #[serde(default)]
    pub wings: BTreeMap<String, WingScope>,
}

/// Load the consent block from config. Absent or malformed → all-off default.
pub fn load() -> RecognitionConsentConfig {
    Config::global()
        .get_param(RECOGNITION_CONSENT_KEY)
        .unwrap_or_default()
}

/// Persist the consent block to config.yaml.
pub fn store(cfg: &RecognitionConsentConfig) -> Result<(), crate::config::ConfigError> {
    Config::global().set_param(RECOGNITION_CONSENT_KEY, cfg)
}

/// The visible "recognition active" state (master switch only — a true here
/// with an empty wing map still consents to nothing).
pub fn recognition_active() -> bool {
    load().active
}

/// Whether an ambient cue from `source_surface`, scoped to `wing`, may enter
/// the recognition stream. This is the single choke point the sink seam
/// consults; the rules are:
///
/// 1. master switch must be on;
/// 2. the source must not be globally excluded;
/// 3. the cue must carry a wing (unscoped ambient activity is never
///    recognized — privacy-conservative);
/// 4. that wing must be explicitly opted in, and must not exclude the source.
pub fn ambient_cue_allowed(wing: Option<&str>, source_surface: &str) -> bool {
    let cfg = load();
    ambient_cue_allowed_with(&cfg, wing, source_surface)
}

/// Pure-decision variant for callers that already hold the config (and tests).
pub fn ambient_cue_allowed_with(
    cfg: &RecognitionConsentConfig,
    wing: Option<&str>,
    source_surface: &str,
) -> bool {
    if !cfg.active {
        return false;
    }
    if cfg.excluded_sources.iter().any(|s| s == source_surface) {
        return false;
    }
    let Some(wing) = wing else {
        return false;
    };
    match cfg.wings.get(wing) {
        Some(scope) => scope.ambient && !scope.excluded_sources.iter().any(|s| s == source_surface),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consented(wing: &str) -> RecognitionConsentConfig {
        let mut cfg = RecognitionConsentConfig {
            active: true,
            ..Default::default()
        };
        cfg.wings.insert(
            wing.to_string(),
            WingScope {
                ambient: true,
                excluded_sources: vec![],
            },
        );
        cfg
    }

    #[test]
    fn default_is_all_off() {
        let cfg = RecognitionConsentConfig::default();
        assert!(!cfg.active);
        assert!(!ambient_cue_allowed_with(
            &cfg,
            Some("permagent"),
            "browser"
        ));
    }

    #[test]
    fn opt_in_wing_allows_and_others_stay_denied() {
        let cfg = consented("permagent");
        assert!(ambient_cue_allowed_with(&cfg, Some("permagent"), "browser"));
        assert!(!ambient_cue_allowed_with(&cfg, Some("getladle"), "browser"));
    }

    #[test]
    fn unscoped_ambient_is_never_recognized() {
        let cfg = consented("permagent");
        assert!(!ambient_cue_allowed_with(&cfg, None, "browser"));
    }

    #[test]
    fn master_switch_overrides_wing_opt_in() {
        let mut cfg = consented("permagent");
        cfg.active = false;
        assert!(!ambient_cue_allowed_with(
            &cfg,
            Some("permagent"),
            "browser"
        ));
    }

    #[test]
    fn source_exclusions_apply_globally_and_per_wing() {
        let mut cfg = consented("permagent");
        cfg.excluded_sources = vec!["terminal".into()];
        assert!(!ambient_cue_allowed_with(
            &cfg,
            Some("permagent"),
            "terminal"
        ));
        assert!(ambient_cue_allowed_with(&cfg, Some("permagent"), "browser"));

        cfg.wings.get_mut("permagent").unwrap().excluded_sources = vec!["browser".into()];
        assert!(!ambient_cue_allowed_with(
            &cfg,
            Some("permagent"),
            "browser"
        ));
    }

    #[test]
    fn round_trips_through_yaml() {
        let cfg = consented("permagent");
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let back: RecognitionConsentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(cfg, back);
    }
}
