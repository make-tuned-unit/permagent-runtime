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
const MODEL_REVIEW_YAML: &str = include_str!("model_review.yaml");
const STORAGE_INSIGHTS_YAML: &str = include_str!("storage_insights.yaml");
const STEWARD_YAML: &str = include_str!("steward.yaml");
const HEALTH_REVIEW_YAML: &str = include_str!("health_review.yaml");
const HEALTH_REVIEW_EVIDENCE_PY: &str = include_str!("health_review_evidence.py");

struct StarterAsset {
    filename: &'static str,
    content: &'static str,
}

const NO_ASSETS: &[StarterAsset] = &[];
const HEALTH_REVIEW_ASSETS: &[StarterAsset] = &[StarterAsset {
    filename: "health_review_evidence.py",
    content: HEALTH_REVIEW_EVIDENCE_PY,
}];

struct StarterRecipe {
    id: &'static str,
    cron: &'static str,
    yaml: &'static str,
    /// Worker persona that OWNS this job, when it is another agent's embodiment
    /// rather than Henry's own automation. `None` means Henry owns it.
    ///
    /// This is what keeps Henry's HUD honest: it reports only jobs it owns, so
    /// the Steward's cron is no longer advertised as Henry's next scheduled run.
    owner: Option<&'static str>,
    /// Deterministic helpers that live beside the YAML and are addressed via
    /// `{{ recipe_dir }}`. These are implementation dependencies, not recipe
    /// customizations, so reconciliation keeps them at the embedded version.
    assets: &'static [StarterAsset],
}

// Crons are stored in 6-field (seconds-prefixed) form so the scheduler never
// has to convert a legacy 5-field expression at load time — see
// `normalize_cron_to_6field` in scheduler.rs, which migrates any pre-existing
// 5-field jobs once.
const STARTERS: &[StarterRecipe] = &[
    StarterRecipe {
        id: "model_review",
        cron: "0 30 19 * * 0",
        yaml: MODEL_REVIEW_YAML,
        owner: None,
        assets: NO_ASSETS,
    },
    StarterRecipe {
        id: "workspace-snapshot",
        cron: "0 0 8 * * 1-5",
        yaml: WORKSPACE_SNAPSHOT_YAML,
        owner: None,
        assets: NO_ASSETS,
    },
    StarterRecipe {
        id: "storage-insights",
        cron: "0 0 19 * * 0",
        yaml: STORAGE_INSIGHTS_YAML,
        owner: None,
        assets: NO_ASSETS,
    },
    // The Steward's embodiment: crate::steward's docs call this recipe the
    // Steward itself, so the job belongs to the Steward, not to Henry.
    StarterRecipe {
        id: "git-steward",
        cron: "0 0 6 * * 1-5",
        yaml: STEWARD_YAML,
        owner: Some("steward"),
        assets: NO_ASSETS,
    },
    // 03:20 daily: after the day's work is in the log and before the morning
    // jobs (workspace-snapshot 08:00, git-steward 06:00) add noise of their
    // own. Off-hour so a long log sweep never competes with a live session.
    //
    // Reviews the daemon's own logs for what broke, what was slow, and what
    // the user worked around without filing anything. Read-only, and it files
    // no Decision-Inbox items — it reports, the user decides.
    StarterRecipe {
        id: "health-review",
        cron: "0 20 3 * * *",
        yaml: HEALTH_REVIEW_YAML,
        owner: None,
        assets: HEALTH_REVIEW_ASSETS,
    },
];

/// Compute SHA-256 hex digest of content.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract the version field from a recipe YAML string.
fn extract_version(yaml: &str) -> String {
    serde_yaml::from_str::<Recipe>(yaml)
        .map(|r| r.version)
        .unwrap_or_else(|_| "1.0.0".to_string())
}

fn sync_starter_assets(
    starter: &StarterRecipe,
    recipe_path: &std::path::Path,
) -> Result<(), String> {
    let recipe_dir = recipe_path
        .parent()
        .ok_or_else(|| format!("recipe path {} has no parent", recipe_path.display()))?;
    for asset in starter.assets {
        let path = recipe_dir.join(asset.filename);
        std::fs::write(&path, asset.content)
            .map_err(|e| format!("write starter asset {}: {}", path.display(), e))?;
    }
    Ok(())
}

fn disabled_starters_path() -> PathBuf {
    permagent::config::paths::Paths::in_state_dir("automation/disabled_starters.json")
}

fn load_disabled_starters() -> Vec<String> {
    load_disabled_starters_at(&disabled_starters_path())
}

