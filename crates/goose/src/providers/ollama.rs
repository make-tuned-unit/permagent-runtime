use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, MessageStream, Provider, ProviderDef, ProviderMetadata};
use super::errors::ProviderError;
use super::inventory::InventoryIdentityInput;
use super::openai_compatible::handle_status_openai_compat;
use super::retry::{ProviderRetry, RetryConfig};
use super::utils::{ImageFormat, RequestLog};
use crate::config::declarative_providers::DeclarativeProviderConfig;
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::formats::ollama::{create_request, response_to_streaming_message_ollama};
use anyhow::{Error, Result};
use async_stream::try_stream;
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::TryStreamExt;
use reqwest::Response;
use rmcp::model::Tool;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::pin;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;
use url::Url;

const OLLAMA_PROVIDER_NAME: &str = "ollama";
pub const OLLAMA_HOST: &str = "localhost";
pub const OLLAMA_TIMEOUT: u64 = 600;
pub const OLLAMA_DEFAULT_PORT: u16 = 11434;
pub const OLLAMA_DEFAULT_MODEL: &str = "qwen3";
pub const OLLAMA_KNOWN_MODELS: &[&str] = &[
    OLLAMA_DEFAULT_MODEL,
    "qwen3-vl",
    "qwen3-coder:30b",
    "qwen3-coder:480b-cloud",
];
pub const OLLAMA_DOC_URL: &str = "https://ollama.com/library";

// Ollama-specific retry config: large models can take 30-120s to load into memory,
// during which Ollama returns 500 errors. Use more retries with gradual backoff
// to wait for the model to become ready.
const OLLAMA_MAX_RETRIES: usize = 10;
const OLLAMA_INITIAL_RETRY_INTERVAL_MS: u64 = 2000;
const OLLAMA_BACKOFF_MULTIPLIER: f64 = 1.5;
const OLLAMA_MAX_RETRY_INTERVAL_MS: u64 = 15_000;

#[derive(serde::Serialize)]
pub struct OllamaProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    name: String,
    skip_canonical_filtering: bool,
    /// Whether the configured Ollama host is loopback (this machine). A remote
    /// Ollama is a cloud egress from the sovereignty point of view, so this
    /// drives [`Provider::data_locality`]. Computed once at construction.
    is_local_host: bool,
    /// What `/api/show` says about each model's window, by model name (see
    /// [`ShowWindow`]). Cached because every `stream` call needs it and the
    /// answer only changes when someone edits the Modelfile.
    #[serde(skip)]
    served_windows: std::sync::Mutex<std::collections::HashMap<String, ShowWindow>>,
}

/// Ollama's own default context window: what a model is loaded with when its
/// Modelfile sets no `num_ctx` and the server has no `OLLAMA_CONTEXT_LENGTH`.
const OLLAMA_SERVER_DEFAULT_NUM_CTX: usize = 4096;

/// Where a [`ServedWindow`] came from, so the refusal can say which knob to
/// turn, and so [`auto_num_ctx`] knows whose choice it would be overriding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServedWindowSource {
    /// The model's Modelfile sets `num_ctx` (e.g. `qwen25-16k:latest`,
    /// `minicpm5-2b-q8:latest`); Ollama loads it at that size.
    Modelfile,
    /// No Modelfile `num_ctx`, and the `OLLAMA_CONTEXT_LENGTH` configured here
    /// says what the server was started with: a server-wide window somebody
    /// chose, usually to save memory — as deliberate as a Modelfile's.
    ServerConfigured,
    /// No Modelfile `num_ctx` and nothing configured: Ollama's own 4096.
    ServerDefault,
}

/// The context window Ollama loads a LOCAL model with when the request does
/// not say otherwise.
///
/// Why this exists: Ollama's OpenAI-compatible `/v1/chat/completions` drops
/// the `options` object, `num_ctx` included. The CLI fallback run of
/// 2026-09-19 (cli log 20260919_030958) sent `options.num_ctx = 32768` for
/// `qwen2.5:7b` there; the server log shows the model stayed loaded at
/// `n_ctx = 4096` and printed `truncating input prompt limit=2050
/// prompt=55631`. Every qwen2.5:7b load in that log is 4096, while the models
/// whose Modelfile sets `num_ctx` (minicpm5 at 32768, qwen25-16k at 16384)
/// load at that size. So over `/v1` the window is the Modelfile's `num_ctx`,
/// else the server default — never the number this provider asks for. That
/// is why a request that needs more goes to the native `/api/chat` instead
/// (see [`request_window`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedWindow {
    pub tokens: usize,
    pub source: ServedWindowSource,
    /// The model's trained context length (`model_info.*.context_length`),
    /// the most a native request may ask for.
    pub trained: Option<usize>,
    /// What the model costs in memory, when `/api/show` says enough to tell
    /// (see [`ModelFootprint`]).
    pub footprint: Option<ModelFootprint>,
}

/// What a local model costs in memory, from `/api/show`: its weights, and the
/// K+V cache each token of context adds. [`auto_num_ctx`] sizes a window from
/// it so a model is never asked to load with more context than the machine
/// can hold beside its weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFootprint {
    pub weight_bytes: u64,
    pub kv_bytes_per_token: u64,
}

/// What `/api/show` tells us about a model's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowWindow {
    /// Runs on this Ollama; the window is knowable.
    Local(ServedWindow),
    /// An Ollama cloud model (`qwen3-coder:480b-cloud` and friends): Ollama
    /// proxies it to ollama.com, which neither loads it at the local server
    /// default nor truncates at 4096. Its window is not ours to guess, so
    /// nothing here sizes or refuses its requests.
    Remote,
}

/// How a request reaches Ollama.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaTransport {
    /// `/v1/chat/completions`: the window is whatever the model is served
    /// with ([`ServedWindow`]).
    OpenAiCompat,
    /// `/api/chat`: honours `options.num_ctx`, so the window is the one this
    /// provider asks for.
    Native,
}

/// The window a request will really run in, and the endpoint that gets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestWindow {
    pub tokens: usize,
    pub transport: OllamaTransport,
    pub served: ServedWindow,
}

impl RequestWindow {
    fn describe(&self) -> &'static str {
        match (self.transport, self.served.source) {
            (OllamaTransport::Native, _) => "the num_ctx this app asks Ollama for",
            (OllamaTransport::OpenAiCompat, ServedWindowSource::Modelfile) => {
                "the num_ctx its Modelfile sets"
            }
            (OllamaTransport::OpenAiCompat, ServedWindowSource::ServerConfigured) => {
                "the server-wide OLLAMA_CONTEXT_LENGTH configured here: its Modelfile sets no \
                 num_ctx"
            }
            (OllamaTransport::OpenAiCompat, ServedWindowSource::ServerDefault) => {
                "Ollama's default window: its Modelfile sets no num_ctx and no \
                 larger context limit is configured for it here"
            }
        }
    }
}

/// Which window and endpoint a request gets.
///
/// `requested` is the `num_ctx` this app configures for the model
/// (`GOOSE_INPUT_LIMIT`, else the model config's context limit — the CLI
/// fallback backfills it from the knowledge row, 32768 for `qwen2.5:7b`).
/// When that is more than the model is served with, `/v1` would silently cut
/// the prompt down to the served window, so the request goes native with
/// `num_ctx` capped at the trained length. Otherwise nothing changes: `/v1`,
/// and a served window at least as large as asked.
pub fn request_window(served: ServedWindow, requested: Option<usize>) -> RequestWindow {
    let wanted = requested
        .filter(|n| *n > 0)
        .map(|n| served.trained.map_or(n, |trained| n.min(trained)));
    match wanted {
        Some(tokens) if tokens > served.tokens => RequestWindow {
            tokens,
            transport: OllamaTransport::Native,
            served,
        },
        _ => RequestWindow {
            tokens: served.tokens,
            transport: OllamaTransport::OpenAiCompat,
            served,
        },
    }
}

/// The most this app asks Ollama for on its own initiative (see
/// [`auto_num_ctx`]) when nothing else sizes the window and `OLLAMA_MAX_NUM_CTX`
/// (env or config) is not set. An upper bound only: the memory ceiling for
/// the model's own size ([`memory_num_ctx_ceiling`]) usually decides first
/// for anything larger than a 7–8B model.
const OLLAMA_DEFAULT_AUTO_NUM_CTX_CAP: usize = 32_768;

fn resolve_ollama_auto_num_ctx_cap() -> usize {
    match crate::config::Config::global().get_param::<usize>("OLLAMA_MAX_NUM_CTX") {
        Ok(n) if n > 0 => n,
        _ => OLLAMA_DEFAULT_AUTO_NUM_CTX_CAP,
    }
}

/// The share of the Ollama host's memory a model's weights and KV cache may
/// take together: Metal's default GPU working set on Apple silicon is about
/// two thirds of RAM on the 8–36 GB machines this runs on, and the rest of
/// the machine still has to run beside it.
const OLLAMA_MODEL_MEMORY_SHARE: (u64, u64) = (2, 3);

/// Held back from that share for Ollama's compute buffers and runtime.
const OLLAMA_RUNTIME_RESERVE_BYTES: u64 = 1 << 30;

/// Bytes one K or V element takes in Ollama's default f16 KV cache. A server
/// run with `OLLAMA_KV_CACHE_TYPE=q8_0`/`q4_0` takes less, so this over-counts
/// for it — the safe direction.
const KV_CACHE_BYTES_PER_ELEMENT: u64 = 2;

/// Pure: the most context a model of `footprint` can be loaded with in
/// `memory_bytes` of host memory — [`OLLAMA_MODEL_MEMORY_SHARE`] of it, less
/// the weights and [`OLLAMA_RUNTIME_RESERVE_BYTES`], divided by the KV cache
/// per token — rounded down to a multiple of 1024. Zero when the weights
/// alone leave no room.
///
/// A 16 GB Mac holds `qwen2.5-coder:7b` (Q4_K_M, ~4.7 GB of weights, 56 KiB
/// of KV per token) at far more than 32768, but `qwen2.5-coder:14b` (~9 GB,
/// 192 KiB per token) only at about 6k: at 32768 its 6 GB cache pushes it out
/// of the working set, onto the CPU or refused outright.
pub fn memory_num_ctx_ceiling(footprint: ModelFootprint, memory_bytes: u64) -> usize {
    let (share, of) = OLLAMA_MODEL_MEMORY_SHARE;
    let usable = memory_bytes / of * share;
    let left = usable
        .saturating_sub(footprint.weight_bytes)
        .saturating_sub(OLLAMA_RUNTIME_RESERVE_BYTES);
    let tokens = left / footprint.kv_bytes_per_token.max(1);
    usize::try_from(tokens / 1024 * 1024).unwrap_or(usize::MAX)
}

