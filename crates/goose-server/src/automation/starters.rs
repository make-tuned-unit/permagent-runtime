//! Starter recipe auto-installation and upgrade reconciliation.
//!
//! On first daemon startup, installs pre-built recipes into the scheduler
//! so the Automate tab has working content from day one. On subsequent
//! startups, reconciles embedded recipe content against on-disk copies,
//! silently upgrading pristine starters and preserving user edits.

use permagent::events::activity;
use permagent::recipe::Recipe;
use permagent::scheduler::{get_default_scheduled_recipes_dir, ScheduledJob, SchedulerError};
use permagent::scheduler_trait::SchedulerTrait;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const WORKSPACE_SNAPSHOT_YAML: &str = include_str!("workspace_snapshot.yaml");
const STORAGE_INSIGHTS_YAML: &str = include_str!("storage_insights.yaml");

struct StarterRecipe {
    id: &'static str,
    cron: &'static str,
    yaml: &'static str,
}

const STARTERS: &[StarterRecipe] = &[
    StarterRecipe {
        id: "workspace-snapshot",
        cron: "0 8 * * 1-5",
        yaml: WORKSPACE_SNAPSHOT_YAML,
    },
    StarterRecipe {
        id: "storage-insights",
        cron: "0 19 * * 0",
        yaml: STORAGE_INSIGHTS_YAML,
    },
];

/// Compute SHA-256 hex digest of content.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Extract the version field from a recipe YAML string.
fn extract_version(yaml: &str) -> String {
    serde_yaml::from_str::<Recipe>(yaml)
        .map(|r| r.version)
        .unwrap_or_else(|_| "1.0.0".to_string())
}

fn disabled_starters_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".permagent")
        .join("automation")
        .join("disabled_starters.json")
}

fn load_disabled_starters() -> Vec<String> {
    let path = disabled_starters_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub fn record_starter_deletion(starter_id: &str) {
    let path = disabled_starters_path();
    let mut disabled = load_disabled_starters();
    if !disabled.contains(&starter_id.to_string()) {
        disabled.push(starter_id.to_string());
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            &path,
            serde_json::to_string_pretty(&disabled).unwrap_or_default(),
        );
    }
}

/// Install starter recipes that don't already exist and haven't been disabled,
/// then reconcile existing starters against embedded content.
pub async fn seed_starter_recipes(scheduler: &dyn SchedulerTrait) {
    let existing_jobs = scheduler.list_scheduled_jobs().await;
    let existing_ids: Vec<&str> = existing_jobs.iter().map(|j| j.id.as_str()).collect();
    let disabled = load_disabled_starters();

    for starter in STARTERS {
        if existing_ids.contains(&starter.id) {
            tracing::debug!(
                "Starter recipe '{}' already installed, skipping seed",
                starter.id
            );
            continue;
        }
        if disabled.contains(&starter.id.to_string()) {
            tracing::debug!(
                "Starter recipe '{}' was disabled by user, skipping",
                starter.id
            );
            continue;
        }

        let recipe: Recipe = match serde_yaml::from_str(starter.yaml) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to parse starter recipe '{}': {}", starter.id, e);
                continue;
            }
        };

        let recipes_dir = match get_default_scheduled_recipes_dir() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to get recipes dir: {}", e);
                continue;
            }
        };

        let recipe_path = recipes_dir.join(format!("{}.yaml", starter.id));
        if let Err(e) = std::fs::write(&recipe_path, starter.yaml) {
            tracing::error!("Failed to write starter recipe '{}': {}", starter.id, e);
            continue;
        }

        let version = extract_version(starter.yaml);
        let hash = content_hash(starter.yaml);

        let job = ScheduledJob {
            id: starter.id.to_string(),
            source: recipe_path.to_string_lossy().into_owned(),
            cron: starter.cron.to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: Some(starter.id.to_string()),
            starter_version: Some(version),
            starter_content_hash: Some(hash),
            user_customized: Some(false),
        };

        match scheduler.add_scheduled_job(job, false).await {
            Ok(()) => {
                tracing::info!(
                    target: "permagentd::automation",
                    "Installed starter recipe: {} ({})",
                    recipe.title,
                    starter.id
                );
            }
            Err(SchedulerError::JobIdExists(_)) => {
                tracing::debug!("Starter '{}' already exists (race), skipping", starter.id);
            }
            Err(e) => {
                tracing::error!("Failed to install starter '{}': {}", starter.id, e);
            }
        }
    }

    // Reconcile existing starters after first-install pass
    reconcile_starter_recipes(scheduler).await;
}

