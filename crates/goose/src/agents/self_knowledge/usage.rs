//! Feature-usage tracking — the memory half of agent-led onboarding.
//!
//! The self-knowledge inventory ([`super`]) says what every capability *is*.
//! This module tracks which of those capabilities the user has actually
//! *engaged*, so the agent can compute what they have not tried yet and teach
//! them into it (see [`super::teachable`]).
//!
//! ## How usage is observed (no fabrication)
//!
//! Usage is derived from the **existing activity event bus**
//! ([`crate::events::activity`]) — the real surfaces already emit
//! `ProjectSelected`, `TerminalCommandCompleted`, `BrowserNavigated`,
//! `SkillExecuted`, `AutomationJobCompleted`, … as the user works.
//! [`emit_activity`](crate::events::activity::emit_activity) forwards every
//! event through [`record_from_activity`], which maps it to a real capability
//! id via [`capability_for_activity`]. Nothing new is emitted; we *consume* the
//! signal the app already produces. The agent's teaching tool additionally
//! marks a feature [`mark_taught`] when it walks the user through it.
//!
//! ## Persistence
//!
//! The authoritative store is an in-process [`UsageStore`]. It is hydrated from
//! durable config on daemon boot ([`hydrate_from_config`]) and written back
//! through the same `Config` seam used by `tour_completed` /
//! `onboarding_memories_seeded`. Writes are **debounced**: a full activity
//! stream would hammer the config file, so we persist only on a *newly engaged*
//! feature or a *day rollover* of `last_used` — bounding disk writes to
//! O(features · days) while keeping the durable signal (used / first / last)
//! honest. Persistence is off until [`enable_persistence`] is called by the
//! daemon, so unit tests mutate the in-memory store without touching config.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::events::activity::{ActivityEventType, SourceSurface};

/// Config key holding the serialized [`UsageStore`] map. A plain config value —
/// no DB schema/migration, mirroring `tour_completed`.
pub const USAGE_CONFIG_KEY: &str = "feature_usage";

/// Self-knowledge descriptor for THIS capability — the onboarding coach. Lets
/// the agent describe that it tracks what the user has used and teaches the
/// rest. Registered in [`super::WORKER_DESCRIPTORS`]; rendered into the
/// `permagent_self` brief. Queryable by the worker contract, but it has no
/// live status to merge (like the Watcher), so it renders with no `_(now: …)_`
/// suffix and stays snapshot-stable.
pub const ONBOARDING_COACH_FEATURE: super::FeatureDescriptor = super::FeatureDescriptor {
    id: "onboarding_coach",
    display_name: "Onboarding coach",
    category: super::FeatureCategory::Worker,
    // Single-line literals (not `\`-continuation) so the rendered brief is
    // transcription-exact for the prompt_manager snapshots.
    what_it_does:
        "Quietly tracks which of your capabilities the user has actually engaged — read from the activity they already generate as they work (opening projects, running terminals and the browser, executing skills, scheduling automations) — and computes, from this same authoritative inventory, what they have not tried yet, ranked by value; teaching a feature marks it learned so it drops off the list",
    why_it_matters:
        "It is how you onboard the user over time instead of all at once: you know the app better than they do, so you can answer what they have not used, offer to walk them through a specific unused feature, open the real surface, and confirm they tried it. Gentle by construction — it surfaces an unused feature at most occasionally and never nags",
    state_source: super::StateSource::Queryable,
    teaching: &[],
};

// ── Record + store ──────────────────────────────────────────────────────────

/// Durable per-capability usage. Presence in the store's map means the feature
/// has been engaged at least once (used or taught).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// How many activity signals mapped to this feature (best-effort; the count
    /// accumulated since the last persist is lost on restart, but used/first/
    /// last are durable).
    #[serde(default)]
    pub count: u64,
    /// The agent has explicitly taught this feature (via the teaching tool).
    #[serde(default)]
    pub taught: bool,
    /// First time this feature was engaged.
    pub first_used: DateTime<Utc>,
    /// Most recent engagement.
    pub last_used: DateTime<Utc>,
}

impl UsageRecord {
    fn newly(now: DateTime<Utc>) -> Self {
        Self {
            count: 0,
            taught: false,
            first_used: now,
            last_used: now,
        }
    }
}

/// In-memory authoritative usage map. Pure and time-injected so it is fully
/// unit-testable without a clock, disk, or global.
#[derive(Debug, Default, Clone)]
pub struct UsageStore {
    records: HashMap<String, UsageRecord>,
}