/// How much memory the Ollama this client talks to has: `OLLAMA_HOST_MEMORY_GB`
/// (env or config) when set — the only way to know it for an Ollama on
/// another machine — else this machine's physical memory when Ollama runs
/// here. `None` for a remote Ollama with nothing configured: its model is
/// then sized by the trained length and the cap alone.
fn resolve_ollama_host_memory_bytes(is_local_host: bool) -> Option<u64> {
    let configured_gb = crate::config::Config::global()
        .get_param::<u64>("OLLAMA_HOST_MEMORY_GB")
        .ok()
        .filter(|gb| *gb > 0);
    if let Some(gb) = configured_gb {
        return Some(gb.saturating_mul(1 << 30));
    }
    if is_local_host {
        physical_memory_bytes()
    } else {
        None
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn physical_memory_bytes() -> Option<u64> {
    // `sysconf` only reads system configuration; it has no preconditions.
    let (pages, page_size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    let pages = u64::try_from(pages).ok().filter(|n| *n > 0)?;
    let page_size = u64::try_from(page_size).ok().filter(|n| *n > 0)?;
    pages.checked_mul(page_size)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn physical_memory_bytes() -> Option<u64> {
    None
}

/// The `num_ctx` to ask for when this app configures none for the model
/// (no `GOOSE_INPUT_LIMIT`, no context limit on the model config — the case
/// for any Ollama model the canonical registry has no row for).
///
/// Why: 2026-09-25 (cli log 20260925_105310, goal 01a0d8d8) a goal worker
/// routed to the local mechanical role, `qwen2.5-coder:7b`, whose model
/// config came back with `context_limit: null`. Nothing asked for more than
/// Ollama's 4096 default, so the request went to `/v1` at 4096 and the
/// worker's FIRST request (brief + tool schemas, 6,361 tokens) was refused
/// before it was sent — for a model trained for 32768. Sizing from the
/// model's own trained length (`/api/show`'s `model_info.*.context_length`)
/// lets the native endpoint serve it.
///
/// What fits decides, never one number for every model: the trained length,
/// `cap` (`OLLAMA_MAX_NUM_CTX`, else 32768), and — when the model's size
/// and the host's memory are both known — [`memory_num_ctx_ceiling`]. Every
/// request to the model then loads at this size, so a 14B on a 16 GB Mac is
/// not loaded with a 32768 cache that will not fit beside its weights.
///
/// `None` (leave the served window alone) when:
/// - the Modelfile sets `num_ctx`, or `OLLAMA_CONTEXT_LENGTH` is configured
///   for the server — both are somebody's explicit choice, usually to save
///   memory, and overriding them could exceed what they sized it for;
/// - the trained length is unknown — nothing to size from;
/// - what fits is no bigger than what is already served.
pub fn auto_num_ctx(served: ServedWindow, cap: usize, memory_bytes: Option<u64>) -> Option<usize> {
    if served.source != ServedWindowSource::ServerDefault {
        return None;
    }
    let mut wanted = served.trained?.min(cap);
    if let (Some(footprint), Some(memory_bytes)) = (served.footprint, memory_bytes) {
        wanted = wanted.min(memory_num_ctx_ceiling(footprint, memory_bytes));
    }
    (wanted > served.tokens).then_some(wanted)
}

/// The `num_ctx` a Modelfile sets, from `/api/show`'s `parameters` text
/// (`"num_ctx                        16384\ntemperature 0"`).
fn modelfile_num_ctx(show: &Value) -> Option<usize> {
    show.get("parameters")?
        .as_str()?
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            if parts.next()? != "num_ctx" {
                return None;
            }
            parts.next()?.parse::<usize>().ok()
        })
        .filter(|n| *n > 0)
}

/// A `model_info` count for the model's own architecture
/// (`"qwen2.block_count": 28`), or the largest entry when it varies per layer
/// (an array, as `head_count_kv` is for some architectures). Keyed by
/// `general.architecture` so a vision tower's `*.vision.block_count` is
/// never read as the language model's.
fn architecture_count(
    info: &serde_json::Map<String, Value>,
    arch: &str,
    name: &str,
) -> Option<u64> {
    let value = info.get(&format!("{arch}.{name}"))?;
    value
        .as_u64()
        .or_else(|| value.as_array()?.iter().filter_map(Value::as_u64).max())
        .filter(|n| *n > 0)
}

/// `details.parameter_size` (`"7.6B"`, `"494.03M"`) as a parameter count.
fn parse_parameter_size(size: &str) -> Option<u64> {
    let size = size.trim();
    let (number, scale) = match size.chars().last()?.to_ascii_uppercase() {
        'B' => (&size[..size.len() - 1], 1e9),
        'M' => (&size[..size.len() - 1], 1e6),
        'K' => (&size[..size.len() - 1], 1e3),
        _ => (size, 1.0),
    };
    let value: f64 = number.trim().parse().ok()?;
    (value.is_finite() && value > 0.0).then_some((value * scale).round() as u64)
}

/// Bits a weight takes at an Ollama `details.quantization_level`, block
/// scales included (llama.cpp's own figures, rounded up), by prefix. An
/// unknown level counts as f16, the size a model ships at before it is
/// quantised: a size that cannot be read is counted big, never small.
fn quantization_bits_per_weight(level: Option<&str>) -> f64 {
    const BITS: &[(&str, f64)] = &[
        ("F32", 32.0),
        ("BF16", 16.0),
        ("F16", 16.0),
        ("Q8", 8.5),
        ("Q6", 6.6),
        ("Q5", 5.7),
        ("Q4", 4.9),
        ("IQ4", 4.9),
        ("MXFP4", 4.9),
        ("Q3", 4.0),
        ("IQ3", 4.0),
        ("Q2", 3.4),
        ("IQ2", 3.4),
        ("IQ1", 3.4),
    ];
    let level = level.unwrap_or_default().trim().to_ascii_uppercase();
    BITS.iter()
        .find(|(prefix, _)| level.starts_with(prefix))
        .map_or(16.0, |(_, bits)| *bits)
}

/// What the model costs in memory (see [`ModelFootprint`]), from
/// `/api/show`: the weights from `general.parameter_count` (else
/// `details.parameter_size`) at `details.quantization_level`, and the f16
/// K+V cache per token from the architecture's layer count, KV heads and
/// head size. `None` when any of those is missing.
fn model_footprint(show: &Value) -> Option<ModelFootprint> {
    let info = show.get("model_info")?.as_object()?;
    let arch = info.get("general.architecture")?.as_str()?;
    let count = |name: &str| architecture_count(info, arch, name);
    let layers = count("block_count")?;
    let heads = count("attention.head_count")?;
    let kv_heads = count("attention.head_count_kv").unwrap_or(heads);
    let kv_per_head = match (
        count("attention.key_length"),
        count("attention.value_length"),
    ) {
        (Some(key), Some(value)) => key.saturating_add(value),
        (Some(one), None) | (None, Some(one)) => one.saturating_mul(2),
        (None, None) => count("embedding_length")?
            .checked_div(heads)?
            .saturating_mul(2),
    };
    let kv_bytes_per_token = layers
        .saturating_mul(kv_heads)
        .saturating_mul(kv_per_head)
        .saturating_mul(KV_CACHE_BYTES_PER_ELEMENT);
    let parameters = info
        .get("general.parameter_count")
        .and_then(Value::as_u64)
        .or_else(|| {
            show.pointer("/details/parameter_size")
                .and_then(Value::as_str)
                .and_then(parse_parameter_size)
        })?;
    let bits = quantization_bits_per_weight(
        show.pointer("/details/quantization_level")
            .and_then(Value::as_str),
    );
    let weight_bytes = (parameters as f64 * bits / 8.0) as u64;
    (kv_bytes_per_token > 0 && weight_bytes > 0).then_some(ModelFootprint {
        weight_bytes,
        kv_bytes_per_token,
    })
}

/// The trained context length, from `/api/show`'s `model_info`
/// (`"qwen2.context_length": 32768`).
fn trained_context_length(show: &Value) -> Option<usize> {
    show.get("model_info")?
        .as_object()?
        .iter()
        .find(|(key, _)| key.ends_with(".context_length"))
        .and_then(|(_, value)| value.as_u64())
        .map(|n| n as usize)
        .filter(|n| *n > 0)
}

/// Whether a model name is an Ollama cloud tag (`gpt-oss:120b-cloud`,
/// `qwen3-coder:480b-cloud`, `deepseek-v3.1:671b-cloud`, `name:cloud`).
fn is_cloud_model_name(model: &str) -> bool {
    model
        .rsplit_once(':')
        .is_some_and(|(_, tag)| tag == "cloud" || tag.ends_with("-cloud"))
}

/// Whether `/api/show` describes a model Ollama proxies to ollama.com rather
/// than runs here: cloud models carry `remote_host` / `remote_model`.
fn show_is_remote(show: &Value) -> bool {
    ["remote_host", "remote_model"].iter().any(|key| {
        show.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    })
}

/// The server-wide window configured for this client, when there is one.
/// `OLLAMA_CONTEXT_LENGTH` is the server's own variable name; setting it here
/// (env or config) tells the client what the server was started with, which
/// it cannot see. `None` means Ollama's own 4096 default applies.
fn resolve_ollama_configured_server_ctx() -> Option<usize> {
    crate::config::Config::global()
        .get_param::<usize>("OLLAMA_CONTEXT_LENGTH")
        .ok()
        .filter(|n| *n > 0)
}

fn served_window_from_show(
    show: &Value,
    model: &str,
    configured_server_ctx: Option<usize>,
) -> ShowWindow {
    if is_cloud_model_name(model) || show_is_remote(show) {
        return ShowWindow::Remote;
    }
    let trained = trained_context_length(show);
    let footprint = model_footprint(show);
    let (tokens, source) = match (modelfile_num_ctx(show), configured_server_ctx) {
        (Some(tokens), _) => (tokens, ServedWindowSource::Modelfile),
        (None, Some(tokens)) => (tokens, ServedWindowSource::ServerConfigured),
        (None, None) => (
            OLLAMA_SERVER_DEFAULT_NUM_CTX,
            ServedWindowSource::ServerDefault,
        ),
    };
    ShowWindow::Local(ServedWindow {
        tokens,
        source,
        trained,
        footprint,
    })
}

/// Whether an estimated prompt is over the window the request will run in.
///
/// No slack above the window. The estimate is o200k over the request JSON:
/// on the reported request, mostly JSON tool schemas, it ran about 12% above
/// Ollama's own count (62,364 against 55,631), but on a prompt that is mostly
/// prose o200k and the qwen tokenizer land about even. Any slack above the
/// window would let a real 4,097-to-4,600-token prose prompt through to be
/// cut, which is the failure this guard exists to stop. So the only error
/// the comparison allows is the over-count, which refuses a request that
/// sits within the tokenizer gap of the edge rather than truncating one.
fn exceeds_window(estimated_tokens: usize, window: usize) -> bool {
    estimated_tokens > window
}

/// Whether the request carries an image. Base64 image data would be counted
/// as text and swamp the estimate, so the preflight stands aside for it.
fn payload_carries_images(payload: &Value) -> bool {
    payload
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part.get("type").and_then(Value::as_str) == Some("image_url")
                        })
                    })
            })
        })
}

