use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{job::JobId, Job, JobScheduler as TokioJobScheduler};
use tokio_util::sync::CancellationToken;

/// Self-knowledge descriptor for the Scheduler worker. Co-located with the
/// worker it describes; aggregated by `crate::agents::self_knowledge`. Queryable
/// — live job count is merged into the brief via `list_scheduled_jobs`.
pub const SELF_KNOWLEDGE_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "scheduler",
        display_name: "Scheduler",
        category: crate::agents::self_knowledge::FeatureCategory::Worker,
        what_it_does: "Runs saved recipes and reminders on a cron schedule in the background",
        why_it_matters:
            "Lets you promise recurring or future work and actually deliver it without the user re-asking",
        state_source: crate::agents::self_knowledge::StateSource::Queryable,
        // Queryable → the cleanest read-back loop in the tour: HasScheduledJob is
        // visible directly in the brief (the Scheduler line goes 0 → 1).
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Open Automate",
                body: "Show them where recurring automations and reminders live.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Automate",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Schedule something real",
                body: "Offer to set up a simple recurring job — a daily digest, a weekly check-in — and create it for them so they see it actually works.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::HasScheduledJob),
            },
        ],
    };

use crate::agents::AgentEvent;
use crate::agents::{Agent, SessionConfig};
use crate::config::paths::Paths;
use crate::config::{resolve_extensions_for_new_session, Config};
use crate::conversation::message::Message;
use crate::conversation::Conversation;
#[cfg(feature = "telemetry")]
use crate::posthog;
use crate::providers::create;
use crate::recipe::Recipe;
use crate::scheduler_trait::SchedulerTrait;
use crate::session::session_manager::SessionType;
use crate::session::{Session, SessionManager};

type RunningTasksMap = HashMap<String, CancellationToken>;
type JobsMap = HashMap<String, (JobId, ScheduledJob)>;

pub fn get_default_scheduler_storage_path() -> Result<PathBuf, io::Error> {
    let data_dir = Paths::data_dir();
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("schedule.json"))
}

pub fn get_default_scheduled_recipes_dir() -> Result<PathBuf, SchedulerError> {
    let data_dir = Paths::data_dir();
    let recipes_dir = data_dir.join("scheduled_recipes");
    fs::create_dir_all(&recipes_dir).map_err(SchedulerError::StorageError)?;
    Ok(recipes_dir)
}

#[derive(Debug)]
pub enum SchedulerError {
    JobIdExists(String),
    JobNotFound(String),
    StorageError(io::Error),
    RecipeLoadError(String),
    AgentSetupError(String),
    PersistError(String),
    CronParseError(String),
    SchedulerInternalError(String),
    AnyhowError(anyhow::Error),
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::JobIdExists(id) => write!(f, "Job ID '{}' already exists.", id),
            SchedulerError::JobNotFound(id) => write!(f, "Job ID '{}' not found.", id),
            SchedulerError::StorageError(e) => write!(f, "Storage error: {}", e),
            SchedulerError::RecipeLoadError(e) => write!(f, "Recipe load error: {}", e),
            SchedulerError::AgentSetupError(e) => write!(f, "Agent setup error: {}", e),
            SchedulerError::PersistError(e) => write!(f, "Failed to persist schedules: {}", e),
            SchedulerError::CronParseError(e) => write!(f, "Invalid cron string: {}", e),
            SchedulerError::SchedulerInternalError(e) => {
                write!(f, "Scheduler internal error: {}", e)
            }
            SchedulerError::AnyhowError(e) => write!(f, "Scheduler operation failed: {}", e),
        }
    }
}

impl std::error::Error for SchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SchedulerError::StorageError(e) => Some(e),
            SchedulerError::AnyhowError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for SchedulerError {
    fn from(err: io::Error) -> Self {
        SchedulerError::StorageError(err)
    }
}

impl From<serde_json::Error> for SchedulerError {
    fn from(err: serde_json::Error) -> Self {
        SchedulerError::PersistError(err.to_string())
    }
}

