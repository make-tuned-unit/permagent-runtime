//! Backend surface for Settings → Agents.
//!
//! The roster keeps background workers, dispatch personas, and capabilities in
//! separate populations because combining them would imply dispatchability and
//! liveness that those populations do not share.

use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use permagent::agent_runs::{self, AgentRun};
use permagent::agents::extension::ExtensionConfig;
use permagent::agents::platform_extensions::{
    RequiredSecretDef, SecretImpact, PLATFORM_EXTENSIONS,
};
use permagent::agents::self_knowledge::{
    self, worker_live_state_for, FeatureDescriptor, FeatureFlags, StateSource, WORKER_DESCRIPTORS,
};
use permagent::agents::{
    run_subagent_task, AgentRunnerConfig, GoosePlatform, SubagentRunParams, TaskConfig,
};
use permagent::briefings::{self, Briefing};
use permagent::config::agent_identity::{self, WorkerEngineKind, WorkerPersona};
use permagent::config::extensions::name_to_key;
use permagent::config::paths::Paths;
use permagent::config::worker_probe;
use permagent::config::{
    extension_is_grantable, get_all_extensions, get_enabled_extensions, is_extension_enabled,
    narrow_extensions_for_agent, Config, GooseMode, PermissionManager,
};
use permagent::providers::base::Provider;
use permagent::recipe::Recipe;
use permagent::session::session_manager::{SessionManager, SessionType};
use permagent::{activity_journal, cards, decisions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const MAX_GRANTS: usize = 100;
const MAX_SECRETS: usize = 100;
const MAX_REQUIRED_SECRETS: usize = 100;

/// A bounded question is a bounded turn, and both caps are load-bearing: the
/// ask box must not be a way to start an unmetered agent run from a settings
/// page. `ASK_MAX_TURNS` bounds the tool loop and `ASK_TIMEOUT` bounds the wall
/// clock; expiry is reported AS a timeout rather than hanging the request or
/// passing a partial answer off as a whole one.
const ASK_MAX_TURNS: usize = 8;
const ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_QUESTION_CHARS: usize = 4000;

/// A sweep walks every project or repository, so it needs far more room than an
/// ask. Finite all the same — an unbounded pass would hold the request open
/// until the client gave up, which is indistinguishable from a hang.
const RUN_PASS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum LiveState {
    Ok { value: String },
    NotQueryable,
    Unavailable { reason: String },
}

/// The one boolean config key that switches this agent on.
///
/// Serialised on BOTH a worker row and a dispatch persona because a gated agent
/// appears as either — the Guard is both — and a page that shows the agent
/// without its switch is what sent a product owner hunting through five panes.
///
/// `None` serialises as `null` and is NEVER omitted: a client must be able to
/// tell "this agent has no switch" from "the switch is off".
///
/// There is deliberately no write route for it here. The switch is written with
/// the existing `POST /config/upsert`, the same call Settings → Features makes,
/// so a second key for the same flag cannot come into existence.
#[derive(Serialize, Clone)]
struct Gate {
    config_key: &'static str,
    enabled: bool,
}

fn gate_for(descriptor_id: &str, flags: FeatureFlags) -> Option<Gate> {
    self_knowledge::worker_gate(descriptor_id).map(|gate| Gate {
        config_key: gate.key,
        enabled: gate.is_on(flags),
    })
}

/// Whether a bounded question can be put to this agent right now, and when it
/// cannot, the TRUE reason.
///
/// Always present on the row, never omitted, and never a bare boolean: a
/// control the UI has to guess about renders as enabled-by-hope, and an agent
/// that answers nothing while its box looks live is the exact failure this
/// whole surface exists to end.
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum AskAvailability {
    Available,
    Unavailable { reason: String },
}

/// Whether this agent has an on-demand pass this process can start, and when it
/// does not, the TRUE reason — which for most agents is not "unsupported" but a
/// concrete statement of where its work actually happens instead.
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RunAvailability {
    Available,
    Unavailable { reason: String },
}

/// The three agents that have an on-demand pass, and the ONE place that knows
/// it. `POST /api/agents/{id}/run` and the `run_now` signal both read this, so
/// the button and the route can never disagree about what is runnable — a
/// disagreement between the two would put an enabled control in front of a
/// route that refuses it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunPass {
    Guard,
    Steward,
    Watcher,
}

/// Keyed on worker-DESCRIPTOR ids (`git_steward`, not the `steward` persona
/// key); callers holding a persona key bridge through
/// `agent_identity::descriptor_id_for_worker_key` first.
fn run_pass_for(descriptor_id: &str) -> Option<RunPass> {
    match descriptor_id {
        "strix" => Some(RunPass::Guard),
        "git_steward" => Some(RunPass::Steward),
        "watcher" => Some(RunPass::Watcher),
        _ => None,
    }
}

/// Why this agent has no on-demand pass. Every sentence here is a claim about
/// where the agent's work really happens, taken from its own module, because
/// "not supported" tells the reader nothing they can act on and invites the
/// guess that the feature is merely missing.
fn run_unavailable_reason(descriptor_id: &str) -> String {
    match descriptor_id {
        "scheduler" => "the Scheduler has no pass of its own to run — it is the cron service that \
             fires OTHER agents' jobs, and an individual automation is run from Automate"
            .into(),
        "librarian" => "the Librarian's curation pass is not started from here — it owns its own \
             schedule and already has its own on-demand run at POST /api/librarian/run-now"
            .into(),
        "initiative" => "this agent has no on-demand pass — its tick is driven natively by the \
             daemon's own initiative loop over recorded activity"
            .into(),
        "onboarding_coach" => "this agent has no pass at all — what it knows is computed on read \
             from the activity the user already generates, so there is nothing here to trigger"
            .into(),
        "growth_measurement" => "this agent has no on-demand pass — its measurement runs only as \
             part of the daemon's nightly growth sweep"
            .into(),
        "playbook" => "this agent has no on-demand pass — its synthesis runs only as part of the \
             daemon's own playbook loop"
            .into(),
        "concierge" => "this agent has no on-demand pass — its triage runs only on the daemon's \
             own Concierge tick"
            .into(),
        other if worker_descriptor(other).is_none() => format!(
            "'{other}' is a dispatch persona, not a background worker: it has no pass of its own \
             — it runs when a goal is dispatched to it"
        ),
        other => format!(
            "'{other}' has no on-demand pass wired in this process, so nothing here can start it"
        ),
    }
}

fn run_availability(id: &str) -> RunAvailability {
    let descriptor_id = agent_identity::descriptor_id_for_worker_key(id);
    if run_pass_for(descriptor_id).is_some() {
        return RunAvailability::Available;
    }
    RunAvailability::Unavailable {
        reason: run_unavailable_reason(descriptor_id),
    }
}

/// The dispatch persona a page id answers under, across the two namespaces.
///
/// A persona key and a worker-descriptor id are separate keyspaces that diverge
/// exactly once (`steward` the persona, `git_steward` the descriptor). A lookup
/// matching only one of them would report the Steward as having no persona on
/// the page addressed by its descriptor id and as having one on the page
/// addressed by its key — the same agent answering differently depending on
/// which row the user clicked.
fn persona_for_page<'a>(
    id: &str,
    personas: &'a HashMap<String, WorkerPersona>,
) -> Option<(&'a str, &'a WorkerPersona)> {
    if let Some((key, worker)) = personas.get_key_value(id) {
        return Some((key.as_str(), worker));
    }
    agent_identity::worker_keys_for_descriptor_id(id)
        .into_iter()
        .find_map(|key| personas.get_key_value(key).map(|(k, w)| (k.as_str(), w)))
}

/// Said in ONE place because the `ask` signal and the ask route's refusal must
/// be the same sentence — two spellings of the same fact drift, and a control
/// disabled for one reason while the route refuses for another is worse than
/// either alone.
fn no_persona_reason(id: &str) -> String {
    format!(
        "'{id}' is a background worker with no dispatch persona in agent.yaml, so there is no \
         persona block to answer in its voice and no extension grants to bound the answer to — \
         there is nothing to ask under"
    )
}

/// `ask` runs a BOUNDED question-answering turn as an in-process subagent
/// carrying this persona's system-prompt block and its extension grants. It is
/// not dispatch, and it never touches the agent's background loop.
///
/// Available iff this process can run the persona as a chat turn. An
/// `ExternalCli` / `SupervisedCli` persona is a binary this process launches
/// against a goal in an isolated worktree; there is no in-process turn for it to
/// take, so the control is refused with that as its reason rather than quietly
/// answering as somebody else.
///
/// `Pending` IS askable, and that is the delicate distinction on this page.
/// `Pending` means "no runnable engine for a handed-off GOAL in a worktree" —
/// see `WorkerEngineKind::Pending` and the orchestrator's refusal to dispatch
/// one. Answering a bounded question is a different act with a different
/// requirement: the persona block and the extension grants are configuration,
/// and both exist. So the Librarian, the Steward and the Guard can be asked,
/// and the answer comes from a subagent wearing their persona — NOT from their
/// background loop, which this path neither starts nor reads.
fn ask_availability_for_persona(key: &str, worker: &WorkerPersona) -> AskAvailability {
    match &worker.engine {
        WorkerEngineKind::InternalSubagent | WorkerEngineKind::Pending => {
            AskAvailability::Available
        }
        WorkerEngineKind::ExternalCli { bin, .. } => AskAvailability::Unavailable {
            reason: format!(
                "'{key}' runs as the external CLI '{bin}', which this process launches against a \
                 goal in an isolated worktree — it has no in-process turn that could answer a \
                 question here"
            ),
        },
        WorkerEngineKind::SupervisedCli { bin } => AskAvailability::Unavailable {
            reason: format!(
                "'{key}' runs as the supervised external CLI '{bin}' in a visible Build-tab \
                 terminal with permission gates — this process cannot run it as a chat turn"
            ),
        },
    }
}

fn ask_availability_for_descriptor(
    descriptor_id: &str,
    personas: &HashMap<String, WorkerPersona>,
) -> AskAvailability {
    match persona_for_page(descriptor_id, personas) {
        Some((key, worker)) => ask_availability_for_persona(key, worker),
        None => AskAvailability::Unavailable {
            reason: no_persona_reason(descriptor_id),
        },
    }
}