/// o200k count of what the request actually carries to the model: the
/// messages (system prompt included) and the tool schemas.
async fn estimated_prompt_tokens(payload: &Value) -> Option<usize> {
    if payload_carries_images(payload) {
        return None;
    }
    let counter = crate::token_counter::create_token_counter().await.ok()?;
    let pieces: Vec<String> = ["messages", "tools"]
        .iter()
        .filter_map(|key| payload.get(*key).map(Value::to_string))
        .collect();
    // A full tool surface is hundreds of KB of JSON; keep the tokenizer off
    // the async runtime the stream runs on.
    tokio::task::spawn_blocking(move || {
        pieces
            .iter()
            .map(|piece| counter.count_tokens(piece))
            .fold(0usize, usize::saturating_add)
    })
    .await
    .ok()
}

/// The refusal for a request Ollama would truncate. Carries llama.cpp-shaped
/// `"n_prompt_tokens"` / `"n_ctx"` numbers so
/// `errors::extract_context_length_token_counts` reads it like the 400 a
/// llama-server endpoint sends for the same overflow.
fn served_window_refusal(
    model: &str,
    estimated_tokens: usize,
    window: RequestWindow,
) -> ProviderError {
    ProviderError::ContextLengthExceeded(format!(
        "this request is about {estimated_tokens} tokens (\"n_prompt_tokens\": {estimated_tokens}) \
         but Ollama runs {model} in a {} token window (\"n_ctx\": {}; {}). Ollama would \
         silently keep only the tail of it, so it was not sent. Use a model with a larger \
         window, fewer extensions, or a larger context limit for this model (GOOSE_INPUT_LIMIT, \
         or a Modelfile num_ctx).",
        window.tokens,
        window.tokens,
        window.describe(),
    ))
}

/// The window a request for `model_config` will really run in on the Ollama
/// this process is configured for (`OLLAMA_HOST`) — the same answer `stream`
/// acts on. `None` for a cloud model or when Ollama cannot be asked. For
/// callers that size a session before its first turn, such as the CLI
/// builder fitting extensions to a local fallback.
pub async fn request_window_for_model_config(model_config: &ModelConfig) -> Option<RequestWindow> {
    let provider = OllamaProvider::from_env(model_config.clone()).await.ok()?;
    provider.request_window(model_config).await
}

// --- Native `/api/chat` translation --------------------------------------
//
// The request is still built by the shared OpenAI formatter (`create_request`)
// so both endpoints carry the same prompt, tools and history; these turn that
// payload into `/api/chat`'s shape, and `/api/chat`'s NDJSON stream back into
// the OpenAI SSE lines `response_to_streaming_message_ollama` already parses.

/// Text and base64 images from an OpenAI `content` (string, parts, or null).
fn native_content(content: Option<&Value>) -> (String, Vec<String>) {
    match content {
        Some(Value::String(text)) => (text.clone(), Vec::new()),
        Some(Value::Array(parts)) => {
            let mut texts = Vec::new();
            let mut images = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            texts.push(text.to_string());
                        }
                    }
                    Some("image_url") => {
                        let data = part
                            .get("image_url")
                            .and_then(|image| image.get("url"))
                            .and_then(Value::as_str)
                            .and_then(|url| url.split_once("base64,"))
                            .map(|(_, data)| data.to_string());
                        match data {
                            Some(data) => images.push(data),
                            None => tracing::warn!(
                                target: "permagent::providers::ollama",
                                "dropping a non-inline image: /api/chat takes base64 only"
                            ),
                        }
                    }
                    _ => {}
                }
            }
            (texts.join("\n\n"), images)
        }
        _ => (String::new(), Vec::new()),
    }
}

/// OpenAI-shaped messages as `/api/chat` takes them: `developer` becomes
/// `system`, tool-call arguments become objects, images move to `images`, a
/// tool result names its tool, and echoed reasoning rides as `thinking`.
fn native_messages(messages: &[Value]) -> Vec<Value> {
    let mut tool_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut out = Vec::with_capacity(messages.len());
    for message in messages {
        let role = match message.get("role").and_then(Value::as_str) {
            Some("developer") => "system",
            Some(role) => role,
            None => "user",
        };
        let (text, images) = native_content(message.get("content"));
        let mut native = json!({ "role": role, "content": text });
        if !images.is_empty() {
            native["images"] = json!(images);
        }
        if let Some(thinking) = message
            .get("reasoning_content")
            .or_else(|| message.get("reasoning"))
            .and_then(Value::as_str)
            .filter(|thinking| !thinking.is_empty())
        {
            native["thinking"] = json!(thinking);
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            let mut native_calls = Vec::with_capacity(calls.len());
            for call in calls {
                let Some(function) = call.get("function") else {
                    continue;
                };
                let Some(name) = function.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments = match function.get("arguments") {
                    Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| json!({})),
                    Some(value) if value.is_object() => value.clone(),
                    _ => json!({}),
                };
                let mut native_call =
                    json!({ "function": { "name": name, "arguments": arguments } });
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    tool_names.insert(id.to_string(), name.to_string());
                    native_call["id"] = json!(id);
                }
                native_calls.push(native_call);
            }
            if !native_calls.is_empty() {
                native["tool_calls"] = json!(native_calls);
            }
        }
        if role == "tool" {
            if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                native["tool_call_id"] = json!(id);
                if let Some(name) = tool_names.get(id) {
                    native["tool_name"] = json!(name);
                }
            }
        }
        out.push(native);
    }
    out
}

/// The `/api/chat` body for an OpenAI-shaped `payload`, run in `num_ctx`.
fn native_chat_request(payload: &Value, num_ctx: usize) -> Value {
    let mut options = payload
        .get("options")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    options.insert("num_ctx".to_string(), json!(num_ctx));
    for key in ["temperature", "top_p", "seed"] {
        if let Some(value) = payload.get(key).filter(|value| !value.is_null()) {
            options.entry(key.to_string()).or_insert(value.clone());
        }
    }
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .map(native_messages)
        .unwrap_or_default();
    let mut body = json!({
        "model": payload.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages,
        "stream": true,
        "options": options,
    });
    if let Some(tools) = payload.get("tools").filter(|tools| !tools.is_null()) {
        body["tools"] = tools.clone();
    }
    // `/v1` maps `reasoning_effort` onto `think` the same way.
    if let Some(effort) = payload.get("reasoning_effort").and_then(Value::as_str) {
        body["think"] = if effort == "none" {
            json!(false)
        } else {
            json!(effort)
        };
    }
    body
}

/// Turns `/api/chat`'s NDJSON stream into OpenAI SSE lines, one line in and
/// zero or more lines out. Tool calls are collected and sent as one
/// `finish_reason: "tool_calls"` chunk when the stream is done, the shape the
/// OpenAI parser completes a tool-call message on.
#[derive(Default)]
struct NativeStreamTranslator {
    model: String,
    tool_calls: Vec<Value>,
}

impl NativeStreamTranslator {
    fn sse(&self, mut chunk: Value) -> String {
        // The parser only reports usage from a chunk that names its model.
        chunk["model"] = json!(self.model);
        format!("data: {chunk}")
    }

    fn translate(&mut self, line: &str) -> anyhow::Result<Vec<String>> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(Vec::new());
        }
        let chunk: Value = serde_json::from_str(line)
            .map_err(|e| anyhow::anyhow!("unreadable Ollama /api/chat stream line: {e}"))?;
        if let Some(error) = chunk.get("error") {
            let detail = error
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return Err(anyhow::anyhow!("Ollama error: {detail}"));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }

        let mut out = Vec::new();
        let message = chunk.get("message");
        let field = |key: &str| {
            message
                .and_then(|m| m.get(key))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let text = field("content");
        let thinking = field("thinking");
        if !text.is_empty() || !thinking.is_empty() {
            let mut delta = json!({ "role": "assistant" });
            if !text.is_empty() {
                delta["content"] = json!(text);
            }
            if !thinking.is_empty() {
                delta["reasoning"] = json!(thinking);
            }
            out.push(self.sse(json!({
                "choices": [{ "index": 0, "delta": delta, "finish_reason": null }]
            })));
        }

        if let Some(calls) = message
            .and_then(|m| m.get("tool_calls"))
            .and_then(Value::as_array)
        {
            for call in calls {
                let Some(function) = call.get("function") else {
                    continue;
                };
                let Some(name) = function.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments = match function.get("arguments") {
                    Some(Value::String(raw)) => raw.clone(),
                    Some(value) if !value.is_null() => value.to_string(),
                    _ => "{}".to_string(),
                };
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
                let index = self.tool_calls.len();
                self.tool_calls.push(json!({
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
        }

        if chunk.get("done").and_then(Value::as_bool) == Some(true) {
            let prompt = chunk
                .get("prompt_eval_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = chunk.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
            let usage = json!({
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": prompt + completion,
            });
            if self.tool_calls.is_empty() {
                let finish = match chunk.get("done_reason").and_then(Value::as_str) {
                    Some("length") => "length",
                    _ => "stop",
                };
                out.push(self.sse(json!({
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }]
                })));
                out.push(self.sse(json!({ "choices": [], "usage": usage })));
            } else {
                let calls = std::mem::take(&mut self.tool_calls);
                out.push(self.sse(json!({
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "tool_calls": calls },
                        "finish_reason": "tool_calls"
                    }],
                    "usage": usage
                })));
            }
            out.push("data: [DONE]".to_string());
        }
        Ok(out)
    }
}

type LineStream = std::pin::Pin<Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>>;

fn native_lines_to_openai_sse(lines: LineStream) -> LineStream {
    Box::pin(try_stream! {
        let mut lines = lines;
        let mut translator = NativeStreamTranslator::default();
        while let Some(line) = lines.next().await {
            for sse in translator.translate(&line?)? {
                yield sse;
            }
        }
    })
}

/// Whether an Ollama base URL points at this machine (loopback). Fail-closed:
/// anything that is not clearly loopback is treated as remote (cloud).
fn is_loopback_ollama(base_url: &Url) -> bool {
    crate::mesh::endpoint_is_loopback(base_url.as_str())
}
fn resolve_ollama_num_ctx(model_config: &ModelConfig) -> Option<usize> {
    let config = crate::config::Config::global();
    let input_limit = match config.get_param::<usize>("GOOSE_INPUT_LIMIT") {
        Ok(limit) if limit > 0 => Some(limit),
        Ok(_) => None,
        Err(crate::config::ConfigError::NotFound(_)) => None,
        Err(e) => {
            tracing::warn!("Invalid GOOSE_INPUT_LIMIT value: {}", e);
            None
        }
    };

    input_limit.or(model_config.context_limit)
}

fn resolve_ollama_stream_usage() -> bool {
    let config = crate::config::Config::global();
    match config.get_param::<bool>("OLLAMA_STREAM_USAGE") {
        Ok(val) => val,
        // Key not set: default to true. Ollama supports stream_options since
        // mid-2025 and most installs benefit from token usage tracking.
        Err(crate::config::ConfigError::NotFound(_)) => true,
        // Invalid value (e.g. "0", "yes", typo): warn and disable stream_options
        // so users who intended to opt out aren't silently left hanging.
        Err(e) => {
            tracing::warn!(
                "Invalid OLLAMA_STREAM_USAGE value ({}); disabling stream_options. \
                 Use true or false.",
                e
            );
            false
        }
    }
}

