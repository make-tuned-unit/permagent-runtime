//! Per-role model defaults — which model answers a CHAT turn, and which one
//! runs the CODING HARNESS.
//!
//! Permagent had one model knob (`GOOSE_PROVIDER` / `GOOSE_MODEL`, "the session
//! model") and three jobs with genuinely different shapes:
//!
//! - **Voice** wants the first spoken syllable fast. Measured in
//!   `docs/research/VOICE_MODEL_BENCH_2026-08-25.md`; configured by
//!   [`crate::config::voice_model`] (`voice_provider` / `voice_model`).
//! - **Chat** wants the first *written* word fast without giving up answer
//!   quality — a person is watching a cursor blink, not waiting out loud.
//! - **The coding harness** wants the highest pass rate per dollar. Latency
//!   barely matters across a ten-minute agent loop; a wrong answer costs a
//!   re-run and a right one is worth paying for.
//!
//! `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md` measured chat and harness
//! the way the voice bench measured voice. This module is where those two
//! answers are configured, and it is deliberately ONE module with a role enum
//! rather than three near-identical files: Chat, Harness — and Voice, once
//! `voice_model.rs` is folded in here — are the same concept seen three times,
//! and the Settings UI shows them as three rows of one table.
//!
//! ## Precedence, and why it differs from voice
//!
//! ```text
//! CLI --provider/--model        (one run, wins outright)
//!   > the resumed session's own saved model
//!   > the recipe's `settings:` block
//!   > <role>_provider + <role>_model          <- this module
//!   > GOOSE_PROVIDER + GOOSE_MODEL            <- the session model
//!   > the measured default for the role       <- this module
//! ```
//!
//! Note where the measured default sits: **below** the session model, not above
//! it. [`crate::config::voice_model`] puts the voice default *above*
//! `GOOSE_MODEL`, and that is right for voice — a spoken turn on a reasoning
//! model is a 10-second silence, so the bench winner should apply even to
//! someone who has set a session model and never thought about voice.
//!
//! Chat and the harness are not like that. The session model IS the chat model
//! today; someone who set `GOOSE_MODEL` chose the model that answers them, and
//! silently outranking that choice with a benchmark result would be changing a
//! setting the user made without asking. So:
//!
//! - **Nothing configured at all** (a fresh install) → the measured default
//!   applies, reported as [`RoleModelSource::Default`]. New users get the bench
//!   winner.
//! - **A session model is set, no role keys** → the session model, reported as
//!   [`RoleModelSource::SessionModel`]. An existing choice is left alone.
//! - **Both role keys set** → that route, [`RoleModelSource::Configured`].
//! - **Exactly one role key set** → [`RoleModelSource::HalfConfigured`]: the
//!   caller WARNs and resolution continues as if neither were set. Half a pair
//!   is a typo, not an intention, and routing to a provider with no model fails
//!   in the middle of the work.
//! - **`session` / `off` / `none`** in either key → [`RoleModelSource::Disabled`]
//!   with no route: run the role on the session model, which is the pre-bench
//!   behaviour and the way back to one model for everything.
//!
//! Pure core plus a thin IO wrapper, so the precedence logic is unit-testable
//! without the process-global config.

use serde::{Deserialize, Serialize};

/// Config key holding the session provider — the model everything falls back to.
pub const SESSION_PROVIDER_KEY: &str = "GOOSE_PROVIDER";
/// Config key holding the session model.
pub const SESSION_MODEL_KEY: &str = "GOOSE_MODEL";

/// Values that mean "no separate model for this role — use the session model".
const DISABLE_VALUES: &[&str] = &["session", "off", "none"];

/// A job with its own model default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// The daemon's chat reply path — Aria answering a typed turn.
    Chat,
    /// The coding harness (`permagent run --recipe permagent-coding`).
    Harness,
}