impl UsageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an engagement of `id` at `now`. Returns `true` when the change is
    /// durable-worthy — a *newly engaged* feature or a `last_used` **day**
    /// rollover — so the caller can debounce persistence.
    pub fn mark_used(&mut self, id: &str, now: DateTime<Utc>) -> bool {
        match self.records.get_mut(id) {
            None => {
                let mut r = UsageRecord::newly(now);
                r.count = 1;
                self.records.insert(id.to_string(), r);
                true
            }
            Some(r) => {
                let day_changed = r.last_used.date_naive() != now.date_naive();
                r.count = r.count.saturating_add(1);
                r.last_used = now;
                day_changed
            }
        }
    }

    /// Mark `id` as explicitly taught by the agent. Always durable-worthy
    /// (teaching is rare). Also engages the feature so it leaves the learn-next
    /// list.
    pub fn mark_taught(&mut self, id: &str, now: DateTime<Utc>) -> bool {
        match self.records.get_mut(id) {
            None => {
                let mut r = UsageRecord::newly(now);
                r.taught = true;
                self.records.insert(id.to_string(), r);
                true
            }
            Some(r) => {
                r.taught = true;
                r.last_used = now;
                true
            }
        }
    }

    /// Whether `id` has been engaged at all (used or taught).
    pub fn is_engaged(&self, id: &str) -> bool {
        self.records.contains_key(id)
    }

    pub fn get(&self, id: &str) -> Option<&UsageRecord> {
        self.records.get(id)
    }

    /// Deterministic (id-sorted) snapshot for serialization / API responses.
    pub fn snapshot(&self) -> Vec<(String, UsageRecord)> {
        let mut v: Vec<(String, UsageRecord)> = self
            .records
            .iter()
            .map(|(k, r)| (k.clone(), r.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }
}

// ── Activity → capability mapping ─────────────────────────────────────────────

/// Map an activity event to the **real** self-knowledge capability id it
/// evidences engagement with, or `None` for events that are not surface
/// engagement (chat turns, token refreshes, internal probes). Every returned id
/// is a live descriptor id (`find_descriptor` resolves it) — asserted in tests.
/// Pure: the whole usage-derivation contract is testable here with no runtime.
pub fn capability_for_activity(
    event_type: &ActivityEventType,
    _surface: &SourceSurface,
) -> Option<&'static str> {
    use ActivityEventType::*;
    match event_type {
        // The Build tab hosts the coding harness — terminals and file work.
        TerminalCommandStarted
        | TerminalCommandCompleted
        | TerminalSessionStarted
        | TerminalSessionEnded
        | TerminalProcessExited
        | FileOpened
        | FileEdited => Some("build"),

        // The browser is its own capability (lives in the Build tab).
        BrowserNavigated | BrowserFormSubmitted | BrowserSessionStarted | BrowserSessionEnded => {
            Some("browser")
        }

        ProjectSelected | ProjectOpened => Some("projects"),

        SkillExecuted => Some("skills"),

        AutomationJobStarted
        | AutomationJobCompleted
        | AutomationJobFailed
        | StarterRecipeUpgraded => Some("scheduler"),

        // Not a user surface-engagement signal: chat is the baseline, and the
        // rest are system/internal events (a token refresh, an internal context
        // probe, a goal the system escalated for approval).
        ChatTurnStarted
        | ChatTurnCompleted
        | IntegrationTokenRefreshed
        | AgentContextProbed
        | GoalEscalated => None,
    }
}

// ── Global store + durable persistence ────────────────────────────────────────

static USAGE: LazyLock<Mutex<UsageStore>> = LazyLock::new(|| Mutex::new(UsageStore::new()));
static PERSIST_ENABLED: AtomicBool = AtomicBool::new(false);

/// Turn on write-through persistence. Called once by the daemon at startup;
/// left off in unit tests so mutations never touch the config file.
pub fn enable_persistence() {
    PERSIST_ENABLED.store(true, Ordering::Relaxed);
}

/// Hydrate the global store from durable config. Best-effort: an absent key or
/// malformed value leaves the store empty. Call once at daemon startup.
pub fn hydrate_from_config() {
    if let Ok(map) = Config::global().get_param::<HashMap<String, UsageRecord>>(USAGE_CONFIG_KEY) {
        if let Ok(mut g) = USAGE.lock() {
            g.records = map;
        }
    }
}

fn persist(records: &HashMap<String, UsageRecord>) {
    if !PERSIST_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = Config::global().set_param(USAGE_CONFIG_KEY, records) {
        tracing::warn!(target: "onboarding", "failed to persist feature usage: {e}");
    }
}

/// Record that the user engaged capability `id` now. Debounced: persists only
/// when the record is newly created or crosses a day boundary.
pub fn record_usage(id: &str) {
    let now = Utc::now();
    let to_persist = {
        let Ok(mut g) = USAGE.lock() else {
            return;
        };
        if g.mark_used(id, now) {
            Some(g.records.clone())
        } else {
            None
        }
    };
    if let Some(map) = to_persist {
        persist(&map);
    }
}

/// Map an activity event to a capability and record engagement. The single seam
/// [`crate::events::activity::emit_activity`] calls — so every real surface that
/// already emits activity feeds usage for free.
pub fn record_from_activity(event_type: &ActivityEventType, surface: &SourceSurface) {
    if let Some(id) = capability_for_activity(event_type, surface) {
        record_usage(id);
    }
}

/// Mark `id` explicitly taught by the agent (and engaged). Always persisted.
pub fn mark_taught(id: &str) {
    let now = Utc::now();
    let to_persist = {
        let Ok(mut g) = USAGE.lock() else {
            return;
        };
        g.mark_taught(id, now);
        Some(g.records.clone())
    };
    if let Some(map) = to_persist {
        persist(&map);
    }
}