fn apply_ollama_options(payload: &mut Value, model_config: &ModelConfig) {
    if let Some(obj) = payload.as_object_mut() {
        // Gate stream_options behind OLLAMA_STREAM_USAGE (default: true).
        // Older Ollama builds that don't support stream_options may stall before
        // emitting any SSE data, blocking until the client timeout (600s).
        // with_line_timeout() only protects after the first line arrives, so
        // users on older builds should set OLLAMA_STREAM_USAGE=false.
        if !resolve_ollama_stream_usage() {
            obj.remove("stream_options");
        }

        // Convert max_completion_tokens / max_tokens to Ollama's options.num_predict.
        // Reasoning models emit max_completion_tokens; non-reasoning models emit max_tokens.
        let max_tokens = obj
            .remove("max_completion_tokens")
            .or_else(|| obj.remove("max_tokens"));
        if let Some(max_tokens) = max_tokens {
            let options = obj.entry("options").or_insert_with(|| json!({}));
            if let Some(options_obj) = options.as_object_mut() {
                options_obj.entry("num_predict").or_insert(max_tokens);
            }
        }

        // Apply num_ctx from context limit settings. `/v1` ignores it (see
        // `ServedWindow`); when it asks for more than the model is served
        // with, `stream` sends the request to `/api/chat` instead, where it
        // is honoured (see `request_window`).
        if let Some(limit) = resolve_ollama_num_ctx(model_config) {
            let options = obj.entry("options").or_insert_with(|| json!({}));
            if let Some(options_obj) = options.as_object_mut() {
                options_obj.insert("num_ctx".to_string(), json!(limit));
            }
        }
    }
}

impl OllamaProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let host: String = config
            .get_param("OLLAMA_HOST")
            .unwrap_or_else(|_| OLLAMA_HOST.to_string());

        let timeout: Duration =
            Duration::from_secs(config.get_param("OLLAMA_TIMEOUT").unwrap_or(OLLAMA_TIMEOUT));

        let base = if host.starts_with("http://") || host.starts_with("https://") {
            host.clone()
        } else {
            format!("http://{}", host)
        };

        let mut base_url =
            Url::parse(&base).map_err(|e| anyhow::anyhow!("Invalid base URL: {e}"))?;

        let explicit_port = host.contains(':');
        let is_localhost = host == "localhost" || host == "127.0.0.1" || host == "::1";

        if base_url.port().is_none() && !explicit_port && !host.starts_with("http") && is_localhost
        {
            base_url
                .set_port(Some(OLLAMA_DEFAULT_PORT))
                .map_err(|_| anyhow::anyhow!("Failed to set default port"))?;
        }

        let api_client =
            ApiClient::with_timeout(base_url.to_string(), AuthMethod::NoAuth, timeout)?;

        let is_local_host = is_loopback_ollama(&base_url);

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: OLLAMA_PROVIDER_NAME.to_string(),
            skip_canonical_filtering: false,
            is_local_host,
            served_windows: Default::default(),
        })
    }

    /// What `/api/show` says about `model`'s window (see [`ShowWindow`]),
    /// cached per model. `None` when it cannot be read: then nothing is known,
    /// and `stream` behaves as it always has rather than guess.
    async fn show_window(&self, model: &str) -> Option<ShowWindow> {
        // A cloud tag is known without asking.
        if is_cloud_model_name(model) {
            return Some(ShowWindow::Remote);
        }
        // Bound in its own statement so the lock guard drops before any await.
        let cached = self
            .served_windows
            .lock()
            .ok()
            .and_then(|cache| cache.get(model).copied());
        if let Some(window) = cached {
            return Some(window);
        }
        let response = self
            .api_client
            .request(None, "api/show")
            .api_post(&json!({ "model": model }))
            .await
            .ok()?;
        if !response.status.is_success() {
            return None;
        }
        let window = served_window_from_show(
            response.payload.as_ref()?,
            model,
            resolve_ollama_configured_server_ctx(),
        );
        if let Ok(mut cache) = self.served_windows.lock() {
            cache.insert(model.to_string(), window);
        }
        Some(window)
    }

    /// The window a local `model` is served with over `/v1` (see
    /// [`ServedWindow`]). `None` for a cloud model or when Ollama cannot be
    /// asked.
    pub async fn served_context_window(&self, model: &str) -> Option<ServedWindow> {
        match self.show_window(model).await? {
            ShowWindow::Local(window) => Some(window),
            ShowWindow::Remote => None,
        }
    }

    /// The window a request for `model_config` will run in, and the endpoint
    /// it goes to (see [`request_window`]). `None` for a cloud model or when
    /// Ollama cannot be asked: `/v1`, unguarded, as before.
    pub async fn request_window(&self, model_config: &ModelConfig) -> Option<RequestWindow> {
        let served = self.served_context_window(&model_config.model_name).await?;
        let requested = resolve_ollama_num_ctx(model_config).or_else(|| {
            auto_num_ctx(
                served,
                resolve_ollama_auto_num_ctx_cap(),
                resolve_ollama_host_memory_bytes(self.is_local_host),
            )
        });
        Some(request_window(served, requested))
    }

    /// The payload `stream` sends, the window it will run in (`None` for a
    /// cloud model or an unreadable `/api/show`), and the native body when
    /// that window needs `/api/chat`. Shared by `stream` and
    /// [`Provider::preflight_request`] so the pre-turn check and the send
    /// judge exactly the same request.
    async fn prepare_request(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Value, Option<RequestWindow>), ProviderError> {
        let mut payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            true,
        )?;
        apply_ollama_options(&mut payload, model_config);
        let window = self.request_window(model_config).await;
        Ok((payload, window))
    }

    /// Refuse, before sending, a request Ollama would silently truncate.
    ///
    /// The CLI's `permagent run` fallback onto `ollama/qwen2.5:7b` sent a
    /// ~55k-token prompt (101KB system prompt + 156 tool schemas) into a
    /// 4096-token window; Ollama kept the last 2,050 tokens and the model
    /// answered from a fragment, with nothing in any log but Ollama's own.
    /// A loud `ContextLengthExceeded` is what a llama-server endpoint returns
    /// for the same overflow, and it is what the agent loop already handles.
    async fn preflight_request_window(
        &self,
        model: &str,
        payload: &Value,
        window: RequestWindow,
    ) -> Result<(), ProviderError> {
        let Some(estimated) = estimated_prompt_tokens(payload).await else {
            return Ok(());
        };
        if !exceeds_window(estimated, window.tokens) {
            return Ok(());
        }
        tracing::warn!(
            target: "permagent::providers::ollama",
            model,
            estimated_prompt_tokens = estimated,
            window = window.tokens,
            transport = ?window.transport,
            served_window = window.served.tokens,
            served_window_source = ?window.served.source,
            "refusing a request Ollama would silently truncate"
        );
        Err(served_window_refusal(model, estimated, window))
    }

    pub fn from_custom_config(
        model: ModelConfig,
        config: DeclarativeProviderConfig,
    ) -> Result<Self> {
        let timeout = Duration::from_secs(config.timeout_seconds.unwrap_or(OLLAMA_TIMEOUT));

        let base =
            if config.base_url.starts_with("http://") || config.base_url.starts_with("https://") {
                config.base_url.clone()
            } else {
                format!("http://{}", config.base_url)
            };

        let mut base_url = Url::parse(&base)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let explicit_default_port =
            config.base_url.ends_with(":80") || config.base_url.ends_with(":443");
        let is_https = base_url.scheme() == "https";

        if base_url.port().is_none() && !explicit_default_port && !is_https {
            base_url
                .set_port(Some(OLLAMA_DEFAULT_PORT))
                .map_err(|_| anyhow::anyhow!("Failed to set default port"))?;
        }

        let mut api_client =
            ApiClient::with_timeout(base_url.to_string(), AuthMethod::NoAuth, timeout)?;

        if let Some(headers) = &config.headers {
            let mut header_map = reqwest::header::HeaderMap::new();
            for (key, value) in headers {
                let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())?;
                let header_value = reqwest::header::HeaderValue::from_str(value)?;
                header_map.insert(header_name, header_value);
            }
            api_client = api_client.with_headers(header_map)?;
        }

        let supports_streaming = config.supports_streaming.unwrap_or(true);

        if !supports_streaming {
            return Err(anyhow::anyhow!(
                "Ollama provider does not support non-streaming mode. All Ollama models support streaming. \
                Please remove 'supports_streaming: false' from your provider configuration."
            ));
        }

        let model = if let Some(ref fast_model_name) = config.fast_model {
            model.with_fast(fast_model_name, &config.name)?
        } else {
            model
        };

        let is_local_host = is_loopback_ollama(&base_url);

        Ok(Self {
            api_client,
            model,
            supports_streaming,
            name: config.name.clone(),
            skip_canonical_filtering: config.skip_canonical_filtering,
            is_local_host,
            served_windows: Default::default(),
        })
    }
}

