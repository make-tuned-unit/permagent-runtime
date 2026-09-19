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
//! ## Only an endpoint that is up
//!
//! The agent's mid-turn switch ([`permanent_failure_fallback_available`])
//! probes the local endpoints the list names — a TCP connect to the split, a
//! GET of Ollama's `/api/tags` — and never picks one that is down. A down LOCAL
//! candidate is replaced, at its own rank, by the best installed Ollama model
//! the knowledge base prices at $0 ([`choose_fallback`]). The selection is a
//! pure function over that snapshot; the probe is the only I/O.
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

/// A streamed body cut off partway through one SSE `data:` line.
///
/// When a proxy or llama-server closes the body cleanly in the middle of a
/// line, the line codec hands the partial line on at EOF and the OpenAI-style
/// chunk parser (`providers::formats::openai::parse_streaming_chunk`) types it
/// `RequestFailed("Failed to parse streaming chunk: EOF while parsing …")`.
/// That is not a transport error, so [`ProviderError::is_stream_decode`] does
/// not match it, and the reply loop sent it to the 4xx arm: the user was told
/// "The provider rejected this request as invalid … Switch the model" for what
/// was a dropped stream, and resending would have worked. Only serde's
/// end-of-input error counts: a complete line that is not a chunk is a real
/// shape mismatch and stays where it was.
pub fn is_cut_off_stream_chunk(err: &ProviderError) -> bool {
    let ProviderError::RequestFailed(msg) = err else {
        return false;
    };
    let m = msg.to_ascii_lowercase();
    m.starts_with("failed to parse streaming chunk") && m.contains("eof while parsing")
}

/// Every shape a dropped response stream arrives in: a transport-level body
/// or decode error, or a chunk cut off mid-line. The agent's stream-drop arm,
/// the failover gate and the failover wording all read this one predicate, so
/// they cannot disagree about what a dropped stream is.
pub fn is_dropped_stream(err: &ProviderError) -> bool {
    err.is_stream_decode() || is_cut_off_stream_chunk(err)
}

/// Whether a stream error may silent-switch to the next model: only before any
/// assistant response was committed, and only for transient reachability
/// failures. Billing/auth stay on the announced path.
pub fn may_silent_precommit_failover(committed: bool, err: &ProviderError) -> bool {
    if committed {
        return false;
    }
    // Checked before `is_permanent()`: a cut-off chunk quotes whatever
    // partial provider bytes it held, and those can contain text such as
    // "invalid_request_error" that would make a dropped body read as a 4xx.
    if is_cut_off_stream_chunk(err) {
        return true;
    }
    if err.is_permanent() {
        return false;
    }
    // A dropped body is transient whatever variant carried it. The agent's
    // stream-drop arm matches `is_dropped_stream()`, which also accepts a
    // `RequestFailed("… error decoding response body")` (session 20260827_1
    // was typed that way), but this gate used to accept only the variants
    // below — so that arm's failover was dead code for exactly the error it
    // was written for, and the user got "resend" instead of a local model.
    //
    // A first-byte stall is a provider that never started answering. The
    // agent retries it once, and when the retry stalls too the turn used to
    // end on "send it again once the provider recovers", even with Ollama up
    // and idle on this machine. It sends nothing to the user, so it is as safe
    // to move as a refused connection. The stream-budget runaway it shares a
    // variant with stays out: `is_permanent()` rejects that one above.
    err.is_stream_decode()
        || err.is_first_byte_provider_stall()
        || matches!(
            err,
            ProviderError::NetworkError(_)
                | ProviderError::ServerError(_)
                | ProviderError::RateLimitExceeded { .. }
        )
}

/// Where a fallback candidate came from — kept so the one precedence rule that
/// depends on it (an explicit override may name the failed provider) survives
/// the move from "first match" to "first match that is actually up".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSource {
    /// `PERMAGENT_FALLBACK_PROVIDER` + `PERMAGENT_FALLBACK_MODEL`.
    ExplicitOverride,
    /// One entry of `PERMAGENT_FALLBACKS`.
    OrderedList,
    /// A configured role→model mapping, by role label.
    Role(&'static str),
}

/// One configured place a failed turn could move to, in precedence order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackCandidate {
    pub target: RoleModel,
    pub source: CandidateSource,
}

/// Pure: every configured fallback, in the strict precedence the module note
/// describes, deduplicated by provider+model (case-insensitive). Nothing is
/// filtered for the failed provider here — [`choose_fallback`] owns that rule,
/// because a DOWN local candidate on the failed provider is still a statement
/// that the user wanted a local model at that rank.
pub fn permanent_failure_candidates(
    read: impl Fn(&str) -> Option<String>,
) -> Vec<FallbackCandidate> {
    let non_empty = |k: &str| {
        read(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mut out: Vec<FallbackCandidate> = Vec::new();
    let mut push = |target: RoleModel, source: CandidateSource| {
        let dup = out.iter().any(|c| {
            c.target.provider.eq_ignore_ascii_case(&target.provider)
                && c.target.model.eq_ignore_ascii_case(&target.model)
        });
        if !dup {
            out.push(FallbackCandidate { target, source });
        }
    };

    if let (Some(provider), Some(model)) = (
        non_empty(KEY_FALLBACK_PROVIDER),
        non_empty(KEY_FALLBACK_MODEL),
    ) {
        push(
            RoleModel { provider, model },
            CandidateSource::ExplicitOverride,
        );
    }
    if let Some(raw) = non_empty(KEY_FALLBACKS) {
        for rm in parse_fallback_list(&raw) {
            push(rm, CandidateSource::OrderedList);
        }
    }
    for role in WorkflowRole::all() {
        if let (Some(provider), Some(model)) =
            (non_empty(&provider_key(role)), non_empty(&model_key(role)))
        {
            push(
                RoleModel { provider, model },
                CandidateSource::Role(role.as_str()),
            );
        }
    }
    out
}

/// Whether one endpoint answered the cheap probe taken when a fallback is
/// being chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Up,
    Down,
    /// Not probed (a cloud provider, or a local runtime this module has no
    /// probe for). Treated exactly as before probing existed: eligible.
    Unknown,
}

/// The local provider id of the two-machine Qwen3.8 split. Nightly-only by
/// design (launchd 01:50–06:10), so for most of the day its port is closed.
pub const SPLIT_PROVIDER: &str = "qwen38_split";
/// The local Ollama provider id (`providers/ollama.rs`). NOT `ollama_cloud`,
/// which is a metered hosted API and never a stand-in for a local model.
pub const OLLAMA_PROVIDER: &str = "ollama";

/// A snapshot of which local endpoints were up at the moment of the switch.
/// Pure data, so the selection over it is unit-testable without sockets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAvailability {
    /// Whether the local Ollama daemon answered `/api/tags`.
    pub ollama: Reach,
    /// The model ids Ollama reported, AS REPORTED (`qwen2.5:7b`,
    /// `minicpm5-2b-q8:latest`). Empty unless `ollama` is [`Reach::Up`].
    pub ollama_models: Vec<String>,
    /// Whether the split's port accepted a TCP connection.
    pub split: Reach,
}