impl From<anyhow::Error> for SchedulerError {
    fn from(err: anyhow::Error) -> Self {
        SchedulerError::AnyhowError(err)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
pub struct ScheduledJob {
    pub id: String,
    pub source: String,
    pub cron: String,
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub currently_running: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub current_session_id: Option<String>,
    #[serde(default)]
    pub process_start_time: Option<DateTime<Utc>>,
    /// Worker persona key from agent.yaml workers map.
    /// When set, the scheduled run uses the worker's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_persona: Option<String>,

    // ── Starter recipe versioning fields ──
    /// Non-null for starter recipes (e.g. "storage-insights", "workspace-snapshot").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_id: Option<String>,
    /// Embedded YAML version at install/upgrade time (e.g. "2.0.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_version: Option<String>,
    /// SHA-256 of the YAML content we last wrote to disk (install or upgrade).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starter_content_hash: Option<String>,
    /// True if the user has manually edited this starter recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_customized: Option<bool>,
}

async fn persist_jobs(
    storage_path: &Path,
    jobs: &Arc<Mutex<JobsMap>>,
) -> Result<(), SchedulerError> {
    let jobs_guard = jobs.lock().await;
    let list: Vec<ScheduledJob> = jobs_guard.values().map(|(_, j)| j.clone()).collect();
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(&list)?;
    fs::write(storage_path, data)?;
    Ok(())
}

pub struct Scheduler {
    tokio_scheduler: TokioJobScheduler,
    jobs: Arc<Mutex<JobsMap>>,
    storage_path: PathBuf,
    running_tasks: Arc<Mutex<RunningTasksMap>>,
    session_manager: Arc<SessionManager>,
    brain: Arc<tokio::sync::RwLock<Option<crate::brain_handle::SafeBrain>>>,
    persona: Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedPersona>>>,
    agent_config:
        Arc<tokio::sync::RwLock<Option<crate::config::agent_identity::SharedAgentConfig>>>,
}

impl Scheduler {
    pub async fn new(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>, SchedulerError> {
        let internal_scheduler = TokioJobScheduler::new()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let jobs = Arc::new(Mutex::new(HashMap::new()));
        let running_tasks = Arc::new(Mutex::new(HashMap::new()));

        let arc_self = Arc::new(Self {
            tokio_scheduler: internal_scheduler,
            jobs,
            storage_path,
            running_tasks,
            session_manager,
            brain: Arc::new(tokio::sync::RwLock::new(None)),
            persona: Arc::new(tokio::sync::RwLock::new(None)),
            agent_config: Arc::new(tokio::sync::RwLock::new(None)),
        });

        arc_self.load_jobs_from_storage().await;
        arc_self
            .tokio_scheduler
            .start()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        Ok(arc_self)
    }

    fn create_cron_task(&self, job: ScheduledJob) -> Result<Job, SchedulerError> {
        let job_for_task = job.clone();
        let jobs_arc = self.jobs.clone();
        let storage_path = self.storage_path.clone();
        let running_tasks_arc = self.running_tasks.clone();
        let brain_arc = self.brain.clone();
        let persona_arc = self.persona.clone();
        let agent_config_arc = self.agent_config.clone();

        let cron_parts: Vec<&str> = job.cron.split_whitespace().collect();
        let cron = match cron_parts.len() {
            5 => {
                tracing::warn!(
                    "Job '{}' has legacy 5-field cron '{}', converting to 6-field",
                    job.id,
                    job.cron
                );
                format!("0 {}", job.cron)
            }
            6 => job.cron.clone(),
            _ => {
                return Err(SchedulerError::CronParseError(format!(
                    "Invalid cron expression '{}': expected 5 or 6 fields, got {}",
                    job.cron,
                    cron_parts.len()
                )))
            }
        };

        let local_tz = Local::now().timezone();

        Job::new_async_tz(&cron, local_tz, move |_uuid, _l| {
            tracing::info!("Cron task triggered for job '{}'", job_for_task.id);
            let task_job_id = job_for_task.id.clone();
            let current_jobs_arc = jobs_arc.clone();
            let local_storage_path = storage_path.clone();
            let job_to_execute = job_for_task.clone();
            let running_tasks = running_tasks_arc.clone();
            let brain_for_task = brain_arc.clone();
            let persona_for_task = persona_arc.clone();
            let agent_config_for_task = agent_config_arc.clone();

            Box::pin(async move {
                let should_execute = {
                    let jobs_guard = current_jobs_arc.lock().await;
                    jobs_guard
                        .get(&task_job_id)
                        .map(|(_, j)| !j.paused)
                        .unwrap_or(false)
                };

                if !should_execute {
                    return;
                }

                let current_time = Utc::now();
                {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    if let Some((_, job)) = jobs_guard.get_mut(&task_job_id) {
                        job.last_run = Some(current_time);
                        job.currently_running = true;
                        job.process_start_time = Some(current_time);
                    }
                }

                if let Err(e) = persist_jobs(&local_storage_path, &current_jobs_arc).await {
                    tracing::error!("Failed to persist job status: {}", e);
                }

                let cancel_token = CancellationToken::new();
                {
                    let mut tasks = running_tasks.lock().await;
                    tasks.insert(task_job_id.clone(), cancel_token.clone());
                }

                // Emit job started activity event
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_started(&task_job_id, &task_job_id),
                );

                let job_start_instant = std::time::Instant::now();
                let brain_snapshot = brain_for_task.read().await.clone();
                let persona_snapshot = persona_for_task.read().await.clone();
                let ac_snapshot = agent_config_for_task.read().await.clone();
                let result = execute_job(
                    job_to_execute,
                    current_jobs_arc.clone(),
                    task_job_id.clone(),
                    cancel_token.clone(),
                    brain_snapshot,
                    persona_snapshot,
                    ac_snapshot,
                )
                .await;

                {
                    let mut tasks = running_tasks.lock().await;
                    tasks.remove(&task_job_id);
                }

                {
                    let mut jobs_guard = current_jobs_arc.lock().await;
                    if let Some((_, job)) = jobs_guard.get_mut(&task_job_id) {
                        job.currently_running = false;
                        job.current_session_id = None;
                        job.process_start_time = None;
                    }
                }

                if let Err(e) = persist_jobs(&local_storage_path, &current_jobs_arc).await {
                    tracing::error!("Failed to persist job completion: {}", e);
                }

                let duration_ms = job_start_instant.elapsed().as_millis() as u64;
                match result {
                    Ok(ref session_id) => {
                        tracing::info!("Job '{}' completed", task_job_id);
                        crate::events::activity::emit_activity(
                            crate::events::activity::automation_job_completed(
                                &task_job_id,
                                &task_job_id,
                                session_id,
                                duration_ms,
                                0, // message count not easily available here
                            ),
                        );
                    }
                    Err(ref e) => {
                        tracing::error!("Job '{}' failed: {}", task_job_id, e);
                        crate::events::activity::emit_activity(
                            crate::events::activity::automation_job_failed(
                                &task_job_id,
                                &task_job_id,
                                &e.to_string(),
                            ),
                        );
                        #[cfg(feature = "telemetry")]
                        crate::posthog::emit_error("scheduler_job_failed", &e.to_string());
                    }
                }
            })
        })
        .map_err(|e| SchedulerError::CronParseError(e.to_string()))
    }

    pub async fn add_scheduled_job(
        &self,
        original_job_spec: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        {
            let jobs_guard = self.jobs.lock().await;
            if jobs_guard.contains_key(&original_job_spec.id) {
                return Err(SchedulerError::JobIdExists(original_job_spec.id.clone()));
            }
        }

        let mut stored_job = original_job_spec;
        if make_copy {
            let original_recipe_path = Path::new(&stored_job.source);
            if !original_recipe_path.is_file() {
                return Err(SchedulerError::RecipeLoadError(format!(
                    "Recipe file not found: {}",
                    stored_job.source
                )));
            }

            let scheduled_recipes_dir = get_default_scheduled_recipes_dir()?;
            let original_extension = original_recipe_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("yaml");

            let destination_filename = format!("{}.{}", stored_job.id, original_extension);
            let destination_recipe_path = scheduled_recipes_dir.join(destination_filename);

            fs::copy(original_recipe_path, &destination_recipe_path)?;
            stored_job.source = destination_recipe_path.to_string_lossy().into_owned();
            stored_job.current_session_id = None;
            stored_job.process_start_time = None;
        }

        let cron_task = self.create_cron_task(stored_job.clone())?;

        let job_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(stored_job.id.clone(), (job_uuid, stored_job));
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;
        Ok(())
    }

    pub async fn schedule_recipe(
        &self,
        recipe_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        let recipe_path_str = recipe_path.to_string_lossy().to_string();

        let existing_job_id = {
            let jobs_guard = self.jobs.lock().await;
            jobs_guard
                .iter()
                .find(|(_, (_, job))| job.source == recipe_path_str)
                .map(|(id, _)| id.clone())
        };

        match cron_schedule {
            Some(cron) => {
                if let Some(job_id) = existing_job_id {
                    self.update_schedule(&job_id, cron).await
                } else {
                    let job_id = self.generate_unique_job_id(&recipe_path).await;
                    let job = ScheduledJob {
                        id: job_id,
                        source: recipe_path_str,
                        cron,
                        last_run: None,
                        currently_running: false,
                        paused: false,
                        current_session_id: None,
                        process_start_time: None,
                        worker_persona: None,
                        starter_id: None,
                        starter_version: None,
                        starter_content_hash: None,
                        user_customized: None,
                    };
                    self.add_scheduled_job(job, false).await
                }
            }
            None => {
                if let Some(job_id) = existing_job_id {
                    self.remove_scheduled_job(&job_id, false).await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn generate_unique_job_id(&self, path: &Path) -> String {
        let base_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let jobs_guard = self.jobs.lock().await;
        let mut id = base_id.clone();
        let mut counter = 1;

        while jobs_guard.contains_key(&id) {
            id = format!("{}_{}", base_id, counter);
            counter += 1;
        }

        id
    }

    async fn load_jobs_from_storage(self: &Arc<Self>) {
        if !self.storage_path.exists() {
            return;
        }
        let data = match fs::read_to_string(&self.storage_path) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!(
                    "Failed to read {}: {}. Starting with empty schedule list.",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };
        if data.trim().is_empty() {
            return;
        }

        let list: Vec<ScheduledJob> = match serde_json::from_str(&data) {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(
                    "Failed to parse {}: {}. Starting with empty schedule list.",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };

        for job_to_load in list {
            if !Path::new(&job_to_load.source).exists() {
                tracing::warn!(
                    "Recipe file {} not found, skipping job '{}'",
                    job_to_load.source,
                    job_to_load.id
                );
                continue;
            }

            let cron_task = match self.create_cron_task(job_to_load.clone()) {
                Ok(task) => task,
                Err(e) => {
                    tracing::error!(
                        "Failed to create cron task for job '{}': {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let job_uuid = match self.tokio_scheduler.add(cron_task).await {
                Ok(uuid) => uuid,
                Err(e) => {
                    tracing::error!(
                        "Failed to add job '{}' to scheduler: {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(job_to_load.id.clone(), (job_uuid, job_to_load));
        }
    }

    async fn sync_from_storage(&self) {
        if !self.storage_path.exists() {
            return;
        }
        let data = match fs::read_to_string(&self.storage_path) {
            Ok(d) => d,
            Err(_) => return,
        };
        if data.trim().is_empty() {
            return;
        }
        let disk_jobs: Vec<ScheduledJob> = match serde_json::from_str(&data) {
            Ok(jobs) => jobs,
            Err(_) => return,
        };

        let disk_ids: std::collections::HashSet<String> =
            disk_jobs.iter().map(|j| j.id.clone()).collect();

        let (jobs_to_add, jobs_to_remove): (Vec<ScheduledJob>, Vec<(String, JobId)>) = {
            let jobs_guard = self.jobs.lock().await;
            let to_add = disk_jobs
                .into_iter()
                .filter(|j| !jobs_guard.contains_key(&j.id))
                .collect();
            let to_remove = jobs_guard
                .iter()
                .filter(|(id, (_, j))| !disk_ids.contains(*id) && !j.currently_running)
                .map(|(id, (uuid, _))| (id.clone(), *uuid))
                .collect();
            (to_add, to_remove)
        };

        for job in jobs_to_add {
            if !Path::new(&job.source).exists() {
                tracing::warn!(
                    "Skipping sync of job '{}': recipe file not found at {}",
                    job.id,
                    job.source
                );
                continue;
            }
            let cron_task = match self.create_cron_task(job.clone()) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(
                        "Failed to create cron task for '{}' during sync: {}",
                        job.id,
                        e
                    );
                    continue;
                }
            };
            let uuid = match self.tokio_scheduler.add(cron_task).await {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("Failed to register job '{}' during sync: {}", job.id, e);
                    continue;
                }
            };
            self.jobs.lock().await.insert(job.id.clone(), (uuid, job));
        }

        for (id, uuid) in jobs_to_remove {
            let _ = self.tokio_scheduler.remove(&uuid).await;
            self.jobs.lock().await.remove(&id);
        }
    }

    pub async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.sync_from_storage().await;
        self.jobs
            .lock()
            .await
            .values()
            .map(|(_, j)| j.clone())
            .collect()
    }

    pub async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_recipe: bool,
    ) -> Result<(), SchedulerError> {
        let (job_uuid, recipe_path) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.remove(id) {
                Some((uuid, job)) => (uuid, job.source.clone()),
                None => return Err(SchedulerError::JobNotFound(id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&job_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        if remove_recipe {
            let path = Path::new(&recipe_path);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;
        Ok(())
    }

    pub async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        let all_sessions = self
            .session_manager
            .list_sessions()
            .await
            .map_err(|e| SchedulerError::StorageError(io::Error::other(e)))?;

        let mut schedule_sessions: Vec<(String, Session)> = all_sessions
            .into_iter()
            .filter(|s| s.schedule_id.as_deref() == Some(sched_id))
            .map(|s| (s.id.clone(), s))
            .collect();

        schedule_sessions.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
        schedule_sessions.truncate(limit);

        Ok(schedule_sessions)
    }

    pub async fn run_now(&self, sched_id: &str) -> Result<String, SchedulerError> {
        let job_to_run = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is already running",
                            sched_id
                        )));
                    }
                    job.currently_running = true;
                    job.process_start_time = Some(Utc::now());
                    job.clone()
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        persist_jobs(&self.storage_path, &self.jobs).await?;

        let cancel_token = CancellationToken::new();
        {
            let mut tasks = self.running_tasks.lock().await;
            tasks.insert(sched_id.to_string(), cancel_token.clone());
        }

        // Emit job started activity event
        crate::events::activity::emit_activity(crate::events::activity::automation_job_started(
            sched_id, sched_id,
        ));

        let job_start_instant = std::time::Instant::now();
        let brain_snapshot = self.brain.read().await.clone();
        let persona_snapshot = self.persona.read().await.clone();
        let ac_snapshot = self.agent_config.read().await.clone();
        let result = execute_job(
            job_to_run,
            self.jobs.clone(),
            sched_id.to_string(),
            cancel_token.clone(),
            brain_snapshot,
            persona_snapshot,
            ac_snapshot,
        )
        .await;

        {
            let mut tasks = self.running_tasks.lock().await;
            tasks.remove(sched_id);
        }

        let duration_ms = job_start_instant.elapsed().as_millis() as u64;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                job.currently_running = false;
                job.current_session_id = None;
                job.process_start_time = None;
                job.last_run = Some(Utc::now());
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await?;

        match result {
            Ok(session_id) => {
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_completed(
                        sched_id,
                        sched_id,
                        &session_id,
                        duration_ms,
                        0,
                    ),
                );
                Ok(session_id)
            }
            Err(e) => {
                crate::events::activity::emit_activity(
                    crate::events::activity::automation_job_failed(
                        sched_id,
                        sched_id,
                        &e.to_string(),
                    ),
                );
                Err(SchedulerError::AnyhowError(anyhow!(
                    "Job '{}' failed: {}",
                    sched_id,
                    e
                )))
            }
        }
    }

    pub async fn pause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot pause running schedule '{}'",
                            sched_id
                        )));
                    }
                    job.paused = true;
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn unpause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => job.paused = false,
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        let (old_uuid, updated_job) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((uuid, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot update running schedule '{}'",
                            sched_id
                        )));
                    }
                    if new_cron == job.cron {
                        return Ok(());
                    }
                    job.cron = new_cron.clone();
                    (*uuid, job.clone())
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&old_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let cron_task = self.create_cron_task(updated_job)?;
        let new_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((uuid, _)) = jobs_guard.get_mut(sched_id) {
                *uuid = new_uuid;
            }
        }

        persist_jobs(&self.storage_path, &self.jobs).await
    }

    pub async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let jobs_guard = self.jobs.lock().await;
            match jobs_guard.get(sched_id) {
                Some((_, job)) if !job.currently_running => {
                    return Err(SchedulerError::AnyhowError(anyhow!(
                        "Schedule '{}' is not running",
                        sched_id
                    )));
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
                _ => {}
            }
        }

        {
            let tasks = self.running_tasks.lock().await;
            if let Some(token) = tasks.get(sched_id) {
                token.cancel();
            }
        }

        Ok(())
    }

    pub async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        let jobs_guard = self.jobs.lock().await;
        match jobs_guard.get(sched_id) {
            Some((_, job)) if job.currently_running => {
                match (&job.current_session_id, &job.process_start_time) {
                    (Some(sid), Some(start)) => Ok(Some((sid.clone(), *start))),
                    _ => Ok(None),
                }
            }
            Some(_) => Ok(None),
            None => Err(SchedulerError::JobNotFound(sched_id.to_string())),
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn execute_job(
    job: ScheduledJob,
    jobs: Arc<Mutex<JobsMap>>,
    job_id: String,
    cancel_token: CancellationToken,
    brain: Option<crate::brain_handle::SafeBrain>,
    persona: Option<crate::config::agent_identity::SharedPersona>,
    agent_config: Option<crate::config::agent_identity::SharedAgentConfig>,
) -> Result<String> {
    if job.source.is_empty() {
        return Ok(job.id.to_string());
    }

    let recipe_path = Path::new(&job.source);
    let recipe_content = fs::read_to_string(recipe_path)?;

    let recipe: Recipe = {
        let extension = recipe_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml")
            .to_lowercase();

        match extension.as_str() {
            "json" | "jsonl" => serde_json::from_str(&recipe_content)?,
            _ => serde_yaml::from_str(&recipe_content)?,
        }
    };

    let agent = Agent::new();

    // Wire persona into agent: worker persona if specified, else primary.
    if let Some(ref worker_key) = job.worker_persona {
        let mut resolved = false;
        if let Some(ref ac) = agent_config {
            let guard = ac.read().await;
            if let Some(worker) = guard.workers.get(worker_key) {
                agent
                    .set_persona_block_override(worker.system_prompt_block(), worker.display_name())
                    .await;
                resolved = true;
                tracing::info!(
                    target: "permagentd::brain",
                    "Scheduled job {} using worker persona: {}",
                    job_id,
                    worker_key
                );
            }
        }
        if !resolved {
            tracing::warn!(
                target: "permagentd::brain",
                "Worker persona '{}' not found for scheduled job {}, falling back to primary",
                worker_key,
                job_id
            );
            if let Some(ref p) = persona {
                agent.set_persona(p.clone()).await;
            }
        }
    } else if let Some(ref p) = persona {
        agent.set_persona(p.clone()).await;
    }

    let config = Config::global();
    let provider_name = config.get_goose_provider()?;
    let model_name = config.get_goose_model()?;
    let model_config =
        crate::model::ModelConfig::new(&model_name)?.with_canonical_limits(&provider_name);

    let session = agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir()?,
            format!("Scheduled job: {}", job.id),
            SessionType::Scheduled,
            agent.config.goose_mode,
        )
        .await?;

    let extensions = resolve_extensions_for_new_session(recipe.extensions.as_deref(), None);
    for ext in &extensions {
        agent.add_extension(ext.clone(), &session.id).await?;
    }

    let agent_provider = create(&provider_name, model_config, extensions).await?;
    agent.update_provider(agent_provider, &session.id).await?;

    let mut jobs_guard = jobs.lock().await;
    if let Some((_, job_def)) = jobs_guard.get_mut(job_id.as_str()) {
        job_def.current_session_id = Some(session.id.clone());
    }
    drop(jobs_guard);

    let start_time = std::time::Instant::now();

    let recipe_display_name = recipe_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&job.id);
    let recipe_version = recipe.version.clone();

    tracing::info!(
        monotonic_counter.goose.session_starts = 1,
        session_type = "schedule",
        interface = "scheduler",
        interactive = false,
        "Scheduled session started"
    );

    tracing::info!(
        monotonic_counter.goose.recipe_runs = 1,
        recipe_name = %recipe_display_name,
        recipe_version = %recipe_version,
        session_type = "schedule",
        interface = "scheduler",
        "Recipe execution started"
    );

    #[cfg(feature = "telemetry")]
    tokio::spawn(async move {
        let mut props = HashMap::new();
        props.insert(
            "trigger".to_string(),
            serde_json::Value::String("automated".to_string()),
        );
        if let Err(e) = posthog::emit_event("schedule_job_started", props).await {
            tracing::debug!("Failed to send schedule telemetry: {}", e);
        }
    });

    let prompt_text = recipe
        .prompt
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            recipe
                .instructions
                .as_deref()
                .filter(|s| !s.trim().is_empty())
        })
        .ok_or_else(|| {
            anyhow!("Recipe must specify at least one of `instructions` or `prompt`.")
        })?;

    let user_message = Message::user().with_text(prompt_text);
    let mut conversation = Conversation::new_unvalidated(vec![user_message.clone()]);

    // ── Phase 3: Recall from brain before scheduled agent invocation ──
    const RECALL_SCORE_FLOOR: f64 = 0.7;
    const RECALL_TOP_K: usize = 3;

    if let Some(ref brain_handle) = brain {
        let recognition_ctx = spectral::graph::RecognitionContext::empty()
            .with_persona(crate::config::agent_identity::DEFAULT_PERSONA_KEY)
            .with_session(session.id.clone());
        match brain_handle
            .recall_cascade(prompt_text, &recognition_ctx)
            .await
        {
            Ok(result) => {
                let top_hits: Vec<_> = result
                    .merged_hits
                    .iter()
                    .filter(|hit| hit.signal_score >= RECALL_SCORE_FLOOR)
                    .take(RECALL_TOP_K)
                    .collect();

                if !top_hits.is_empty() {
                    let mut prefix = String::from("Relevant memories from past context:\n");
                    for hit in &top_hits {
                        prefix.push_str(&format!("- {}\n", hit.content));
                    }

                    tracing::info!(
                        target: "permagentd::brain",
                        "Recall injected {} memories into system prompt for scheduled job: {}",
                        top_hits.len(),
                        job_id
                    );

                    agent
                        .extend_system_prompt("memory_recall".to_string(), prefix)
                        .await;
                } else {
                    tracing::debug!(
                        target: "permagentd::brain",
                        "Recall returned no hits above {} threshold for scheduled job: {}",
                        RECALL_SCORE_FLOOR,
                        job_id
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::brain",
                    "Brain recall failed for scheduled job {}: {}",
                    job_id,
                    e
                );
            }
        }
    }

    let session_config = SessionConfig {
        id: session.id.clone(),
        schedule_id: Some(job.id.clone()),
        max_turns: None,
        retry_config: None,
    };

    let stream = agent
        .reply(user_message, session_config, Some(cancel_token))
        .await?;

    use futures::StreamExt;
    let mut stream = std::pin::pin!(stream);

    let mut stream_error = false;
    while let Some(message_result) = stream.next().await {
        tokio::task::yield_now().await;

        match message_result {
            Ok(AgentEvent::Message(msg)) => {
                conversation.push(msg);
            }
            Ok(AgentEvent::HistoryReplaced(updated)) => {
                conversation = updated;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Error in agent stream: {}", e);
                stream_error = true;
                break;
            }
        }
    }

    // ── Phase 4: Remember scheduled turn after response completes ──
    if let Some(ref brain_handle) = brain {
        let user_text = prompt_text.to_string();
        let assistant_text = conversation
            .messages()
            .iter()
            .rev()
            .find(|m| m.role == rmcp::model::Role::Assistant)
            .map(|m| m.as_concat_text())
            .unwrap_or_default();
        let turn_idx = conversation.len();

        if !user_text.is_empty() && !assistant_text.is_empty() {
            let brain_clone = brain_handle.clone();
            let remember_job_id = job_id.clone();

            tokio::spawn(async move {
                let key = format!("scheduled-{}-{}", remember_job_id, turn_idx);
                let content = format!("User: {}\nAssistant: {}", user_text, assistant_text);
                let device_id = *brain_clone.device_id();
                let key_for_log = key.clone();

                match brain_clone
                    .remember_with(
                        &key,
                        &content,
                        spectral::RememberOpts {
                            source: Some("scheduled".into()),
                            device_id: Some(device_id),
                            confidence: Some(1.0),
                            visibility: spectral::Visibility::Private,
                            wing: None,
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            target: "permagentd::brain",
                            "Remembered scheduled turn: {}",
                            key_for_log
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Failed to remember scheduled turn {}: {}",
                            key_for_log,
                            e
                        );
                    }
                }
            });
        }
    }

    // ── Phase 5: Extract structured findings from agent output ──
    // If the agent's response contains a <findings>[...]</findings> block,
    // parse it and store as actionable findings for the Automate tab UI.
    {
        let full_output = conversation
            .messages()
            .iter()
            .filter(|m| m.role == rmcp::model::Role::Assistant)
            .map(|m| m.as_concat_text())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(start) = full_output.find("<findings>") {
            if let Some(end) = full_output.find("</findings>") {
                let json_str = full_output
                    .get(start + "<findings>".len()..end)
                    .unwrap_or("")
                    .trim();
                match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                    Ok(findings) => {
                        let findings_dir = std::env::var("HOME")
                            .map(|h| {
                                std::path::PathBuf::from(h).join(".permagent/automation/findings")
                            })
                            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/findings"));
                        let _ = std::fs::create_dir_all(&findings_dir);
                        let findings_path = findings_dir.join(format!("{}.json", session.id));
                        let findings_data = serde_json::json!({
                            "run_id": session.id,
                            "findings": findings,
                        });
                        let _ = std::fs::write(
                            &findings_path,
                            serde_json::to_string_pretty(&findings_data).unwrap_or_default(),
                        );
                        tracing::info!(
                            target: "permagentd::automation",
                            "Extracted {} findings from scheduled job {} (session {})",
                            findings.len(), job_id, session.id
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::automation",
                            "Failed to parse findings JSON from job {}: {}",
                            job_id, e
                        );
                    }
                }
            }
        }
    }

    agent
        .config
        .session_manager
        .update(&session.id)
        .schedule_id(Some(job.id.clone()))
        .recipe(Some(recipe))
        .apply()
        .await?;

    {
        let session_duration = start_time.elapsed();
        let exit_type = if stream_error { "error" } else { "normal" };
        let (total_tokens, message_count) = agent
            .config
            .session_manager
            .get_session(&session.id, false)
            .await
            .map(|s| (s.total_tokens.unwrap_or(0), s.message_count))
            .unwrap_or((0, 0));

        tracing::info!(
            monotonic_counter.goose.session_completions = 1,
            session_type = "schedule",
            interface = "scheduler",
            exit_type,
            duration_ms = session_duration.as_millis() as u64,
            total_tokens,
            message_count,
            "Session completed"
        );

        tracing::info!(
            monotonic_counter.goose.session_duration_ms = session_duration.as_millis() as u64,
            session_type = "schedule",
            interface = "scheduler",
            "Session duration"
        );

        if total_tokens > 0 {
            tracing::info!(
                monotonic_counter.goose.session_tokens = total_tokens,
                session_type = "schedule",
                interface = "scheduler",
                "Session tokens"
            );
        }
    }

    #[cfg(feature = "telemetry")]
    {
        let duration_secs = start_time.elapsed().as_secs();
        tokio::spawn(async move {
            let mut props = HashMap::new();
            props.insert(
                "trigger".to_string(),
                serde_json::Value::String("automated".to_string()),
            );
            props.insert(
                "status".to_string(),
                serde_json::Value::String("completed".to_string()),
            );
            props.insert(
                "duration_seconds".to_string(),
                serde_json::Value::Number(serde_json::Number::from(duration_secs)),
            );
            if let Err(e) = posthog::emit_event("schedule_job_completed", props).await {
                tracing::debug!("Failed to send schedule telemetry: {}", e);
            }
        });
    }

    Ok(session.id)
}

