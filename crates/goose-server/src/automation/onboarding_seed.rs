//! First-run Brain seeding (#298).
//!
//! When onboarding completes, the daemon seeds a small set of welcome /
//! orientation memories into the Brain through the EXISTING `remember_with`
//! write path — no schema migration, and no new public brain-write route (the
//! write happens daemon-internally). Idempotent via a config marker
//! (`onboarding_memories_seeded`); the stable keys also make any re-run
//! harmless.
//!
//! Two call sites, both idempotent:
//! - the `/config` upsert handler, the moment `wizard_complete` flips true
//!   (immediate, same-session), and
//! - daemon startup, gated on `wizard_complete` (covers onboarding that
//!   completed before the Brain was ready, or before this feature shipped).

use permagent::agents::platform_extensions::get_global_brain;
use permagent::config::Config;
use spectral::{RememberOpts, Visibility};

/// Config marker set once the welcome memories have been seeded.
const SEED_MARKER_KEY: &str = "onboarding_memories_seeded";
/// `source` tag recorded on every seeded memory.
const SEED_SOURCE: &str = "permagent.onboarding";
/// Episode every seeded memory is filed under — the first-run seeding itself.
const SEED_EPISODE_ID: &str = "onboarding:first-run";

struct SeedMemory {
    key: &'static str,
    content: &'static str,
}

/// The welcome/orientation memories written on first run.
const SEED_MEMORIES: &[SeedMemory] = &[
    SeedMemory {
        key: "onboarding:welcome",
        content: "Welcome to Permagent. I remember the context of what we work on together — \
                  decisions, the projects that matter to you, and the details in between — so you \
                  don't have to repeat yourself across sessions.",
    },
    SeedMemory {
        key: "onboarding:how-memory-works",
        content: "Permagent's memory (the Brain) captures durable facts and context as you work. \
                  You can ask what I remember, and I draw on it automatically when it's relevant. \
                  It stays on your machine.",
    },
    SeedMemory {
        key: "onboarding:getting-started",
        content: "To get started, just tell me what you're working on — open a workspace, share \
                  files, or ask a question. I'll keep track of the details and surface them when \
                  they help.",
    },
];

fn already_seeded() -> bool {
    Config::global()
        .get_param::<bool>(SEED_MARKER_KEY)
        .unwrap_or(false)
}

/// Seed welcome/orientation memories into the Brain, at most once.
///
/// No-ops if already seeded or if the Brain isn't ready yet — in the latter
/// case the marker is left unset so the next call (startup or a later trigger)
/// retries rather than silently skipping. Callers are responsible for only
/// invoking this once onboarding is complete.
pub async fn seed_onboarding_memories() {
    if already_seeded() {
        return;
    }
    let Some(brain) = get_global_brain() else {
        tracing::debug!("onboarding seed: Brain not ready yet, will retry on next trigger");
        return;
    };

    let mut wrote = 0usize;
    for mem in SEED_MEMORIES {
        let opts = RememberOpts {
            source: Some(SEED_SOURCE.to_string()),
            visibility: Visibility::Private,
            // All three welcome memories are written in one pass, on first run,
            // and are one episode (R45) — the moment the brain was seeded.
            // A constant: the seed runs at most once, and a retry after a
            // partial write rejoins the same episode instead of splitting the
            // welcome set in two.
            episode_id: Some(SEED_EPISODE_ID.to_string()),
            ..Default::default()
        };
        match brain.remember_with(mem.key, mem.content, opts).await {
            Ok(_) => wrote += 1,
            Err(e) => tracing::warn!("onboarding seed: failed to write '{}': {e}", mem.key),
        }
    }

    if wrote == SEED_MEMORIES.len() {
        if let Err(e) = Config::global().set_param(SEED_MARKER_KEY, true) {
            tracing::warn!("onboarding seed: wrote memories but failed to set marker: {e}");
        }
        tracing::info!("onboarding seed: seeded {wrote} welcome memories into the Brain");
    } else {
        tracing::warn!(
            "onboarding seed: wrote {}/{} memories; leaving marker unset to retry",
            wrote,
            SEED_MEMORIES.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_memories_present_unique_and_namespaced() {
        assert!(!SEED_MEMORIES.is_empty(), "must seed at least one memory");
        let keys: Vec<&str> = SEED_MEMORIES.iter().map(|m| m.key).collect();
        let mut deduped = keys.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), keys.len(), "seed keys must be unique");
        for m in SEED_MEMORIES {
            assert!(m.key.starts_with("onboarding:"), "seed keys are namespaced");
            assert!(!m.content.trim().is_empty(), "seed content is non-empty");
        }
    }
}