impl LocalAvailability {
    /// Nothing probed — selection behaves exactly like the pre-probe resolver.
    pub fn unprobed() -> Self {
        Self {
            ollama: Reach::Unknown,
            ollama_models: Vec::new(),
            split: Reach::Unknown,
        }
    }

    /// Whether Ollama reported `model`. An untagged id is Ollama's own
    /// shorthand for `:latest`, so `qwen3` matches a reported `qwen3:latest`.
    fn ollama_has(&self, model: &str) -> bool {
        let model = model.trim();
        self.ollama_models.iter().any(|m| {
            m.eq_ignore_ascii_case(model)
                || (!model.contains(':') && m.eq_ignore_ascii_case(&format!("{model}:latest")))
        })
    }

    /// How reachable one candidate is. A model Ollama does not have is as
    /// unusable as a daemon that is down: the call would 404 "model not found".
    pub fn reach_of(&self, target: &RoleModel) -> Reach {
        if target.provider.eq_ignore_ascii_case(SPLIT_PROVIDER) {
            return self.split;
        }
        if target.provider.eq_ignore_ascii_case(OLLAMA_PROVIDER) {
            return match self.ollama {
                Reach::Up if self.ollama_has(&target.model) => Reach::Up,
                Reach::Up => Reach::Down,
                other => other,
            };
        }
        Reach::Unknown
    }
}

/// Pure: the installed Ollama model best fit to stand in for a local candidate
/// whose endpoint is down, or `None` when Ollama is not up or has nothing the
/// knowledge base vouches for as free and local.
///
/// Only models [`super::reservation::is_free_local_pair`] accepts are eligible,
/// so the stand-in can never be refused by the spend gate as "unknown budget"
/// — the failure that killed the turn after the 2026-09-18 failover. Ranked by
/// the knowledge base's orchestration strength (the fallback has to run the
/// agent loop, tools included), then edit reliability, then model id, so the
/// same install always yields the same answer.
pub fn best_installed_ollama_model(avail: &LocalAvailability) -> Option<RoleModel> {
    best_installed_ollama_model_for_request(avail, None)
}

/// [`best_installed_ollama_model`], limited to the installed models whose
/// window can hold a request of `request_tokens` ([`fits_local_window`]).
/// `None` tokens means the request could not be counted, and every model is
/// eligible, as before.
pub fn best_installed_ollama_model_for_request(
    avail: &LocalAvailability,
    request_tokens: Option<usize>,
) -> Option<RoleModel> {
    if avail.ollama != Reach::Up {
        return None;
    }
    avail
        .ollama_models
        .iter()
        .filter(|m| super::reservation::is_free_local_pair(OLLAMA_PROVIDER, m))
        .filter(|m| {
            fits_local_window(
                &RoleModel {
                    provider: OLLAMA_PROVIDER.to_string(),
                    model: (*m).clone(),
                },
                request_tokens,
            )
        })
        .filter_map(|m| super::knowledge::lookup(OLLAMA_PROVIDER, m).map(|row| (m, row)))
        .min_by(|(am, a), (bm, b)| {
            b.orchestration_strength
                .total_cmp(&a.orchestration_strength)
                .then_with(|| {
                    b.edit_format_reliability
                        .total_cmp(&a.edit_format_reliability)
                })
                .then_with(|| am.cmp(bm))
        })
        .map(|(m, _)| RoleModel {
            provider: OLLAMA_PROVIDER.to_string(),
            model: m.clone(),
        })
}

/// The share of a local window the request may fill, leaving the rest for the
/// reply. The same three quarters the CLI trims its extensions to at startup
/// (`goose-cli` `session::builder::local_request_budget`).
pub fn local_request_budget(window: usize) -> usize {
    window - window / 4
}

/// Pure: whether `target` can take a request of `request_tokens` (system
/// prompt, tools and history, counted the way the reservation counts them).
///
/// Only LOCAL rows are judged: their window is the `num_ctx` this app serves
/// them with, and nothing on this machine can make it larger mid-turn. A
/// cloud model keeps the ordinary compaction path, so it always fits here.
/// An uncounted request (`None`: images in context) fits everything, which is
/// the behaviour before this check existed.
///
/// 2026-09-19: the QA run of `permagent run --provider qwen38_split -t 'Reply
/// with the single word: ok'` (cli log 20260919_030958, session 20260919_2)
/// failed over at startup to ollama/qwen2.5:7b with a 62,364-token request,
/// against the 32,768 that model is served in. The installed build had no
/// Ollama preflight, so Ollama silently truncated the prompt at n_ctx 4096 and
/// the run still replied "ok". The preflight added in `providers::ollama` now
/// refuses a request that size outright, so a switch that ignored the size
/// would move the turn to a model certain to refuse it; the agent's usual
/// answer to that refusal is to have the 7B summarise the user's history.
pub fn fits_local_window(target: &RoleModel, request_tokens: Option<usize>) -> bool {
    let Some(tokens) = request_tokens else {
        return true;
    };
    match local_context_limit(&target.provider, &target.model) {
        Some(window) => tokens <= local_request_budget(window),
        None => true,
    }
}

/// The fallback that was chosen, and why — enough for the routing log line to
/// explain after the fact which configured model was passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackChoice {
    /// The provider+model the session moves to.
    pub target: RoleModel,
    /// The configured local candidate this stands in for, when that
    /// candidate's endpoint was down (e.g. the nightly split at noon) or its
    /// window was too small for the request.
    pub replaced: Option<RoleModel>,
    /// Configured candidates skipped because their endpoint was down and no
    /// local stand-in was available.
    pub skipped_down: Vec<RoleModel>,
    /// Configured candidates skipped because their window cannot hold the
    /// request ([`fits_local_window`]) and no larger local model was
    /// installed to stand in.
    pub skipped_too_small: Vec<RoleModel>,
}

/// Pure: pick the first configured fallback that can actually serve the turn.
///
/// Walks `candidates` in precedence order:
///
/// - A candidate whose endpoint is DOWN is never chosen. If it was a local
///   candidate (the configured rank asked for a local model), the best
///   installed Ollama model stands in AT THAT RANK — before any later cloud
///   entry. 2026-09-18: every permanent failure fell back to
///   `qwen38_split`/`qwen3.8-27b`, whose port is closed outside 01:50–06:10
///   (and the split refused to start every night that week for want of
///   9.5 GB free), while Ollama sat up on :11434 with four models pulled. The
///   user was told "Could not connect to 127.0.0.1:8081" instead of getting
///   the local model they had asked for.
/// - Otherwise the old rule: the explicit override is taken as written; any
///   other candidate on the failed provider is skipped (same account, same
///   wall); the first remaining candidate wins.
/// - A LOCAL candidate skipped for being on the failed provider (the split
///   failed while up) is also replaced by an installed Ollama model at its
///   rank, so a split failure never jumps straight to a metered cloud model.
///
/// Deterministic for a given snapshot: no clocks, no randomness. Takes no
/// account of the request's size; the agent's mid-turn switch uses
/// [`choose_fallback_for_request`].
pub fn choose_fallback(
    failed_provider: &str,
    candidates: &[FallbackCandidate],
    avail: &LocalAvailability,
) -> Option<FallbackChoice> {
    choose_fallback_for_request(failed_provider, candidates, avail, None)
}

