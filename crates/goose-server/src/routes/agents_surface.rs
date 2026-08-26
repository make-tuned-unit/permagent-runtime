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
use permagent::agents::extension::ExtensionConfig;
use permagent::agents::platform_extensions::{
    RequiredSecretDef, SecretImpact, PLATFORM_EXTENSIONS,
};
use permagent::agents::self_knowledge::{
    self, worker_live_state_for, FeatureDescriptor, FeatureFlags, StateSource, WORKER_DESCRIPTORS,
};
use permagent::config::agent_identity::{self, WorkerPersona};
use permagent::config::extensions::name_to_key;
use permagent::config::worker_probe;
use permagent::config::{extension_is_grantable, get_all_extensions, is_extension_enabled, Config};
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
) -> Vec<BackgroundWorker> {
    WORKER_DESCRIPTORS
        .iter()
        .map(|d| background_worker(d, scheduled_job_count, flags))
        .collect()
}

fn background_worker(
    d: &FeatureDescriptor,
    scheduled_job_count: Option<usize>,
    flags: FeatureFlags,
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
    Json(RosterResponse {
        workers: background_workers(Some(jobs.len()), flags),
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
        return Ok(Json(AgentDetail::Worker(background_worker(
            d,
            Some(jobs.len()),
            flags,
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

#[derive(Serialize)]
struct WorkReview {
    activity: ActivitySection,
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
    let pool = state.session_manager().pool_clone().await;
    let (activity, goals, spend) = match pool {
        Err(error) => {
            let reason = error.to_string();
            (
                ListSection::Unavailable {
                    reason: reason.clone(),
                },
                ListSection::Unavailable {
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
                // `sql` interpolates only "?" placeholders (count = goal_ids.len()).
                let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
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
            (activity, goals, spend)
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

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // matchit prefers this static segment over `{id}`. The write handlers
        // also reserve `roster`, preventing a persona from shadowing it.
        .route("/api/agents/roster", get(roster))
        .route("/api/agents/{id}", get(detail))
        .route("/api/agents/{id}/work", get(work))
        .route("/api/agents/{id}/grants", post(set_grants))
        .route("/api/agents/{id}/secrets", post(set_secret))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_are_never_dispatchable_and_have_no_affordance() {
        let value =
            serde_json::to_value(background_workers(Some(3), FeatureFlags::default())).unwrap();
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
            workers: background_workers(Some(0), FeatureFlags::default()),
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
            }],
            capabilities: capabilities(),
        };
        let detail =
            AgentDetail::Worker(background_workers(Some(0), FeatureFlags::default()).remove(0));
        let work = WorkReview {
            activity: ActivitySection {
                attribution: "actor_exact_match",
                result: ListSection::Ok {
                    items: Vec::new(),
                    truncated: false,
                },
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
        let value =
            serde_json::to_value(background_workers(Some(0), FeatureFlags::default())).unwrap();
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
                council_enabled: true,
            },
        ] {
            let rows = background_workers(Some(0), flags);
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
            let body = serde_json::to_value(background_worker(d, Some(0), off)).unwrap();
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
}
