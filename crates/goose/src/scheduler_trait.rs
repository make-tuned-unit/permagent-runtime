use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

use crate::scheduler::{ScheduledJob, SchedulerError};
use crate::session::Session;

#[async_trait]
pub trait SchedulerTrait: Send + Sync {
    /// Set the SafeBrain handle for recall/remember in scheduled jobs.
    /// Default no-op; overridden by Scheduler.
    async fn set_brain(&self, _brain: Option<crate::brain_handle::SafeBrain>) {}
    /// Set the shared persona for scheduled job system prompts.
    async fn set_persona(&self, _persona: crate::config::agent_identity::SharedPersona) {}
    /// Set the shared agent config (primary + workers) for worker persona resolution.
    async fn set_agent_config(&self, _config: crate::config::agent_identity::SharedAgentConfig) {}
    async fn add_scheduled_job(
        &self,
        job: ScheduledJob,
        copy_recipe: bool,
    ) -> Result<(), SchedulerError>;
    async fn schedule_recipe(
        &self,
        recipe_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> anyhow::Result<(), SchedulerError>;
    async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob>;
    async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_recipe: bool,
    ) -> Result<(), SchedulerError>;
    async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError>;
    async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError>;
    async fn run_now(&self, id: &str) -> Result<String, SchedulerError>;
    async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError>;
    async fn update_schedule(&self, sched_id: &str, new_cron: String)
        -> Result<(), SchedulerError>;
    async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError>;
    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError>;

    /// Update starter recipe versioning metadata on a scheduled job.
    async fn update_starter_fields(
        &self,
        _sched_id: &str,
        _starter_id: Option<String>,
        _version: Option<String>,
        _content_hash: Option<String>,
        _user_customized: bool,
    ) {
        // Default no-op for test doubles
    }
}