/// Reconcile existing starter recipes against embedded content.
///
/// Two-pass approach:
///   Pass 1 — Detect user edits: if on-disk hash differs from stored hash,
///            mark as user_customized and skip upgrade.
///   Pass 2 — Apply upgrades: if pristine and embedded content differs from
///            on-disk, overwrite with embedded version.
async fn reconcile_starter_recipes(scheduler: &dyn SchedulerTrait) {
    let jobs = scheduler.list_scheduled_jobs().await;

    for starter in STARTERS {
        let job = match jobs.iter().find(|j| j.id == starter.id) {
            Some(j) => j,
            None => continue, // Not installed (disabled or new-install just handled)
        };

        let embedded_version = extract_version(starter.yaml);
        let embedded_hash = content_hash(starter.yaml);

        // Backfill: existing starters without versioning fields get them now
        let stored_hash = job.starter_content_hash.clone();
        let stored_version = job.starter_version.clone();
        let mut is_customized = job.user_customized.unwrap_or(false);

        // Read current on-disk YAML
        let on_disk_content = match std::fs::read_to_string(&job.source) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Cannot read on-disk recipe for starter '{}': {}",
                    starter.id,
                    e
                );
                continue;
            }
        };
        let on_disk_hash = content_hash(&on_disk_content);

        let starter_id_val = Some(starter.id.to_string());

        // ── Pass 1: Detect user edits ──
        if let Some(ref stored) = stored_hash {
            if on_disk_hash != *stored && !is_customized {
                // On-disk content differs from what we last wrote → user edited it
                is_customized = true;
                tracing::info!(
                    target: "permagentd::automation",
                    "Detected manual edit to starter '{}', preserving user version",
                    starter.id
                );
                scheduler
                    .update_starter_fields(
                        &job.id,
                        starter_id_val,
                        job.starter_version.clone(),
                        job.starter_content_hash.clone(),
                        true,
                    )
                    .await;
                continue;
            }
        }

        // ── Backfill starters that predate the versioning system ──
        if stored_hash.is_none() {
            tracing::info!(
                target: "permagentd::automation",
                "Backfilling version metadata for starter '{}'",
                starter.id
            );
        }

        // ── Pass 2: Apply upgrades ──
        if is_customized {
            // Backfill starter_id if missing (migrated rows)
            if job.starter_id.is_none() {
                scheduler
                    .update_starter_fields(
                        &job.id,
                        starter_id_val,
                        job.starter_version.clone(),
                        job.starter_content_hash.clone(),
                        true,
                    )
                    .await;
            }
            // Log if an update is available
            if embedded_hash != on_disk_hash {
                tracing::info!(
                    target: "permagentd::automation",
                    "Update available for customized starter '{}' (embedded v{} != installed)",
                    starter.id,
                    embedded_version
                );
            }
            continue;
        }

        if embedded_hash == on_disk_hash
            && stored_version.as_deref() == Some(embedded_version.as_str())
        {
            // Content and version match — no-op
            // Still ensure metadata fields are populated (backfill path)
            if stored_hash.is_none() || job.starter_id.is_none() {
                scheduler
                    .update_starter_fields(
                        &job.id,
                        starter_id_val,
                        Some(embedded_version),
                        Some(embedded_hash),
                        false,
                    )
                    .await;
            }
            continue;
        }

        // Pristine + content or version changed → UPGRADE
        let old_version = stored_version.as_deref().unwrap_or("1.0.0").to_string();

        // Write new embedded content to disk
        if let Err(e) = std::fs::write(&job.source, starter.yaml) {
            tracing::error!("Failed to write upgraded starter '{}': {}", starter.id, e);
            continue;
        }

        // Update scheduler metadata
        scheduler
            .update_starter_fields(
                &job.id,
                starter_id_val,
                Some(embedded_version.clone()),
                Some(embedded_hash.clone()),
                false,
            )
            .await;

        tracing::info!(
            target: "permagentd::automation",
            "Upgraded starter '{}' from version {} to {}",
            starter.id,
            old_version,
            embedded_version
        );

        // Emit activity event for UI toast
        activity::emit_activity(activity::starter_recipe_upgraded(
            starter.id,
            &old_version,
            &embedded_version,
        ));
    }
}