/// [`choose_fallback`], never choosing a local model whose window cannot hold
/// a request of `request_tokens` ([`fits_local_window`]).
///
/// A local candidate that is up but too small is first replaced, at its rank,
/// by the best installed Ollama model that does fit; with none, it is skipped
/// and the walk goes on to the next candidate. A turn that can only fail on
/// the stand-in is better moved to the next configured model than sent to a
/// window that refuses it (2026-09-19 QA CLI run: 62,364 tokens against
/// qwen2.5:7b's 32,768, which the Ollama preflight now refuses; see
/// [`fits_local_window`]). With `request_tokens` `None` this is exactly
/// [`choose_fallback`].
pub fn choose_fallback_for_request(
    failed_provider: &str,
    candidates: &[FallbackCandidate],
    avail: &LocalAvailability,
    request_tokens: Option<usize>,
) -> Option<FallbackChoice> {
    let failed = failed_provider.trim();
    let differs = |p: &str| !p.eq_ignore_ascii_case(failed);
    let stand_in = || {
        best_installed_ollama_model_for_request(avail, request_tokens)
            .filter(|s| differs(&s.provider))
    };
    let mut skipped_down: Vec<RoleModel> = Vec::new();
    let mut skipped_too_small: Vec<RoleModel> = Vec::new();

    for c in candidates {
        if avail.reach_of(&c.target) == Reach::Down {
            if let Some(target) = stand_in() {
                return Some(FallbackChoice {
                    target,
                    replaced: Some(c.target.clone()),
                    skipped_down,
                    skipped_too_small,
                });
            }
            skipped_down.push(c.target.clone());
            continue;
        }
        if c.source == CandidateSource::ExplicitOverride || differs(&c.target.provider) {
            if fits_local_window(&c.target, request_tokens) {
                return Some(FallbackChoice {
                    target: c.target.clone(),
                    replaced: None,
                    skipped_down,
                    skipped_too_small,
                });
            }
            // Up, but its window cannot hold this request. The rank still
            // asked for a local model, so a larger installed one may serve it.
            if is_local_provider(&c.target.provider) {
                if let Some(target) = stand_in() {
                    return Some(FallbackChoice {
                        target,
                        replaced: Some(c.target.clone()),
                        skipped_down,
                        skipped_too_small,
                    });
                }
            }
            skipped_too_small.push(c.target.clone());
            continue;
        }
        // A LOCAL candidate skipped only because it is on the failed provider
        // (the split failed while its port was up — a 5xx or a dropped stream
        // inside the nightly window) still asked for a local model at this
        // rank. Without this, the next candidate — the metered zai/glm-5.2 —
        // won even with Ollama up and four models pulled.
        if is_local_provider(&c.target.provider) {
            if let Some(target) = stand_in() {
                return Some(FallbackChoice {
                    target,
                    replaced: Some(c.target.clone()),
                    skipped_down,
                    skipped_too_small,
                });
            }
        }
    }
    None
}

/// Whether `provider` is one of the local runtimes this module knows how to
/// probe (and so a rank at which a local stand-in honours the user's intent).
fn is_local_provider(provider: &str) -> bool {
    provider.eq_ignore_ascii_case(SPLIT_PROVIDER) || provider.eq_ignore_ascii_case(OLLAMA_PROVIDER)
}

/// Pure: resolve the fallback for a provider that was permanently rejected,
/// WITHOUT probing any endpoint (every candidate is [`Reach::Unknown`]).
///
/// `read` is a key reader so this is unit-testable without the process-global
/// config. `failed_provider` is compared case-insensitively; a candidate on the
/// same provider is skipped (same account, same empty balance). The explicit
/// override wins even if it names the failed provider — the operator said so,
/// and a same-provider override is a deliberate choice (e.g. a different key on
/// a different account).
pub fn resolve_permanent_failure_fallback(
    failed_provider: &str,
    read: impl Fn(&str) -> Option<String>,
) -> Option<RoleModel> {
    choose_fallback(
        failed_provider,
        &permanent_failure_candidates(read),
        &LocalAvailability::unprobed(),
    )
    .map(|c| c.target)
}

/// Config-backed [`resolve_permanent_failure_fallback`] (no probing). Kept for
/// sync callers; the agent's mid-turn switch uses
/// [`permanent_failure_fallback_available`].
pub fn permanent_failure_fallback(failed_provider: &str) -> Option<RoleModel> {
    let cfg = crate::config::Config::global();
    resolve_permanent_failure_fallback(failed_provider, |k| cfg.get_param::<String>(k).ok())
}

/// How long the Ollama `/api/tags` probe may take. A daemon that is up answers
/// in milliseconds; this only bounds how long a dead one delays the switch.
const OLLAMA_FALLBACK_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);
/// How long the split's TCP connect may take. It is on loopback: a closed port
/// is refused immediately, so this only bounds a wedged listener.
const SPLIT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(700);
/// The split's default endpoint — the same default
/// `providers/declarative/qwen38_split.json` gives `QWEN38_SPLIT_HOST`.
const SPLIT_DEFAULT_HOST: &str = "http://127.0.0.1:8081";

/// GET `<OLLAMA_HOST>/api/tags`: `(Up, models)` on a 2xx, `(Down, [])` on any
/// failure. Unlike `recommend::discover_ollama_models_async` this does NOT fold
/// in the configured `GOOSE_MODEL` floor — that floor would make a dead daemon
/// look like one with a model pulled.
async fn probe_ollama() -> (Reach, Vec<String>) {
    let raw_host = crate::config::Config::global()
        .get_param::<String>("OLLAMA_HOST")
        .ok();
    let url = format!("{}/api/tags", super::recommend::ollama_base_url(raw_host));
    let Ok(client) = reqwest::Client::builder()
        .timeout(OLLAMA_FALLBACK_PROBE_TIMEOUT)
        .build()
    else {
        return (Reach::Down, Vec::new());
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => (Reach::Up, super::recommend::parse_ollama_tags(&body)),
            Err(_) => (Reach::Down, Vec::new()),
        },
        _ => (Reach::Down, Vec::new()),
    }
}

/// Pure: the `(host, port)` a split base URL points at, or `None` when it is
/// not a parseable http(s) URL.
fn split_socket(raw: &str) -> Option<(String, u16)> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let host = url
        .host_str()?
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    let port = url.port_or_known_default()?;
    Some((host, port))
}

/// TCP connect to the split's port. A connect, not an HTTP request: the
/// question is only "is anything listening", and a loaded 27B split can take
/// far longer to answer a request than the switch should wait.
async fn probe_split() -> Reach {
    let raw = crate::config::Config::global()
        .get_param::<String>("QWEN38_SPLIT_HOST")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| SPLIT_DEFAULT_HOST.to_string());
    let Some((host, port)) = split_socket(&raw) else {
        return Reach::Down;
    };
    match tokio::time::timeout(
        SPLIT_CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(_)) => Reach::Up,
        _ => Reach::Down,
    }
}