impl ModelRole {
    /// The config key holding this role's provider id.
    pub fn provider_key(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat_provider",
            ModelRole::Harness => "harness_provider",
        }
    }

    /// The config key holding this role's model id.
    pub fn model_key(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat_model",
            ModelRole::Harness => "harness_model",
        }
    }

    /// Human label used in logs and in Settings.
    pub fn label(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat",
            ModelRole::Harness => "harness",
        }
    }

    /// The measured winner for this role. Applies only when the user has
    /// configured no model at all — see the module docs on precedence.
    ///
    /// Both numbers below come from
    /// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`; if you change a value
    /// here, change it there too, with the measurement that justifies it.
    pub fn measured_default(self) -> RoleModel {
        match self {
            ModelRole::Chat => RoleModel::new(DEFAULT_CHAT_PROVIDER_ID, DEFAULT_CHAT_MODEL_ID),
            ModelRole::Harness => {
                RoleModel::new(DEFAULT_HARNESS_PROVIDER_ID, DEFAULT_HARNESS_MODEL_ID)
            }
        }
    }

    /// Every role, for callers that render all of them (Settings, `permagent info`).
    pub fn all() -> [ModelRole; 2] {
        [ModelRole::Chat, ModelRole::Harness]
    }
}

/// Provider id of the chat default. See
/// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`.
pub const DEFAULT_CHAT_PROVIDER_ID: &str = "anthropic";
/// Model id of the chat default: Claude Haiku 4.5. It does not emit a reasoning
/// block before a one-line social reply, has full prompt-cache support on the
/// path the daemon already uses, and the chat bench confirmed it against the
/// stated bar (p90 time-to-first-token under 2.5 s at no meaningful quality
/// cost). The voice path lands on the same model in #1116, which is the point:
/// one model answers the person in front of you, whether they typed or spoke.
pub const DEFAULT_CHAT_MODEL_ID: &str = "claude-haiku-4-5-20251001";

/// Provider id of the measured coding-harness default. See
/// `docs/research/MODEL_DEFAULTS_BENCH_2026-08-25.md`.
pub const DEFAULT_HARNESS_PROVIDER_ID: &str = "anthropic";
/// Model id of the measured coding-harness default: Claude Haiku 4.5.
///
/// Six candidates ran the real harness on the same tasks. Every one of them
/// solved every task, so pass rate decided nothing at this sample size; cost per
/// solved task and wall time decided it. Haiku was cheapest ($0.25/solved
/// against $0.27 for gpt-5.4-mini, $0.32 for deepseek, $0.69 for the incumbent
/// GLM-5.3 and $1.16 for Sonnet 5) AND fastest by 2–12x (41 s median against
/// 90 s, 84 s, 489 s and 355 s). On the hardest task measured — an API signature
/// change threaded through two call sites — it was 25 s and $0.14 while GLM-5.3
/// took 401 s.
///
/// **This default is provisional and says so on purpose.** Haiku is the only
/// candidate whose prompt cache works on the harness path today (79% cache-read
/// share; the OpenAI-format candidates got 0% because of a prefix-stability bug
/// fixed separately). Corrected for that bug, deepseek and gpt-5.4-mini project
/// CHEAPER than Haiku. Re-run the bench once that fix lands before treating this
/// as settled.
pub const DEFAULT_HARNESS_MODEL_ID: &str = "claude-haiku-4-5-20251001";

/// A resolved route: the concrete provider+model a role's turn goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModel {
    pub provider: String,
    pub model: String,
}

impl RoleModel {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// Where a resolved route came from — carried so the caller can log it rather
/// than leaving the operator to guess which model answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleModelSource {
    /// Both role keys were set by the operator.
    Configured,
    /// No role keys; the session model (`GOOSE_*`) applies.
    SessionModel,
    /// Nothing configured anywhere; the measured default applies.
    Default,
    /// Exactly one role key was set. Resolution continued as if neither were,
    /// and the caller should WARN — this is a typo or an unfinished edit.
    HalfConfigured,
    /// A role key held `session` / `off` / `none`: run on the session model.
    Disabled,
}