#[derive(Serialize, Clone)]
struct BackgroundWorker {
    id: String,
    display_name: String,
    what_it_does: String,
    why_it_matters: String,
    state_source: &'static str,
    live_state: LiveState,
    dispatchable: bool,
    gate: Option<Gate>,
    /// Capability signals, ALWAYS present. See [`AskAvailability`].
    ask: AskAvailability,
    run_now: RunAvailability,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Availability {
    Available,
    Unavailable { reason: String },
    ProbeFailed { reason: String },
}

#[derive(Serialize, Clone)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Grants {
    InheritGlobal,
    Explicit {
        extensions: Vec<String>,
        truncated: bool,
    },
}

#[derive(Serialize, Clone)]
struct SecretItem {
    name: String,
    presence: agent_identity::SecretPresence,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Secrets {
    Ok {
        items: Vec<SecretItem>,
        truncated: bool,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Serialize, Clone)]
struct DispatchPersona {
    key: String,
    display_name: String,
    role: String,
    engine: String,
    cost_tier: String,
    workflow_role: Option<String>,
    availability: Availability,
    grants: Grants,
    grants_enforced: bool,
    secrets: Secrets,
    gate: Option<Gate>,
    /// Capability signals, ALWAYS present. `availability` above answers "is the
    /// binary/credential this persona needs on this machine"; these two answer
    /// "is there a control here that will do anything", which is a different
    /// question and used to be answered by guesswork in the client.
    ask: AskAvailability,
    run_now: RunAvailability,
}

#[derive(Serialize, Clone)]
struct RequiredSecret {
    name: String,
    present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    impact: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unlocks: Option<String>,
}

/// Three states, and the third is the one that is easy to get wrong.
///
/// * `declared` + `present: true` — the key is set.
/// * `declared` + `present: false` — the source names a key and it is NOT set.
///   This is the actionable state: it is what tells the user which key to fill
///   in. It must never be reported as `not_declared`.
/// * `not_declared` — the source made no statement about secrets at all. Since
///   every platform-registry entry now carries a `required_secrets` field (an
///   empty one being the positive claim "needs none"), this now arises only for
///   a configured extension whose transport enumerates no env keys. It must
///   never be read as "this extension needs no secrets".
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RequiredSecrets {
    Declared {
        items: Vec<RequiredSecret>,
        truncated: bool,
    },
    NotDeclared,
}

#[derive(Serialize, Clone)]
struct Capability {
    key: String,
    display_name: String,
    description: String,
    enabled: bool,
    /// `None` means the source declares no default, not that the default is off.
    default_enabled: Option<bool>,
    source: &'static str,
    required_secrets: RequiredSecrets,
}

#[derive(Serialize)]
struct RosterResponse {
    workers: Vec<BackgroundWorker>,
    dispatch_roster: Vec<DispatchPersona>,
    capabilities: Vec<Capability>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AgentDetail {
    Worker(BackgroundWorker),
    DispatchPersona(DispatchPersona),
}

fn bounded<T>(mut items: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    let truncated = items.len() > limit;
    items.truncate(limit);
    (items, truncated)
}

/// EVERY worker descriptor, gated or not. The `permagent_self` brief still hides
/// a gated-off worker (`worker_descriptor_visible`, unchanged, at its own call
/// sites) because the brief describes what the agent can DO. This surface must
/// list it, because the switch is HERE: a worker that vanished while its flag
/// was off left the user looking for a control on a page that had removed the
/// row it belongs to.
fn background_workers(
    scheduled_job_count: Option<usize>,
    flags: FeatureFlags,
    personas: &HashMap<String, WorkerPersona>,
) -> Vec<BackgroundWorker> {
    WORKER_DESCRIPTORS
        .iter()
        .map(|d| background_worker(d, scheduled_job_count, flags, personas))
        .collect()
}

/// `personas` is the live dispatch roster, needed for one honest sentence: a
/// worker's ask box is only real if some persona answers for it, and most
/// workers have none. Passing the roster in rather than reading it here keeps
/// this a pure function, which is what lets the roster's shape be tested.
fn background_worker(
    d: &FeatureDescriptor,
    scheduled_job_count: Option<usize>,
    flags: FeatureFlags,
    personas: &HashMap<String, WorkerPersona>,
) -> BackgroundWorker {
    let state_source = match d.state_source {
        StateSource::Queryable => "queryable",
        StateSource::Static => "static",
    };
    let live_state = shape_live_state(Ok(worker_live_state_for(d, scheduled_job_count, flags)));
    BackgroundWorker {
        id: d.id.to_string(),
        display_name: d.display_name.to_string(),
        what_it_does: d.what_it_does.to_string(),
        why_it_matters: d.why_it_matters.to_string(),
        state_source,
        live_state,
        dispatchable: false,
        gate: gate_for(d.id, flags),
        ask: ask_availability_for_descriptor(d.id, personas),
        run_now: run_availability(d.id),
    }
}

fn shape_live_state(read: Result<Option<String>, String>) -> LiveState {
    match read {
        Ok(Some(value)) => LiveState::Ok { value },
        Ok(None) => LiveState::NotQueryable,
        Err(reason) => LiveState::Unavailable { reason },
    }
}

/// Enumeration and presence are separate reads: the flat keyspace is the only
/// way to discover names, while only the presence read can distinguish a
/// resolvable value from a removed or unreadable one. Values are discarded at
/// the keyspace boundary and cannot enter any response-shaping path.
fn secret_names_from_map(id: &str, secrets: HashMap<String, Value>) -> Secrets {
    let prefix = format!("agent_secret.{}.", name_to_key(id));
    let mut names: Vec<String> = secrets
        .into_keys()
        .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
        .collect();
    names.sort();
    let (names, truncated) = bounded(names, MAX_SECRETS);
    Secrets::Ok {
        items: names
            .into_iter()
            .map(|name| SecretItem {
                presence: agent_identity::agent_secret_presence(id, &name),
                name,
            })
            .collect(),
        truncated,
    }
}

fn secrets_for_agent(id: &str) -> Secrets {
    match Config::global().all_secrets() {
        Ok(secrets) => secret_names_from_map(id, secrets),
        Err(error) => Secrets::Unavailable {
            reason: error.to_string(),
        },
    }
}

fn dispatch_persona(
    key: String,
    worker: WorkerPersona,
    availability: Availability,
    flags: FeatureFlags,
) -> DispatchPersona {
    let workflow_role = worker.routing_role().map(|role| role.as_str().to_string());
    let grants = match worker.extension_grants.clone() {
        None => Grants::InheritGlobal,
        Some(items) => {
            let (extensions, truncated) = bounded(items, MAX_GRANTS);
            Grants::Explicit {
                extensions,
                truncated,
            }
        }
    };
    DispatchPersona {
        secrets: secrets_for_agent(&key),
        ask: ask_availability_for_persona(&key, &worker),
        run_now: run_availability(&key),
        // A persona key and a worker-descriptor id are separate namespaces
        // (`steward` vs `git_steward`), so the gate is looked up through the
        // bridge — without it the Steward's persona page would carry no switch
        // while its worker row did.
        gate: gate_for(agent_identity::descriptor_id_for_worker_key(&key), flags),
        key,
        display_name: worker.display_name(),
        role: worker.role,
        engine: worker.engine.label().to_string(),
        cost_tier: worker.cost_tier,
        workflow_role,
        availability,
        grants,
        grants_enforced: worker.engine.grants_enforced(),
    }
}

async fn dispatch_roster(state: &AppState, flags: FeatureFlags) -> Vec<DispatchPersona> {
    let workers: Vec<(String, WorkerPersona)> = state
        .agent_config
        .read()
        .await
        .workers
        .iter()
        .map(|(key, worker)| (key.clone(), worker.clone()))
        .collect();
    let probed = tokio::task::spawn_blocking(move || {
        workers
            .into_iter()
            .map(|(key, worker)| {
                let (available, reason) = worker_probe::probe_worker(&worker.availability_check);
                let availability = if available {
                    Availability::Available
                } else {
                    Availability::Unavailable {
                        reason: reason
                            .unwrap_or_else(|| "availability probe returned no reason".into()),
                    }
                };
                (key, worker, availability)
            })
            .collect::<Vec<_>>()
    })
    .await;
    match probed {
        Ok(rows) => rows
            .into_iter()
            .map(|(key, worker, availability)| dispatch_persona(key, worker, availability, flags))
            .collect(),
        Err(error) => {
            let reason = error.to_string();
            state
                .agent_config
                .read()
                .await
                .workers
                .iter()
                .map(|(key, worker)| {
                    dispatch_persona(
                        key.clone(),
                        worker.clone(),
                        Availability::ProbeFailed {
                            reason: reason.clone(),
                        },
                        flags,
                    )
                })
                .collect()
        }
    }
}

fn extension_fields(config: &ExtensionConfig) -> (String, String, Vec<String>) {
    match config {
        ExtensionConfig::Stdio {
            description,
            env_keys,
            ..
        }
        | ExtensionConfig::StreamableHttp {
            description,
            env_keys,
            ..
        } => (config.name(), description.clone(), env_keys.clone()),
        ExtensionConfig::Sse { description, .. }
        | ExtensionConfig::Builtin { description, .. }
        | ExtensionConfig::Platform { description, .. }
        | ExtensionConfig::Frontend { description, .. }
        | ExtensionConfig::InlinePython { description, .. } => {
            (config.name(), description.clone(), Vec::new())
        }
    }
}

/// Presence, never the value. A blank value is NOT presence: the capabilities
/// that read these keys (see `market_data::FUNDAMENTALS_KEY`) treat an empty
/// string as unset, so reporting it present here would tell the user they are
/// done while the capability still refuses to run.
fn key_is_present(env_value: Option<String>, stored: Option<Value>) -> bool {
    fn meaningful(value: &str) -> bool {
        !value.trim().is_empty()
    }
    if env_value.is_some_and(|value| meaningful(&value)) {
        return true;
    }
    match stored {
        Some(Value::String(value)) => meaningful(&value),
        Some(Value::Null) | None => false,
        // A non-string secret (a structured credential) is present as stored.
        Some(_) => true,
    }
}

fn config_key_present(name: &str) -> bool {
    key_is_present(
        std::env::var(name).ok(),
        Config::global().get_secret::<Value>(name).ok(),
    )
}

/// Secrets a PLATFORM extension declares in the registry. Every declaration
/// carries its impact and its human sentence, which is what separates this
/// from the configured-transport path below.
fn platform_required_secrets(declared: &'static [RequiredSecretDef]) -> RequiredSecrets {
    let items = declared
        .iter()
        .map(|secret| RequiredSecret {
            name: secret.key.to_string(),
            present: config_key_present(secret.key),
            impact: Some(match secret.impact {
                SecretImpact::Degraded => "degraded",
                SecretImpact::Unavailable => "unavailable",
            }),
            unlocks: Some(secret.unlocks.to_string()),
        })
        .collect();
    let (items, truncated) = bounded(items, MAX_REQUIRED_SECRETS);
    RequiredSecrets::Declared { items, truncated }
}

/// Secrets a CONFIGURED extension's transport enumerates. Only Stdio and
/// StreamableHttp carry `env_keys`; for every other transport the config makes
/// no statement at all, which is `not_declared` — reporting an empty
/// declaration would be this surface inventing a "needs nothing" claim the
/// configuration never made. These entries carry no impact or unlocks sentence
/// because the transport does not supply one; absent is honest, "degraded" by
/// default would not be.
fn configured_required_secrets(
    declares_secret_metadata: bool,
    env_keys: Vec<String>,
) -> RequiredSecrets {
    if !declares_secret_metadata {
        return RequiredSecrets::NotDeclared;
    }
    let items = env_keys
        .into_iter()
        .map(|name| RequiredSecret {
            present: config_key_present(&name),
            name,
            impact: None,
            unlocks: None,
        })
        .collect();
    let (items, truncated) = bounded(items, MAX_REQUIRED_SECRETS);
    RequiredSecrets::Declared { items, truncated }
}

fn capabilities() -> Vec<Capability> {
    let mut by_key: HashMap<String, Capability> = PLATFORM_EXTENSIONS
        .values()
        .filter(|def| !def.hidden)
        .map(|def| {
            let key = name_to_key(def.name);
            Capability {
                enabled: is_extension_enabled(&key),
                key,
                display_name: def.display_name.to_string(),
                description: def.description.to_string(),
                default_enabled: Some(def.default_enabled),
                source: "platform",
                required_secrets: platform_required_secrets(def.required_secrets),
            }
        })
        .map(|capability| (capability.key.clone(), capability))
        .collect();
    for entry in get_all_extensions() {
        let configured_key = entry.config.key();
        let declares_secret_metadata = matches!(
            entry.config,
            ExtensionConfig::Stdio { .. } | ExtensionConfig::StreamableHttp { .. }
        );
        let (display_name, description, env_keys) = extension_fields(&entry.config);
        let key = name_to_key(&configured_key);
        let capability = Capability {
            key: key.clone(),
            display_name,
            description,
            enabled: entry.enabled,
            default_enabled: None,
            source: "configured",
            required_secrets: configured_required_secrets(declares_secret_metadata, env_keys),
        };
        // A configured transport replaces a bare registry entry because it
        // carries actual secret metadata; otherwise retain the registry's real
        // default rather than manufacturing one from configuration.
        if declares_secret_metadata || !by_key.contains_key(&key) {
            by_key.insert(key, capability);
        }
    }
    let mut result: Vec<_> = by_key.into_values().collect();
    result.sort_by(|a, b| a.key.cmp(&b.key));
    result
}

async fn roster(State(state): State<Arc<AppState>>) -> Json<RosterResponse> {
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let flags = FeatureFlags::from_live_config();
    let personas = state.agent_config.read().await.workers.clone();
    Json(RosterResponse {
        workers: background_workers(Some(jobs.len()), flags, &personas),
        dispatch_roster: dispatch_roster(&state, flags).await,
        capabilities: capabilities(),
    })
}

async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<AgentDetail> {
    if let Some(worker) = state.agent_config.read().await.workers.get(&id).cloned() {
        let key = id.clone();
        let check = worker.availability_check.clone();
        let availability =
            match tokio::task::spawn_blocking(move || worker_probe::probe_worker(&check)).await {
                Ok((true, _)) => Availability::Available,
                Ok((false, reason)) => Availability::Unavailable {
                    reason: reason
                        .unwrap_or_else(|| "availability probe returned no reason".into()),
                },
                Err(error) => Availability::ProbeFailed {
                    reason: error.to_string(),
                },
            };
        return Ok(Json(AgentDetail::DispatchPersona(dispatch_persona(
            key,
            worker,
            availability,
            FeatureFlags::from_live_config(),
        ))));
    }
    let jobs = state.scheduler().list_scheduled_jobs().await;
    let flags = FeatureFlags::from_live_config();
    if let Some(d) = worker_descriptor(&id) {
        let personas = state.agent_config.read().await.workers.clone();
        return Ok(Json(AgentDetail::Worker(background_worker(
            d,
            Some(jobs.len()),
            flags,
            &personas,
        ))));
    }
    if capabilities()
        .iter()
        .any(|capability| capability.key == name_to_key(&id))
    {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("'{id}' is a capability, not an agent"),
        ));
    }
    Err(ApiError(
        StatusCode::NOT_FOUND,
        format!("agent '{id}' was not found"),
    ))
}