/// Probe the local endpoints the candidate list actually names — nothing when
/// every candidate is a cloud model, so a cloud-only config pays no latency.
/// Ollama is probed whenever ANY local candidate is present, because it is
/// the stand-in for a down one.
pub async fn probe_local_availability(candidates: &[FallbackCandidate]) -> LocalAvailability {
    let names = |p: &str| {
        candidates
            .iter()
            .any(|c| c.target.provider.eq_ignore_ascii_case(p))
    };
    let wants_split = names(SPLIT_PROVIDER);
    let wants_ollama = wants_split || names(OLLAMA_PROVIDER);
    let mut avail = LocalAvailability::unprobed();
    let (split, ollama) = tokio::join!(
        async {
            if wants_split {
                Some(probe_split().await)
            } else {
                None
            }
        },
        async {
            if wants_ollama {
                Some(probe_ollama().await)
            } else {
                None
            }
        }
    );
    if let Some(reach) = split {
        avail.split = reach;
    }
    if let Some((reach, models)) = ollama {
        avail.ollama = reach;
        avail.ollama_models = models;
    }
    avail
}

/// Config-backed, availability-aware fallback: the candidates from config, a
/// cheap probe of the local endpoints they name, then [`choose_fallback`].
/// Logs the snapshot and the decision on `permagent::cost_router`, so a switch
/// that passed over the configured model can be explained after the fact.
pub async fn permanent_failure_fallback_available(failed_provider: &str) -> Option<FallbackChoice> {
    permanent_failure_fallback_for_request(failed_provider, None).await
}