impl ProviderDef for OllamaProvider {
    type Provider = Self;

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            OLLAMA_PROVIDER_NAME,
            "Ollama",
            "Local open source models",
            OLLAMA_DEFAULT_MODEL,
            OLLAMA_KNOWN_MODELS.to_vec(),
            OLLAMA_DOC_URL,
            vec![
                ConfigKey::new("OLLAMA_HOST", true, false, Some(OLLAMA_HOST), true),
                ConfigKey::new(
                    "OLLAMA_TIMEOUT",
                    false,
                    false,
                    Some(&(OLLAMA_TIMEOUT.to_string())),
                    false,
                ),
            ],
        )
    }

    fn from_env(
        model: ModelConfig,
        _extensions: Vec<crate::config::ExtensionConfig>,
    ) -> BoxFuture<'static, Result<Self::Provider>> {
        Box::pin(Self::from_env(model))
    }

    fn supports_inventory_refresh() -> bool {
        true
    }

    fn inventory_identity() -> Result<InventoryIdentityInput> {
        let config = crate::config::Config::global();
        Ok(
            InventoryIdentityInput::new(OLLAMA_PROVIDER_NAME, OLLAMA_PROVIDER_NAME).with_public(
                "host",
                config
                    .get_param::<String>("OLLAMA_HOST")
                    .unwrap_or_else(|_| OLLAMA_HOST.to_string()),
            ),
        )
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn cost_tier(&self) -> crate::session::CostTier {
        crate::session::CostTier::LocalFree
    }

    fn skip_canonical_filtering(&self) -> bool {
        self.skip_canonical_filtering
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    /// Local only when the Ollama host is loopback. A remote Ollama ships the
    /// prompt off this machine, so it is treated as cloud egress (fail-closed).
    fn data_locality(&self) -> crate::sovereignty::DataLocality {
        if self.is_local_host {
            crate::sovereignty::DataLocality::Local
        } else {
            crate::sovereignty::DataLocality::Cloud
        }
    }

    fn retry_config(&self) -> RetryConfig {
        RetryConfig::new(
            OLLAMA_MAX_RETRIES,
            OLLAMA_INITIAL_RETRY_INTERVAL_MS,
            OLLAMA_BACKOFF_MULTIPLIER,
            OLLAMA_MAX_RETRY_INTERVAL_MS,
        )
        .transient_only()
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        // Which window this request really runs in decides the endpoint: a
        // `num_ctx` bigger than the model is served with only takes effect on
        // `/api/chat` (the 2026-09-19 fallback asked `/v1` for 32768 and got
        // 4096). A cloud model, or an Ollama whose `/api/show` cannot be read,
        // stays on `/v1` with no window check, as before.
        let (payload, window) = self
            .prepare_request(model_config, system, messages, tools)
            .await?;
        let transport = window.map_or(OllamaTransport::OpenAiCompat, |w| w.transport);
        let native_body = match window {
            Some(window) if window.transport == OllamaTransport::Native => {
                Some(native_chat_request(&payload, window.tokens))
            }
            _ => None,
        };
        let (path, body) = match native_body.as_ref() {
            Some(native) => ("api/chat", native),
            None => ("v1/chat/completions", &payload),
        };
        let mut log = RequestLog::start(model_config, body)?;
        if let Some(window) = window {
            if let Err(refusal) = self
                .preflight_request_window(&model_config.model_name, &payload, window)
                .await
            {
                let _ = log.error(&refusal);
                return Err(refusal);
            }
        }

        let response = self
            .with_retry(|| async {
                let resp = self
                    .api_client
                    .response_post(Some(session_id), path, body)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;
        stream_ollama(response, log, transport)
    }

    /// The window `stream` will run `model_config`'s requests in: the
    /// served window, or the `num_ctx` sent natively when that is bigger.
    /// `None` for a cloud model or when `/api/show` cannot be read.
    async fn request_context_window(&self, model_config: &ModelConfig) -> Option<usize> {
        self.request_window(model_config).await.map(|w| w.tokens)
    }

    /// The same refusal `stream` would return, without sending anything:
    /// the same payload, the same window, the same count.
    async fn preflight_request(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(), ProviderError> {
        let (payload, window) = self
            .prepare_request(model_config, system, messages, tools)
            .await?;
        match window {
            Some(window) => {
                self.preflight_request_window(&model_config.model_name, &payload, window)
                    .await
            }
            None => Ok(()),
        }
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        let response = self
            .api_client
            .request(None, "api/tags")
            .response_get()
            .await
            .map_err(|e| ProviderError::RequestFailed(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch models: HTTP {}",
                response.status()
            )));
        }

        let json_response = response.json::<Value>().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to parse response: {}", e))
        })?;

        let models = json_response
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| {
                ProviderError::RequestFailed("No models array in response".to_string())
            })?;

        let mut model_names: Vec<String> = models
            .iter()
            .filter_map(|model| model.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        model_names.sort();

        Ok(model_names)
    }
}

/// Default per-chunk timeout for Ollama streaming responses (seconds).
/// Configurable via OLLAMA_STREAM_TIMEOUT, GOOSE_STREAM_TIMEOUT, or falls back
/// to OLLAMA_TIMEOUT. Set high to accommodate slower models (CPU inference,
/// large parameter counts, complex reasoning).
const OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS: u64 = 120;

/// Resolve the per-chunk stream timeout from config.
/// Priority: OLLAMA_STREAM_TIMEOUT > GOOSE_STREAM_TIMEOUT > OLLAMA_TIMEOUT > default (120s).
/// Zero values are treated as invalid and skipped, since a zero timeout would
/// cause every chunk after the first to be treated as a stall.
fn resolve_ollama_chunk_timeout() -> u64 {
    let config = crate::config::Config::global();

    if let Ok(val) = config.get_param::<u64>("OLLAMA_STREAM_TIMEOUT") {
        if val > 0 {
            return val;
        }
    }
    if let Ok(val) = config.get_param::<u64>("GOOSE_STREAM_TIMEOUT") {
        if val > 0 {
            return val;
        }
    }
    match config.get_param::<u64>("OLLAMA_TIMEOUT") {
        Ok(val) if val > 0 => val,
        _ => OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS,
    }
}