/// A worker descriptor by id, with no flag argument at all. A gated-off worker
/// EXISTS — its page is where its switch lives — so resolving it must not depend
/// on the flag. Reporting an absence as a 404 is what made
/// `GET /api/agents/strix/work` answer "not found" for an agent the daemon ships.
fn worker_descriptor(id: &str) -> Option<&'static FeatureDescriptor> {
    WORKER_DESCRIPTORS.iter().find(|d| d.id == id)
}

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListSection<T> {
    Ok { items: Vec<T>, truncated: bool },
    Unavailable { reason: String },
}

#[derive(Serialize)]
struct ActivitySection {
    attribution: &'static str,
    #[serde(flatten)]
    result: ListSection<activity_journal::JournalItem>,
}

#[derive(Serialize)]
struct GoalItem {
    id: String,
    title: String,
    project_id: String,
    state: String,
    updated_at: String,
    review_decisions: ListSection<ReviewDecision>,
}

#[derive(Serialize, Clone)]
struct ReviewDecision {
    answer: Option<String>,
    acted_by: Option<String>,
}

#[derive(Serialize)]
struct SpendItem {
    cost_usd: f64,
    call_count: i64,
    estimated_call_count: i64,
    attribution: &'static str,
    note: Option<&'static str>,
}

#[derive(Serialize)]
struct ScheduledJobItem {
    id: String,
    cron: String,
    at: Option<String>,
    every: Option<u64>,
    paused: bool,
    run_count: u64,
    last_run: Option<String>,
    last_status: Option<permagent::scheduler::ScheduleRunStatus>,
    last_error: Option<String>,
    consecutive_failures: u32,
}