/// [`permanent_failure_fallback_available`] for a request of known size: a
/// local model whose window cannot hold `request_tokens` is not chosen
/// ([`choose_fallback_for_request`]). The agent's mid-turn switch passes the
/// size of the request that just failed.
pub async fn permanent_failure_fallback_for_request(
    failed_provider: &str,
    request_tokens: Option<usize>,
) -> Option<FallbackChoice> {
    let cfg = crate::config::Config::global();
    let candidates = permanent_failure_candidates(|k| cfg.get_param::<String>(k).ok());
    if candidates.is_empty() {
        return None;
    }
    let avail = probe_local_availability(&candidates).await;
    let choice = choose_fallback_for_request(failed_provider, &candidates, &avail, request_tokens);
    let fmt = |rm: &RoleModel| format!("{}/{}", rm.provider, rm.model);
    let candidate_list = candidates
        .iter()
        .map(|c| fmt(&c.target))
        .collect::<Vec<_>>()
        .join(",");
    let chosen = choice
        .as_ref()
        .map(|c| fmt(&c.target))
        .unwrap_or_else(|| "none".to_string());
    let replaced = choice
        .as_ref()
        .and_then(|c| c.replaced.as_ref())
        .map(fmt)
        .unwrap_or_default();
    let skipped_down = choice
        .as_ref()
        .map(|c| c.skipped_down.iter().map(fmt).collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    let skipped_too_small = choice
        .as_ref()
        .map(|c| {
            c.skipped_too_small
                .iter()
                .map(fmt)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let ollama_models = avail.ollama_models.join(",");
    tracing::info!(
        target: "permagent::cost_router",
        from_provider = %failed_provider,
        candidates = %candidate_list,
        ollama = ?avail.ollama,
        ollama_models = %ollama_models,
        split = ?avail.split,
        request_tokens = ?request_tokens,
        chosen = %chosen,
        replaced = %replaced,
        skipped_down = %skipped_down,
        skipped_too_small = %skipped_too_small,
        "fallback selection over live local availability"
    );
    choice
}

/// The context window to give a LOCAL fallback whose `ModelConfig` came back
/// without one. An Ollama model has no canonical row, so without this the
/// provider sends no `num_ctx` and Ollama serves the agent's multi-thousand-
/// token system prompt through its small default window, silently truncated.
/// `None` for cloud models (their limits come from the canonical registry).
pub fn local_context_limit(provider: &str, model: &str) -> Option<usize> {
    super::knowledge::lookup(provider, model)
        .filter(|row| row.is_local && row.context_window > 0)
        .map(|row| row.context_window as usize)
}

/// Whether `provider`/`model` is a model this machine serves itself, by the
/// knowledge base's `is_local` flag. Used by the agent to decide that a
/// context overflow right after a mid-turn failover is the stand-in's small
/// window, not a history worth compacting.
pub fn is_local_model(provider: &str, model: &str) -> bool {
    super::knowledge::lookup(provider, model).is_some_and(|row| row.is_local)
}

/// The sentence for a turn that failed over to a LOCAL model whose window
/// could not hold the request after all.
///
/// The switch already refuses a local model that is too small by its own
/// count ([`fits_local_window`]); this is for when the Ollama preflight, which
/// measures the real request, still says no, or when the request could not be
/// counted. The agent's usual answer to that refusal is to compact the
/// conversation and retry, which here would have the 7B stand-in summarise the
/// user's history and still end on "context limit still exceeded". Instead the
/// agent switches the session BACK to the model it failed over from
/// (`restored`), so the next message is neither sent to the too-small window
/// nor compacted against it, and this says so.
///
/// `restored` false means the switch back itself failed and the session is
/// still on the stand-in: then the user has to switch the model, and resending
/// alone would reach the same window.
///
/// `permanent_cause` is the [`ProviderError::user_facing_summary`] of a
/// billing or auth rejection when that is what the turn failed over from. The
/// session still goes back (staying on the stand-in would compact against its
/// window on the next message), but "resend once it is reachable" would be
/// false: a provider out of credit or with a rejected key rejects the resend
/// too, which fails over to the same stand-in and is refused again. So the
/// advice is to fix the rejection or pick another model, never to resend or to
/// switch back to it. Payload-free like its siblings.
pub fn local_fallback_overflow_reply(
    failed_provider: &str,
    fallback: &RoleModel,
    restored: bool,
    permanent_cause: Option<&str>,
) -> String {
    let head = format!(
        "This turn moved from {failed_provider} to {}/{}, but that model's context window \
         is too small for this session's request, so nothing was sent to it and your \
         conversation was left as it is.",
        fallback.provider, fallback.model
    );
    match (permanent_cause, restored) {
        (None, true) => format!(
            "{head} This session is back on {failed_provider}: resend once it is reachable \
             again, or pick a model with a larger window in Settings → Models."
        ),
        (None, false) => format!(
            "{head} This session is still on {}/{}, which cannot hold it: switch back to \
             {failed_provider}, or to another model with a larger window, in Settings → Models \
             before you resend.",
            fallback.provider, fallback.model
        ),
        (Some(cause), true) => format!(
            "{head} This session is back on {failed_provider}, but {cause}. A resend will be rejected the same way until that is fixed on the \
             provider's side (its Plans & Billing page or API key), so fix it or pick another \
             model with a larger window in Settings → Models first."
        ),
        (Some(cause), false) => format!(
            "{head} This session is still on {}/{}, which cannot hold it, and \
             {failed_provider} cannot take it either ({cause}). Pick another model with a \
             larger window in Settings → Models before you resend.",
            fallback.provider, fallback.model
        ),
    }
}

/// Pure: the cause of a mid-turn model switch, for the routing-snapshot log
/// line. That line used to say `permanent_provider_rejection` for every
/// switch, so a network failover, a 5xx, a spent rate limit or a stall all
/// read in the log as a billing or auth rejection.
pub fn failover_cause(err: &ProviderError) -> &'static str {
    if is_dropped_stream(err) {
        return "stream_dropped";
    }
    if err.is_first_byte_provider_stall() {
        return "first_byte_stall";
    }
    match err {
        ProviderError::CreditsExhausted { .. } => "credits_exhausted",
        ProviderError::Authentication(_) => "authentication_rejected",
        ProviderError::NetworkError(_) => "network_unreachable",
        ProviderError::ServerError(_) => "server_error",
        ProviderError::RateLimitExceeded { .. } => "rate_limited",
        _ if err.is_permanent() => "permanent_provider_rejection",
        _ => "transient_provider_error",
    }
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

/// What happened to `provider`, as a payload-free clause whose VERB is chosen
/// by the cause. Derived from the error VARIANT, never from its string:
/// `NetworkError`'s own text carries the socket we failed to open
/// (`127.0.0.1:8081`), which is log material, not something to read aloud.
///
/// The verb matters. 2026-09-18: OpenAI's stream died mid-body after DNS, TCP,
/// TLS and a 200 status had all succeeded, and the transcript said "openai
/// unreachable (the model stream dropped …)" — a host that answered was
/// reported as one that could not be reached. "Unreachable" is reserved for the
/// one cause that means it: a connection that never opened.
fn precommit_failure_clause(provider: &str, err: &ProviderError) -> String {
    if is_dropped_stream(err) {
        return format!("{provider} dropped the response stream before any reply started");
    }
    // Checked before the variant match: a stall is an `ExecutionError`, which
    // would otherwise read as the vague "failed before any reply started". The
    // host may well have accepted the connection, so it is not "unreachable".
    if err.is_first_byte_provider_stall() {
        return format!("{provider} sent nothing back in time");
    }
    match err {
        ProviderError::NetworkError(_) => {
            format!("{provider} unreachable (connection refused or unroutable)")
        }
        ProviderError::ServerError(_) => {
            format!("{provider} returned a server error before any reply started")
        }
        ProviderError::RateLimitExceeded { .. } => {
            format!("{provider} is rate-limiting this model")
        }
        _ => format!("{provider} failed before any reply started"),
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
        "{} — switched this session to {}/{}.",
        precommit_failure_clause(failed_provider, err),
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

    /// The agent's stream-decode arm fires for a dropped body typed as
    /// `RequestFailed` too (session 20260827_1's shape). The gate must agree
    /// with that arm, or its failover can never run for that error. A genuine
    /// 4xx rejection stays out: it is permanent, not a dropped stream.
    #[test]
    fn a_dropped_stream_typed_as_request_failed_may_still_fail_over() {
        let typed_as_request_failed = ProviderError::RequestFailed(
            "Stream decode error: error decoding response body".into(),
        );
        assert!(typed_as_request_failed.is_stream_decode());
        assert!(may_silent_precommit_failover(
            false,
            &typed_as_request_failed
        ));
        assert!(!may_silent_precommit_failover(
            true,
            &typed_as_request_failed
        ));
        assert!(!may_silent_precommit_failover(
            false,
            &ProviderError::RequestFailed("Bad request (400): invalid_request_error".into())
        ));
    }

    /// A body closed partway through a `data:` line reaches the agent as
    /// the OpenAI chunk parser's `RequestFailed("Failed to parse streaming
    /// chunk: EOF while parsing …")`. It is a dropped stream: the gate lets it
    /// fail over before commit, the log cause and the notice say "dropped",
    /// and it is never read as a 4xx even when the partial bytes it quotes
    /// happen to contain "invalid_request_error".
    #[test]
    fn a_chunk_cut_off_mid_line_is_a_dropped_stream() {
        let cut = ProviderError::RequestFailed(
            "Failed to parse streaming chunk: EOF while parsing a string at line 1 column 58: \
             \"{\\\"choices\\\":[{\\\"delta\\\":{\\\"content\\\":\\\"the sec\""
                .into(),
        );
        assert!(is_cut_off_stream_chunk(&cut));
        assert!(is_dropped_stream(&cut));
        assert!(may_silent_precommit_failover(false, &cut));
        assert!(!may_silent_precommit_failover(true, &cut));
        assert_eq!(failover_cause(&cut), "stream_dropped");
        let target = RoleModel {
            provider: "ollama".into(),
            model: "qwen2.5:7b".into(),
        };
        let reply = precommit_failover_reply("qwen38_split", &cut, &target);
        assert!(reply.contains("dropped the response stream"), "{reply}");
        assert!(!reply.contains("the sec"), "leaked the payload: {reply}");

        let quoting_a_4xx = ProviderError::RequestFailed(
            "Failed to parse streaming chunk: EOF while parsing a string at line 1 column 40: \
             \"{\\\"error\\\":{\\\"type\\\":\\\"invalid_request_error\""
                .into(),
        );
        assert!(may_silent_precommit_failover(false, &quoting_a_4xx));
    }

    /// A COMPLETE line that is not a chunk is a real shape mismatch, and a
    /// genuine 4xx is a rejection: neither may pass as a dropped stream.
    #[test]
    fn only_end_of_input_counts_as_a_cut_off_chunk() {
        let shape = ProviderError::RequestFailed(
            "Failed to parse streaming chunk: missing field `choices` at line 1 column 12: \"{}\""
                .into(),
        );
        assert!(!is_cut_off_stream_chunk(&shape));
        assert!(!is_dropped_stream(&shape));
        let rejected =
            ProviderError::RequestFailed("Bad request (400): invalid_request_error".into());
        assert!(!is_dropped_stream(&rejected));
        assert!(!may_silent_precommit_failover(false, &rejected));
        assert!(!is_cut_off_stream_chunk(&ProviderError::NetworkError(
            "Failed to parse streaming chunk: EOF while parsing".into()
        )));
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

    /// The 2026-09-18 defect: a stream that died mid-body after DNS, TCP, TLS
    /// and a 200 status all succeeded was announced as "openai unreachable".
    /// The host answered; only the body died. The verb must say so.
    #[test]
    fn a_dropped_stream_is_never_called_unreachable() {
        let target = RoleModel {
            provider: "ollama".into(),
            model: "qwen2.5:7b".into(),
        };
        let reply = precommit_failover_reply(
            "openai",
            &ProviderError::stream_decode("error decoding response body"),
            &target,
        );
        assert!(
            !reply.to_lowercase().contains("unreachable"),
            "a host that answered is not unreachable: {reply}"
        );
        assert!(
            reply.starts_with("openai dropped the response stream before any reply started"),
            "{reply}"
        );
        assert!(
            reply.contains("switched this session to ollama/qwen2.5:7b"),
            "{reply}"
        );
    }

    /// Each cause picks its own verb; only a connection that never opened is
    /// "unreachable".
    #[test]
    fn each_precommit_cause_chooses_its_own_verb() {
        let target = RoleModel {
            provider: "ollama".into(),
            model: "qwen2.5:7b".into(),
        };
        let reply = |e: ProviderError| precommit_failover_reply("openai", &e, &target);
        let net = reply(ProviderError::NetworkError("Could not connect".into()));
        assert!(
            net.starts_with("openai unreachable (connection refused"),
            "{net}"
        );
        let server = reply(ProviderError::ServerError("503".into()));
        assert!(
            server.starts_with("openai returned a server error"),
            "{server}"
        );
        assert!(!server.contains("unreachable"), "{server}");
        let rate = reply(ProviderError::RateLimitExceeded {
            details: "slow down".into(),
            retry_delay: None,
        });
        assert!(
            rate.starts_with("openai is rate-limiting this model"),
            "{rate}"
        );
        assert!(!rate.contains("unreachable"), "{rate}");
        let stall = reply(ProviderError::first_byte_stall("0 chars in 120 s"));
        assert!(
            stall.starts_with("openai sent nothing back in time"),
            "{stall}"
        );
        assert!(!stall.contains("unreachable"), "{stall}");
        assert!(
            !stall.contains("120 s"),
            "the detail is log material: {stall}"
        );
    }

    /// A first-byte stall that survived its one retry used to end the turn on
    /// "send it again once the provider recovers", with Ollama up and idle.
    /// Nothing reached the user, so it may move. A stream-budget RUNAWAY
    /// shares the variant and must not: it is ours, and permanent.
    #[test]
    fn a_first_byte_stall_may_fail_over_but_a_runaway_may_not() {
        let stall = ProviderError::first_byte_stall("0 chars in 120 s");
        assert!(may_silent_precommit_failover(false, &stall));
        assert!(!may_silent_precommit_failover(true, &stall));
        let runaway = ProviderError::stream_budget_exceeded("441 s, 3.0 MB");
        assert!(!runaway.is_first_byte_provider_stall());
        assert!(!may_silent_precommit_failover(false, &runaway));
    }

    /// The stand-in's window is the local row's, and only local rows count:
    /// the agent must not skip compaction for a cloud fallback.
    #[test]
    fn only_local_rows_count_as_local_models() {
        assert!(is_local_model("ollama", "qwen2.5:7b"));
        assert!(is_local_model("qwen38_split", "qwen3.8-27b"));
        assert!(!is_local_model("zai", "glm-5.2"));
        assert!(!is_local_model("ollama", "not-a-pulled-model:1b"));
    }

    /// The overflow after a failover says which model it moved to, that
    /// nothing was sent, and that the history is untouched, never that it is
    /// compacting. The advice follows where the session actually is: back on
    /// the failed provider, so resending is right; or still on the too-small
    /// stand-in, so resending alone would hit the same window.
    #[test]
    fn a_local_overflow_after_failover_is_named_and_leaves_history_alone() {
        let target = rm("ollama", "qwen2.5:7b");
        let restored = local_fallback_overflow_reply("anthropic", &target, true, None);
        assert!(
            restored.contains("from anthropic to ollama/qwen2.5:7b"),
            "{restored}"
        );
        assert!(restored.contains("nothing was sent"), "{restored}");
        assert!(restored.contains("left as it is"), "{restored}");
        assert!(!restored.to_lowercase().contains("compact"), "{restored}");
        assert!(
            restored.contains("back on anthropic"),
            "the session was switched back and must say so: {restored}"
        );

        let stuck = local_fallback_overflow_reply("anthropic", &target, false, None);
        assert!(
            stuck.contains("still on ollama/qwen2.5:7b"),
            "a failed switch-back must not pretend the session moved: {stuck}"
        );
        assert!(
            stuck.contains("switch back to anthropic"),
            "the user has to switch the model before a resend can work: {stuck}"
        );
        assert!(
            !stuck.contains("resend once"),
            "resending on the stand-in reaches the same window: {stuck}"
        );
    }

    /// A failover from a billing or auth rejection must not promise that a
    /// resend will work: the provider it hands back to rejects every request
    /// until the credit or key is fixed, and the resend would fail over to the
    /// same stand-in and be refused again. Nor may it send the user back to it.
    #[test]
    fn a_local_overflow_after_a_permanent_rejection_never_promises_a_resend() {
        let target = rm("ollama", "qwen2.5:7b");
        let cause = ProviderError::CreditsExhausted {
            details: "credit balance too low".into(),
            top_up_url: None,
        }
        .user_facing_summary();

        let restored = local_fallback_overflow_reply("anthropic", &target, true, Some(&cause));
        assert!(restored.contains("back on anthropic"), "{restored}");
        assert!(restored.contains(&cause), "names the rejection: {restored}");
        assert!(
            !restored.contains("resend once"),
            "a resend is rejected the same way: {restored}"
        );
        assert!(restored.contains("until that is fixed"), "{restored}");
        assert!(!restored.to_lowercase().contains("compact"), "{restored}");

        let stuck = local_fallback_overflow_reply("anthropic", &target, false, Some(&cause));
        assert!(stuck.contains("still on ollama/qwen2.5:7b"), "{stuck}");
        assert!(
            !stuck.contains("switch back to anthropic"),
            "anthropic is out of credit; switching back cannot help: {stuck}"
        );
        assert!(!stuck.contains("resend once"), "{stuck}");
    }

    /// The routing line names the real cause. It said
    /// `permanent_provider_rejection` for network, 5xx, rate-limit and stall
    /// failovers alike.
    #[test]
    fn each_failover_names_its_own_cause() {
        assert_eq!(
            failover_cause(&ProviderError::NetworkError("refused".into())),
            "network_unreachable"
        );
        assert_eq!(
            failover_cause(&ProviderError::ServerError("503".into())),
            "server_error"
        );
        assert_eq!(
            failover_cause(&ProviderError::RateLimitExceeded {
                details: "slow down".into(),
                retry_delay: None,
            }),
            "rate_limited"
        );
        assert_eq!(
            failover_cause(&ProviderError::first_byte_stall("0 chars in 120 s")),
            "first_byte_stall"
        );
        assert_eq!(
            failover_cause(&ProviderError::RequestFailed(
                "error decoding response body".into()
            )),
            "stream_dropped"
        );
        assert_eq!(
            failover_cause(&ProviderError::Authentication("401".into())),
            "authentication_rejected"
        );
        assert_eq!(failover_cause(&billing_error()), "credits_exhausted");
        for transient in [
            ProviderError::NetworkError("x".into()),
            ProviderError::ServerError("x".into()),
            ProviderError::first_byte_stall("x"),
        ] {
            assert_ne!(
                failover_cause(&transient),
                "permanent_provider_rejection",
                "{transient:?}"
            );
        }
    }

    // ── request-size-aware selection ──────────────────────────────────────

    /// The request size logged on 2026-09-19 for the QA CLI run that failed
    /// over to ollama/qwen2.5:7b (cli log 20260919_030958, session
    /// 20260919_2): the system prompt and tool schemas alone.
    const QA_RUN_REQUEST_TOKENS: usize = 62_364;

    #[test]
    fn a_local_window_fits_only_what_leaves_room_for_the_reply() {
        let small = rm("ollama", "qwen2.5:7b");
        assert!(fits_local_window(&small, Some(20_000)));
        assert!(!fits_local_window(&small, Some(QA_RUN_REQUEST_TOKENS)));
        assert!(
            !fits_local_window(&small, Some(30_000)),
            "under the window but no room for the reply"
        );
        assert!(
            fits_local_window(&small, None),
            "an uncounted request keeps the old behaviour"
        );
        assert!(
            fits_local_window(&rm("zai", "glm-5.2"), Some(QA_RUN_REQUEST_TOKENS)),
            "cloud models keep the ordinary compaction path"
        );
        assert!(
            !fits_local_window(&rm("qwen38_split", "qwen3.8-27b"), Some(5_000)),
            "the split is served with -c 4096"
        );
    }

    /// The QA run's shape mid-turn: split down, Ollama up with only
    /// 32k-or-smaller models, a 62k request. `choose_fallback` picks
    /// qwen2.5:7b, whose window the Ollama preflight refuses for a request
    /// that size. The next configured model now serves the turn instead.
    #[test]
    fn a_request_too_big_for_every_installed_model_moves_to_the_next_candidate() {
        let choice = choose_fallback_for_request(
            "anthropic",
            &live_config(),
            &ollama_up_with_the_installed_models(),
            Some(QA_RUN_REQUEST_TOKENS),
        )
        .expect("the configured cloud model can take the request");
        assert_eq!(choice.target, rm("zai", "glm-5.2"));
        assert_eq!(choice.replaced, None);
        assert_eq!(
            choice.skipped_down,
            vec![rm("qwen38_split", "qwen3.8-27b")],
            "the split was down and no installed model could stand in for it"
        );
    }

    /// Same request with a large-window local model pulled: it stands in at
    /// the split's rank, ahead of the metered model.
    #[test]
    fn a_larger_installed_model_stands_in_when_the_best_ranked_one_is_too_small() {
        let mut avail = ollama_up_with_the_installed_models();
        avail.ollama_models.push("qwen3-coder:30b".into());
        let choice = choose_fallback_for_request(
            "anthropic",
            &live_config(),
            &avail,
            Some(QA_RUN_REQUEST_TOKENS),
        )
        .unwrap();
        assert_eq!(choice.target, rm("ollama", "qwen3-coder:30b"));
        assert_eq!(choice.replaced, Some(rm("qwen38_split", "qwen3.8-27b")));
    }

    /// An UP local candidate whose window is too small (the split at night,
    /// served with -c 4096) is not chosen for a request it cannot hold.
    #[test]
    fn an_up_local_candidate_too_small_for_the_request_is_skipped() {
        let avail = LocalAvailability {
            ollama: Reach::Down,
            ollama_models: vec![],
            split: Reach::Up,
        };
        let choice =
            choose_fallback_for_request("anthropic", &live_config(), &avail, Some(20_000)).unwrap();
        assert_eq!(choice.target, rm("zai", "glm-5.2"));
        assert_eq!(
            choice.skipped_too_small,
            vec![rm("qwen38_split", "qwen3.8-27b")]
        );
    }

    /// A request that fits changes nothing: the ordinary stand-in is taken.
    #[test]
    fn a_request_that_fits_keeps_the_ordinary_stand_in() {
        let sized = choose_fallback_for_request(
            "anthropic",
            &live_config(),
            &ollama_up_with_the_installed_models(),
            Some(8_000),
        )
        .unwrap();
        let unsized_choice = choose_fallback(
            "anthropic",
            &live_config(),
            &ollama_up_with_the_installed_models(),
        )
        .unwrap();
        assert_eq!(sized, unsized_choice);
        assert_eq!(sized.target.provider, "ollama");
    }

    /// Nothing configured can hold the request: no switch at all, so the turn
    /// ends on its own honest message instead of a refusal from a stand-in.
    #[test]
    fn nothing_is_chosen_when_no_candidate_can_hold_the_request() {
        let cands = permanent_failure_candidates(reader(&[(
            KEY_FALLBACKS,
            "qwen38_split/qwen3.8-27b,ollama/qwen2.5:7b",
        )]));
        assert_eq!(
            choose_fallback_for_request(
                "anthropic",
                &cands,
                &ollama_up_with_the_installed_models(),
                Some(QA_RUN_REQUEST_TOKENS),
            ),
            None
        );
    }

    // ── availability-aware selection ─────────────────────────────────────

    fn rm(provider: &str, model: &str) -> RoleModel {
        RoleModel {
            provider: provider.into(),
            model: model.into(),
        }
    }

    /// The live config on 2026-09-18: MECHANICAL and LOCAL both on the split,
    /// REVIEW on a metered cloud model.
    fn live_config() -> Vec<FallbackCandidate> {
        permanent_failure_candidates(reader(&[
            ("PERMAGENT_ROLE_MECHANICAL_PROVIDER", "qwen38_split"),
            ("PERMAGENT_ROLE_MECHANICAL_MODEL", "qwen3.8-27b"),
            ("PERMAGENT_ROLE_REVIEW_PROVIDER", "zai"),
            ("PERMAGENT_ROLE_REVIEW_MODEL", "glm-5.2"),
            ("PERMAGENT_ROLE_LOCAL_PROVIDER", "qwen38_split"),
            ("PERMAGENT_ROLE_LOCAL_MODEL", "qwen3.8-27b"),
        ]))
    }

    /// What Ollama on :11434 reported the same day.
    fn ollama_up_with_the_installed_models() -> LocalAvailability {
        LocalAvailability {
            ollama: Reach::Up,
            ollama_models: vec![
                "qwen2.5-coder:7b".into(),
                "minicpm5-2b-q8:latest".into(),
                "qwen25-16k:latest".into(),
                "qwen2.5:7b".into(),
            ],
            split: Reach::Down,
        }
    }

    #[test]
    fn candidates_keep_precedence_and_drop_duplicates() {
        let c = live_config();
        assert_eq!(
            c,
            vec![
                FallbackCandidate {
                    target: rm("qwen38_split", "qwen3.8-27b"),
                    source: CandidateSource::Role("mechanical"),
                },
                FallbackCandidate {
                    target: rm("zai", "glm-5.2"),
                    source: CandidateSource::Role("review"),
                },
            ],
            "LOCAL repeats MECHANICAL's pair and must not appear twice"
        );
    }

    /// THE regression: at noon the split is down and Ollama is up. The switch
    /// used to target 127.0.0.1:8081 anyway; now the best installed local
    /// model stands in at the split's rank — ahead of the metered REVIEW model.
    #[test]
    fn a_down_split_is_replaced_by_an_installed_ollama_model_not_skipped_to_cloud() {
        let choice = choose_fallback(
            "openai",
            &live_config(),
            &ollama_up_with_the_installed_models(),
        )
        .expect("an up local model must be chosen");
        assert_eq!(choice.target.provider, "ollama");
        assert_ne!(choice.target.provider, "qwen38_split");
        assert_eq!(choice.replaced, Some(rm("qwen38_split", "qwen3.8-27b")));
        assert!(
            crate::cost_router::reservation::is_free_local_pair(
                &choice.target.provider,
                &choice.target.model
            ),
            "the stand-in must pass the spend gate as free/local: {:?}",
            choice.target
        );
    }

    /// Same snapshot, same answer — and the answer is the documented ranking
    /// (strongest orchestration first), not the order Ollama listed its tags.
    #[test]
    fn stand_in_choice_is_deterministic_regardless_of_tag_order() {
        let a = ollama_up_with_the_installed_models();
        let mut b = a.clone();
        b.ollama_models.reverse();
        let pick_a = best_installed_ollama_model(&a).unwrap();
        let pick_b = best_installed_ollama_model(&b).unwrap();
        assert_eq!(pick_a, pick_b);
        let best = crate::cost_router::knowledge::lookup("ollama", &pick_a.model).unwrap();
        for m in &a.ollama_models {
            let row = crate::cost_router::knowledge::lookup("ollama", m).unwrap();
            assert!(
                best.orchestration_strength >= row.orchestration_strength,
                "{} outranks the pick {}",
                m,
                pick_a.model
            );
        }
    }

    /// The split is up during its nightly window: then it is exactly what the
    /// user configured, and it is taken as before.
    #[test]
    fn an_up_split_is_still_chosen() {
        let mut avail = ollama_up_with_the_installed_models();
        avail.split = Reach::Up;
        let choice = choose_fallback("openai", &live_config(), &avail).unwrap();
        assert_eq!(choice.target, rm("qwen38_split", "qwen3.8-27b"));
        assert_eq!(choice.replaced, None);
    }

    /// Both local endpoints down: never pick the dead split; fall through to
    /// the next configured candidate and say which ones were skipped.
    #[test]
    fn with_everything_local_down_the_split_is_skipped_not_chosen() {
        let avail = LocalAvailability {
            ollama: Reach::Down,
            ollama_models: vec![],
            split: Reach::Down,
        };
        let choice = choose_fallback("openai", &live_config(), &avail).unwrap();
        assert_eq!(choice.target, rm("zai", "glm-5.2"));
        assert_eq!(choice.skipped_down, vec![rm("qwen38_split", "qwen3.8-27b")]);
    }

    /// The split itself failed pre-commit (a harness role defaulted to it):
    /// the local intent is honoured by Ollama rather than jumping to cloud.
    #[test]
    fn when_the_split_itself_failed_a_local_model_still_stands_in() {
        let choice = choose_fallback(
            "qwen38_split",
            &live_config(),
            &ollama_up_with_the_installed_models(),
        )
        .unwrap();
        assert_eq!(choice.target.provider, "ollama");
    }

    /// The split itself failed while its port was UP (a server error or a
    /// dropped stream inside the nightly window). It is skipped as the failed
    /// provider — but the rank still asked for a local model, so Ollama stands
    /// in instead of the metered zai/glm-5.2 that follows it.
    #[test]
    fn when_an_up_split_failed_a_local_model_stands_in_not_cloud() {
        let mut avail = ollama_up_with_the_installed_models();
        avail.split = Reach::Up;
        let choice = choose_fallback("qwen38_split", &live_config(), &avail).unwrap();
        assert_eq!(choice.target.provider, "ollama");
        assert_ne!(choice.target, rm("zai", "glm-5.2"));
        assert_eq!(choice.replaced, Some(rm("qwen38_split", "qwen3.8-27b")));
        assert!(choice.skipped_down.is_empty());
    }

    /// Same failure with Ollama down: nothing local can serve, so the old
    /// rule holds and the next configured candidate is taken.
    #[test]
    fn when_an_up_split_failed_and_ollama_is_down_the_next_candidate_wins() {
        let avail = LocalAvailability {
            ollama: Reach::Down,
            ollama_models: vec![],
            split: Reach::Up,
        };
        let choice = choose_fallback("qwen38_split", &live_config(), &avail).unwrap();
        assert_eq!(choice.target, rm("zai", "glm-5.2"));
        assert_eq!(choice.replaced, None);
    }

    /// Ollama is up but the configured model is not pulled: calling it would
    /// 404, so it counts as down and an installed model stands in.
    #[test]
    fn a_configured_ollama_model_that_is_not_pulled_counts_as_down() {
        let cands = permanent_failure_candidates(reader(&[(
            KEY_FALLBACKS,
            "ollama/qwen3-coder:30b,openai/gpt-5.4",
        )]));
        let choice =
            choose_fallback("anthropic", &cands, &ollama_up_with_the_installed_models()).unwrap();
        assert_eq!(choice.target.provider, "ollama");
        assert_eq!(choice.replaced, Some(rm("ollama", "qwen3-coder:30b")));
    }

    /// An installed Ollama model that IS configured is used as written, and an
    /// untagged id matches Ollama's `:latest` shorthand.
    #[test]
    fn a_configured_installed_ollama_model_is_used_as_written() {
        let cands = permanent_failure_candidates(reader(&[(
            KEY_FALLBACKS,
            "ollama/minicpm5-2b-q8,openai/gpt-5.4",
        )]));
        let choice =
            choose_fallback("anthropic", &cands, &ollama_up_with_the_installed_models()).unwrap();
        assert_eq!(choice.target, rm("ollama", "minicpm5-2b-q8"));
        assert_eq!(choice.replaced, None);
    }

    /// If Ollama is the provider that failed, another Ollama model is no
    /// stand-in (same daemon).
    #[test]
    fn ollama_never_stands_in_for_itself() {
        let cands = permanent_failure_candidates(reader(&[(
            KEY_FALLBACKS,
            "qwen38_split/qwen3.8-27b,openai/gpt-5.4",
        )]));
        let choice =
            choose_fallback("ollama", &cands, &ollama_up_with_the_installed_models()).unwrap();
        assert_eq!(choice.target, rm("openai", "gpt-5.4"));
    }

    /// Unprobed (cloud-only or sync callers): identical to the old resolver.
    #[test]
    fn unprobed_snapshot_keeps_the_configured_first_choice() {
        let choice =
            choose_fallback("openai", &live_config(), &LocalAvailability::unprobed()).unwrap();
        assert_eq!(choice.target, rm("qwen38_split", "qwen3.8-27b"));
    }

    /// Unknown installed models (no knowledge-base row) are never picked: the
    /// spend gate would refuse them as an unknown paid budget.
    #[test]
    fn unpriced_installed_models_are_not_eligible_stand_ins() {
        let avail = LocalAvailability {
            ollama: Reach::Up,
            ollama_models: vec!["llama9-unknown:13b".into()],
            split: Reach::Down,
        };
        assert_eq!(best_installed_ollama_model(&avail), None);
    }

    #[test]
    fn split_socket_parses_the_declarative_default() {
        assert_eq!(
            split_socket("http://127.0.0.1:8081"),
            Some(("127.0.0.1".to_string(), 8081))
        );
        assert_eq!(
            split_socket("http://[::1]:8081/"),
            Some(("::1".to_string(), 8081))
        );
        assert_eq!(split_socket("not a url"), None);
    }

    #[test]
    fn local_context_limit_comes_from_the_local_row_only() {
        assert_eq!(
            local_context_limit("ollama", "qwen25-16k:latest"),
            Some(16_384)
        );
        assert_eq!(
            local_context_limit("qwen38_split", "qwen3.8-27b"),
            Some(4_096)
        );
        assert_eq!(local_context_limit("anthropic", "claude-opus-4-8"), None);
    }
}
