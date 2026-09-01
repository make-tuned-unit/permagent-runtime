//! What to do when the current model is PERMANENTLY rejected.
//!
//! Session 20260823_4 (2026-08-23): Anthropic returned HTTP 400
//! `invalid_request_error` — "Your credit balance is too low to access the
//! Anthropic API" — on every request. Three things went wrong at once, and this
//! module fixes the third:
//!
//! 1. The retry layer retried it 3/3 times ([`crate::providers::retry`] now
//!    classifies it permanent and stops at one attempt).
//! 2. It was typed as a generic `RequestFailed`, which routed it around the
//!    safe `CreditsExhausted` reply path (fixed in
//!    `providers::openai_compatible::map_http_error_to_provider_error`).
//! 3. **Nothing switched models.** The user noticed the failure themself and
//!    manually moved to DeepSeek. A depleted balance is the one failure where
//!    the *right* recovery is obvious — use a different provider — and the
//!    routing tables already hold concrete provider+model pairs.
//!
//! ## Where a fallback comes from
//!
//! Strict precedence, so the routing tables stay one source of truth:
//!
//! 1. `PERMAGENT_FALLBACK_PROVIDER` + `PERMAGENT_FALLBACK_MODEL` — an explicit
//!    operator override. Both must be set; a half-set pair is treated as unset,
//!    matching [`super::role_map::resolve_role_model`].
//! 2. `PERMAGENT_FALLBACKS` — an ordered `provider/model,provider/model` list.
//!    The first entry whose provider differs from the one that just failed.
//! 3. The first configured role→model mapping (in [`WorkflowRole::all`] order)
//!    whose provider differs from the one that just failed. A role mapped to the
//!    SAME provider is useless here: the same account is out of credit.
//!
//! Permanent (billing/auth) failures skip same-provider list/role entries.
//! Transient pre-commit failures may silent-switch via the same chain
//! ([`may_silent_precommit_failover`]).
//!
//! `None` means "no fallback is configured" — which the reply path must then say
//! plainly rather than retrying into the same wall.

use super::recommend::WorkflowRole;
use super::role_map::{model_key, provider_key, RoleModel};
use crate::providers::errors::ProviderError;

/// Explicit operator override for the provider to fall back to.
pub const KEY_FALLBACK_PROVIDER: &str = "PERMAGENT_FALLBACK_PROVIDER";
/// Explicit operator override for the model to fall back to.
pub const KEY_FALLBACK_MODEL: &str = "PERMAGENT_FALLBACK_MODEL";
/// Ordered `provider/model,provider/model` list. Consulted after the explicit
/// pair, before role mappings. Same-provider entries are skipped for
/// permanent (billing/auth) failures.
pub const KEY_FALLBACKS: &str = "PERMAGENT_FALLBACKS";

/// Parse `provider/model,provider/model`. Segments without `/` or with an
/// empty side are skipped so a typo does not disable the rest of the list.
pub fn parse_fallback_list(raw: &str) -> Vec<RoleModel> {
    raw.split(',')
        .filter_map(|seg| {
            let seg = seg.trim();
            if seg.is_empty() {
                return None;
            }
            let (provider, model) = seg.split_once('/')?;
            let provider = provider.trim();
            let model = model.trim();
            if provider.is_empty() || model.is_empty() {
                return None;
            }
            Some(RoleModel {
                provider: provider.to_string(),
                model: model.to_string(),
            })
        })
        .collect()
}

/// Whether a stream error may silent-switch to the next model: only before any
/// assistant response was committed, and only for transient reachability
/// failures. Billing/auth stay on the announced path.
pub fn may_silent_precommit_failover(committed: bool, err: &ProviderError) -> bool {
    if committed || err.is_permanent() {
        return false;
    }
    matches!(
        err,
        ProviderError::NetworkError(_)
            | ProviderError::ServerError(_)
            | ProviderError::RateLimitExceeded { .. }
    )
}