/// Durable evidence that a pass happened at all — the section that answers "did
/// this agent actually run?" rather than "did it produce something".
///
/// Three states, and the middle one is the point of the whole type. An agent
/// whose code never records a run can only ever produce an empty list, and an
/// empty list renders as an idle agent that is presumably about to do
/// something. `NotRecorded` says the true thing instead: nothing will EVER
/// appear here, so stop reading this panel as a liveness light.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RunsSection {
    Ok {
        items: Vec<AgentRun>,
        truncated: bool,
    },
    /// This agent's code calls nothing that records a run. NOT the same fact as
    /// "has not run yet", and the surface must never render it as one.
    NotRecorded {
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

/// Where this agent's work goes INSTEAD of a run row. Every sentence names a
/// real store this agent writes, taken from its own module, so the reader is
/// pointed at evidence rather than left with an absence.
fn runs_not_recorded_reason(descriptor_id: &str) -> String {
    match descriptor_id {
        "scheduler" => "the Scheduler records no run rows: it is the cron service, and what it \
             did is in this page's scheduled_jobs section — per-job fire count, last run, last \
             status, and consecutive failures"
            .into(),
        "librarian" => "the Librarian records no run rows: its curation passes report through \
             its own schedule and status endpoints (GET /api/librarian/run-status), and the \
             journal rows it writes carry its own id, so this page's activity section holds them"
            .into(),
        "concierge" => "the Concierge records no run rows: a tick that finds nothing leaves \
             nothing behind, and a tick that finds something surfaces as an editable \
             Decision-Inbox draft card and a once-a-day digest notification"
            .into(),
        "initiative" => "the Initiative driver records no run rows: a tick stopped by the free \
             Tier-0 gate writes nothing at all, and a tick that proceeds surfaces as a \
             Decision-Inbox proposal"
            .into(),
        "playbook" => "the Playbook synthesis records no run rows: what it distills is stored as \
             its own class of Brain memory, not as a run"
            .into(),
        "growth_measurement" => "the growth measurement pass records no run rows: each closed \
             window is written as a growth_action_outcomes verdict, and a pass with no window to \
             judge deliberately writes nothing"
            .into(),
        "onboarding_coach" => "the onboarding coach has no pass to record: what it knows is \
             computed on read from the activity the user already generated"
            .into(),
        other => format!(
            "'{other}' records no run rows, so nothing will ever appear here — which is not the \
             same fact as 'has not run yet'"
        ),
    }
}

/// Gated on `agent_runs::records_runs` BEFORE the read, so an agent that
/// records nothing can never reach the query and come back with the empty list
/// that would read as idleness. A read error propagates as `Unavailable`:
/// "could not look" and "nothing happened" are the two facts this page exists
/// to keep apart.
async fn runs_section(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    descriptor_id: &str,
    limit: i64,
) -> RunsSection {
    if !agent_runs::records_runs(descriptor_id) {
        return RunsSection::NotRecorded {
            reason: runs_not_recorded_reason(descriptor_id),
        };
    }
    match agent_runs::recent_for_agent(pool, descriptor_id, limit + 1).await {
        Ok(items) => {
            let (items, truncated) = bounded(items, limit as usize);
            RunsSection::Ok { items, truncated }
        }
        Err(error) => RunsSection::Unavailable {
            reason: error.to_string(),
        },
    }
}

/// The agent's own reports, read across EVERY persona key it files under.
///
/// The Steward files briefings as `from_agent = "steward"` while this page is
/// addressed by the descriptor id `git_steward`. Matching the id alone returns
/// nothing, and nothing renders exactly like an agent that has never reported —
/// for the agent that reports most. `worker_keys_for_descriptor_id` is the
/// bridge, and `the_briefings_section_finds_the_stewards_rows_filed_under_its_persona_key`
/// is the regression that keeps it wired.
async fn briefings_section(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    descriptor_id: &str,
    limit: i64,
) -> ListSection<Briefing> {
    let keys = agent_identity::worker_keys_for_descriptor_id(descriptor_id);
    match briefings::for_agent(pool, &keys, limit + 1).await {
        Ok(items) => {
            let (items, truncated) = bounded(items, limit as usize);
            ListSection::Ok { items, truncated }
        }
        Err(error) => ListSection::Unavailable {
            reason: error.to_string(),
        },
    }
}

#[derive(Serialize)]
struct WorkReview {
    activity: ActivitySection,
    /// Did it run? See [`RunsSection`] — the only section here that can say
    /// "this agent leaves no trace of running at all", which for most of the
    /// roster is the honest answer.
    runs: RunsSection,
    /// What it reported. Unlike `activity`, these rows are written by the agent
    /// under its own persona key, so this section carries real rows for the
    /// workers whose journal attribution goes to `henry` instead of themselves.
    briefings: ListSection<Briefing>,
    goals: ListSection<GoalItem>,
    spend: ListSection<SpendItem>,
    scheduled_jobs: ListSection<ScheduledJobItem>,
}

/// Activity is an exact actor match. Goal moves now carry a real actor — the
/// worker that owns the goal, or the person or policy that authorized the move
/// — so a dispatched agent's rows land under its own id instead of `system`.
///
/// `attribution` still ships, because an empty page is still not proof the
/// agent was idle: rows written before that change keep their old `system`
/// actor, task failures carry no worker at all, and a run that produced no
/// journaled event was never counted. Absence means "nothing is attributed to
/// this id", never "this agent did nothing".
async fn work(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LimitQuery>,
) -> ApiResult<WorkReview> {
    let known = state.agent_config.read().await.workers.contains_key(&id)
        || worker_descriptor(&id).is_some();
    if !known {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("agent '{id}' was not found"),
        ));
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    // Runs and briefings are stored under the worker-DESCRIPTOR id and under
    // the agent's persona keys respectively, while this route is addressed by
    // either. Both reads bridge, so the Steward's page carries its rows whether
    // the user arrived at `steward` or at `git_steward`.
    let descriptor_id = agent_identity::descriptor_id_for_worker_key(&id).to_string();
    let pool = state.session_manager().pool_clone().await;
    let (activity, goals, spend, runs, briefings) = match pool {
        Err(error) => {
            let reason = error.to_string();
            (
                ListSection::Unavailable {
                    reason: reason.clone(),
                },
                ListSection::Unavailable {
                    reason: reason.clone(),
                },
                ListSection::Unavailable {
                    reason: reason.clone(),
                },
                RunsSection::Unavailable {
                    reason: reason.clone(),
                },
                ListSection::Unavailable { reason },
            )
        }
        Ok(pool) => {
            let activity =
                match activity_journal::page(&pool, None, limit + 1, None, Some(&id)).await {
                    Ok(items) => {
                        let (items, truncated) = bounded(items, limit as usize);
                        ListSection::Ok { items, truncated }
                    }
                    Err(error) => ListSection::Unavailable {
                        reason: error.to_string(),
                    },
                };
            let goals_read = cards::list_goals_for_worker(&pool, &id, limit + 1).await;
            let goals = match goals_read.as_ref() {
                Err(error) => ListSection::Unavailable {
                    reason: error.clone(),
                },
                Ok(rows) => {
                    let decisions = decisions::decision_history(&pool, MAX_LIMIT, None).await;
                    let history_saturated = decisions
                        .as_ref()
                        .is_ok_and(|history| history.len() >= MAX_LIMIT as usize);
                    let mut by_goal: HashMap<String, Vec<ReviewDecision>> = HashMap::new();
                    if let Ok(history) = &decisions {
                        for item in history {
                            if let Some(goal_id) = &item.decision.goal_id {
                                by_goal
                                    .entry(goal_id.clone())
                                    .or_default()
                                    .push(ReviewDecision {
                                        answer: item.decision.answer.clone(),
                                        acted_by: item.decision.acted_by.clone(),
                                    });
                            }
                        }
                    }
                    let mut items = Vec::new();
                    for goal in rows.iter().take(limit as usize) {
                        let review_decisions = match &decisions {
                            Ok(_) => {
                                let decisions = by_goal.remove(&goal.id).unwrap_or_default();
                                // A clean empty is only knowable when the global
                                // history window was not saturated; otherwise this
                                // goal's decision may simply be older than the read.
                                let truncated = decisions.len() >= MAX_LIMIT as usize
                                    || (decisions.is_empty() && history_saturated);
                                ListSection::Ok {
                                    items: decisions,
                                    truncated,
                                }
                            }
                            Err(error) => ListSection::Unavailable {
                                reason: error.clone(),
                            },
                        };
                        items.push(GoalItem {
                            id: goal.id.clone(),
                            title: goal.title.clone(),
                            project_id: goal.project_id.clone(),
                            state: goal.state.clone(),
                            updated_at: goal.updated_at.clone(),
                            review_decisions,
                        });
                    }
                    ListSection::Ok {
                        items,
                        truncated: rows.len() > limit as usize,
                    }
                }
            };
            let goal_ids: Vec<String> = goals_read
                .as_ref()
                .map(|v| v.iter().map(|g| g.id.clone()).collect())
                .unwrap_or_default();
            let spend = if goals_read.is_err() {
                ListSection::Unavailable {
                    reason: "goals could not be read, so spend attribution could not be resolved"
                        .into(),
                }
            } else if goal_ids.is_empty() {
                ListSection::Ok {
                    items: vec![SpendItem {
                        cost_usd: 0.0,
                        call_count: 0,
                        estimated_call_count: 0,
                        attribution: "via_goal_id",
                        note: Some("this agent has no attributable goals"),
                    }],
                    truncated: false,
                }
            } else {
                let placeholders = vec!["?"; goal_ids.len()].join(",");
                let sql = format!("SELECT COALESCE(SUM(cost_usd), 0.0) AS total, COUNT(*) AS calls, COALESCE(SUM(CASE WHEN is_estimated = 1 THEN 1 ELSE 0 END), 0) AS estimated FROM cost_ledger WHERE goal_id IN ({placeholders})");
                let mut q = sqlx::query(&sql);
                for goal_id in &goal_ids {
                    q = q.bind(goal_id);
                }
                match q.fetch_one(&pool).await {
                    Ok(row) => ListSection::Ok {
                        items: vec![SpendItem {
                            cost_usd: row.get("total"),
                            call_count: row.get("calls"),
                            estimated_call_count: row.get("estimated"),
                            attribution: "via_goal_id",
                            note: None,
                        }],
                        truncated: false,
                    },
                    Err(error) => ListSection::Unavailable {
                        reason: error.to_string(),
                    },
                }
            };
            let runs = runs_section(&pool, &descriptor_id, limit).await;
            let briefings = briefings_section(&pool, &descriptor_id, limit).await;
            (activity, goals, spend, runs, briefings)
        }
    };
    // Scheduled jobs expose last-fire and aggregate counters only. There is no
    // per-fire history table, so no field here claims to be a run history.
    let jobs: Vec<ScheduledJobItem> = state
        .scheduler()
        .list_scheduled_jobs()
        .await
        .into_iter()
        .filter(|job| job.worker_persona.as_deref() == Some(&id))
        .map(|job| ScheduledJobItem {
            id: job.id,
            cron: job.cron,
            at: job.at.map(|v| v.to_rfc3339()),
            every: job.every_seconds,
            paused: job.paused,
            run_count: job.run_count,
            last_run: job.last_run.map(|v| v.to_rfc3339()),
            last_status: job.last_status,
            last_error: job.last_error,
            consecutive_failures: job.consecutive_failures,
        })
        .collect();
    let (jobs, jobs_truncated) = bounded(jobs, limit as usize);
    Ok(Json(WorkReview {
        activity: ActivitySection {
            attribution: "actor_exact_match",
            result: activity,
        },
        runs,
        briefings,
        goals,
        spend,
        scheduled_jobs: ListSection::Ok {
            items: jobs,
            truncated: jobs_truncated,
        },
    }))
}

#[derive(Deserialize)]
struct GrantsRequest {
    extensions: Option<Vec<String>>,
}

fn validate_grants(extensions: Option<Vec<String>>) -> Result<Option<Vec<String>>, ApiError> {
    let normalized = extensions.map(|items| {
        items
            .into_iter()
            .map(|key| name_to_key(&key))
            .collect::<Vec<_>>()
    });
    if normalized
        .as_ref()
        .is_some_and(|items| items.len() > MAX_GRANTS)
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("at most {MAX_GRANTS} extension grants are accepted"),
        ));
    }
    if let Some(items) = &normalized {
        let invalid: Vec<&String> = items
            .iter()
            .filter(|key| !extension_is_grantable(key))
            .collect();
        if !invalid.is_empty() {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!(
                    "extensions are not grantable: {}",
                    invalid
                        .into_iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    Ok(normalized)
}

async fn set_grants(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<GrantsRequest>,
) -> ApiResult<DispatchPersona> {
    if id == "roster" {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "'roster' is reserved and cannot be an agent id".into(),
        ));
    }
    let normalized = validate_grants(body.extensions)?;
    let worker = {
        let mut config = state.agent_config.write().await;
        let Some(worker) = config.workers.get_mut(&id) else {
            let message = if WORKER_DESCRIPTORS.iter().any(|d| d.id == id) {
                format!(
                    "background worker '{id}' has no agent.yaml entry and cannot receive grants"
                )
            } else {
                format!("dispatch persona '{id}' was not found")
            };
            return Err(ApiError(StatusCode::NOT_FOUND, message));
        };
        worker.extension_grants = normalized;
        let worker = worker.clone();
        let disk_config = agent_identity::AgentConfig {
            primary: state.persona.read().await.clone(),
            workers: config.workers.clone(),
        };
        agent_identity::save_agent_config(&disk_config)
            .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        worker
    };
    let check = worker.availability_check.clone();
    let availability =
        match tokio::task::spawn_blocking(move || worker_probe::probe_worker(&check)).await {
            Ok((true, _)) => Availability::Available,
            Ok((false, reason)) => Availability::Unavailable {
                reason: reason.unwrap_or_else(|| "availability probe returned no reason".into()),
            },
            Err(error) => Availability::ProbeFailed {
                reason: error.to_string(),
            },
        };
    // Read fresh rather than threaded in: the grants write is unrelated to the
    // gate, and the response must describe the flag as it stands right now.
    Ok(Json(dispatch_persona(
        id,
        worker,
        availability,
        FeatureFlags::from_live_config(),
    )))
}