fn load_disabled_starters_at(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Record a deleted starter in the tombstone file so it is not re-seeded on
/// the next boot. The write is atomic (temp + rename) and failures propagate:
/// a swallowed error here meant a deleted starter automation silently
/// resurrected on reboot (bug-sweep wave 1).
pub fn record_starter_deletion(starter_id: &str) -> Result<(), String> {
    record_starter_deletion_at(starter_id, &disabled_starters_path())
}

fn record_starter_deletion_at(starter_id: &str, path: &std::path::Path) -> Result<(), String> {
    let mut disabled = load_disabled_starters_at(path);
    if disabled.contains(&starter_id.to_string()) {
        return Ok(());
    }
    disabled.push(starter_id.to_string());
    let parent = path
        .parent()
        .ok_or_else(|| format!("tombstone path {} has no parent", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("create dir {}: {}", parent.display(), e))?;
    let json = serde_json::to_string_pretty(&disabled)
        .map_err(|e| format!("serialize disabled starters: {}", e))?;
    let tmp = parent.join(".disabled_starters.json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename tombstone into place {}: {}", path.display(), e)
    })?;
    Ok(())
}

/// Install new starter recipes paused for user review, then reconcile existing
/// starters without changing the user's enabled or paused choice.
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
        if let Err(e) = sync_starter_assets(starter, &recipe_path) {
            tracing::error!("Failed to write assets for starter '{}': {}", starter.id, e);
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
            paused: true,
            requires_approval: true,
            current_session_id: None,
            process_start_time: None,
            worker_persona: starter.owner.map(str::to_string),
            starter_id: Some(starter.id.to_string()),
            starter_version: Some(version),
            starter_content_hash: Some(hash),
            user_customized: Some(false),
            ..Default::default()
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

        // Assets are runtime dependencies rather than editable recipe text.
        // Sync them even when the YAML is current or user-customized so a
        // newly bundled collector appears on the very next daemon start.
        if let Err(e) = sync_starter_assets(starter, std::path::Path::new(&job.source)) {
            tracing::error!(
                "Failed to reconcile assets for starter '{}': {}",
                starter.id,
                e
            );
            continue;
        }

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

        // ── Pass 0: a recipe the scheduler cannot run is not a user edit ──
        // A hash mismatch alone was read as "the user edited it". When a test
        // run overwrote git-steward.yaml with a 13-byte `prompt: test`, that
        // rule marked the wreck user_customized and preserved it forever while
        // every fire failed on "missing field title". An edit that does not
        // parse cannot be run, so it is set aside and the embedded version
        // reinstalled — even over an earlier user_customized flag.
        if let Some(reason) = unrunnable_recipe_reason(&on_disk_content) {
            let backup = format!("{}.unparseable", job.source);
            if let Err(e) = std::fs::write(&backup, &on_disk_content) {
                tracing::error!(
                    "Starter '{}' on disk is not a runnable recipe ({}), and setting it \
                     aside failed: {}; leaving it in place",
                    starter.id,
                    reason,
                    e
                );
                continue;
            }
            if let Err(e) = std::fs::write(&job.source, starter.yaml) {
                tracing::error!("Failed to reinstall starter '{}': {}", starter.id, e);
                continue;
            }
            tracing::warn!(
                target: "permagentd::automation",
                "Starter '{}' on disk is not a runnable recipe ({}); reinstalled the \
                 embedded v{} (previous content kept at {})",
                starter.id,
                reason,
                embedded_version,
                backup
            );
            scheduler
                .update_starter_fields(
                    &job.id,
                    starter_id_val,
                    Some(embedded_version),
                    Some(embedded_hash),
                    false,
                )
                .await;
            continue;
        }

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

/// Why `content` cannot run as a scheduled recipe, or `None` if it can.
///
/// Parsed exactly as `execute_job` parses a `.yaml` source, so "runnable here"
/// means "runnable there". A recipe with a blank title parses but is not a
/// starter anyone wrote on purpose.
fn unrunnable_recipe_reason(content: &str) -> Option<String> {
    match serde_yaml::from_str::<Recipe>(content) {
        Ok(recipe) if recipe.title.trim().is_empty() => Some("empty title".to_string()),
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
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
    sync_starter_assets(starter, std::path::Path::new(&job.source))?;

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

    #[test]
    fn all_starters_parse_as_recipes() {
        for s in STARTERS {
            let recipe: Recipe = serde_yaml::from_str(s.yaml)
                .unwrap_or_else(|e| panic!("starter '{}' failed to parse: {e}", s.id));
            assert!(!recipe.title.is_empty(), "{} has empty title", s.id);
        }
    }

    /// CASE B fix #2: this recipe mutates (writes the evidence JSON + report),
    /// so `ReviewerMandate` fires on it — but before this fix it declared
    /// only `developer` (auto-widened to `[developer, analyze]` by
    /// `ensure_analyze_for_developer`), never `summon`, so the `delegate` tool
    /// the mandate asks for was never loaded and the mandate degraded to a
    /// suggestion the recipe could not act on.
    #[test]
    fn health_review_loads_with_the_summon_extension() {
        let recipe =
            Recipe::from_content(HEALTH_REVIEW_YAML).expect("health review starter must parse");
        let names: Vec<String> = recipe
            .extensions
            .as_ref()
            .map(|exts| exts.iter().map(|e| e.name()).collect())
            .unwrap_or_default();
        assert!(
            names.iter().any(|n| n == "summon"),
            "the reviewer mandate needs the `delegate` tool, which only `summon` provides — \
             loaded extensions were {names:?}"
        );
    }

    #[test]
    fn health_review_bundles_its_evidence_collector() {
        let starter = STARTERS
            .iter()
            .find(|starter| starter.id == "health-review")
            .expect("health review starter");
        let tmp = TempDir::new().unwrap();
        let yaml = tmp.path().join("health-review.yaml");
        std::fs::write(&yaml, starter.yaml).unwrap();
        sync_starter_assets(starter, &yaml).unwrap();
        let installed = std::fs::read_to_string(tmp.path().join("health_review_evidence.py"))
            .expect("collector installed beside recipe");
        assert_eq!(installed, HEALTH_REVIEW_EVIDENCE_PY);
        assert!(starter
            .yaml
            .contains("{{ recipe_dir }}/health_review_evidence.py"));
    }

    // ── record_starter_deletion tombstone (bug-sweep wave 1) ────────────────

    #[test]
    fn record_starter_deletion_persists_and_dedupes() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("automation/disabled_starters.json");

        record_starter_deletion_at("storage-insights", &path).unwrap();
        assert_eq!(
            load_disabled_starters_at(&path),
            vec!["storage-insights".to_string()]
        );

        // Duplicate is a no-op, second id appends; no temp artifact remains.
        record_starter_deletion_at("storage-insights", &path).unwrap();
        record_starter_deletion_at("git-steward", &path).unwrap();
        assert_eq!(
            load_disabled_starters_at(&path),
            vec!["storage-insights".to_string(), "git-steward".to_string()]
        );
        let dir = path.clone();
        let dir = dir.parent().unwrap();
        assert!(!dir.join(".disabled_starters.json.tmp").exists());
    }

    #[test]
    #[cfg(unix)]
    fn record_starter_deletion_surfaces_write_failure() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("automation/disabled_starters.json");

        // Pre-create the automation dir read-only so the tombstone write fails.
        let dir = path.clone();
        let dir = dir.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = record_starter_deletion_at("storage-insights", &path)
            .expect_err("read-only tombstone dir must surface as an error, not a silent drop");
        assert!(err.contains("disabled_starters"), "{}", err);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Nothing was recorded — the caller keeps the schedule and can retry.
        assert!(load_disabled_starters_at(&path).is_empty());
    }

    #[test]
    fn steward_recipe_wires_developer_and_steward_extensions() {
        use permagent::agents::extension::ExtensionConfig;
        let s = STARTERS
            .iter()
            .find(|s| s.id == "git-steward")
            .expect("git-steward starter registered");
        let recipe: Recipe = serde_yaml::from_str(s.yaml).unwrap();
        let names: Vec<String> = recipe
            .extensions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|e| match e {
                ExtensionConfig::Builtin { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert!(
            names.iter().any(|n| n == "developer"),
            "developer ext present: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "steward"),
            "steward (safety gate) ext present: {names:?}"
        );
    }

    /// Minimal valid Recipe YAML at v1.0.0 (different from embedded v2.0.0).
    const OLD_STORAGE_YAML: &str = concat!(
        "version: \"1.0.0\"\n",
        "title: \"Weekly Storage Cleanup\"\n",
        "description: \"Old version.\"\n",
        "prompt: \"Run storage scan.\"\n",
    );

    struct MockScheduler {
        jobs: Mutex<Vec<ScheduledJob>>,
        #[allow(clippy::type_complexity)]
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
            ..Default::default()
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
        // Initialize the process-lifetime test root before overriding the env.
        // Otherwise a concurrent test can initialize it while this guard is
        // held and redirect later starter writes away from this tempdir.
        let _ = crate::test_support::test_root();
        let tmp = TempDir::new().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);

        let scheduler = MockScheduler::new(vec![]);

        seed_starter_recipes(&scheduler).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert_eq!(jobs.len(), STARTERS.len(), "all fresh starters should seed");
        for starter in STARTERS {
            let job = jobs.iter().find(|job| job.id == starter.id).unwrap();
            assert!(job.paused, "{} must await review", starter.id);
            assert!(
                job.requires_approval,
                "{} must require approval",
                starter.id
            );
        }
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

    #[tokio::test]
    async fn reseed_preserves_existing_enabled_and_paused_choices() {
        let _ = crate::test_support::test_root();
        let tmp = TempDir::new().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);
        let recipes_dir = tmp.path().join("scheduled_recipes");
        std::fs::create_dir_all(&recipes_dir).unwrap();

        let enabled = STARTERS
            .iter()
            .find(|starter| starter.id == "model_review")
            .unwrap();
        let paused = STARTERS
            .iter()
            .find(|starter| starter.id == "storage-insights")
            .unwrap();
        let existing_job = |starter: &StarterRecipe, is_paused: bool| {
            let path = recipes_dir.join(format!("{}.yaml", starter.id));
            std::fs::write(&path, starter.yaml).unwrap();
            let mut job = make_job(
                starter.id,
                path.to_str().unwrap(),
                &extract_version(starter.yaml),
                &content_hash(starter.yaml),
                false,
            );
            job.paused = is_paused;
            job.requires_approval = false;
            job
        };
        let scheduler = MockScheduler::new(vec![
            existing_job(enabled, false),
            existing_job(paused, true),
        ]);

        for _ in 0..2 {
            seed_starter_recipes(&scheduler).await;
            let jobs = scheduler.list_scheduled_jobs().await;
            assert_eq!(jobs.len(), STARTERS.len(), "reseed must not duplicate jobs");
            let enabled_job = jobs.iter().find(|job| job.id == enabled.id).unwrap();
            assert!(!enabled_job.paused, "existing enabled job stays enabled");
            assert!(!enabled_job.requires_approval);
            let paused_job = jobs.iter().find(|job| job.id == paused.id).unwrap();
            assert!(
                paused_job.paused,
                "existing manually paused job stays paused"
            );
            assert!(
                !paused_job.requires_approval,
                "manual pause is not approval-pending"
            );
            for starter in STARTERS
                .iter()
                .filter(|starter| starter.id != enabled.id && starter.id != paused.id)
            {
                let new_job = jobs.iter().find(|job| job.id == starter.id).unwrap();
                assert!(new_job.paused, "{} remains held", starter.id);
                assert!(
                    new_job.requires_approval,
                    "{} remains approval-pending",
                    starter.id
                );
            }
        }
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
    // ── Test 9: an unparseable on-disk starter is reinstalled, not preserved ──

    /// The git-steward incident: a test fixture (`prompt: test`) overwrote the
    /// live recipe. Reconcile read the hash mismatch as a user edit and froze
    /// the wreck as user_customized. It must reinstall the embedded recipe —
    /// on first detection AND when the flag was already (wrongly) set — and
    /// keep the broken content beside it.
    #[tokio::test]
    async fn unparseable_starter_is_reinstalled_not_preserved_as_an_edit() {
        for already_customized in [false, true] {
            let tmp = TempDir::new().unwrap();
            let yaml_path = tmp.path().join("git-steward.yaml");
            std::fs::write(&yaml_path, "prompt: test\n").unwrap();

            let mut job = make_job(
                "git-steward",
                yaml_path.to_str().unwrap(),
                &extract_version(STEWARD_YAML),
                &content_hash(STEWARD_YAML),
                already_customized,
            );
            job.paused = true;
            job.consecutive_failures = 3;
            let scheduler = MockScheduler::new(vec![job]);

            reconcile_starter_recipes(&scheduler).await;

            assert_eq!(
                std::fs::read_to_string(&yaml_path).unwrap(),
                STEWARD_YAML,
                "customized={already_customized}: the embedded recipe must be reinstalled"
            );
            let backup = tmp.path().join("git-steward.yaml.unparseable");
            assert_eq!(std::fs::read_to_string(backup).unwrap(), "prompt: test\n");

            let jobs = scheduler.list_scheduled_jobs().await;
            let updated = jobs.iter().find(|j| j.id == "git-steward").unwrap();
            assert_eq!(updated.user_customized, Some(false));
            assert_eq!(
                updated.starter_content_hash.as_deref(),
                Some(content_hash(STEWARD_YAML).as_str())
            );
            // Reinstalling fixes the recipe; resuming is still the user's call.
            assert!(updated.paused);
        }
    }

    #[test]
    fn unrunnable_recipe_reason_names_the_failure() {
        let missing_title = unrunnable_recipe_reason("prompt: test\n").unwrap();
        assert!(missing_title.contains("title"), "{missing_title}");
        assert!(unrunnable_recipe_reason(
            "version: \"1\"\ntitle: \"  \"\ndescription: d\nprompt: p\n"
        )
        .is_some());
        for s in STARTERS {
            assert_eq!(unrunnable_recipe_reason(s.yaml), None, "{}", s.id);
        }
    }

    #[tokio::test]
    async fn model_review_preserves_pause_and_customization() {
        // Tombstones and starter files both follow the isolated path root.
        let _ = crate::test_support::test_root();
        let tmp = TempDir::new().unwrap();
        let _guard =
            env_lock::lock_env([("PERMAGENT_PATH_ROOT", Some(tmp.path().to_str().unwrap()))]);
        let path = tmp.path().join("model_review.yaml");
        let edited = format!("{}# custom note\n", MODEL_REVIEW_YAML);
        std::fs::write(&path, &edited).unwrap();
        let mut job = make_job(
            "model_review",
            path.to_str().unwrap(),
            "1.0.0",
            &content_hash(MODEL_REVIEW_YAML),
            false,
        );
        job.paused = true;
        job.cron = "0 0 12 * * 1".into();
        let scheduler = MockScheduler::new(vec![job]);
        reconcile_starter_recipes(&scheduler).await;
        let jobs = scheduler.list_scheduled_jobs().await;
        let job = jobs.iter().find(|j| j.id == "model_review").unwrap();
        assert!(job.paused);
        assert_eq!(job.cron, "0 0 12 * * 1");
        assert_eq!(job.user_customized, Some(true));
        assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
        let starter = STARTERS.iter().find(|s| s.id == "model_review").unwrap();
        assert_eq!(starter.cron, "0 30 19 * * 0");
        assert_eq!(ScheduledJob::default().tz, None);
    }

    /// Render a starter's prompt the way the scheduler does: every declared
    /// default, plus the two built-ins.
    fn render_starter_prompt(yaml: &str, overrides: &[(&str, &str)]) -> String {
        let recipe = Recipe::from_content(yaml).expect("starter parses");
        let mut params: std::collections::HashMap<String, String> = recipe
            .parameters
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|p| (p.key.clone(), p.default.clone().unwrap_or_default()))
            .collect();
        params.insert("recipe_dir".into(), "/recipes".into());
        params.insert(
            permagent::recipe::BUILT_IN_DEV_ROOTS_PARAM.into(),
            String::new(),
        );
        for (k, v) in overrides {
            params.insert((*k).into(), (*v).into());
        }
        permagent::recipe::template_recipe::render_recipe_content_with_params(
            recipe.prompt.as_deref().expect("starter has a prompt"),
            &params,
        )
        .expect("starter prompt renders")
    }

    /// With no code folder known, the steward says so and stops rather than
    /// sweeping a guessed path; with folders known it sweeps them; an explicit
    /// repos_root still wins.
    #[test]
    fn steward_resolves_roots_from_dev_roots_or_asks() {
        let unknown = render_starter_prompt(STEWARD_YAML, &[]);
        assert!(unknown.contains("No code folders are set"), "{unknown}");
        assert!(
            !unknown.contains("{{") && !unknown.contains("{%"),
            "{unknown}"
        );

        let known = render_starter_prompt(STEWARD_YAML, &[("dev_roots", "/a/code, /b/src")]);
        assert!(
            known.contains("user's code folders: /a/code, /b/src."),
            "{known}"
        );
        assert!(!known.contains("No code folders are set"));
        assert!(known.contains(r#"repo_path: "/a/code, /b/src""#), "{known}");

        let explicit = render_starter_prompt(
            STEWARD_YAML,
            &[("dev_roots", "/a/code"), ("repos_root", "/x/repos")],
        );
        assert!(explicit.contains("sweep /x/repos."));
        assert!(explicit.contains(r#"repo_path: "/x/repos""#));
    }

    #[test]
    fn health_review_names_the_checkout_through_dev_roots() {
        let rendered = render_starter_prompt(HEALTH_REVIEW_YAML, &[("dev_roots", "/a/code")]);
        assert!(
            rendered.contains("user's code folders — /a/code —"),
            "{rendered}"
        );
    }
}