/// Whether `id` has been engaged (used or taught) — read from the global store.
pub fn is_engaged(id: &str) -> bool {
    USAGE.lock().map(|g| g.is_engaged(id)).unwrap_or(false)
}

/// Id-sorted snapshot of the global store, for API responses.
pub fn usage_snapshot() -> Vec<(String, UsageRecord)> {
    USAGE.lock().map(|g| g.snapshot()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn mark_used_round_trips_and_accumulates() {
        let mut s = UsageStore::new();
        assert!(!s.is_engaged("build"));

        // First engagement is newly-used → durable-worthy.
        assert!(s.mark_used("build", at(2026, 7, 16, 9)));
        let r = s.get("build").expect("recorded");
        assert_eq!(r.count, 1);
        assert!(!r.taught);
        assert_eq!(r.first_used, at(2026, 7, 16, 9));
        assert!(s.is_engaged("build"));

        // Same-day re-use bumps count but is NOT durable-worthy (debounced).
        assert!(!s.mark_used("build", at(2026, 7, 16, 14)));
        assert_eq!(s.get("build").unwrap().count, 2);
        assert_eq!(s.get("build").unwrap().last_used, at(2026, 7, 16, 14));
        // first_used is stable.
        assert_eq!(s.get("build").unwrap().first_used, at(2026, 7, 16, 9));

        // Next-day re-use IS durable-worthy (day rollover).
        assert!(s.mark_used("build", at(2026, 7, 17, 8)));
        assert_eq!(s.get("build").unwrap().count, 3);
    }

    #[test]
    fn mark_taught_engages_and_sets_flag() {
        let mut s = UsageStore::new();
        assert!(s.mark_taught("brain", at(2026, 7, 16, 9)));
        let r = s.get("brain").unwrap();
        assert!(r.taught);
        assert!(s.is_engaged("brain"));
        // Teaching an already-used feature keeps it used and flips taught on.
        s.mark_used("projects", at(2026, 7, 16, 9));
        assert!(!s.get("projects").unwrap().taught);
        s.mark_taught("projects", at(2026, 7, 16, 10));
        assert!(s.get("projects").unwrap().taught);
    }

    #[test]
    fn serde_round_trip_is_the_hydrate_contract() {
        // The persistence contract is: the store's map survives a
        // serialize→deserialize cycle byte-for-byte. This proves hydrate/persist
        // without touching disk or global config.
        let mut s = UsageStore::new();
        s.mark_used("build", at(2026, 7, 16, 9));
        s.mark_taught("brain", at(2026, 7, 16, 10));
        let map: HashMap<String, UsageRecord> = s.records.clone();

        let json = serde_json::to_string(&map).unwrap();
        let back: HashMap<String, UsageRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(map, back);

        // YAML too — config persists via serde_yaml.
        let yaml = serde_yaml::to_string(&map).unwrap();
        let back_y: HashMap<String, UsageRecord> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(map, back_y);
    }

    #[test]
    fn activity_maps_only_to_real_capability_ids() {
        // Every mapped id must resolve to a live self-knowledge descriptor — no
        // fabricated capability. This is the "no fake usage target" guard.
        use ActivityEventType::*;
        let cases = [
            TerminalCommandCompleted,
            FileEdited,
            BrowserNavigated,
            ProjectSelected,
            SkillExecuted,
            AutomationJobCompleted,
            StarterRecipeUpgraded,
        ];
        for ev in cases {
            let id = capability_for_activity(&ev, &SourceSurface::Agent)
                .unwrap_or_else(|| panic!("{ev:?} should map to a capability"));
            assert!(
                super::super::find_descriptor(id).is_some(),
                "mapped id {id:?} for {ev:?} must be a real descriptor"
            );
        }

        // Chat turns and internal signals are NOT surface engagement.
        assert!(capability_for_activity(&ChatTurnCompleted, &SourceSurface::Chat).is_none());
        assert!(capability_for_activity(&AgentContextProbed, &SourceSurface::Agent).is_none());
        assert!(
            capability_for_activity(&IntegrationTokenRefreshed, &SourceSurface::External).is_none()
        );
    }

    #[test]
    fn expected_surfaces_are_covered() {
        use ActivityEventType::*;
        assert_eq!(
            capability_for_activity(&TerminalCommandCompleted, &SourceSurface::Terminal),
            Some("build")
        );
        assert_eq!(
            capability_for_activity(&BrowserNavigated, &SourceSurface::Browser),
            Some("browser")
        );
        assert_eq!(
            capability_for_activity(&ProjectSelected, &SourceSurface::ProjectPicker),
            Some("projects")
        );
        assert_eq!(
            capability_for_activity(&SkillExecuted, &SourceSurface::SkillsEngine),
            Some("skills")
        );
        assert_eq!(
            capability_for_activity(&AutomationJobCompleted, &SourceSurface::Scheduler),
            Some("scheduler")
        );
    }
}
