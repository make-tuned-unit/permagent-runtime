//! First-run Brain seeding (#298).
//!
//! When onboarding completes, the daemon seeds a small set of welcome /
//! orientation memories into the Brain through the EXISTING `remember_with`
//! write path — no schema migration, and no new public brain-write route (the
//! write happens daemon-internally). Idempotent via a config marker
//! (`onboarding_memories_seeded`); the stable keys also make any re-run
//! harmless. Additive batches use their own marker so already-seeded installs
//! still receive later orientation (the original marker is never reset).
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
/// Additive orientation (how to use the app). Separate marker so installs that
/// already ran the v1 welcome set still get the map of current surfaces.
const HOW_TO_USE_MARKER: &str = "onboarding_how_to_use_seeded";
/// `source` tag recorded on every seeded memory.
const SEED_SOURCE: &str = "permagent.onboarding";
/// Episode every seeded memory is filed under — the first-run seeding itself.
const SEED_EPISODE_ID: &str = "onboarding:first-run";
const HOW_TO_USE_EPISODE_ID: &str = "onboarding:how-to-use";

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
        content: "To get started, tell me what you're working on — or ask for a short tour. \
                  Dashboard is the customizable Home: Decision Inbox proposals wait there for \
                  approve or reject. Projects holds your codebases, Brain is what I remember, \
                  People is who we know, Automate is scheduled work. Optional workers (The Guard, \
                  The Council, Concierge, Initiative) stay off until you flip them under \
                  Settings → Features. I will offer to walk you through unused features over time.",
    },
];

/// Additive map of the app. Written even when the v1 welcome marker is already
/// set, so existing installs learn the current surfaces without rewriting the
/// original three memories.
const HOW_TO_USE_MEMORIES: &[SeedMemory] = &[SeedMemory {
    key: "onboarding:how-to-use",
    content: "How to use Permagent: talk in chat, or ask for a tour. Dashboard (Home) is a \
                  customizable card layout — Decision Inbox items wait for approve/reject, and \
                  The Council's weekly report lands there once that worker is on. Projects are \
                  your codebases; Brain is durable memory; People is the CRM; Build is the coding \
                  workspace; Automate is scheduled recipes; Grow and Finance are go-to-market and \
                  the Financier ledger. Settings → Features is where optional workers live \
                  (Initiative, Playbook, Concierge, Steward git-health, The Guard, The Council) — \
                  they spend API credits and stay off until you flip them. After the first tour I \
                  offer unused features one at a time; say yes and I open the real surface. For \
                  The Council, council_status is the cheap live query; council_convene spends \
                  every seated chat model.",
}];

fn marker_set(key: &str) -> bool {
    Config::global().get_param::<bool>(key).unwrap_or(false)
}

async fn seed_batch(marker_key: &str, episode_id: &str, memories: &[SeedMemory]) {
    if marker_set(marker_key) {
        return;
    }
    let Some(brain) = get_global_brain() else {
        tracing::debug!(
            "onboarding seed: Brain not ready yet ({marker_key}), will retry on next trigger"
        );
        return;
    };

    let mut wrote = 0usize;
    for mem in memories {
        let opts = RememberOpts {
            source: Some(SEED_SOURCE.to_string()),
            visibility: Visibility::Private,
            episode_id: Some(episode_id.to_string()),
            ..Default::default()
        };
        match brain.remember_with(mem.key, mem.content, opts).await {
            Ok(_) => wrote += 1,
            Err(e) => tracing::warn!("onboarding seed: failed to write '{}': {e}", mem.key),
        }
    }

    if wrote == memories.len() {
        if let Err(e) = Config::global().set_param(marker_key, true) {
            tracing::warn!("onboarding seed: wrote memories but failed to set {marker_key}: {e}");
        }
        tracing::info!("onboarding seed: seeded {wrote} memories into the Brain ({marker_key})");
    } else {
        tracing::warn!(
            "onboarding seed: wrote {}/{} memories for {marker_key}; leaving marker unset to retry",
            wrote,
            memories.len()
        );
    }
}

/// Seed welcome/orientation memories into the Brain, at most once per batch.
///
/// No-ops a batch if already seeded or if the Brain isn't ready yet — in the
/// latter case the marker is left unset so the next call (startup or a later
/// trigger) retries rather than silently skipping. Callers are responsible for
/// only invoking this once onboarding is complete.
pub async fn seed_onboarding_memories() {
    seed_batch(SEED_MARKER_KEY, SEED_EPISODE_ID, SEED_MEMORIES).await;
    seed_batch(
        HOW_TO_USE_MARKER,
        HOW_TO_USE_EPISODE_ID,
        HOW_TO_USE_MEMORIES,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_batch(memories: &[SeedMemory]) {
        assert!(!memories.is_empty(), "must seed at least one memory");
        let keys: Vec<&str> = memories.iter().map(|m| m.key).collect();
        let mut deduped = keys.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), keys.len(), "seed keys must be unique");
        for m in memories {
            assert!(m.key.starts_with("onboarding:"), "seed keys are namespaced");
            assert!(!m.content.trim().is_empty(), "seed content is non-empty");
        }
    }

    #[test]
    fn seed_memories_present_unique_and_namespaced() {
        assert_batch(SEED_MEMORIES);
        assert_batch(HOW_TO_USE_MEMORIES);
        let mut all: Vec<&str> = SEED_MEMORIES
            .iter()
            .chain(HOW_TO_USE_MEMORIES.iter())
            .map(|m| m.key)
            .collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "v1 and additive keys must not collide");
        let how = HOW_TO_USE_MEMORIES
            .iter()
            .find(|m| m.key == "onboarding:how-to-use")
            .unwrap();
        assert!(how.content.contains("Decision Inbox"));
        assert!(how.content.contains("The Council"));
        assert!(how.content.contains("council_status"));
        assert!(how.content.contains("Settings → Features"));
    }
}