#[derive(Deserialize)]
struct SecretRequest {
    name: String,
    // No `Debug` derive: request debugging must never expose this value.
    value: Option<String>,
}

#[derive(Serialize)]
struct SecretResponse {
    name: String,
    presence: &'static str,
}

async fn set_secret(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SecretRequest>,
) -> ApiResult<SecretResponse> {
    if id == "roster" {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "'roster' is reserved and cannot be an agent id".into(),
        ));
    }
    if body.name.trim().is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "secret name must not be empty".into(),
        ));
    }
    if body
        .value
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "secret value must not be empty; use null to delete the secret".into(),
        ));
    }
    if !state.agent_config.read().await.workers.contains_key(&id) {
        let message = if WORKER_DESCRIPTORS.iter().any(|d| d.id == id) {
            format!("background worker '{id}' has no agent.yaml entry and cannot store per-agent secrets")
        } else {
            format!("dispatch persona '{id}' was not found")
        };
        return Err(ApiError(StatusCode::NOT_FOUND, message));
    }
    let key = agent_identity::agent_secret_key(&id, &body.name);
    let presence = match body.value {
        Some(value) => {
            Config::global()
                .set_secret(&key, &value)
                .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            "present"
        }
        None => {
            Config::global()
                .delete_secret(&key)
                .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            "absent"
        }
    };
    Ok(Json(SecretResponse {
        name: body.name,
        presence,
    }))
}

// ── Ask one agent a bounded question ────────────────────────────────────────

#[derive(Deserialize)]
struct AskRequest {
    question: String,
}

/// What the tools of this turn actually were.
///
/// Named for what happened rather than for what was intended, because the two
/// diverge: `narrow_extensions_for_agent` can only ever REMOVE, so a grant
/// naming an extension that is globally disabled silently produces nothing.
/// Reporting only the declared list would tell the user the agent held a tool
/// it never had.
#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum AppliedToolScope {
    /// The persona declares no grants, so the turn carried the globally enabled
    /// set unchanged — the same set any in-process run here would get. Said
    /// plainly: the reader must not be told "its own tools" when the set is
    /// everyone's.
    InheritGlobal { extensions: Vec<String> },
    /// Narrowed to the persona's own grants. `granted` is what agent.yaml
    /// declares; `applied` is what survived the narrowing, and the two differ
    /// exactly when a declared grant was not available to narrow from.
    Explicit {
        granted: Vec<String>,
        applied: Vec<String>,
    },
}

#[derive(Serialize)]
struct AskResponse {
    answer: String,
    display_name: String,
    /// True when the answer came from a subagent carrying THIS persona's
    /// system-prompt block. It is never a claim that the agent's background
    /// loop ran, and nothing on this path can start one.
    persona_applied: bool,
    tool_scope: AppliedToolScope,
}

/// Narrow the turn's extension set to the persona's grants, and describe what
/// actually happened.
///
/// TRAP, deliberately avoided: `summon`'s `handle_delegate` does NOT apply
/// `WorkerPersona::extension_grants`, so copying that path would hand this turn
/// the full tool set while the response claimed a scope. The narrowing here is
/// the same `narrow_extensions_for_agent` the orchestrator's in-process
/// dispatch uses (`platform_extensions/orchestrator.rs`), which is a `retain`
/// over the caller's own set and so can only ever remove — a grant cannot
/// widen, and this route cannot become a privilege-escalation seam.
fn apply_tool_scope(
    worker: &WorkerPersona,
    base: Vec<ExtensionConfig>,
) -> (Vec<ExtensionConfig>, AppliedToolScope) {
    let granted = worker.extension_grants.clone();
    let narrowed = narrow_extensions_for_agent(base, granted.as_deref());
    let applied: Vec<String> = narrowed.iter().map(|config| config.key()).collect();
    let scope = match granted {
        None => AppliedToolScope::InheritGlobal {
            extensions: applied,
        },
        Some(granted) => AppliedToolScope::Explicit { granted, applied },
    };
    (narrowed, scope)
}

/// The route's gate, and it is the SAME fact the `ask` signal serialises — a
/// route more permissive than its own signal would answer for an agent the page
/// showed as unaskable, and the user would have no way to know which was true.
///
/// The two refusals are different facts and carry different codes: an id nobody
/// has heard of is a 404, while an agent that exists and cannot be asked is a
/// 409 with the reason the signal already gave.
fn resolve_ask_target(
    id: &str,
    personas: &HashMap<String, WorkerPersona>,
) -> Result<(String, WorkerPersona), ApiError> {
    let Some((key, worker)) = persona_for_page(id, personas) else {
        if worker_descriptor(id).is_some() {
            return Err(ApiError(StatusCode::CONFLICT, no_persona_reason(id)));
        }
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("agent '{id}' was not found"),
        ));
    };
    match ask_availability_for_persona(key, worker) {
        AskAvailability::Available => Ok((key.to_string(), worker.clone())),
        AskAvailability::Unavailable { reason } => Err(ApiError(StatusCode::CONFLICT, reason)),
    }
}

/// The provider for the ask turn: the persona's configured role→model when it
/// has one (the same mapping a dispatch to it would take), otherwise the
/// configured default.
///
/// A missing or unbuildable provider is an ERROR, never an empty answer. There
/// is no template, no fallback sentence and no "the agent had nothing to say"
/// on this path — an ask box that answers when nothing answered it is the exact
/// class of false surface this work exists to remove.
async fn ask_provider(worker: &WorkerPersona) -> Result<Arc<dyn Provider>, ApiError> {
    let unavailable = |message: String| ApiError(StatusCode::SERVICE_UNAVAILABLE, message);
    let config = Config::global();
    let (provider_name, model_name) = match worker
        .routing_role()
        .and_then(permagent::cost_router::role_model)
    {
        Some(mapped) => (mapped.provider, mapped.model),
        None => {
            let provider_name = config.get_goose_provider().map_err(|error| {
                unavailable(format!(
                    "no provider is configured, so there is nothing to answer the question: {error}"
                ))
            })?;
            let model_name = config.get_goose_model().map_err(|error| {
                unavailable(format!(
                    "no model is configured, so there is nothing to answer the question: {error}"
                ))
            })?;
            (provider_name, model_name)
        }
    };
    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return Err(unavailable(
            "the configured provider or model is blank, so there is nothing to answer the question"
                .into(),
        ));
    }
    permagent::providers::create_with_named_model(&provider_name, &model_name, Vec::new())
        .await
        .map_err(|error| {
            unavailable(format!(
                "the provider could not be created, so no answer was produced: {error}"
            ))
        })
}

/// `POST /api/agents/{id}/ask` — one bounded question, answered by an
/// in-process subagent wearing this agent's persona and holding only this
/// agent's granted tools.
///
/// What this IS: a question-answering turn carrying the persona's
/// system-prompt block and its narrowed extension set.
///
/// What this is NOT, and must never be described as: a message to the agent's
/// background loop. Nothing here starts, reads, or reaches the Steward's sweep,
/// the Guard's scan or the Librarian's curation. `POST /api/agents/{id}/run` is
/// the only route that starts a pass, and it exists for exactly three agents.
async fn ask(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<AskRequest>,
) -> ApiResult<AskResponse> {
    let question = body.question.trim().to_string();
    if question.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "question must not be empty".into(),
        ));
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("question must be at most {MAX_QUESTION_CHARS} characters"),
        ));
    }
    let personas = state.agent_config.read().await.workers.clone();
    let (key, worker) = resolve_ask_target(&id, &personas)?;

    let (extensions, tool_scope) = apply_tool_scope(&worker, get_enabled_extensions());
    let provider = ask_provider(&worker).await?;

    // The daemon's own directory, and no project is implied by it. A question
    // put from a settings page carries no working context, so claiming one by
    // reaching for a project's path would be an invention.
    let working_dir = std::env::current_dir().unwrap_or_else(|_| Paths::data_dir());
    let session_manager = Arc::new(SessionManager::instance());
    let display_name = worker.display_name();
    let session = session_manager
        .create_session(
            working_dir.clone(),
            format!("Question for {display_name}"),
            SessionType::SubAgent,
            GooseMode::Auto,
        )
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the question could not be given a session to run in: {error}"),
            )
        })?;

    let task_config = TaskConfig::new(provider, &session.id, &working_dir, extensions)
        .with_max_turns(Some(ASK_MAX_TURNS));
    let recipe = Recipe::builder()
        .version("1.0.0")
        .title(format!("Question for {display_name}"))
        .description("A bounded question put to one agent from Settings → Agents")
        .prompt(&question)
        .build()
        .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let runner = AgentRunnerConfig::new(
        session_manager,
        PermissionManager::instance(),
        None,
        // Auto, because nobody is here to answer an approval prompt: a mode
        // that parks would hold the request open rather than refuse it, and a
        // hung request is the least honest failure available.
        GooseMode::Auto,
        true,
        GoosePlatform::GooseCli,
    );

    let turn = run_subagent_task(SubagentRunParams {
        config: runner,
        recipe,
        task_config,
        return_last_only: true,
        session_id: session.id,
        cancellation_token: None,
        on_message: None,
        notification_tx: None,
        persona_override: Some((worker.system_prompt_block(), display_name.clone())),
    });
    let answer = match tokio::time::timeout(ASK_TIMEOUT, turn).await {
        Err(_) => {
            return Err(ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "'{key}' did not answer within {}s — the turn is abandoned and no answer is \
                     reported, rather than a partial one passed off as whole",
                    ASK_TIMEOUT.as_secs()
                ),
            ))
        }
        Ok(Err(error)) => {
            return Err(ApiError(
                StatusCode::BAD_GATEWAY,
                format!("the turn for '{key}' failed, so there is no answer: {error}"),
            ))
        }
        Ok(Ok(answer)) => answer,
    };
    if answer.trim().is_empty() {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            format!("'{key}' produced no text — an empty answer is an error here, never an answer"),
        ));
    }
    Ok(Json(AskResponse {
        answer,
        display_name,
        persona_applied: true,
        tool_scope,
    }))
}

// ── Run one agent's pass on demand ──────────────────────────────────────────

#[derive(Serialize)]
struct RunNowResponse {
    run: AgentRun,
}