/// Pure: resolve the fallback for a provider that was permanently rejected.
///
/// `read` is a key reader so this is unit-testable without the process-global
/// config. `failed_provider` is compared case-insensitively; a candidate on the
/// same provider is skipped (same account, same empty balance).
pub fn resolve_permanent_failure_fallback(
    failed_provider: &str,
    read: impl Fn(&str) -> Option<String>,
) -> Option<RoleModel> {
    let non_empty = |k: &str| {
        read(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let differs = |p: &str| !p.eq_ignore_ascii_case(failed_provider.trim());

    // 1. Explicit override wins, even if it names the failed provider — the
    //    operator said so, and a same-provider override is a deliberate choice
    //    (e.g. a different key on a different account).
    if let (Some(provider), Some(model)) = (
        non_empty(KEY_FALLBACK_PROVIDER),
        non_empty(KEY_FALLBACK_MODEL),
    ) {
        return Some(RoleModel { provider, model });
    }

    // 1b. Ordered list — first entry whose provider is genuinely different.
    if let Some(raw) = non_empty(KEY_FALLBACKS) {
        for rm in parse_fallback_list(&raw) {
            if differs(&rm.provider) {
                return Some(rm);
            }
        }
    }

    // 2. First configured role whose provider is genuinely different.
    for role in WorkflowRole::all() {
        let (Some(provider), Some(model)) =
            (non_empty(&provider_key(role)), non_empty(&model_key(role)))
        else {
            continue;
        };
        if differs(&provider) {
            return Some(RoleModel { provider, model });
        }
    }

    None
}

/// Config-backed [`resolve_permanent_failure_fallback`].
pub fn permanent_failure_fallback(failed_provider: &str) -> Option<RoleModel> {
    let cfg = crate::config::Config::global();
    resolve_permanent_failure_fallback(failed_provider, |k| cfg.get_param::<String>(k).ok())
}

/// The ONE sentence the user sees or hears when a model is permanently
/// rejected.
///
/// Built ONLY from [`ProviderError::user_facing_summary`] and the two model
/// names — never from the provider payload. The raw error still goes to the log
/// (`error!` at the call site), where request ids and JSON belong. Session
/// 20260823_4 had the raw body read aloud over TTS; this function is the reason
/// that cannot happen again, and [`tests::reply_never_contains_raw_api_error`]
/// is the reason it stays true.
pub fn permanent_failure_reply(
    failed_provider: &str,
    err: &ProviderError,
    fallback: Option<&RoleModel>,
) -> String {
    let cause = err.user_facing_summary();
    match fallback {
        Some(target) => format!(
            "{failed_provider} rejected the request — {cause}. \
             Switching to {} for the rest of this turn.",
            target.model
        ),
        None => format!(
            "{failed_provider} rejected the request — {cause}, \
             and no fallback model is configured. \
             Check the provider's Plans & Billing page, or pick another model in Settings → Models, \
             then resend your message."
        ),
    }
}

/// A short, payload-free phrase for the class of pre-commit failure that
/// triggered a silent switch. Derived from the error VARIANT, never from its
/// string: `NetworkError`'s own text carries the socket we failed to open
/// (`127.0.0.1:8081`), which is log material, not something to read aloud.
fn precommit_failure_cause(err: &ProviderError) -> &'static str {
    if err.is_stream_decode() {
        return "the model stream dropped before any reply started";
    }
    match err {
        ProviderError::NetworkError(_) => "connection refused or unroutable",
        ProviderError::ServerError(_) => "the provider returned a server error",
        ProviderError::RateLimitExceeded { .. } => "the provider is rate-limiting this model",
        _ => "the request failed before any reply started",
    }
}

/// The ONE sentence for a *silent* pre-commit failover — the arm that used to
/// say nothing at all.
///
/// Session 20260831_10 (2026-08-31): the coding harness resolved
/// `qwen38_split`/`qwen3.8-27b` from the harness-role default, the split's port
/// was dead in the evening, and [`may_silent_precommit_failover`] switched the
/// session to `anthropic/claude-haiku-4-5`. The ONLY trace was one
/// `tracing::warn!` in the daemon log; the transcript, the CLI and TTS heard
/// nothing, and the model went on to self-report an identity it never had.
///
/// Unlike [`permanent_failure_reply`] this path always HAS a target — it is
/// only reached after the switch succeeded — so the sentence states the switch
/// as done, not as an offer. Same payload-free rule
/// ([`tests::precommit_reply_never_contains_the_raw_error`]).
pub fn precommit_failover_reply(
    failed_provider: &str,
    err: &ProviderError,
    fallback: &RoleModel,
) -> String {
    format!(
        "{failed_provider} unreachable ({}) — switched this session to {}/{}.",
        precommit_failure_cause(err),
        fallback.provider,
        fallback.model
    )
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
        move |k: &str| map.get(k).cloned()
    }

    fn billing_error() -> ProviderError {
        ProviderError::CreditsExhausted {
            details: "{\"type\":\"invalid_request_error\",\"message\":\"Your credit balance is \
                      too low to access the Anthropic API. Please go to Plans & Billing to \
                      upgrade or purchase credits.\"}"
                .to_string(),
            top_up_url: Some("https://console.anthropic.com/settings/billing".to_string()),
        }
    }

    #[test]
    fn no_configuration_means_no_fallback() {
        assert_eq!(
            resolve_permanent_failure_fallback("anthropic", reader(&[])),
            None
        );
    }

    #[test]
    fn explicit_override_wins() {
        let r = reader(&[
            (KEY_FALLBACK_PROVIDER, "custom_deepseek"),
            (KEY_FALLBACK_MODEL, "deepseek-v4-flash"),
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "openai"),
            ("PERMAGENT_ROLE_EDIT_MODEL", "gpt-5.4"),
        ]);
        assert_eq!(
            resolve_permanent_failure_fallback("anthropic", r),
            Some(RoleModel {
                provider: "custom_deepseek".into(),
                model: "deepseek-v4-flash".into(),
            })
        );
    }

    /// A half-set override is not a fallback — same rule as `resolve_role_model`.
    #[test]
    fn half_set_override_falls_through_to_roles() {
        let r = reader(&[
            (KEY_FALLBACK_PROVIDER, "custom_deepseek"),
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "openai"),
            ("PERMAGENT_ROLE_EDIT_MODEL", "gpt-5.4"),
        ]);
        assert_eq!(
            resolve_permanent_failure_fallback("anthropic", r),
            Some(RoleModel {
                provider: "openai".into(),
                model: "gpt-5.4".into(),
            })
        );
    }

    /// The whole point: the account that just ran out of credit is not a place
    /// to fall back to.
    #[test]
    fn skips_roles_on_the_failed_provider() {
        let r = reader(&[
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "anthropic"),
            ("PERMAGENT_ROLE_EDIT_MODEL", "claude-haiku-4-5"),
        ]);
        assert_eq!(
            resolve_permanent_failure_fallback("Anthropic", r),
            None,
            "a role on the failed provider must not be offered as a fallback"
        );
    }

    #[test]
    fn blank_values_are_treated_as_unset() {
        let r = reader(&[
            (KEY_FALLBACK_PROVIDER, "   "),
            (KEY_FALLBACK_MODEL, ""),
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "  "),
            ("PERMAGENT_ROLE_EDIT_MODEL", "gpt-5.4"),
        ]);
        assert_eq!(resolve_permanent_failure_fallback("anthropic", r), None);
    }

    /// THE regression for the 2026-08-23 incident: whatever the provider sent,
    /// none of it reaches the user-facing sentence.
    #[test]
    fn reply_never_contains_raw_api_error() {
        let err = billing_error();
        let target = RoleModel {
            provider: "custom_deepseek".into(),
            model: "deepseek-v4-flash".into(),
        };
        for reply in [
            permanent_failure_reply("anthropic", &err, Some(&target)),
            permanent_failure_reply("anthropic", &err, None),
        ] {
            for leaked in [
                "invalid_request_error",
                "console.anthropic.com",
                "https://",
                "{",
                "}",
                "\"type\"",
                "purchase credits",
            ] {
                assert!(
                    !reply.contains(leaked),
                    "user-facing reply leaked {leaked:?}: {reply}"
                );
            }
            assert!(
                !reply.contains("HTTP 400") && !reply.contains("400"),
                "user-facing reply leaked a status code: {reply}"
            );
        }
    }

    #[test]
    fn reply_names_the_fallback_when_one_exists() {
        let target = RoleModel {
            provider: "custom_deepseek".into(),
            model: "deepseek-v4-flash".into(),
        };
        let reply = permanent_failure_reply("anthropic", &billing_error(), Some(&target));
        assert!(reply.contains("anthropic"), "{reply}");
        assert!(reply.contains("deepseek-v4-flash"), "{reply}");
        assert!(reply.contains("credit balance"), "{reply}");
    }

    /// "…no fallback configured — say so plainly", not "please retry".
    #[test]
    fn reply_says_plainly_when_no_fallback_exists() {
        let reply = permanent_failure_reply("anthropic", &billing_error(), None);
        assert!(reply.contains("no fallback model is configured"), "{reply}");
        assert!(reply.contains("Plans & Billing"), "{reply}");
        assert!(
            !reply.to_lowercase().contains("try again"),
            "must not invite a retry into the same wall: {reply}"
        );
    }

    /// An auth rejection uses the same path and is equally payload-free.
    #[test]
    fn auth_error_reply_is_also_clean() {
        let err = ProviderError::Authentication(
            "Authentication failed. Status: 401. Response: {\"error\":{\"message\":\"invalid \
             x-api-key sk-ant-api03-XXXX\"}}"
                .to_string(),
        );
        let reply = permanent_failure_reply("anthropic", &err, None);
        assert!(!reply.contains("sk-ant"), "leaked a key fragment: {reply}");
        assert!(!reply.contains("401"), "{reply}");
        assert!(reply.contains("API key"), "{reply}");
    }

    #[test]
    fn parse_fallback_list_skips_garbage_and_keeps_valid_pairs() {
        let out = parse_fallback_list(" deepseek/deepseek-chat , not-a-pair, /empty, openai/ ");
        assert_eq!(
            out,
            vec![RoleModel {
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
            }]
        );
    }

    #[test]
    fn ordered_list_is_consulted_after_explicit_override() {
        let r = reader(&[
            (KEY_FALLBACKS, "openai/gpt-5.4,ollama/qwen3"),
            ("PERMAGENT_ROLE_EDIT_PROVIDER", "custom_deepseek"),
            ("PERMAGENT_ROLE_EDIT_MODEL", "deepseek-v4-flash"),
        ]);
        assert_eq!(
            resolve_permanent_failure_fallback("anthropic", r),
            Some(RoleModel {
                provider: "openai".into(),
                model: "gpt-5.4".into(),
            })
        );
    }

    #[test]
    fn ordered_list_skips_the_failed_provider() {
        let r = reader(&[(KEY_FALLBACKS, "anthropic/claude-haiku-4-5,openai/gpt-5.4")]);
        assert_eq!(
            resolve_permanent_failure_fallback("anthropic", r),
            Some(RoleModel {
                provider: "openai".into(),
                model: "gpt-5.4".into(),
            })
        );
    }

    #[test]
    fn silent_precommit_only_for_uncommitted_transient_errors() {
        let net = ProviderError::NetworkError("connection reset".into());
        let server = ProviderError::ServerError("503".into());
        let rate = ProviderError::RateLimitExceeded {
            details: "slow down".into(),
            retry_delay: None,
        };
        let stream = ProviderError::stream_decode("error decoding response body");
        assert!(may_silent_precommit_failover(false, &net));
        assert!(may_silent_precommit_failover(false, &server));
        assert!(may_silent_precommit_failover(false, &rate));
        assert!(may_silent_precommit_failover(false, &stream));
        assert!(!may_silent_precommit_failover(true, &stream));
        assert!(!may_silent_precommit_failover(true, &net));
        assert!(!may_silent_precommit_failover(false, &billing_error()));
        assert!(!may_silent_precommit_failover(
            false,
            &ProviderError::Authentication("nope".into())
        ));
    }

    /// M2: the pre-commit failover used to be silent. Session 20260831_10 was
    /// served by `anthropic/claude-haiku-4-5` after `qwen38_split`'s port was
    /// found dead, and NOTHING in the transcript said so.
    #[test]
    fn precommit_reply_names_both_ends_of_the_switch() {
        let err = ProviderError::NetworkError(
            "Could not connect to 127.0.0.1:8081 — check your network connection and try again."
                .to_string(),
        );
        let target = RoleModel {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
        };
        let reply = precommit_failover_reply("qwen38_split", &err, &target);
        assert!(reply.contains("qwen38_split"), "{reply}");
        assert!(reply.contains("anthropic/claude-haiku-4-5"), "{reply}");
        assert!(reply.to_lowercase().contains("unreachable"), "{reply}");
        assert!(
            reply.to_lowercase().contains("switched this session"),
            "{reply}"
        );
    }

    /// Same payload-free guarantee as the announced permanent path: a raw
    /// socket/endpoint string is log material, never TTS material.
    #[test]
    fn precommit_reply_never_contains_the_raw_error() {
        let err = ProviderError::NetworkError(
            "Could not connect to 127.0.0.1:8081 — check your network connection and try again."
                .to_string(),
        );
        let target = RoleModel {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
        };
        let reply = precommit_failover_reply("qwen38_split", &err, &target);
        for leaked in ["127.0.0.1", "8081", "check your network"] {
            assert!(!reply.contains(leaked), "leaked {leaked:?}: {reply}");
        }
    }

    /// The other silent-failover arm shares the sentence, and must not claim a
    /// dropped stream was an unreachable host.
    #[test]
    fn precommit_reply_distinguishes_a_dropped_stream_from_a_dead_host() {
        let target = RoleModel {
            provider: "openai".into(),
            model: "gpt-5.4".into(),
        };
        let stream = precommit_failover_reply(
            "anthropic",
            &ProviderError::stream_decode("error decoding response body"),
            &target,
        );
        assert!(stream.contains("stream"), "{stream}");
        assert!(
            !stream.to_lowercase().contains("connection refused"),
            "{stream}"
        );
    }
}