impl RoleModelSource {
    /// Whether the caller should emit a warning about a misconfiguration.
    pub fn should_warn(self) -> bool {
        matches!(self, RoleModelSource::HalfConfigured)
    }
}

/// The outcome of resolving one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleResolution {
    /// The route to use, or `None` when the caller should leave the session's
    /// existing model alone (nothing role-specific applies).
    pub route: Option<RoleModel>,
    /// How [`Self::route`] was arrived at.
    pub source: RoleModelSource,
}

/// Pure: resolve one role's route from a key reader.
///
/// `read` is given raw config keys — the role's two keys and, when they do not
/// decide it, [`SESSION_PROVIDER_KEY`] / [`SESSION_MODEL_KEY`].
pub fn resolve_role_model(
    role: ModelRole,
    read: impl Fn(&str) -> Option<String>,
) -> RoleResolution {
    let value = |key: &str| {
        read(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let is_disable = |v: &Option<String>| v.as_ref().is_some_and(|v| is_disable_value(v.as_str()));

    let provider = value(role.provider_key());
    let model = value(role.model_key());

    if is_disable(&provider) || is_disable(&model) {
        return RoleResolution {
            route: None,
            source: RoleModelSource::Disabled,
        };
    }

    let half_configured = match (&provider, &model) {
        (Some(provider), Some(model)) => {
            return RoleResolution {
                route: Some(RoleModel::new(provider.clone(), model.clone())),
                source: RoleModelSource::Configured,
            }
        }
        (None, None) => false,
        _ => true,
    };

    // No usable role keys. The session model is an explicit user choice and
    // outranks the bench; only a machine with nothing configured at all falls
    // through to the measured default.
    let session_provider = value(SESSION_PROVIDER_KEY);
    let session_model = value(SESSION_MODEL_KEY);
    let session_is_set = session_provider.is_some() && session_model.is_some();

    if half_configured {
        return RoleResolution {
            route: (!session_is_set).then(|| role.measured_default()),
            source: RoleModelSource::HalfConfigured,
        };
    }
    if session_is_set {
        return RoleResolution {
            route: None,
            source: RoleModelSource::SessionModel,
        };
    }
    RoleResolution {
        route: Some(role.measured_default()),
        source: RoleModelSource::Default,
    }
}

fn is_disable_value(value: &str) -> bool {
    DISABLE_VALUES.contains(&value.trim().to_lowercase().as_str())
}

/// One role's route, read from the process-global config
/// (`~/.permagent/config.yaml`, or the matching environment variables).
pub fn role_model_from_config(role: ModelRole) -> RoleResolution {
    let config = crate::config::Config::global();
    resolve_role_model(role, |key| config.get_param::<String>(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn reader(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn a_fresh_install_gets_the_measured_default() {
        for role in ModelRole::all() {
            let resolved = resolve_role_model(role, reader(&[]));
            assert_eq!(resolved.source, RoleModelSource::Default);
            assert_eq!(resolved.route, Some(role.measured_default()));
        }
    }

    #[test]
    fn an_explicit_session_model_outranks_the_measured_default() {
        // The whole point of the precedence choice: a user who set GOOSE_MODEL
        // chose their model, and a benchmark does not get to overrule that.
        for role in ModelRole::all() {
            let resolved = resolve_role_model(
                role,
                reader(&[
                    (SESSION_PROVIDER_KEY, "zai"),
                    (SESSION_MODEL_KEY, "glm-5.3"),
                ]),
            );
            assert_eq!(resolved.source, RoleModelSource::SessionModel);
            assert_eq!(
                resolved.route,
                None,
                "{}: the caller must be left to use the session model",
                role.label()
            );
        }
    }

    #[test]
    fn both_role_keys_set_outrank_the_session_model() {
        let resolved = resolve_role_model(
            ModelRole::Harness,
            reader(&[
                ("harness_provider", "anthropic"),
                ("harness_model", "claude-sonnet-5"),
                (SESSION_PROVIDER_KEY, "zai"),
                (SESSION_MODEL_KEY, "glm-5.3"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Configured);
        assert_eq!(
            resolved.route,
            Some(RoleModel::new("anthropic", "claude-sonnet-5"))
        );
    }

    #[test]
    fn the_roles_do_not_read_each_others_keys() {
        let resolved = resolve_role_model(
            ModelRole::Chat,
            reader(&[
                ("harness_provider", "anthropic"),
                ("harness_model", "claude-sonnet-5"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn a_half_configured_pair_warns_and_resolves_as_if_unset() {
        for role in ModelRole::all() {
            for key in [role.provider_key(), role.model_key()] {
                let resolved = resolve_role_model(role, reader(&[(key, "anthropic")]));
                assert_eq!(
                    resolved.source,
                    RoleModelSource::HalfConfigured,
                    "{}: {key} alone",
                    role.label()
                );
                assert!(resolved.source.should_warn());
                assert_eq!(resolved.route, Some(role.measured_default()));

                // …and with a session model set, half a pair falls back to it
                // rather than to the bench winner.
                let with_session = resolve_role_model(
                    role,
                    reader(&[
                        (key, "anthropic"),
                        (SESSION_PROVIDER_KEY, "zai"),
                        (SESSION_MODEL_KEY, "glm-5.3"),
                    ]),
                );
                assert_eq!(with_session.source, RoleModelSource::HalfConfigured);
                assert_eq!(with_session.route, None);
            }
        }
    }

    #[test]
    fn session_off_and_none_pin_the_role_to_the_session_model() {
        for role in ModelRole::all() {
            for value in ["session", "off", "none", "SESSION", " Off "] {
                for key in [role.provider_key(), role.model_key()] {
                    let resolved = resolve_role_model(role, reader(&[(key, value)]));
                    assert_eq!(
                        resolved,
                        RoleResolution {
                            route: None,
                            source: RoleModelSource::Disabled,
                        },
                        "{}: {key}={value:?} should disable the role override",
                        role.label()
                    );
                }
            }
        }
    }

    #[test]
    fn a_disable_value_beats_a_fully_configured_partner_key() {
        // `harness_provider: session` with a stale model id left behind means
        // "back to one model for everything", not "route to the stale id".
        let resolved = resolve_role_model(
            ModelRole::Harness,
            reader(&[
                ("harness_provider", "session"),
                ("harness_model", "claude-sonnet-5"),
            ]),
        );
        assert_eq!(resolved.source, RoleModelSource::Disabled);
        assert_eq!(resolved.route, None);
    }

    #[test]
    fn empty_and_whitespace_values_are_unset_not_a_provider_named_space() {
        let resolved = resolve_role_model(
            ModelRole::Chat,
            reader(&[("chat_provider", "   "), ("chat_model", "")]),
        );
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn a_half_set_session_model_is_not_a_session_model() {
        // GOOSE_PROVIDER without GOOSE_MODEL cannot route anything, so it must
        // not suppress the measured default.
        let resolved =
            resolve_role_model(ModelRole::Chat, reader(&[(SESSION_PROVIDER_KEY, "zai")]));
        assert_eq!(resolved.source, RoleModelSource::Default);
        assert_eq!(resolved.route, Some(ModelRole::Chat.measured_default()));
    }

    #[test]
    fn role_keys_are_distinct_and_stable() {
        let keys: Vec<&str> = ModelRole::all()
            .iter()
            .flat_map(|r| [r.provider_key(), r.model_key()])
            .collect();
        assert_eq!(
            keys,
            vec![
                "chat_provider",
                "chat_model",
                "harness_provider",
                "harness_model"
            ]
        );
    }
}