/// Wraps a line stream with a per-item timeout at the raw SSE level.
/// This detects dead connections without false-positive stalls during long
/// tool-call generations where response_to_streaming_message_ollama buffers.
fn with_line_timeout(
    stream: impl futures::Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
    timeout_secs: u64,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>> {
    let timeout = Duration::from_secs(timeout_secs);
    Box::pin(try_stream! {
        let mut stream = stream;

        // Allow time-to-first-token to be governed by the request timeout.
        // Only enforce per-chunk timeout after first SSE line arrives.
        match stream.next().await {
            Some(first_item) => yield first_item?,
            None => return,
        }
        loop {
            match tokio::time::timeout(timeout, stream.next()).await {
                Ok(Some(item)) => yield item?,
                Ok(None) => break,
                Err(_) => {
                    Err::<(), anyhow::Error>(anyhow::anyhow!(
                        "Ollama stream stalled: no data received for {}s. \
                         This may indicate the model is overwhelmed by the request payload. \
                         Try a smaller model or reduce the number of tools.",
                        timeout_secs
                    ))?;
                }
            }
        }
    })
}

/// Ollama-specific streaming handler with XML tool call fallback.
/// Uses the Ollama format module which buffers text when XML tool calls are detected,
/// preventing duplicate content from being emitted to the UI.
/// Timeout is applied at the raw SSE line level via with_line_timeout so that
/// buffering inside response_to_streaming_message_ollama does not cause false stalls.
fn stream_ollama(
    response: Response,
    mut log: RequestLog,
    transport: OllamaTransport,
) -> Result<MessageStream, ProviderError> {
    let stream = response.bytes_stream().map_err(std::io::Error::other);

    Ok(Box::pin(try_stream! {
        let stream_reader = StreamReader::new(stream);
        let framed = FramedRead::new(stream_reader, LinesCodec::new())
            .map_err(Error::from);

        let chunk_timeout = resolve_ollama_chunk_timeout();
        let timed_lines = with_line_timeout(framed, chunk_timeout);
        // `/api/chat` streams NDJSON; the parser below reads OpenAI SSE.
        let sse_lines = match transport {
            OllamaTransport::Native => native_lines_to_openai_sse(timed_lines),
            OllamaTransport::OpenAiCompat => timed_lines,
        };
        let message_stream = response_to_streaming_message_ollama(sse_lines);
        pin!(message_stream);

        while let Some(message) = message_stream.next().await {
            let (message, usage) = message.map_err(ProviderError::stream_decode)?;
            log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
            yield (message, usage);
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_loopback(u: &str) -> bool {
        is_loopback_ollama(&Url::parse(u).unwrap())
    }

    #[test]
    fn loopback_ollama_is_local() {
        // Loopback hosts -> local (data stays on this machine).
        assert!(is_loopback("http://localhost:11434"));
        assert!(is_loopback("http://LOCALHOST:11434"));
        assert!(is_loopback("http://127.0.0.1:11434"));
        assert!(is_loopback("http://127.1.2.3:11434"));
        assert!(is_loopback("http://[::1]:11434"));
    }

    #[test]
    fn remote_ollama_is_cloud_fail_closed() {
        // Anything not clearly loopback -> cloud (fail-closed): a remote Ollama
        // ships the prompt off this machine.
        assert!(!is_loopback("http://192.168.1.50:11434"));
        assert!(!is_loopback("http://10.0.0.5:11434"));
        assert!(!is_loopback("https://ollama.example.com"));
        assert!(!is_loopback("http://ollama.internal:11434"));
    }

    #[test]
    fn test_apply_ollama_options_uses_input_limit() {
        let _guard = env_lock::lock_env([("GOOSE_INPUT_LIMIT", Some("8192"))]);
        let model_config = ModelConfig::new("qwen3")
            .unwrap()
            .with_context_limit(Some(16_000));
        let mut payload = json!({});
        apply_ollama_options(&mut payload, &model_config);
        assert_eq!(payload["options"]["num_ctx"], 8192);
    }

    #[test]
    fn test_apply_ollama_options_falls_back_to_context_limit() {
        let _guard = env_lock::lock_env([("GOOSE_INPUT_LIMIT", None::<&str>)]);
        let model_config = ModelConfig::new("qwen3")
            .unwrap()
            .with_context_limit(Some(12_000));
        let mut payload = json!({});
        apply_ollama_options(&mut payload, &model_config);
        assert_eq!(payload["options"]["num_ctx"], 12_000);
    }

    #[test]
    fn test_apply_ollama_options_skips_when_no_limit() {
        let _guard = env_lock::lock_env([("GOOSE_INPUT_LIMIT", None::<&str>)]);
        let mut model_config = ModelConfig::new("qwen3").unwrap();
        model_config.context_limit = None;
        let mut payload = json!({});
        apply_ollama_options(&mut payload, &model_config);
        assert!(payload.get("options").is_none());
    }

    #[test]
    fn test_raw_create_request_contains_unsupported_ollama_fields() {
        use crate::providers::formats::ollama::create_request;
        use crate::providers::utils::ImageFormat;

        let model_config = ModelConfig::new("llama3.1")
            .unwrap()
            .with_max_tokens(Some(4096));
        let messages = vec![crate::conversation::message::Message::user().with_text("hi")];

        let payload = create_request(
            &model_config,
            "You are a helpful assistant.",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            true,
        )
        .unwrap();

        assert!(
            payload.get("stream_options").is_some(),
            "create_request should produce stream_options for usage tracking"
        );
        assert!(
            payload.get("max_tokens").is_some(),
            "create_request should produce max_tokens (unsupported by Ollama)"
        );
    }

    #[test]
    fn test_apply_ollama_options_preserves_stream_options_by_default() {
        use crate::providers::formats::ollama::create_request;
        use crate::providers::utils::ImageFormat;

        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("OLLAMA_STREAM_USAGE", None::<&str>),
        ]);
        let model_config = ModelConfig::new("llama3.1")
            .unwrap()
            .with_max_tokens(Some(4096));
        let messages = vec![crate::conversation::message::Message::user().with_text("hi")];

        let mut payload = create_request(
            &model_config,
            "You are a helpful assistant.",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            true,
        )
        .unwrap();

        apply_ollama_options(&mut payload, &model_config);

        assert!(
            payload.get("stream_options").is_some(),
            "stream_options should be preserved by default for usage tracking"
        );
        assert!(
            payload.get("max_tokens").is_none(),
            "max_tokens should be removed for Ollama"
        );
        assert!(
            payload.get("max_completion_tokens").is_none(),
            "max_completion_tokens should be removed for Ollama"
        );
        assert_eq!(
            payload["options"]["num_predict"], 4096,
            "max_tokens should be moved to options.num_predict"
        );
        assert_eq!(payload["stream"], true, "stream field should be preserved");
    }

    #[test]
    fn test_apply_ollama_options_strips_stream_options_when_disabled() {
        use crate::providers::formats::ollama::create_request;
        use crate::providers::utils::ImageFormat;

        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("OLLAMA_STREAM_USAGE", Some("false")),
        ]);
        let model_config = ModelConfig::new("llama3.1")
            .unwrap()
            .with_max_tokens(Some(4096));
        let messages = vec![crate::conversation::message::Message::user().with_text("hi")];

        let mut payload = create_request(
            &model_config,
            "You are a helpful assistant.",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            true,
        )
        .unwrap();

        apply_ollama_options(&mut payload, &model_config);

        assert!(
            payload.get("stream_options").is_none(),
            "stream_options should be removed when OLLAMA_STREAM_USAGE=false"
        );
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_defaults_to_ollama_timeout() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", None::<&str>),
            ("GOOSE_STREAM_TIMEOUT", None::<&str>),
            ("OLLAMA_TIMEOUT", Some("300")),
        ]);
        assert_eq!(resolve_ollama_chunk_timeout(), 300);
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_prefers_stream_override() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", Some("60")),
            ("GOOSE_STREAM_TIMEOUT", Some("90")),
            ("OLLAMA_TIMEOUT", Some("300")),
        ]);
        assert_eq!(resolve_ollama_chunk_timeout(), 60);
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_uses_goose_stream_fallback() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", None::<&str>),
            ("GOOSE_STREAM_TIMEOUT", Some("90")),
            ("OLLAMA_TIMEOUT", Some("300")),
        ]);
        assert_eq!(resolve_ollama_chunk_timeout(), 90);
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_uses_default_when_unset() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", None::<&str>),
            ("GOOSE_STREAM_TIMEOUT", None::<&str>),
            ("OLLAMA_TIMEOUT", None::<&str>),
        ]);
        assert_eq!(
            resolve_ollama_chunk_timeout(),
            OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS
        );
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_skips_zero_values() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", Some("0")),
            ("GOOSE_STREAM_TIMEOUT", Some("0")),
            ("OLLAMA_TIMEOUT", Some("300")),
        ]);
        assert_eq!(resolve_ollama_chunk_timeout(), 300);
    }

    #[test]
    fn test_resolve_ollama_chunk_timeout_skips_all_zero_to_default() {
        let _guard = env_lock::lock_env([
            ("OLLAMA_STREAM_TIMEOUT", Some("0")),
            ("GOOSE_STREAM_TIMEOUT", Some("0")),
            ("OLLAMA_TIMEOUT", Some("0")),
        ]);
        assert_eq!(
            resolve_ollama_chunk_timeout(),
            OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS
        );
    }

    #[test]
    fn test_ollama_retry_config_is_transient_only() {
        let config = RetryConfig::new(
            OLLAMA_MAX_RETRIES,
            OLLAMA_INITIAL_RETRY_INTERVAL_MS,
            OLLAMA_BACKOFF_MULTIPLIER,
            OLLAMA_MAX_RETRY_INTERVAL_MS,
        )
        .transient_only();

        assert!(config.transient_only);

        use super::super::errors::ProviderError;
        use super::super::retry::should_retry;

        assert!(!should_retry(
            &ProviderError::RequestFailed("Resource not found (404)".into()),
            &config
        ));
        assert!(!should_retry(
            &ProviderError::RequestFailed("Bad request (400)".into()),
            &config
        ));
        assert!(should_retry(
            &ProviderError::ServerError("500 model loading".into()),
            &config
        ));
        assert!(should_retry(
            &ProviderError::NetworkError("connection refused".into()),
            &config
        ));
    }

    // --- Request window and native transport (r3-finding-1). The CLI
    // fallback sent a ~55k-token request to `qwen2.5:7b` over `/v1` with
    // `num_ctx: 32768`; Ollama ignored the num_ctx, served it in 4096 tokens
    // and printed `truncating input prompt limit=2050 prompt=55631`, while
    // the turn "succeeded". These pin the fix: a bigger window goes native
    // where it is honoured, and whatever still does not fit is refused.

    use crate::conversation::message::MessageContent;
    use crate::providers::base::ProviderUsage;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// `/api/show`'s `parameters` for `qwen25-16k:latest` on this machine.
    const QWEN25_16K_PARAMETERS: &str =
        "num_ctx                        16384\ntemperature                    0";

    /// `/api/show` for `qwen2.5:7b` on this machine: no `parameters`, trained
    /// for 32768.
    fn qwen25_7b_show() -> Value {
        json!({ "model_info": { "qwen2.context_length": 32768 } })
    }

    fn served(tokens: usize, source: ServedWindowSource, trained: Option<usize>) -> ServedWindow {
        ServedWindow {
            tokens,
            source,
            trained,
            footprint: None,
        }
    }

    fn provider_at(uri: String) -> OllamaProvider {
        OllamaProvider {
            api_client: ApiClient::with_timeout(uri, AuthMethod::NoAuth, Duration::from_secs(5))
                .expect("api client"),
            model: ModelConfig::new("qwen2.5:7b").unwrap(),
            supports_streaming: true,
            name: OLLAMA_PROVIDER_NAME.to_string(),
            skip_canonical_filtering: false,
            is_local_host: true,
            served_windows: Default::default(),
        }
    }

    #[test]
    fn a_modelfile_num_ctx_is_the_served_window() {
        let show = json!({
            "parameters": QWEN25_16K_PARAMETERS,
            "model_info": { "qwen2.context_length": 32768 }
        });
        assert_eq!(modelfile_num_ctx(&show), Some(16_384));
        assert_eq!(
            served_window_from_show(&show, "qwen25-16k:latest", None),
            ShowWindow::Local(served(16_384, ServedWindowSource::Modelfile, Some(32_768)))
        );
    }

    /// `qwen2.5:7b` has no `parameters` at all: over `/v1` the server default
    /// serves it, NOT its trained 32768 and NOT the num_ctx the request carries.
    #[test]
    fn no_modelfile_num_ctx_means_the_server_default() {
        let show = qwen25_7b_show();
        assert_eq!(modelfile_num_ctx(&show), None);
        assert_eq!(trained_context_length(&show), Some(32_768));
        assert_eq!(
            served_window_from_show(&show, "qwen2.5:7b", None),
            ShowWindow::Local(served(
                4096,
                ServedWindowSource::ServerDefault,
                Some(32_768)
            ))
        );
        // A parameter merely containing the text is not num_ctx.
        let show = json!({ "parameters": "num_ctx_extra 99\ntemperature 0.7" });
        assert_eq!(modelfile_num_ctx(&show), None);
    }

    /// A cloud model has no Modelfile num_ctx either, but Ollama proxies it to
    /// ollama.com, which does not truncate at 4096. It must never be sized at
    /// the local default and refused on that basis.
    #[test]
    fn an_ollama_cloud_model_is_never_sized_at_the_local_default() {
        let cloud_show = json!({
            "remote_host": "https://ollama.com:443",
            "remote_model": "qwen3-coder:480b",
            "model_info": {}
        });
        assert_eq!(
            served_window_from_show(&cloud_show, "my-alias:latest", None),
            ShowWindow::Remote
        );
        // The tag alone is enough, whatever /api/show says.
        for model in [
            "qwen3-coder:480b-cloud",
            "gpt-oss:120b-cloud",
            "deepseek-v3.1:671b-cloud",
            "kimi-k2:cloud",
        ] {
            assert!(is_cloud_model_name(model), "{model}");
            assert_eq!(
                served_window_from_show(&json!({}), model, None),
                ShowWindow::Remote,
                "{model}"
            );
        }
        for model in ["qwen2.5:7b", "qwen25-16k:latest", "cloudy:7b", "qwen3"] {
            assert!(!is_cloud_model_name(model), "{model}");
        }
    }

    #[test]
    fn a_bigger_requested_window_goes_native_capped_at_the_trained_length() {
        let qwen25_7b = served(4096, ServedWindowSource::ServerDefault, Some(32_768));
        // The CLI fallback's backfilled 32768: native, where it is honoured.
        assert_eq!(
            request_window(qwen25_7b, Some(32_768)),
            RequestWindow {
                tokens: 32_768,
                transport: OllamaTransport::Native,
                served: qwen25_7b,
            }
        );
        // Never more than the model was trained for.
        assert_eq!(request_window(qwen25_7b, Some(131_072)).tokens, 32_768);
        // Nothing asked: `/v1` at what it is served with, as before.
        assert_eq!(
            request_window(qwen25_7b, None),
            RequestWindow {
                tokens: 4096,
                transport: OllamaTransport::OpenAiCompat,
                served: qwen25_7b,
            }
        );
        // The Modelfile already serves what is asked, or more: stay on `/v1`.
        let qwen25_16k = served(16_384, ServedWindowSource::Modelfile, Some(32_768));
        assert_eq!(
            request_window(qwen25_16k, Some(16_384)).transport,
            OllamaTransport::OpenAiCompat
        );
        let smaller = request_window(qwen25_16k, Some(8_192));
        assert_eq!(smaller.transport, OllamaTransport::OpenAiCompat);
        assert_eq!(smaller.tokens, 16_384);
    }

    /// With nothing configured, the window is sized from the model's own
    /// trained length, capped for memory — never from its name.
    #[test]
    fn with_nothing_configured_the_window_is_sized_from_the_trained_length() {
        let cap = OLLAMA_DEFAULT_AUTO_NUM_CTX_CAP;
        // The reported case: qwen2.5-coder:7b, served at 4096, trained 32768.
        let coder = served(4096, ServedWindowSource::ServerDefault, Some(32_768));
        assert_eq!(auto_num_ctx(coder, cap, None), Some(32_768));
        assert_eq!(
            request_window(coder, auto_num_ctx(coder, cap, None)),
            RequestWindow {
                tokens: 32_768,
                transport: OllamaTransport::Native,
                served: coder,
            }
        );
        // A long-context model is capped, not loaded at 128k.
        let long = served(4096, ServedWindowSource::ServerDefault, Some(131_072));
        assert_eq!(auto_num_ctx(long, cap, None), Some(cap));
        assert_eq!(auto_num_ctx(long, 16_384, None), Some(16_384));
        // Nothing bigger to ask for: trained no larger than served, a cap no
        // larger than served, or an unknown trained length.
        let small = served(4096, ServedWindowSource::ServerDefault, Some(4096));
        assert_eq!(auto_num_ctx(small, cap, None), None);
        assert_eq!(auto_num_ctx(coder, 4096, None), None);
        let unknown = served(4096, ServedWindowSource::ServerDefault, None);
        assert_eq!(auto_num_ctx(unknown, cap, None), None);
        // A Modelfile num_ctx is somebody's explicit choice: left alone.
        let modelfile = served(16_384, ServedWindowSource::Modelfile, Some(32_768));
        assert_eq!(auto_num_ctx(modelfile, cap, None), None);
        // A server started with a bigger OLLAMA_CONTEXT_LENGTH already
        // serves enough.
        let big_default = served(65_536, ServedWindowSource::ServerConfigured, Some(131_072));
        assert_eq!(auto_num_ctx(big_default, cap, None), None);
        // And one started with a SMALLER one chose that size, usually to
        // save memory: it is not raised to the trained length or the cap
        // either. Before, a configured 8192 on a model trained for 131072
        // was loaded at 32768 on every request.
        let small_default = served(8_192, ServedWindowSource::ServerConfigured, Some(131_072));
        assert_eq!(auto_num_ctx(small_default, cap, None), None);
        assert_eq!(auto_num_ctx(small_default, cap, Some(64 << 30)), None);
    }

    /// `/api/show` for `qwen2.5-coder:14b` (Q4_K_M): 48 layers, 8 KV heads of
    /// 128 (5120 / 40), 14.8B parameters, trained for 32768, no Modelfile
    /// num_ctx.
    fn qwen25_coder_14b_show() -> Value {
        json!({
            "details": {
                "family": "qwen2",
                "parameter_size": "14.8B",
                "quantization_level": "Q4_K_M"
            },
            "model_info": {
                "general.architecture": "qwen2",
                "general.parameter_count": 14_770_033_664u64,
                "qwen2.block_count": 48,
                "qwen2.context_length": 32_768,
                "qwen2.embedding_length": 5_120,
                "qwen2.attention.head_count": 40,
                "qwen2.attention.head_count_kv": 8
            }
        })
    }

    /// `/api/show` for `qwen2.5-coder:7b` (Q4_K_M): 28 layers, 4 KV heads of
    /// 128 (3584 / 28), 7.6B parameters, trained for 32768.
    fn qwen25_coder_7b_show() -> Value {
        json!({
            "details": { "parameter_size": "7.6B", "quantization_level": "Q4_K_M" },
            "model_info": {
                "general.architecture": "qwen2",
                "general.parameter_count": 7_615_616_512u64,
                "qwen2.block_count": 28,
                "qwen2.context_length": 32_768,
                "qwen2.embedding_length": 3_584,
                "qwen2.attention.head_count": 28,
                "qwen2.attention.head_count_kv": 4
            }
        })
    }

    #[test]
    fn a_models_memory_footprint_is_read_from_api_show() {
        let footprint = model_footprint(&qwen25_coder_14b_show()).expect("14b footprint");
        // 48 layers x 8 KV heads x (128 + 128) x 2 bytes.
        assert_eq!(footprint.kv_bytes_per_token, 196_608);
        // 14.8B weights at ~4.9 bits: about 9 GB.
        assert!(
            (8_900_000_000..9_200_000_000).contains(&footprint.weight_bytes),
            "{footprint:?}"
        );
        let small = model_footprint(&qwen25_coder_7b_show()).expect("7b footprint");
        assert_eq!(small.kv_bytes_per_token, 57_344);
        // Explicit key/value lengths win over embedding / heads, and a
        // per-layer KV-head list counts its largest entry.
        let explicit = model_footprint(&json!({
            "details": { "quantization_level": "Q8_0" },
            "model_info": {
                "general.architecture": "gemma3",
                "general.parameter_count": 1_000_000_000u64,
                "gemma3.block_count": 10,
                "gemma3.attention.head_count": 8,
                "gemma3.attention.head_count_kv": [2, 4, 4],
                "gemma3.attention.key_length": 256,
                "gemma3.attention.value_length": 256,
                "gemma3.embedding_length": 1_024,
                "gemma3.vision.block_count": 99
            }
        }))
        .expect("footprint");
        assert_eq!(explicit.kv_bytes_per_token, 10 * 4 * 512 * 2);
        assert_eq!(explicit.weight_bytes, 1_062_500_000);
        // Nothing to size from: no footprint, never a guess.
        assert_eq!(model_footprint(&qwen25_7b_show()), None);
        assert_eq!(model_footprint(&json!({})), None);
        // The parameter size from `details` stands in for a missing count;
        // an unknown quantisation is counted at f16.
        assert_eq!(parse_parameter_size("7.6B"), Some(7_600_000_000));
        assert_eq!(parse_parameter_size("494.03M"), Some(494_030_000));
        assert_eq!(parse_parameter_size("big"), None);
        assert_eq!(quantization_bits_per_weight(Some("Q4_K_M")), 4.9);
        assert_eq!(quantization_bits_per_weight(Some("IQ4_XS")), 4.9);
        assert_eq!(quantization_bits_per_weight(Some("BF16")), 16.0);
        assert_eq!(quantization_bits_per_weight(None), 16.0);
    }

    /// The reported case: an unconfigured 14B on a 16 GB Mac is not loaded
    /// with a 32768 cache (~6 GB on top of ~9 GB of weights), but at what
    /// fits beside its weights; the 7B that motivated sizing at all still
    /// gets its full trained 32768; and a machine too small for the weights
    /// alone gets nothing raised.
    #[test]
    fn the_auto_window_is_capped_by_what_fits_in_memory() {
        let cap = OLLAMA_DEFAULT_AUTO_NUM_CTX_CAP;
        let sixteen_gb = 16u64 << 30;
        let ShowWindow::Local(coder_14b) =
            served_window_from_show(&qwen25_coder_14b_show(), "qwen2.5-coder:14b", None)
        else {
            panic!("a local model");
        };
        assert_eq!(coder_14b.source, ServedWindowSource::ServerDefault);
        let sized = auto_num_ctx(coder_14b, cap, Some(sixteen_gb)).expect("raised above 4096");
        assert_eq!(sized, 6_144);
        assert_eq!(sized % 1024, 0);
        // More memory, more room: capped by the trained length again.
        assert_eq!(
            auto_num_ctx(coder_14b, cap, Some(64u64 << 30)),
            Some(32_768)
        );
        // Unknown memory (a remote Ollama with nothing configured): the
        // trained length and the cap alone, as before.
        assert_eq!(auto_num_ctx(coder_14b, cap, None), Some(32_768));

        let ShowWindow::Local(coder_7b) =
            served_window_from_show(&qwen25_coder_7b_show(), "qwen2.5-coder:7b", None)
        else {
            panic!("a local model");
        };
        assert_eq!(auto_num_ctx(coder_7b, cap, Some(sixteen_gb)), Some(32_768));
        // 8 GB: the 7B's weights leave no room for more than it is served.
        assert_eq!(auto_num_ctx(coder_7b, cap, Some(8u64 << 30)), None);
        assert_eq!(
            memory_num_ctx_ceiling(coder_7b.footprint.expect("footprint"), 8u64 << 30),
            0
        );
    }

    /// `OLLAMA_CONTEXT_LENGTH` configured here is the server's own window,
    /// not Ollama's default: a Modelfile still wins over it, and it is
    /// recorded as configured so nothing sizes past it.
    #[test]
    fn a_configured_server_window_is_not_the_default() {
        let show = json!({ "model_info": { "llama.context_length": 131_072 } });
        assert_eq!(
            served_window_from_show(&show, "llama3.1:8b", Some(8_192)),
            ShowWindow::Local(served(
                8_192,
                ServedWindowSource::ServerConfigured,
                Some(131_072)
            ))
        );
        let with_modelfile = json!({
            "parameters": QWEN25_16K_PARAMETERS,
            "model_info": { "qwen2.context_length": 32_768 }
        });
        assert_eq!(
            served_window_from_show(&with_modelfile, "qwen25-16k:latest", Some(8_192)),
            ShowWindow::Local(served(16_384, ServedWindowSource::Modelfile, Some(32_768)))
        );
    }

    /// No slack above the window: a prose prompt of 4,097 real tokens must
    /// not pass a 4096 window to be cut.
    #[test]
    fn the_window_check_allows_nothing_over_the_window() {
        assert!(exceeds_window(4_097, 4096));
        assert!(!exceeds_window(4_096, 4096));
        // counted_prompt_tokens=Some(62364) from the reported reservation line:
        // not even qwen2.5:7b's trained 32768 holds the untrimmed request.
        assert!(exceeds_window(62_364, 32_768));
    }

    #[test]
    fn an_image_request_is_not_estimated_as_text() {
        let with_image = json!({ "messages": [{ "role": "user", "content": [
            { "type": "text", "text": "what is this" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
        ]}]});
        assert!(payload_carries_images(&with_image));
        let text_only = json!({ "messages": [{ "role": "user", "content": "hi" }] });
        assert!(!payload_carries_images(&text_only));
    }

    #[test]
    fn the_refusal_reads_like_a_llama_server_overflow() {
        let err = served_window_refusal(
            "qwen2.5:7b",
            62_364,
            request_window(
                served(4096, ServedWindowSource::ServerDefault, Some(32_768)),
                Some(32_768),
            ),
        );
        assert!(err.is_permanent());
        let text = err.to_string();
        assert!(
            text.starts_with(crate::providers::errors::CONTEXT_LENGTH_EXCEEDED_MARKER),
            "{text}"
        );
        assert_eq!(
            crate::providers::errors::extract_context_length_token_counts(&text),
            Some((62_364, 32_768)),
            "{text}"
        );
        assert!(text.contains("qwen2.5:7b"), "{text}");
    }

    /// The `/api/chat` body carries the window in `options.num_ctx` (the
    /// field `/v1` drops) and the conversation in the native shape.
    #[test]
    fn a_native_request_carries_num_ctx_and_the_history_in_native_shape() {
        let payload = json!({
            "model": "qwen2.5:7b",
            "messages": [
                { "role": "system", "content": "You are the agent." },
                { "role": "user", "content": [
                    { "type": "text", "text": "what is this" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUJD" } }
                ]},
                { "role": "assistant", "content": null, "reasoning_content": "look first",
                  "tool_calls": [{ "id": "call_1", "type": "function",
                    "function": { "name": "developer__shell", "arguments": "{\"command\":\"ls\"}" } }] },
                { "role": "tool", "content": "Cargo.toml", "tool_call_id": "call_1" }
            ],
            "tools": [{ "type": "function", "function": { "name": "developer__shell",
                "description": "run", "parameters": { "type": "object", "properties": {} } } }],
            "temperature": 0.2,
            "stream": true,
            "stream_options": { "include_usage": true },
            "options": { "num_predict": 4096, "num_ctx": 32768 }
        });
        let body = native_chat_request(&payload, 32_768);
        assert_eq!(body["model"], "qwen2.5:7b");
        assert_eq!(body["stream"], true);
        assert_eq!(body["options"]["num_ctx"], 32_768);
        assert_eq!(body["options"]["num_predict"], 4096);
        assert_eq!(body["options"]["temperature"], 0.2);
        assert!(body.get("stream_options").is_none());
        assert_eq!(body["tools"], payload["tools"]);

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages[0],
            json!({ "role": "system", "content": "You are the agent." })
        );
        assert_eq!(messages[1]["content"], "what is this");
        assert_eq!(messages[1]["images"], json!(["QUJD"]));
        assert_eq!(messages[2]["content"], "");
        assert_eq!(messages[2]["thinking"], "look first");
        assert_eq!(
            messages[2]["tool_calls"],
            json!([{ "id": "call_1", "function": {
                "name": "developer__shell", "arguments": { "command": "ls" } } }])
        );
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["content"], "Cargo.toml");
        assert_eq!(messages[3]["tool_name"], "developer__shell");
        assert_eq!(messages[3]["tool_call_id"], "call_1");

        // `developer` is a system message to /api/chat.
        let reasoning = json!({ "model": "m", "reasoning_effort": "none",
            "messages": [{ "role": "developer", "content": "rules" }] });
        let body = native_chat_request(&reasoning, 8192);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["think"], false);
    }

    async fn collect_native(lines: &[&str]) -> Vec<(Option<Message>, Option<ProviderUsage>)> {
        let owned: Vec<anyhow::Result<String>> =
            lines.iter().map(|line| Ok(line.to_string())).collect();
        let native: LineStream = Box::pin(tokio_stream::iter(owned));
        let parsed = response_to_streaming_message_ollama(native_lines_to_openai_sse(native));
        pin!(parsed);
        let mut out = Vec::new();
        while let Some(item) = parsed.next().await {
            out.push(item.expect("stream item"));
        }
        out
    }

    #[tokio::test]
    async fn a_native_text_stream_reads_as_text_and_usage() {
        let items = collect_native(&[
            r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":"o"},"done":false}"#,
            r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":"k"},"done":false}"#,
            r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":15000,"eval_count":2}"#,
        ])
        .await;
        let text: String = items
            .iter()
            .filter_map(|(message, _)| message.as_ref().map(|m| m.as_concat_text()))
            .collect();
        assert_eq!(text, "ok");
        let usage = items
            .iter()
            .find_map(|(_, usage)| usage.clone())
            .expect("usage reported");
        assert_eq!(usage.usage.input_tokens, Some(15_000));
        assert_eq!(usage.usage.output_tokens, Some(2));
        assert_eq!(usage.model, "qwen2.5:7b");
    }

    #[tokio::test]
    async fn a_native_tool_call_reads_as_a_tool_request() {
        let items = collect_native(&[
            r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"developer__shell","arguments":{"command":"ls"}}}]},"done":false}"#,
            r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":900,"eval_count":20}"#,
        ])
        .await;
        let requests: Vec<_> = items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::ToolRequest(request) => request.tool_call.as_ref().ok(),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 1, "{} stream items", items.len());
        assert_eq!(requests[0].name, "developer__shell");
        assert_eq!(
            requests[0]
                .arguments
                .as_ref()
                .and_then(|a| a.get("command")),
            Some(&json!("ls"))
        );
    }

    #[test]
    fn a_native_error_line_is_an_error() {
        let mut translator = NativeStreamTranslator::default();
        let err = translator
            .translate(r#"{"error":"model requires more system memory"}"#)
            .unwrap_err();
        assert!(err.to_string().contains("more system memory"), "{err}");
    }

    /// The reported case end to end through `stream`: `qwen2.5:7b` (no
    /// Modelfile num_ctx) with the CLI fallback's 32768 window and a prompt far
    /// over Ollama's 4096 default. Before this fix it went to `/v1`, where the
    /// 32768 was dropped and the prompt cut. Now it goes to `/api/chat` with
    /// `num_ctx: 32768`, and `/v1` is never called.
    #[tokio::test]
    async fn stream_sends_a_bigger_window_to_api_chat_where_num_ctx_is_honoured() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_string_contains("qwen2.5:7b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(qwen25_7b_show()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_string_contains("\"num_ctx\":32768"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-ndjson")
                    .set_body_string(concat!(
                        r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":"ok"},"done":false}"#,
                        "\n",
                        r#"{"model":"qwen2.5:7b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":15000,"eval_count":1}"#,
                        "\n"
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let provider = provider_at(server.uri());
        let model_config = ModelConfig::new("qwen2.5:7b")
            .unwrap()
            .with_context_limit(Some(32_768));
        // ~15k tokens: over 4096, inside 32768.
        let system = "Agent tool inventory line. ".repeat(3_000);
        let messages = vec![Message::user().with_text("Reply with the single word: ok")];
        let mut stream = provider
            .stream(&model_config, "", &system, &messages, &[])
            .await
            .expect("a request inside the native window is sent");
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            let (message, _usage) = item.expect("stream item");
            if let Some(message) = message {
                text.push_str(&message.as_concat_text());
            }
        }
        assert_eq!(text, "ok");
        // `expect(1)` / `expect(0)` are verified when `server` drops.
    }

    /// The 2026-09-25 case end to end: a model with no configured context
    /// limit (`context_limit: null`, as the goal worker's
    /// `qwen2.5-coder:7b` had) and a first request over Ollama's 4096
    /// default. Before this fix it was refused; now it goes to `/api/chat`
    /// with `num_ctx` sized from the trained length, and `/v1` is never
    /// called. The pre-send check agrees and does not refuse it either.
    #[tokio::test]
    async fn stream_sizes_an_unconfigured_model_from_its_trained_length() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_string_contains("qwen2.5-coder:7b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(qwen25_7b_show()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_string_contains("\"num_ctx\":32768"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-ndjson")
                    .set_body_string(concat!(
                        r#"{"model":"qwen2.5-coder:7b","message":{"role":"assistant","content":"ok"},"done":false}"#,
                        "\n",
                        r#"{"model":"qwen2.5-coder:7b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":6000,"eval_count":1}"#,
                        "\n"
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let provider = provider_at(server.uri());
        let mut model_config = ModelConfig::new("qwen2.5-coder:7b").unwrap();
        model_config.context_limit = None;
        assert_eq!(
            provider.request_context_window(&model_config).await,
            Some(32_768)
        );
        // ~6k tokens: over 4096, inside 32768.
        let system = "Worker brief and tool schema line. ".repeat(1_000);
        let messages = vec![Message::user().with_text("Begin.")];
        provider
            .preflight_request(&model_config, &system, &messages, &[])
            .await
            .expect("the pre-send check judges the same window the send uses");
        let mut stream = provider
            .stream(&model_config, "", &system, &messages, &[])
            .await
            .expect("a request inside the sized window is sent");
        while let Some(item) = stream.next().await {
            item.expect("stream item");
        }
    }

    /// A server-wide `OLLAMA_CONTEXT_LENGTH` configured here is left alone
    /// end to end: the model is trained for 131072 and nothing else sizes
    /// it, but the request stays on `/v1` at the configured 8192 instead of
    /// loading the model at 32768 on the native endpoint.
    #[tokio::test]
    async fn a_configured_server_window_is_never_overridden() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", Some("8192")),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
            ("OLLAMA_HOST_MEMORY_GB", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "model_info": { "llama.context_length": 131_072 } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: [DONE]\n\n"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let provider = provider_at(server.uri());
        let mut model_config = ModelConfig::new("llama3.1:8b").unwrap();
        model_config.context_limit = None;
        assert_eq!(
            provider.request_context_window(&model_config).await,
            Some(8_192)
        );
        let messages = vec![Message::user().with_text("hi")];
        let mut stream = provider
            .stream(&model_config, "", "A short system prompt.", &messages, &[])
            .await
            .expect("a request inside the configured window is sent");
        while let Some(item) = stream.next().await {
            item.expect("stream item");
        }
    }

    /// The reported case end to end: an unconfigured `qwen2.5-coder:14b` on a
    /// 16 GB host goes native with the num_ctx that fits beside its weights,
    /// not the 32768 cap.
    #[tokio::test]
    async fn stream_sizes_a_large_model_to_the_hosts_memory() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
            ("OLLAMA_HOST_MEMORY_GB", Some("16")),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_string_contains("qwen2.5-coder:14b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(qwen25_coder_14b_show()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_string_contains("\"num_ctx\":6144"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-ndjson")
                    .set_body_string(concat!(
                        r#"{"model":"qwen2.5-coder:14b","message":{"role":"assistant","content":"ok"},"done":false}"#,
                        "\n",
                        r#"{"model":"qwen2.5-coder:14b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":5000,"eval_count":1}"#,
                        "\n"
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let provider = provider_at(server.uri());
        let mut model_config = ModelConfig::new("qwen2.5-coder:14b").unwrap();
        model_config.context_limit = None;
        assert_eq!(
            provider.request_context_window(&model_config).await,
            Some(6_144)
        );
        // ~5k tokens: over 4096, inside 6144.
        let system = "Worker brief and tool schema line. ".repeat(700);
        let messages = vec![Message::user().with_text("Begin.")];
        let mut stream = provider
            .stream(&model_config, "", &system, &messages, &[])
            .await
            .expect("a request inside the memory-sized window is sent");
        while let Some(item) = stream.next().await {
            item.expect("stream item");
        }
    }

    /// Nothing can make the window larger — the model was only trained for
    /// the 4096 it is served with — so `/v1` at 4096, and a request over it
    /// is refused BEFORE it is posted, so Ollama never cuts it.
    #[tokio::test]
    async fn stream_refuses_a_request_ollama_would_truncate_without_sending_it() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "model_info": { "llama.context_length": 4096 } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        for endpoint in ["/v1/chat/completions", "/api/chat"] {
            Mock::given(method("POST"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&server)
                .await;
        }

        let provider = provider_at(server.uri());
        let model_config = ModelConfig::new("qwen2.5:7b").unwrap();
        let system = "Agent tool inventory line. ".repeat(3_000);
        let messages = vec![Message::user().with_text("Reply with the single word: ok")];
        match provider
            .stream(&model_config, "", &system, &messages, &[])
            .await
        {
            Err(ProviderError::ContextLengthExceeded(detail)) => {
                assert!(detail.contains("\"n_ctx\": 4096"), "{detail}");
            }
            Err(other) => panic!("expected ContextLengthExceeded, got {other:?}"),
            Ok(_) => panic!("a request over the served window must not be sent"),
        }
    }

    /// A cloud model is neither sized at 4096 nor refused: it goes to `/v1`
    /// as it always did, without even asking `/api/show`.
    #[tokio::test]
    async fn stream_leaves_an_ollama_cloud_model_alone() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: [DONE]\n\n"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = provider_at(server.uri());
        let model_config = ModelConfig::new("qwen3-coder:480b-cloud").unwrap();
        let system = "Agent tool inventory line. ".repeat(3_000);
        let messages = vec![Message::user().with_text("hi")];
        let mut stream = provider
            .stream(&model_config, "", &system, &messages, &[])
            .await
            .expect("a cloud request is sent, not refused");
        while let Some(item) = stream.next().await {
            item.expect("stream item");
        }
    }

    /// The window is looked up once per model, not once per call.
    #[tokio::test]
    async fn the_window_is_looked_up_once_per_model() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "parameters": QWEN25_16K_PARAMETERS })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider = provider_at(server.uri());
        let model_config = ModelConfig::new("qwen25-16k:latest").unwrap();
        for _ in 0..2 {
            let window = provider
                .request_window(&model_config)
                .await
                .expect("window known");
            assert_eq!(window.tokens, 16_384);
            assert_eq!(window.transport, OllamaTransport::OpenAiCompat);
        }
    }

    /// When Ollama cannot be asked, nothing is known: stand aside rather than
    /// refuse on a guess.
    #[tokio::test]
    async fn an_unreadable_show_does_not_block_the_request() {
        let _guard = env_lock::lock_env([
            ("GOOSE_INPUT_LIMIT", None::<&str>),
            ("GOOSE_CONTEXT_LIMIT", None::<&str>),
            ("OLLAMA_CONTEXT_LENGTH", None::<&str>),
            ("OLLAMA_MAX_NUM_CTX", None::<&str>),
        ]);
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let provider = provider_at(server.uri());
        assert!(provider.served_context_window("qwen2.5:7b").await.is_none());
        assert!(provider
            .request_window(&ModelConfig::new("qwen2.5:7b").unwrap())
            .await
            .is_none());
    }
}
