//! Starter recipe auto-installation for the Automate tab.
//!
//! On first daemon startup, installs pre-built recipes into the scheduler
//! so the Automate tab has working content from day one. Respects user
//! deletions — if a user removes a starter, it won't be re-installed.

use permagent::recipe::Recipe;
use permagent::scheduler::{get_default_scheduled_recipes_dir, ScheduledJob, SchedulerError};
use permagent::scheduler_trait::SchedulerTrait;
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

/// Install starter recipes that don't already exist and haven't been disabled.
pub async fn seed_starter_recipes(scheduler: &dyn SchedulerTrait) {
    let existing_jobs = scheduler.list_scheduled_jobs().await;
    let existing_ids: Vec<&str> = existing_jobs.iter().map(|j| j.id.as_str()).collect();
    let disabled = load_disabled_starters();

    for starter in STARTERS {
        if existing_ids.contains(&starter.id) {
            tracing::debug!(
                "Starter recipe '{}' already installed, skipping",
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
}
