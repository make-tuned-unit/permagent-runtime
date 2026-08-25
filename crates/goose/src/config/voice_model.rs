//! The VOICE model — which model answers a spoken turn.
//!
//! A voice turn wants something a chat turn does not: the first spoken syllable,
//! fast. `docs/research/VOICE_LATENCY_AND_ORB_2026-08-25.md` measured the live
//! path at a 7.4 s median time-to-first-token — 73 % of a 10.6 s
//! speech-end→first-audio — because the session model is a reasoning model that
//! thinks before every spoken word, including one-line social replies.
//! `docs/research/VOICE_MODEL_BENCH_2026-08-25.md` then measured the
//! alternatives on the real prompt and the real 124 tool schemas. This module is
//! where its answer is configured.
//!
//! ## The rules, and why
//!
//! - **There IS a default here, and it is deliberate.** Unlike
//!   [`crate::cost_router::role_map`] — which routes delegated work and must never
//!   pick a vendor on the user's behalf — the voice path has a measured winner and
//!   a user who is waiting out loud. [`default_voice_model`] is that winner, and
//!   [`resolve_voice_model`] reports [`VoiceModelSource::Default`] when it applies
//!   so the log says plainly which model is speaking and why.
//! - **Setting both keys overrides it.** Provider and model together, or not at
//!   all: a half-configured pair falls back to the default and says so
//!   ([`VoiceModelSource::HalfConfigured`]), because routing a spoken turn to a
//!   provider with no model fails in the middle of someone talking.
//! - **`session` turns it off.** Either key set to `session` (or `off` / `none`)
//!   means "answer voice on the session model", which is the pre-bench behaviour.
//!   Without this there would be no way back to one model for everything.
//! - **Voice only.** These keys are read on the voice reply path and nowhere else.
//!   The chat path keeps `GOOSE_PROVIDER`/`GOOSE_MODEL`, untouched.
//!
//! Pure core plus a thin IO wrapper, so the precedence logic is unit-testable
//! without the process-global config.

use serde::{Deserialize, Serialize};

/// Config key holding the voice provider id (e.g. `custom_deepseek`).
pub const VOICE_PROVIDER_KEY: &str = "voice_provider";

/// Config key holding the voice model id (e.g. `deepseek-chat`).
pub const VOICE_MODEL_KEY: &str = "voice_model";

/// Values that mean "no separate voice model — use the session model".
const DISABLE_VALUES: &[&str] = &["session", "off", "none"];

/// A resolved voice route: the concrete provider+model a spoken turn goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceModel {
    pub provider: String,
    pub model: String,
}

/// Provider id of the measured default. See [`default_voice_model`].
pub const DEFAULT_VOICE_PROVIDER_ID: &str = "custom_deepseek";
/// Model id of the measured default. See [`default_voice_model`].
pub const DEFAULT_VOICE_MODEL_ID: &str = "deepseek-chat";

/// The bench winner (2026-08-25). Of five candidates measured on the real prompt
/// and the real 124 tool schemas, it was the only one that did not emit a
/// reasoning block on a single turn — and the only fast one. Warm: a 1.58 s
/// median time-to-first-token with a 1.89 s p90, against 3.20 s / 4.36 s for the
/// MiniMax reasoning model the voice path used before and 4.05 s / 11.34 s for
/// the cheapest alternative. It also went silent on only 1 turn in 20 (others: 5
/// to 7), speaking while it called the tool, which is the larger perceived
/// latency win. Quality, judged blind by a model from none of their families,
/// came out nominally ahead. See
/// `docs/research/VOICE_MODEL_BENCH_2026-08-25.md`.
pub fn default_voice_model() -> VoiceModel {
    VoiceModel {
        provider: DEFAULT_VOICE_PROVIDER_ID.to_string(),
        model: DEFAULT_VOICE_MODEL_ID.to_string(),
    }
}

/// Where a resolved voice route came from — carried so the caller can log it
/// rather than leaving the operator to guess which model answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceModelSource {
    /// Both keys were set by the operator.
    Configured,
    /// Nothing was set; the measured default applies.
    Default,
    /// Exactly one key was set. The default applies and the caller should WARN —
    /// this is a typo or an unfinished edit, not an intention.
    HalfConfigured,
}