/// `POST /api/agents/{id}/run` — start this agent's pass, then report the run
/// row the pass itself recorded.
///
/// The response body is the AGENT'S OWN record, not this route's account of
/// what it did. That is deliberate: a handler that returned its own "ok" would
/// be a status light with no evidence behind it, which is the failure the
/// `agent_runs` table was added to end. Three checks keep the claim real:
///
/// * the run store is read BEFORE the pass, so a pass that records nothing
///   cannot be reported using a previous run's row;
/// * a pass that finishes without recording anything is an ERROR here, not a
///   synthesised success; and
/// * expiry is reported as a timeout, with no run claimed, because the pass may
///   still be running and will record its own row when it finishes.
async fn run_now(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<RunNowResponse> {
    let known = state.agent_config.read().await.workers.contains_key(&id)
        || worker_descriptor(&id).is_some();
    if !known {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("agent '{id}' was not found"),
        ));
    }
    let descriptor_id = agent_identity::descriptor_id_for_worker_key(&id).to_string();
    let Some(pass) = run_pass_for(&descriptor_id) else {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            run_unavailable_reason(&descriptor_id),
        ));
    };

    // Read the store first. Without a readable run table the pass could still
    // run, but its result would be unverifiable — and reporting a run this
    // route cannot see would be precisely the unevidenced claim it exists to
    // avoid. Refusing before starting is the honest order.
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "the run store could not be read, so a pass would leave no record this route \
                 could verify — not started: {error}"
                ),
            )
        })?;
    let before = agent_runs::recent_for_agent(&pool, &descriptor_id, 1)
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "the run store could not be read, so a pass would leave no record this route \
                     could verify — not started: {error}"
                ),
            )
        })?
        .into_iter()
        .next()
        .map(|run| run.id);

    let outcome = match pass {
        RunPass::Guard => {
            tokio::time::timeout(RUN_PASS_TIMEOUT, crate::strix::run_pass_now(&state)).await
        }
        RunPass::Steward => {
            tokio::time::timeout(RUN_PASS_TIMEOUT, crate::steward_sweep::run_pass_now(&state)).await
        }
        RunPass::Watcher => {
            tokio::time::timeout(
                RUN_PASS_TIMEOUT,
                crate::watcher_insights::run_pass_now(&state),
            )
            .await
        }
    };
    match outcome {
        Err(_) => {
            return Err(ApiError(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "the pass for '{descriptor_id}' did not finish within {}s; it may still be \
                     running, and no run is reported here until one is recorded",
                    RUN_PASS_TIMEOUT.as_secs()
                ),
            ))
        }
        Ok(Err(reason)) => {
            return Err(ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the pass for '{descriptor_id}' failed: {reason}"),
            ))
        }
        Ok(Ok(())) => {}
    }

    let latest = agent_runs::recent_for_agent(&pool, &descriptor_id, 1)
        .await
        .map_err(|error| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the pass completed but its run could not be read back: {error}"),
            )
        })?
        .into_iter()
        .next();
    match latest {
        Some(run) if Some(&run.id) != before.as_ref() => Ok(Json(RunNowResponse { run })),
        _ => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "the pass for '{descriptor_id}' completed but recorded no run, so there is \
                 nothing to show — reporting success here without the agent's own row would be \
                 exactly the claim this page exists to stop making"
            ),
        )),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // matchit prefers this static segment over `{id}`. The write handlers
        // also reserve `roster`, preventing a persona from shadowing it.
        .route("/api/agents/roster", get(roster))
        .route("/api/agents/{id}", get(detail))
        .route("/api/agents/{id}/work", get(work))
        .route("/api/agents/{id}/grants", post(set_grants))
        .route("/api/agents/{id}/secrets", post(set_secret))
        .route("/api/agents/{id}/ask", post(ask))
        .route("/api/agents/{id}/run", post(run_now))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::briefings::{NewBriefing, Severity};

    /// The seeded dispatch roster, which is what a real install carries: every
    /// default worker is merged into agent.yaml on load, so testing against it
    /// is testing against what the surface actually sees.
    fn personas() -> HashMap<String, WorkerPersona> {
        agent_identity::default_roster()
    }

    async fn runs_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        permagent::session::spectral_schema::apply_agent_runs_schema(&pool)
            .await
            .unwrap();
        pool
    }

    async fn briefings_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        permagent::session::spectral_schema::apply_briefings_schema(&pool)
            .await
            .unwrap();
        pool
    }

    #[test]
    fn workers_are_never_dispatchable_and_have_no_affordance() {
        let value = serde_json::to_value(background_workers(
            Some(3),
            FeatureFlags::default(),
            &personas(),
        ))
        .unwrap();
        let workers = value.as_array().unwrap();
        // The roster no longer filters on host config, so the count IS pinnable
        // now — the caveat this comment used to carry ("6 of 9 here, depends on
        // the machine") no longer applies. `every_worker_descriptor_reaches_the_roster`
        // owns that assertion; this one keeps the floor so "never dispatchable"
        // is not vacuously true over an empty list.
        assert_eq!(workers.len(), WORKER_DESCRIPTORS.len());
        assert!(
            !workers.is_empty(),
            "roster returned no workers, so the assertions below inspected nothing"
        );
        let known: Vec<&str> = permagent::agents::self_knowledge::WORKER_DESCRIPTORS
            .iter()
            .map(|d| d.id)
            .collect();
        for worker in workers {
            let id = worker["id"].as_str().expect("worker must carry an id");
            assert!(
                known.contains(&id),
                "roster returned {id:?}, which is not a WORKER_DESCRIPTORS id"
            );
        }
        for worker in workers {
            assert_eq!(worker["dispatchable"], false);
            assert!(worker.get("dispatch").is_none());
        }
    }

    #[test]
    fn unavailable_is_not_an_empty_success() {
        let value =
            serde_json::to_value(shape_live_state(Err("worker state reader failed".into())))
                .unwrap();
        assert_eq!(value["status"], "unavailable");
        assert!(!value["reason"].as_str().unwrap().is_empty());
        assert!(value.get("value").is_none());

        let value = serde_json::to_value(Secrets::Unavailable {
            reason: "keychain locked".into(),
        })
        .unwrap();
        assert_eq!(value["status"], "unavailable");
        assert!(!value["reason"].as_str().unwrap().is_empty());
        assert!(value.get("items").is_none());
    }

    #[test]
    fn bounds_are_explicit() {
        let (items, truncated) = bounded(vec![1, 2, 3], 2);
        assert_eq!(items, vec![1, 2]);
        assert!(truncated);
    }

    #[test]
    fn secret_response_never_serializes_value() {
        let distinctive = "NEVER_LEAK_8d97d55c";
        let seeded = HashMap::from([(
            agent_identity::agent_secret_key("researcher", "token"),
            Value::String(distinctive.into()),
        )]);
        let shaped_secrets = secret_names_from_map("researcher", seeded);
        let roster = RosterResponse {
            workers: background_workers(Some(0), FeatureFlags::default(), &personas()),
            dispatch_roster: vec![DispatchPersona {
                key: "researcher".into(),
                display_name: "Researcher".into(),
                role: String::new(),
                engine: "internal_subagent".into(),
                cost_tier: "local_free".into(),
                workflow_role: None,
                availability: Availability::Available,
                grants: Grants::InheritGlobal,
                grants_enforced: true,
                secrets: shaped_secrets,
                gate: None,
                ask: AskAvailability::Available,
                run_now: RunAvailability::Unavailable {
                    reason: "no pass".into(),
                },
            }],
            capabilities: capabilities(),
        };
        let detail = AgentDetail::Worker(
            background_workers(Some(0), FeatureFlags::default(), &personas()).remove(0),
        );
        let work = WorkReview {
            activity: ActivitySection {
                attribution: "actor_exact_match",
                result: ListSection::Ok {
                    items: Vec::new(),
                    truncated: false,
                },
            },
            runs: RunsSection::NotRecorded {
                reason: "records nothing".into(),
            },
            briefings: ListSection::Ok {
                items: Vec::new(),
                truncated: false,
            },
            goals: ListSection::Ok {
                items: Vec::new(),
                truncated: false,
            },
            spend: ListSection::Ok {
                items: Vec::new(),
                truncated: false,
            },
            scheduled_jobs: ListSection::Ok {
                items: Vec::new(),
                truncated: false,
            },
        };
        let response = SecretResponse {
            name: "token".into(),
            presence: "present",
        };
        for body in [
            serde_json::to_string(&roster).unwrap(),
            serde_json::to_string(&detail).unwrap(),
            serde_json::to_string(&work).unwrap(),
            serde_json::to_string(&response).unwrap(),
        ] {
            assert!(!body.contains(distinctive));
        }
    }

    #[test]
    fn globally_disabled_grant_is_rejected_by_validation() {
        let key = "definitely_disabled_agents_surface_extension";
        let error = validate_grants(Some(vec![key.into()])).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(error.1.contains(key));
    }

    #[test]
    fn capabilities_have_unique_keys() {
        let capabilities = capabilities();
        let mut keys: Vec<_> = capabilities.iter().map(|item| &item.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), capabilities.len());
    }

    /// A key that is DECLARED but not set must report `declared` with
    /// `present: false`. Collapsing it into `not_declared` would tell the user
    /// the registry knows of no key to fill in — the opposite of the truth, and
    /// the whole reason this surface distinguishes three states. Both branches
    /// are asserted through the real mapping functions, not hand-built enum
    /// values, so a revert in `capabilities()` cannot pass this.
    #[test]
    fn declared_but_unset_is_absent_not_undeclared() {
        // A key no machine has. `present` is therefore knowably false here,
        // which a real registry key could not guarantee on a dev box.
        static NEVER_SET: &[RequiredSecretDef] = &[RequiredSecretDef {
            key: "PERMAGENT_AGENTS_SURFACE_KEY_THAT_IS_NEVER_SET_4f19c0",
            impact: SecretImpact::Degraded,
            unlocks: "Nothing; this exists only to pin the declared-and-absent state.",
        }];
        let declared = serde_json::to_value(platform_required_secrets(NEVER_SET)).unwrap();
        assert_eq!(declared["status"], "declared");
        assert_eq!(declared["items"][0]["present"], false);
        assert_eq!(declared["items"][0]["impact"], "degraded");
        assert!(!declared["items"][0]["unlocks"].as_str().unwrap().is_empty());

        let undeclared =
            serde_json::to_value(configured_required_secrets(false, Vec::new())).unwrap();
        assert_eq!(undeclared["status"], "not_declared");
        assert!(undeclared.get("items").is_none());
        assert_ne!(declared["status"], undeclared["status"]);
    }

    /// A capability whose registry entry declares nothing is a POSITIVE claim
    /// ("needs no secret"), so it is `declared` with zero items — while a
    /// configured transport that enumerates no env keys made no claim at all.
    /// The two look alike and mean opposite things.
    #[test]
    fn empty_declaration_and_no_declaration_are_not_the_same_answer() {
        let declares_nothing = serde_json::to_value(platform_required_secrets(&[])).unwrap();
        assert_eq!(declares_nothing["status"], "declared");
        assert_eq!(declares_nothing["items"].as_array().unwrap().len(), 0);

        // Sse / Builtin / Platform / Frontend / InlinePython carry no env_keys.
        for config in [
            ExtensionConfig::Sse {
                name: "x".into(),
                description: String::new(),
                uri: Some("http://localhost/x".into()),
            },
            ExtensionConfig::Frontend {
                name: "y".into(),
                description: String::new(),
                tools: Vec::new(),
                instructions: None,
                bundled: None,
                available_tools: Vec::new(),
            },
        ] {
            let declares_secret_metadata = matches!(
                config,
                ExtensionConfig::Stdio { .. } | ExtensionConfig::StreamableHttp { .. }
            );
            let (_, _, env_keys) = extension_fields(&config);
            let value = serde_json::to_value(configured_required_secrets(
                declares_secret_metadata,
                env_keys,
            ))
            .unwrap();
            assert_eq!(value["status"], "not_declared", "{:?}", config.name());
        }
    }

    /// The Financier's declaration must reach the API. This is the wiring the
    /// whole gap was about: without it the roster still answers `not_declared`
    /// and the user is never told which key to fill in.
    #[test]
    fn financier_declaration_reaches_the_roster() {
        let capabilities = serde_json::to_value(capabilities()).unwrap();
        let financier = capabilities
            .as_array()
            .unwrap()
            .iter()
            .find(|capability| capability["key"] == "finance")
            .expect("the Financier is a visible capability");
        let secrets = &financier["required_secrets"];
        assert_eq!(secrets["status"], "declared");
        let item = secrets["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == permagent::market_data::FUNDAMENTALS_KEY)
            .expect("the fundamentals key is declared");
        assert_eq!(item["impact"], "degraded");
        assert!(item["present"].is_boolean());
    }

    /// Presence-only, enforced on the SHAPE. `RequiredSecret` may never grow a
    /// field that could carry a value; the leak test above can only catch a
    /// value it happens to seed, this catches the field itself.
    #[test]
    fn required_secret_exposes_no_value_bearing_field() {
        let value = serde_json::to_value(RequiredSecret {
            name: "SOME_KEY".into(),
            present: true,
            impact: Some("degraded"),
            unlocks: Some("what it unlocks".into()),
        })
        .unwrap();
        let mut fields: Vec<&str> = value.as_object().unwrap().keys().map(|k| &**k).collect();
        fields.sort();
        assert_eq!(fields, vec!["impact", "name", "present", "unlocks"]);
    }

    /// A blank value is not a configured secret. Reporting one as present
    /// would tell the user to stop looking for a key the capability still
    /// refuses to use — `market_data`'s reader applies the same rule, and the
    /// two must not disagree about whether the Financier has its key.
    #[test]
    fn blank_value_is_absent_not_present() {
        assert!(!key_is_present(None, None));
        assert!(!key_is_present(Some("   ".into()), None));
        assert!(!key_is_present(None, Some(Value::String("".into()))));
        assert!(!key_is_present(Some("".into()), Some(Value::Null)));
        assert!(key_is_present(Some("k".into()), None));
        // A blank env var must not shadow a real stored secret.
        assert!(key_is_present(
            Some("".into()),
            Some(Value::String("k".into()))
        ));
    }

    /// REGRESSION. Before this change `background_workers` filtered on
    /// `worker_descriptor_visible`, so with `strix_enabled` off the Guard's row
    /// was absent from Settings → Agents entirely — the user had to switch the
    /// agent on before the page that switches it on would show it. The old code
    /// would return no `strix` row here at all and fail on the `expect`.
    #[test]
    fn gated_worker_is_listed_while_its_flag_is_off() {
        let value = serde_json::to_value(background_workers(
            Some(0),
            FeatureFlags::default(),
            &personas(),
        ))
        .unwrap();
        let guard = value
            .as_array()
            .unwrap()
            .iter()
            .find(|worker| worker["id"] == "strix")
            .expect("the Guard is listed while its flag is off");
        assert_eq!(guard["gate"]["config_key"], "strix_enabled");
        assert_eq!(guard["gate"]["enabled"], false);
    }

    /// The roster is now a pure function of WORKER_DESCRIPTORS, not of the
    /// host's config — which is what lets the count be pinned at all.
    #[test]
    fn every_worker_descriptor_reaches_the_roster() {
        let expected: Vec<&str> = WORKER_DESCRIPTORS.iter().map(|d| d.id).collect();
        for flags in [
            FeatureFlags::default(),
            FeatureFlags {
                playbook_enabled: true,
                concierge_enabled: true,
                strix_enabled: true,
                initiative_enabled: true,
                steward_scan_enabled: true,
            },
        ] {
            let rows = background_workers(Some(0), flags, &personas());
            assert_eq!(rows.len(), WORKER_DESCRIPTORS.len());
            let ids: Vec<&str> = rows.iter().map(|w| w.id.as_str()).collect();
            assert_eq!(ids, expected);
        }
    }

    /// REGRESSION. `visible_worker_descriptor(id, FeatureFlags::default())`
    /// returned `None` for a gated-off worker, so `GET /api/agents/playbook` and
    /// `GET /api/agents/strix/work` answered 404 — an absence reported as a
    /// not-found for an agent the daemon actually ships, and the 404 landed on
    /// the very page that carries the switch.
    ///
    /// `detail` and `work` need an `AppState` and are not driven here; what is
    /// driven is the whole of what they consult for a worker id — the lookup
    /// (`worker_descriptor`, which no longer takes flags at all, so the fix is
    /// structural) composed with the body `detail` returns
    /// (`background_worker`). The gated-off Guard must come back with its own
    /// switch attached, because that is what its page renders.
    #[test]
    fn worker_detail_and_work_resolve_while_the_flag_is_off() {
        let off = FeatureFlags::default();
        for id in ["playbook", "strix"] {
            let d = worker_descriptor(id).unwrap_or_else(|| panic!("{id} stopped resolving"));
            let body =
                serde_json::to_value(background_worker(d, Some(0), off, &personas())).unwrap();
            assert_eq!(body["id"], id);
            assert!(
                body["gate"]["config_key"].is_string(),
                "{id} resolved without the switch its page renders: {body}"
            );
            assert_eq!(body["gate"]["enabled"], false);
        }
        // The unknown id still 404s: the fix widened what exists, it did not
        // make every string an agent.
        assert!(worker_descriptor("no_such_worker_9f13").is_none());
    }

    /// REGRESSION. The persona key is `steward` while the descriptor id is
    /// `git_steward`; without the bridge the Steward's persona page — the page
    /// the user actually lands on — would carry no switch at all.
    #[test]
    fn gate_reaches_a_persona_under_its_own_key() {
        let persona = |key: &str| {
            let roster = agent_identity::default_roster();
            serde_json::to_value(dispatch_persona(
                key.to_string(),
                roster[key].clone(),
                Availability::Available,
                FeatureFlags::default(),
            ))
            .unwrap()
        };
        let steward = persona("steward");
        assert_eq!(steward["gate"]["config_key"], "steward_scan_enabled");
        assert_eq!(steward["gate"]["enabled"], false);

        let guard = persona("strix");
        assert_eq!(guard["gate"]["config_key"], "strix_enabled");
        assert_eq!(guard["gate"]["enabled"], false);
    }

    /// The key is PRESENT and null, never omitted: a client must be able to tell
    /// "this agent has no switch" from "the switch is off". An omitted field
    /// reads as `undefined` in the UI, which renders as a toggle claiming off.
    #[test]
    fn an_ungated_persona_serialises_gate_as_null() {
        let roster = agent_identity::default_roster();
        let persona = dispatch_persona(
            "claude_code".to_string(),
            roster["claude_code"].clone(),
            Availability::Available,
            FeatureFlags::default(),
        );
        assert!(persona.gate.is_none());
        let body = serde_json::to_string(&persona).unwrap();
        assert!(body.contains("\"gate\":null"), "{body}");
    }

    /// THE point of `RunsSection`, and the assertion the whole liveness surface
    /// rests on. The Playbook's code calls nothing that records a run, so this
    /// panel can only ever be empty for it — and an empty `ok` list renders as
    /// "idle, presumably about to do something", which is a claim nobody made.
    /// Asserted on the serialised TAG, because the enum variant is invisible to
    /// the client and the tag is what the UI branches on.
    #[tokio::test]
    async fn an_agent_that_records_no_runs_serialises_not_recorded_not_an_empty_ok_list() {
        let pool = runs_pool().await;
        for id in [
            "playbook",
            "scheduler",
            "librarian",
            "concierge",
            "onboarding_coach",
        ] {
            let value = serde_json::to_value(runs_section(&pool, id, 10).await).unwrap();
            assert_eq!(
                value["status"], "not_recorded",
                "{id} records no runs, so its section must say so: {value}"
            );
            assert!(
                value.get("items").is_none(),
                "{id} must not carry a list a reader could mistake for a run history"
            );
            let reason = value["reason"].as_str().unwrap();
            assert!(!reason.is_empty(), "{id} gave no reason");
            assert!(
                !agent_runs::records_runs(id),
                "{id} is now a run-recording agent; this test's premise moved"
            );
        }
    }

    /// The other side of the same distinction: an agent that DOES record runs
    /// and has not run yet is a genuine empty list — worth waiting for — and
    /// must serialise differently from the agent that will never appear here.
    #[tokio::test]
    async fn an_agent_that_records_runs_but_has_none_yet_serialises_an_empty_ok_list() {
        let pool = runs_pool().await;
        let value = serde_json::to_value(runs_section(&pool, "strix", 10).await).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["items"].as_array().unwrap().len(), 0);
        assert_eq!(value["truncated"], false);

        let never = serde_json::to_value(runs_section(&pool, "playbook", 10).await).unwrap();
        assert_ne!(
            value["status"], never["status"],
            "'has not run yet' and 'records no runs' must not serialise alike"
        );
    }

    /// An unreadable table must not render as an idle agent. `recent_for_agent`
    /// propagates the error; this pins that the section keeps it rather than
    /// degrading it into `ok` with nothing in it.
    #[tokio::test]
    async fn a_missing_runs_table_is_unavailable_not_an_empty_history() {
        let bare = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let value = serde_json::to_value(runs_section(&bare, "strix", 10).await).unwrap();
        assert_eq!(value["status"], "unavailable");
        assert!(!value["reason"].as_str().unwrap().is_empty());
    }

    /// REGRESSION, and the bridge this whole section needed. The Steward files
    /// briefings under `from_agent = "steward"` while its page is addressed as
    /// `git_steward`. Without `worker_keys_for_descriptor_id` the read returns
    /// nothing, and nothing renders exactly like an agent that has never
    /// reported — for the agent that reports most.
    #[tokio::test]
    async fn the_briefings_section_finds_the_stewards_rows_filed_under_its_persona_key() {
        let pool = briefings_pool().await;
        permagent::briefings::file_briefing(
            &pool,
            NewBriefing {
                from_agent: "steward".into(),
                kind: "repo_health".into(),
                severity: Severity::Attention,
                summary: "three merged branches are still around".into(),
                detail: None,
                ref_kind: None,
                ref_id: None,
            },
        )
        .await
        .expect("the Steward filed a briefing");

        let value =
            serde_json::to_value(briefings_section(&pool, "git_steward", 10).await).unwrap();
        assert_eq!(value["status"], "ok");
        let items = value["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "the page addressed as git_steward must reach the Steward's own rows: {value}"
        );
        assert_eq!(items[0]["from_agent"], "steward");

        // And the read is still scoped: another agent's briefings do not leak in.
        let watcher = serde_json::to_value(briefings_section(&pool, "watcher", 10).await).unwrap();
        assert_eq!(watcher["items"].as_array().unwrap().len(), 0);
    }

    /// Both signals must carry a REAL reason for an agent that has neither
    /// affordance — the Onboarding coach has no dispatch persona and no pass at
    /// all — and neither may be a bare boolean or an omitted field, because a
    /// missing field renders as an enabled control in the client.
    #[test]
    fn ask_and_run_now_are_unavailable_with_a_true_reason_for_an_agent_that_has_neither() {
        let roster = personas();
        let value = serde_json::to_value(background_worker(
            worker_descriptor("onboarding_coach").unwrap(),
            Some(0),
            FeatureFlags::default(),
            &roster,
        ))
        .unwrap();
        assert_eq!(value["ask"]["status"], "unavailable");
        assert!(value["ask"]["reason"]
            .as_str()
            .unwrap()
            .contains("no dispatch persona"));
        assert_eq!(value["run_now"]["status"], "unavailable");
        assert!(!value["run_now"]["reason"].as_str().unwrap().is_empty());

        // Every worker row carries both keys, always — an absent key is the
        // failure mode this replaces.
        for row in background_workers(Some(0), FeatureFlags::default(), &roster) {
            let row = serde_json::to_value(row).unwrap();
            assert!(row["ask"]["status"].is_string(), "{row}");
            assert!(row["run_now"]["status"].is_string(), "{row}");
        }
    }

    /// The Guard has both: an `engine: Pending` persona (askable — see
    /// `ask_availability_for_persona` on why Pending is not the same bar) and a
    /// real on-demand pass.
    #[test]
    fn ask_and_run_now_are_available_for_the_agent_that_has_both() {
        let roster = personas();
        let value = serde_json::to_value(background_worker(
            worker_descriptor("strix").unwrap(),
            Some(0),
            FeatureFlags::default(),
            &roster,
        ))
        .unwrap();
        assert_eq!(value["ask"]["status"], "available");
        assert!(value["ask"].get("reason").is_none());
        assert_eq!(value["run_now"]["status"], "available");

        // The Steward's WORKER row and its PERSONA row must agree, across the
        // `git_steward` / `steward` namespace split — the same agent cannot be
        // askable on one of its two pages and not the other.
        let worker = serde_json::to_value(background_worker(
            worker_descriptor("git_steward").unwrap(),
            Some(0),
            FeatureFlags::default(),
            &roster,
        ))
        .unwrap();
        let persona = serde_json::to_value(dispatch_persona(
            "steward".to_string(),
            roster["steward"].clone(),
            Availability::Available,
            FeatureFlags::default(),
        ))
        .unwrap();
        assert_eq!(worker["ask"]["status"], "available");
        assert_eq!(persona["ask"]["status"], "available");
        assert_eq!(worker["run_now"]["status"], "available");
        assert_eq!(persona["run_now"]["status"], "available");
    }

    /// An external CLI is a binary launched against a goal in a worktree; there
    /// is no in-process turn for it to take. The signal says so, and the route's
    /// gate must refuse for the SAME reason — a route more permissive than its
    /// own signal would answer for an agent the page showed as unaskable.
    #[test]
    fn the_ask_route_refuses_an_unknown_agent_and_an_external_cli_persona_with_a_real_message() {
        let roster = personas();

        let unknown = resolve_ask_target("no_such_agent_4c19", &roster).unwrap_err();
        assert_eq!(unknown.0, StatusCode::NOT_FOUND);
        assert!(unknown.1.contains("no_such_agent_4c19"));

        let external = resolve_ask_target("claude_code", &roster).unwrap_err();
        assert_eq!(external.0, StatusCode::CONFLICT);
        assert!(
            external.1.contains("claude"),
            "the refusal must name the binary it cannot run as a turn: {}",
            external.1
        );
        // The signal and the refusal are the same sentence.
        match ask_availability_for_persona("claude_code", &roster["claude_code"]) {
            AskAvailability::Unavailable { reason } => assert_eq!(reason, external.1),
            AskAvailability::Available => panic!("an external CLI must not be askable"),
        }

        // A background worker with no persona is a 409 with the shared reason,
        // never a 404: the agent exists, it just cannot be asked.
        let no_persona = resolve_ask_target("onboarding_coach", &roster).unwrap_err();
        assert_eq!(no_persona.0, StatusCode::CONFLICT);
        assert_eq!(no_persona.1, no_persona_reason("onboarding_coach"));

        // And an askable one resolves, so the refusals above are not vacuous.
        let (key, _) = resolve_ask_target("git_steward", &roster).unwrap();
        assert_eq!(
            key, "steward",
            "the page id must resolve through the bridge"
        );
    }

    /// The trap this route was written around. `summon`'s delegate path does
    /// not apply `extension_grants`, so a straight copy would hand the turn
    /// every tool while the response claimed a scope. Proven on the real
    /// narrowing: a persona granted one extension must not come away holding
    /// the other, and the reported scope must name both what was granted and
    /// what survived.
    #[test]
    fn a_grant_narrows_the_asked_agents_tools_and_the_scope_reports_what_survived() {
        let builtin = |name: &str| ExtensionConfig::Builtin {
            name: name.to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        let base = vec![builtin("developer"), builtin("computercontroller")];

        let mut granted = WorkerPersona {
            extension_grants: Some(vec!["developer".to_string()]),
            ..Default::default()
        };
        let (narrowed, scope) = apply_tool_scope(&granted, base.clone());
        assert_eq!(
            narrowed.iter().map(|c| c.key()).collect::<Vec<_>>(),
            vec!["developer".to_string()],
            "the ungranted extension must not reach the turn"
        );
        assert_eq!(
            scope,
            AppliedToolScope::Explicit {
                granted: vec!["developer".to_string()],
                applied: vec!["developer".to_string()],
            }
        );

        // A grant naming something the run never had cannot manufacture it, and
        // the scope must show the gap rather than repeating the declaration.
        granted.extension_grants = Some(vec!["developer".into(), "not_installed_9f2a".into()]);
        let (narrowed, scope) = apply_tool_scope(&granted, base.clone());
        assert_eq!(narrowed.len(), 1);
        match scope {
            AppliedToolScope::Explicit { granted, applied } => {
                assert_eq!(granted.len(), 2);
                assert_eq!(applied, vec!["developer".to_string()]);
            }
            other => panic!("expected an explicit scope, got {other:?}"),
        }

        // No grants means the global set, and it is named as inherited rather
        // than dressed up as the agent's own.
        let inherits = WorkerPersona::default();
        let (narrowed, scope) = apply_tool_scope(&inherits, base);
        assert_eq!(narrowed.len(), 2);
        assert!(matches!(scope, AppliedToolScope::InheritGlobal { .. }));
    }

    /// The `run_now` signal and the run route read one function, so they cannot
    /// disagree — and every agent it declares runnable must be a real worker
    /// that RECORDS the run it is about to start. A pass that runs and records
    /// nothing would make the route's own honesty check (no new row ⇒ error)
    /// fire on every press.
    #[test]
    fn every_agent_with_an_on_demand_pass_is_a_real_worker_that_records_its_runs() {
        let runnable: Vec<&str> = WORKER_DESCRIPTORS
            .iter()
            .map(|d| d.id)
            .filter(|id| run_pass_for(id).is_some())
            .collect();
        assert_eq!(
            runnable,
            vec!["git_steward", "watcher", "strix"],
            "the runnable set changed; the reasons on every other agent may now be stale"
        );
        for id in &runnable {
            assert!(
                agent_runs::records_runs(id),
                "'{id}' has a run button but records no run, so pressing it can only error"
            );
            assert!(matches!(run_availability(id), RunAvailability::Available));
        }
        // Every other worker states a real reason instead.
        for d in WORKER_DESCRIPTORS
            .iter()
            .filter(|d| run_pass_for(d.id).is_none())
        {
            match run_availability(d.id) {
                RunAvailability::Unavailable { reason } => {
                    assert!(!reason.is_empty(), "{} gave no reason", d.id);
                    assert!(
                        reason.len() > 40,
                        "{}'s reason is too thin to act on: {reason}",
                        d.id
                    );
                }
                RunAvailability::Available => {
                    panic!("{} claims a pass that run_pass_for does not have", d.id)
                }
            }
        }
    }

    /// The Steward's two page ids must answer identically. `steward` the
    /// persona key and `git_steward` the descriptor id are one agent, and a
    /// run button that worked on one page and refused on the other would be the
    /// same half-wired split this surface keeps closing.
    #[test]
    fn the_stewards_two_ids_answer_the_same_way_about_running() {
        assert!(matches!(
            run_availability("steward"),
            RunAvailability::Available
        ));
        assert!(matches!(
            run_availability("git_steward"),
            RunAvailability::Available
        ));
    }
}