#[async_trait]
impl SchedulerTrait for Scheduler {
    async fn set_brain(&self, brain: Option<crate::brain_handle::SafeBrain>) {
        let mut guard = self.brain.write().await;
        *guard = brain;
    }

    async fn set_persona(&self, persona: crate::config::agent_identity::SharedPersona) {
        let mut guard = self.persona.write().await;
        *guard = Some(persona);
    }

    async fn set_agent_config(&self, config: crate::config::agent_identity::SharedAgentConfig) {
        let mut guard = self.agent_config.write().await;
        *guard = Some(config);
    }

    async fn add_scheduled_job(
        &self,
        job: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        self.add_scheduled_job(job, make_copy).await
    }

    async fn schedule_recipe(
        &self,
        recipe_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        self.schedule_recipe(recipe_path, cron_schedule).await
    }

    async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.list_scheduled_jobs().await
    }

    async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_recipe: bool,
    ) -> Result<(), SchedulerError> {
        self.remove_scheduled_job(id, remove_recipe).await
    }

    async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.pause_schedule(id).await
    }

    async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.unpause_schedule(id).await
    }

    async fn run_now(&self, id: &str) -> Result<String, SchedulerError> {
        self.run_now(id).await
    }

    async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        self.sessions(sched_id, limit).await
    }

    async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        self.update_schedule(sched_id, new_cron).await
    }

    async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        self.kill_running_job(sched_id).await
    }

    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        self.get_running_job_info(sched_id).await
    }

    async fn update_starter_fields(
        &self,
        sched_id: &str,
        starter_id: Option<String>,
        version: Option<String>,
        hash: Option<String>,
        user_customized: bool,
    ) {
        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((_, job)) = jobs_guard.get_mut(sched_id) {
                if let Some(sid) = starter_id {
                    job.starter_id = Some(sid);
                }
                if let Some(v) = version {
                    job.starter_version = Some(v);
                }
                if let Some(h) = hash {
                    job.starter_content_hash = Some(h);
                }
                job.user_customized = Some(user_customized);
            }
        }
        if let Err(e) = persist_jobs(&self.storage_path, &self.jobs).await {
            tracing::error!("Failed to persist starter field update: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    fn create_test_recipe(dir: &Path, name: &str) -> PathBuf {
        let recipe_path = dir.join(format!("{}.yaml", name));
        fs::write(&recipe_path, "prompt: test\n").unwrap();
        recipe_path
    }

    #[tokio::test]
    async fn test_job_runs_on_schedule() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "scheduled_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "scheduled_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(jobs[0].last_run.is_some(), "Job should have run");
    }

    #[tokio::test]
    async fn test_paused_job_does_not_run() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let recipe_path = create_test_recipe(temp_dir.path(), "paused_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "paused_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        scheduler.pause_schedule("paused_job").await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(jobs[0].last_run.is_none(), "Paused job should not run");
    }

    #[tokio::test]
    async fn test_job_with_no_prompt_does_not_panic() {
        let _guard = env_lock::lock_env([
            ("GOOSE_PROVIDER", Some("openai")),
            ("GOOSE_MODEL", Some("gpt-4o")),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
            ("OPENAI_CUSTOM_HEADERS", Some("")),
        ]);
        let temp_dir = tempdir().unwrap();
        let recipe_path = temp_dir.path().join("no_prompt.yaml");
        fs::write(
            &recipe_path,
            "title: missing\ndescription: no prompt or instructions\n",
        )
        .unwrap();

        let storage_path = temp_dir.path().join("schedule.json");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "no_prompt_job".to_string(),
            source: recipe_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: None,
            starter_version: None,
            starter_content_hash: None,
            user_customized: None,
        };

        // Schedule the job and let it run — should not panic
        scheduler.add_scheduled_job(job, true).await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        // The job should have attempted to run (last_run set) but not crashed the scheduler
        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(
            jobs[0].last_run.is_some(),
            "Job should have attempted to run without panicking"
        );
    }
}