/// Reset a starter recipe to its embedded default.
/// Called when the user clicks "Reset to default" in the UI.
pub async fn reset_starter_to_default(
    scheduler: &dyn SchedulerTrait,
    starter_id: &str,
) -> Result<(), String> {
    let starter = STARTERS
        .iter()
        .find(|s| s.id == starter_id)
        .ok_or_else(|| format!("Unknown starter: {}", starter_id))?;

    let jobs = scheduler.list_scheduled_jobs().await;
    let job = jobs
        .iter()
        .find(|j| j.id == starter_id)
        .ok_or_else(|| format!("Starter '{}' not installed", starter_id))?;

    // Write embedded YAML to disk
    std::fs::write(&job.source, starter.yaml).map_err(|e| format!("Failed to write: {}", e))?;

    let version = extract_version(starter.yaml);
    let hash = content_hash(starter.yaml);

    scheduler
        .update_starter_fields(
            &job.id,
            Some(starter_id.to_string()),
            Some(version.clone()),
            Some(hash),
            false,
        )
        .await;

    tracing::info!(
        target: "permagentd::automation",
        "Reset starter '{}' to default (v{})",
        starter_id,
        version
    );

    Ok(())
}

/// Get the embedded version for a known starter.
pub fn embedded_starter_version(id: &str) -> Option<String> {
    STARTERS
        .iter()
        .find(|s| s.id == id)
        .map(|s| extract_version(s.yaml))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use permagent::scheduler::{ScheduledJob, SchedulerError};
    use permagent::scheduler_trait::SchedulerTrait;
    use permagent::session::Session;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    /// Minimal valid Recipe YAML at v1.0.0 (different from embedded v2.0.0).
    const OLD_STORAGE_YAML: &str = concat!(
        "version: \"1.0.0\"\n",
        "title: \"Weekly Storage Cleanup\"\n",
        "description: \"Old version.\"\n",
        "prompt: \"Run storage scan.\"\n",
    );

    struct MockScheduler {
        jobs: Mutex<Vec<ScheduledJob>>,
        update_calls: Mutex<Vec<(String, Option<String>, Option<String>, bool)>>,
    }

    impl MockScheduler {
        fn new(jobs: Vec<ScheduledJob>) -> Self {
            Self {
                jobs: Mutex::new(jobs),
                update_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SchedulerTrait for MockScheduler {
        async fn add_scheduled_job(
            &self,
            job: ScheduledJob,
            _copy: bool,
        ) -> Result<(), SchedulerError> {
            let mut jobs = self.jobs.lock().await;
            if jobs.iter().any(|j| j.id == job.id) {
                return Err(SchedulerError::JobIdExists(job.id));
            }
            jobs.push(job);
            Ok(())
        }

        async fn schedule_recipe(
            &self,
            _: PathBuf,
            _: Option<String>,
        ) -> Result<(), SchedulerError> {
            unimplemented!()
        }

        async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
            self.jobs.lock().await.clone()
        }

        async fn remove_scheduled_job(&self, _: &str, _: bool) -> Result<(), SchedulerError> {
            unimplemented!()
        }
        async fn pause_schedule(&self, _: &str) -> Result<(), SchedulerError> {
            unimplemented!()
        }
        async fn unpause_schedule(&self, _: &str) -> Result<(), SchedulerError> {
            unimplemented!()
        }
        async fn run_now(&self, _: &str) -> Result<String, SchedulerError> {
            unimplemented!()
        }
        async fn sessions(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<(String, Session)>, SchedulerError> {
            unimplemented!()
        }
        async fn update_schedule(&self, _: &str, _: String) -> Result<(), SchedulerError> {
            unimplemented!()
        }
        async fn kill_running_job(&self, _: &str) -> Result<(), SchedulerError> {
            unimplemented!()
        }
        async fn get_running_job_info(
            &self,
            _: &str,
        ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
            unimplemented!()
        }

        async fn update_starter_fields(
            &self,
            sched_id: &str,
            starter_id: Option<String>,
            version: Option<String>,
            hash: Option<String>,
            user_customized: bool,
        ) {
            self.update_calls.lock().await.push((
                sched_id.to_string(),
                version.clone(),
                hash.clone(),
                user_customized,
            ));
            let mut jobs = self.jobs.lock().await;
            if let Some(job) = jobs.iter_mut().find(|j| j.id == sched_id) {
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
    }

    fn make_job(
        id: &str,
        source: &str,
        version: &str,
        hash: &str,
        customized: bool,
    ) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: source.to_string(),
            cron: "0 0 * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            worker_persona: None,
            starter_id: Some(id.to_string()),
            starter_version: Some(version.to_string()),
            starter_content_hash: Some(hash.to_string()),
            user_customized: Some(customized),
        }
    }

    /// The current embedded version of storage-insights (may change across releases).
    fn si_version() -> String {
        extract_version(STORAGE_INSIGHTS_YAML)
    }

    // ── Test 1: pristine starter gets upgraded ──

    #[tokio::test]
    async fn pristine_starter_gets_upgraded() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");
        std::fs::write(&yaml_path, OLD_STORAGE_YAML).unwrap();

        let old_hash = content_hash(OLD_STORAGE_YAML);
        let job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            "1.0.0",
            &old_hash,
            false,
        );
        let scheduler = MockScheduler::new(vec![job]);

        reconcile_starter_recipes(&scheduler).await;

        // On-disk should now be the current embedded version
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, STORAGE_INSIGHTS_YAML);

        // Metadata updated
        let jobs = scheduler.list_scheduled_jobs().await;
        let updated = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(
            updated.starter_version.as_deref(),
            Some(si_version().as_str())
        );
        assert_eq!(
            updated.starter_content_hash.as_deref(),
            Some(content_hash(STORAGE_INSIGHTS_YAML).as_str())
        );
        assert_eq!(updated.user_customized, Some(false));

        // update_starter_fields was called
        let calls = scheduler.update_calls.lock().await;
        assert!(calls
            .iter()
            .any(|(id, _, _, cust)| id == "storage-insights" && !cust));
    }

    // ── Test 2: customized starter is not upgraded ──

    #[tokio::test]
    async fn customized_starter_is_not_upgraded_and_flag_persists() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");

        // Write old YAML but then modify it (simulating user edit)
        let modified_yaml = format!("{}# user comment\n", OLD_STORAGE_YAML);
        std::fs::write(&yaml_path, &modified_yaml).unwrap();

        // Job still has the OLD hash (before user edit)
        let old_hash = content_hash(OLD_STORAGE_YAML);
        let job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            "1.0.0",
            &old_hash,
            false,
        );
        let scheduler = MockScheduler::new(vec![job]);

        reconcile_starter_recipes(&scheduler).await;

        // On-disk should NOT be overwritten — still has user's modification
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, modified_yaml);

        // user_customized set to true
        let jobs = scheduler.list_scheduled_jobs().await;
        let updated = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(updated.user_customized, Some(true));
        assert_eq!(updated.starter_version.as_deref(), Some("1.0.0"));

        // update_starter_fields called with user_customized=true
        let calls = scheduler.update_calls.lock().await;
        assert!(calls
            .iter()
            .any(|(id, _, _, cust)| id == "storage-insights" && *cust));
    }

    // ── Test 3: reset clears customized and restores embedded ──

    #[tokio::test]
    async fn reset_clears_user_customized_and_restores_embedded() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");
        std::fs::write(&yaml_path, "# user-modified content\n").unwrap();

        let job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            "1.0.0",
            &content_hash(OLD_STORAGE_YAML),
            true,
        );
        let scheduler = MockScheduler::new(vec![job]);

        reset_starter_to_default(&scheduler, "storage-insights")
            .await
            .unwrap();

        // On-disk replaced with embedded content
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, STORAGE_INSIGHTS_YAML);

        // Metadata updated
        let jobs = scheduler.list_scheduled_jobs().await;
        let updated = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(
            updated.starter_version.as_deref(),
            Some(si_version().as_str())
        );
        assert_eq!(
            updated.starter_content_hash.as_deref(),
            Some(content_hash(STORAGE_INSIGHTS_YAML).as_str())
        );
        assert_eq!(updated.user_customized, Some(false));
    }

    // ── Test 4: migration is idempotent ──

    #[tokio::test]
    async fn migration_is_idempotent_no_op_on_second_run() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");
        // Already at current embedded version
        std::fs::write(&yaml_path, STORAGE_INSIGHTS_YAML).unwrap();

        let embedded_hash = content_hash(STORAGE_INSIGHTS_YAML);
        let job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            &si_version(),
            &embedded_hash,
            false,
        );
        let scheduler = MockScheduler::new(vec![job]);

        // First run — should be no-op since already current
        reconcile_starter_recipes(&scheduler).await;
        let calls_after_first = scheduler.update_calls.lock().await.len();
        assert_eq!(calls_after_first, 0, "First run should be no-op");

        // Second run — still no-op
        reconcile_starter_recipes(&scheduler).await;
        let calls_after_second = scheduler.update_calls.lock().await.len();
        assert_eq!(calls_after_second, 0, "Second run should also be no-op");
    }

    // ── Test 5: new install seeds with current version and hash ──

    #[tokio::test]
    async fn new_install_seeds_with_current_version_and_hash() {
        let tmp = TempDir::new().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        let scheduler = MockScheduler::new(vec![]);

        seed_starter_recipes(&scheduler).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        let si = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(si.starter_id, Some("storage-insights".to_string()));
        assert_eq!(si.starter_version, Some(si_version()));
        assert_eq!(
            si.starter_content_hash,
            Some(content_hash(STORAGE_INSIGHTS_YAML))
        );
        assert_eq!(si.user_customized, Some(false));

        // Verify on-disk file
        let recipes_dir = tmp.path().join("scheduled_recipes");
        let on_disk = std::fs::read_to_string(recipes_dir.join("storage-insights.yaml")).unwrap();
        assert_eq!(on_disk, STORAGE_INSIGHTS_YAML);
    }

    // ── Test 6: workspace-snapshot no-op when versions match ──

    #[tokio::test]
    async fn workspace_snapshot_no_op_when_versions_match() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("workspace-snapshot.yaml");
        std::fs::write(&yaml_path, WORKSPACE_SNAPSHOT_YAML).unwrap();

        let hash = content_hash(WORKSPACE_SNAPSHOT_YAML);
        let job = make_job(
            "workspace-snapshot",
            yaml_path.to_str().unwrap(),
            "1.0.0",
            &hash,
            false,
        );
        let scheduler = MockScheduler::new(vec![job]);

        reconcile_starter_recipes(&scheduler).await;

        // No upgrade applied
        let calls = scheduler.update_calls.lock().await;
        let ws_calls: Vec<_> = calls
            .iter()
            .filter(|(id, _, _, _)| id == "workspace-snapshot")
            .collect();
        assert!(
            ws_calls.is_empty(),
            "No update calls for workspace-snapshot"
        );

        // On-disk unchanged
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, WORKSPACE_SNAPSHOT_YAML);
    }

    // ── Test 7: user edit between restarts detected ──

    #[tokio::test]
    async fn user_edit_between_restarts_is_detected_and_flag_set() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");
        // Start with current embedded version (pristine, up to date)
        std::fs::write(&yaml_path, STORAGE_INSIGHTS_YAML).unwrap();

        let embedded_hash = content_hash(STORAGE_INSIGHTS_YAML);
        let job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            &si_version(),
            &embedded_hash,
            false,
        );
        let scheduler = MockScheduler::new(vec![job]);

        // First reconcile — no-op (pristine, current)
        reconcile_starter_recipes(&scheduler).await;
        assert_eq!(scheduler.update_calls.lock().await.len(), 0);

        // Simulate user editing the file between restarts
        let edited = format!("{}# my custom rule\n", STORAGE_INSIGHTS_YAML);
        std::fs::write(&yaml_path, &edited).unwrap();

        // Second reconcile — detects the edit
        reconcile_starter_recipes(&scheduler).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        let updated = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(updated.user_customized, Some(true));

        // On-disk NOT overwritten
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, edited);
    }

    // ── Test 8: starter_id is backfilled for migrated rows ──

    #[tokio::test]
    async fn starter_id_is_backfilled_for_migrated_rows() {
        let tmp = TempDir::new().unwrap();
        let yaml_path = tmp.path().join("storage-insights.yaml");
        // User-modified content on disk
        let modified = format!("{}# user tweak\n", STORAGE_INSIGHTS_YAML);
        std::fs::write(&yaml_path, &modified).unwrap();

        // Migrated row: has version metadata but starter_id is None
        let mut job = make_job(
            "storage-insights",
            yaml_path.to_str().unwrap(),
            &si_version(),
            &content_hash(&modified),
            true,
        );
        job.starter_id = None; // simulate pre-backfill migration state

        let scheduler = MockScheduler::new(vec![job]);

        reconcile_starter_recipes(&scheduler).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        let updated = jobs.iter().find(|j| j.id == "storage-insights").unwrap();
        assert_eq!(
            updated.starter_id,
            Some("storage-insights".to_string()),
            "starter_id should be backfilled"
        );
        assert_eq!(
            updated.user_customized,
            Some(true),
            "user_customized preserved"
        );

        // On-disk NOT overwritten
        let on_disk = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(on_disk, modified);
    }
}