/// Pure: resolve the voice route from a key reader.
///
/// `None` means the operator turned the voice model off (`session` / `off` /
/// `none`) and the spoken turn should run on the session model.
pub fn resolve_voice_model(
    read: impl Fn(&str) -> Option<String>,
) -> Option<(VoiceModel, VoiceModelSource)> {
    let value = |key: &str| {
        read(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let provider = value(VOICE_PROVIDER_KEY);
    let model = value(VOICE_MODEL_KEY);

    let disabled = |v: &Option<String>| {
        v.as_ref()
            .is_some_and(|v| DISABLE_VALUES.contains(&v.to_lowercase().as_str()))
    };
    if disabled(&provider) || disabled(&model) {
        return None;
    }

    match (provider, model) {
        (Some(provider), Some(model)) => {
            Some((VoiceModel { provider, model }, VoiceModelSource::Configured))
        }
        (None, None) => Some((default_voice_model(), VoiceModelSource::Default)),
        _ => Some((default_voice_model(), VoiceModelSource::HalfConfigured)),
    }
}

/// The voice route, read from the process-global config
/// (`~/.permagent/config.yaml`, or the matching environment variables).
pub fn voice_model_from_config() -> Option<(VoiceModel, VoiceModelSource)> {
    let config = crate::config::Config::global();
    resolve_voice_model(|key| config.get_param::<String>(key).ok())
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
    fn nothing_configured_uses_the_measured_default() {
        let (route, source) = resolve_voice_model(reader(&[])).expect("default applies");
        assert_eq!(route, default_voice_model());
        assert_eq!(source, VoiceModelSource::Default);
    }

    #[test]
    fn both_keys_set_overrides_the_default() {
        let (route, source) = resolve_voice_model(reader(&[
            (VOICE_PROVIDER_KEY, "minimax"),
            (VOICE_MODEL_KEY, "MiniMax-M2.7-highspeed"),
        ]))
        .expect("configured route");
        assert_eq!(
            route,
            VoiceModel {
                provider: "minimax".to_string(),
                model: "MiniMax-M2.7-highspeed".to_string(),
            }
        );
        assert_eq!(source, VoiceModelSource::Configured);
    }

    #[test]
    fn a_half_configured_pair_falls_back_to_the_default_and_says_so() {
        for pairs in [
            vec![(VOICE_PROVIDER_KEY, "anthropic")],
            vec![(VOICE_MODEL_KEY, "claude-haiku-4-5-20251001")],
        ] {
            let (route, source) = resolve_voice_model(reader(&pairs)).expect("default applies");
            assert_eq!(route, default_voice_model());
            assert_eq!(source, VoiceModelSource::HalfConfigured);
        }
    }

    #[test]
    fn session_off_and_none_turn_the_voice_model_off() {
        for value in ["session", "off", "none", "SESSION", " Off "] {
            assert_eq!(
                resolve_voice_model(reader(&[(VOICE_MODEL_KEY, value)])),
                None,
                "{value} should disable the voice model"
            );
            assert_eq!(
                resolve_voice_model(reader(&[(VOICE_PROVIDER_KEY, value)])),
                None,
                "{value} should disable the voice model"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_values_are_unset_not_a_provider_named_space() {
        let (route, source) = resolve_voice_model(reader(&[
            (VOICE_PROVIDER_KEY, "   "),
            (VOICE_MODEL_KEY, ""),
        ]))
        .expect("default applies");
        assert_eq!(route, default_voice_model());
        assert_eq!(source, VoiceModelSource::Default);
    }

    #[test]
    fn values_are_trimmed_so_a_pasted_id_still_routes() {
        let (route, _) = resolve_voice_model(reader(&[
            (VOICE_PROVIDER_KEY, "  minimax \n"),
            (VOICE_MODEL_KEY, " MiniMax-M2.7-highspeed "),
        ]))
        .expect("configured route");
        assert_eq!(route.provider, "minimax");
        assert_eq!(route.model, "MiniMax-M2.7-highspeed");
    }

    /// Round-trip through the real config layer, not just the pure resolver: the
    /// keys the Settings UI writes are the keys the voice path reads. The test
    /// binary's config root is pinned to a temp dir by `config::base`'s ctor, so
    /// this never touches a developer's `~/.permagent/config.yaml`.
    #[test]
    fn config_round_trip_writes_and_reads_the_same_two_keys() {
        let config = crate::config::Config::global();

        config.set_param(VOICE_PROVIDER_KEY, "minimax").unwrap();
        config
            .set_param(VOICE_MODEL_KEY, "MiniMax-M2.7-highspeed")
            .unwrap();
        let (route, source) = voice_model_from_config().expect("configured route");
        assert_eq!(route.provider, "minimax");
        assert_eq!(route.model, "MiniMax-M2.7-highspeed");
        assert_eq!(source, VoiceModelSource::Configured);

        config.set_param(VOICE_MODEL_KEY, "session").unwrap();
        assert_eq!(
            voice_model_from_config(),
            None,
            "`session` written through the config layer must turn the voice model off"
        );

        config.delete(VOICE_PROVIDER_KEY).unwrap();
        config.delete(VOICE_MODEL_KEY).unwrap();
        let (route, source) = voice_model_from_config().expect("default applies once cleared");
        assert_eq!(route, default_voice_model());
        assert_eq!(source, VoiceModelSource::Default);
    }

    #[test]
    fn the_default_is_the_bench_winner() {
        // Guards the doc: if someone changes the default they must change the
        // research note that justifies it.
        assert_eq!(default_voice_model().provider, "custom_deepseek");
        assert_eq!(default_voice_model().model, "deepseek-chat");
    }
}
