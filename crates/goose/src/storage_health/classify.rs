//! What a storage finding actually costs to remove.
//!
//! # Why this exists
//!
//! Before this module, a finding's recommendation was a lookup on its *type*:
//! `dev_cache | app_cache | build_artifact | stale_file` → "Safe to remove",
//! everything else → "Review before removing". Three whole categories of
//! directory collapsed into one word.
//!
//! On 2026-08-24, at ~393 MiB free, that word emptied a machine in five
//! seconds: a 133 GB `target/` with five live `rustc` processes writing into
//! it, `~/.cargo/registry`, `~/.cache/huggingface` (a 900 MB model download),
//! `~/.npm`, this repo's own hermit toolchain cache, and a row of
//! `~/Library/Caches/com.apple.*` directories. All "Safe to remove". Five
//! builds died mid-compile and the rest cost hours of re-downloading.
//!
//! Rebuildable is not one property. It is at least four:
//!
//! | category | what it means |
//! |---|---|
//! | [`CAT_SAFE`] | rebuilt locally, cheaply, from code already on this disk |
//! | [`CAT_REGENERABLE`] | rebuilt only by re-downloading gigabytes over the network |
//! | [`CAT_IN_USE`] | a live process is writing here **right now** |
//! | [`CAT_MACOS`] | an Apple system cache macOS maintains and refills itself |
//!
//! The classifier decides which one a path is, and hands back a one-line
//! consequence the user can read before they act — "5 rustc processes are
//! compiling here", "re-downloads 1.3 GB".

use super::inuse::{self, InUseConfig, ProcessProbe};
use super::safety;
use super::size::format_bytes;
use std::path::Path;

// ── Category keys ───────────────────────────────────────────────────
//
// These strings cross the wire into the findings ledger and the UI, so they
// are the stable contract. Labels are for humans and may be reworded; keys
// may not.

/// Rebuilt locally and cheaply. The only category in a default bulk selection.
pub const CAT_SAFE: &str = "safe_to_remove";
/// Rebuilt only by re-downloading. Never in a default bulk selection.
pub const CAT_REGENERABLE: &str = "regenerable_costly";
/// A live process is using this. Never bulk-removable at all.
pub const CAT_IN_USE: &str = "in_use";
/// Apple's own cache. Never bulk-removable at all.
pub const CAT_MACOS: &str = "managed_by_macos";
/// Anything the scanner cannot vouch for.
pub const CAT_REVIEW: &str = "review_before_removing";

/// Every category key, for exhaustive tests and UI validation.
pub const ALL_CATEGORIES: &[&str] = &[CAT_SAFE, CAT_REGENERABLE, CAT_IN_USE, CAT_MACOS, CAT_REVIEW];

/// True when a category may take part in a bulk (trash-all) action.
///
/// This is the single rule the daemon enforces and the UI mirrors. "In use"
/// and "Managed by macOS" are not merely deselected by default — a bulk action
/// refuses them outright, so no click sequence can sweep them up.
pub fn bulk_trashable(category: &str) -> bool {
    !matches!(category, CAT_IN_USE | CAT_MACOS)
}

/// True when a category belongs in the *pre-checked* bulk selection.
///
/// Only [`CAT_SAFE`]. A regenerable cache is trashable in bulk but only when
/// the user opts in having seen the download cost.
pub fn default_selected(category: &str) -> bool {
    category == CAT_SAFE
}

/// True when trashing a single item of this category needs a second, explicit
/// confirmation that shows its consequence.
pub fn needs_second_confirmation(category: &str) -> bool {
    matches!(category, CAT_IN_USE | CAT_MACOS)
}

/// Human label for a category key.
pub fn category_label(category: &str) -> &'static str {
    match category {
        CAT_SAFE => "Safe to remove",
        CAT_REGENERABLE => "Regenerable but costly",
        CAT_IN_USE => "In use — do not remove",
        CAT_MACOS => "Managed by macOS — leave",
        _ => "Review before removing",
    }
}

// ── The classification ──────────────────────────────────────────────

/// What the scanner decided about one path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    /// Stable category key — see the `CAT_*` constants.
    pub category: &'static str,
    /// The recommendation string carried in the finding. For a regenerable
    /// cache this includes the download size, because that IS the decision:
    /// "Regenerable but costly (re-downloads 1.3 GB)".
    pub recommendation: String,
    /// One line explaining what removing this costs, if there is one worth
    /// saying.
    pub consequence: Option<String>,
}

impl Classification {
    fn new(category: &'static str, consequence: Option<String>) -> Self {
        Self {
            category,
            recommendation: category_label(category).to_string(),
            consequence,
        }
    }
}

/// Classify one finding.
///
/// Precedence is deliberate and runs most-dangerous-first:
///
/// 1. **In use** — beats everything. A cache being regenerable does not make
///    it safe to yank out from under a running build.
/// 2. **Managed by macOS** — Apple's caches are not ours to reclaim.
/// 3. **Regenerable but costly** — a toolchain or package cache.
/// 4. The finding type's own default.
///
/// `probe` is injected so tests are hermetic and so a caller that already
/// knows the machine is idle (or that cannot afford the probe) can pass
/// [`inuse::NoProcessProbe`].
pub fn classify(
    finding_type: &str,
    path: &Path,
    size_bytes: u64,
    cfg: &InUseConfig,
    probe: &dyn ProcessProbe,
) -> Classification {
    // 1. In use. Only build/cache directories are probed: a stale download or
    //    a large user file is a single file nobody is compiling into, and the
    //    probe would just be cost.
    if probes_for_use(finding_type) {
        if let Some(reason) = inuse::in_use_reason(path, cfg, probe) {
            return Classification::new(CAT_IN_USE, Some(reason.consequence));
        }
    }

    // 2. Apple's own caches.
    if is_macos_managed(path) {
        return Classification::new(
            CAT_MACOS,
            Some("macOS refills this cache on its own; deleting it only costs CPU".to_string()),
        );
    }

    // 3. Toolchain and package caches — regenerable, but only over the wire.
    if let Some(what) = costly_cache_label(path) {
        let mut c = Classification::new(CAT_REGENERABLE, None);
        c.recommendation = format!(
            "Regenerable but costly (re-downloads {})",
            format_bytes(size_bytes)
        );
        c.consequence = Some(format!(
            "re-downloads {} the next time {} is used",
            format_bytes(size_bytes),
            what
        ));
        return c;
    }

    // 4. The type default. Delegated to `safety::recommendation_for` so the
    //    old type-only rule stays the single source of the fallback rather
    //    than being duplicated here — this classifier narrows that rule, it
    //    does not replace it.
    if safety::recommendation_for(finding_type) == "Safe to remove" {
        Classification::new(CAT_SAFE, local_rebuild_consequence(finding_type, path))
    } else {
        Classification::new(CAT_REVIEW, None)
    }
}

/// Which finding types are worth probing for live use.
fn probes_for_use(finding_type: &str) -> bool {
    matches!(finding_type, "dev_cache" | "build_artifact" | "app_cache")
}

// ── macOS-managed caches ────────────────────────────────────────────

/// Apple caches that do not carry the `com.apple.` prefix. Kept short and
/// explicit rather than pattern-guessed: a wrong entry here silently hides a
/// real cleanup opportunity, so each one is a cache we have actually seen
/// macOS refill by itself.
const APPLE_CACHE_NAMES: &[&str] = &["SiriTTS", "GeoServices", "CloudKit", "TelephonyUtilities"];

/// True for `~/Library/Caches/com.apple.*` and the named Apple caches.
pub fn is_macos_managed(path: &Path) -> bool {
    let in_library_caches = path.to_string_lossy().contains("/Library/Caches/");
    if !in_library_caches {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.starts_with("com.apple.") || APPLE_CACHE_NAMES.contains(&name)
}

// ── Costly-to-regenerate caches ─────────────────────────────────────

/// Caches whose only regeneration path is the network, matched as a run of
/// consecutive path components so both `~/.cargo/registry` and anything below
/// it classify the same way.
///
/// The label is what gets named in the consequence line ("re-downloads 1.3 GB
/// the next time cargo builds").
const COSTLY_CACHE_PATHS: &[(&[&str], &str)] = &[
    (&["Library", "Caches", "hermit"], "the hermit toolchain"),
    (&[".cargo", "registry"], "cargo"),
    (&[".cargo", "git"], "cargo"),
    (&[".rustup"], "rustup"),
    (&[".cache", "uv"], "uv"),
    (&[".cache", "huggingface"], "a model download"),
    (&[".cache", "pip"], "pip"),
    (&[".cache", "pypoetry"], "poetry"),
    (&[".npm"], "npm"),
];

/// Caches identified by their own directory name wherever they live.
/// `(prefix, label)` — prefix so `ms-playwright-mcp` matches alongside
/// `ms-playwright`.
const COSTLY_CACHE_NAMES: &[(&str, &str)] = &[
    ("ms-playwright", "Playwright"),
    ("ort.pyke.io", "the ONNX runtime"),
    ("node-gyp", "node-gyp"),
    ("puppeteer", "Puppeteer"),
];

/// Name of the tool that would re-download this cache, if it is one.
pub fn costly_cache_label(path: &Path) -> Option<&'static str> {
    let components: Vec<String> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
        .collect();

    for (suffix, label) in COSTLY_CACHE_PATHS {
        if contains_run(&components, suffix) {
            return Some(label);
        }
    }

    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        for (prefix, label) in COSTLY_CACHE_NAMES {
            if name.starts_with(prefix) {
                return Some(label);
            }
        }
    }
    None
}

/// True when `needle` appears as consecutive elements of `haystack`.
fn contains_run(haystack: &[String], needle: &[&str]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|w| w.iter().zip(needle).all(|(a, b)| a == b))
}

// ── Local rebuild costs ─────────────────────────────────────────────

/// The consequence line for something that IS safe to remove — safe still has
/// a price, and saying it is how "safe" stops meaning "free".
fn local_rebuild_consequence(finding_type: &str, path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if finding_type == "build_artifact" {
        if name.starts_with("ModuleCache") {
            return Some("Xcode rebuilds its module cache on the next build".to_string());
        }
        let project = name.split('-').next().unwrap_or(name);
        return Some(format!(
            "the next Xcode build of {} is a full rebuild",
            project
        ));
    }

    if name == "target" {
        let project = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("this repo");
        return Some(format!("a full cargo rebuild of {}", project));
    }

    if name == "node_modules" {
        return Some("npm install re-fetches this package tree".to_string());
    }

    if name.ends_with("cargo-target") || path.to_string_lossy().contains("/.shared-target/") {
        return Some("a full cargo rebuild in that lane".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_health::inuse::{NoProcessProbe, ProcessRef};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A probe that reports the given processes for every query — enough to
    /// force the in-use branch without touching the machine.
    struct BusyProbe(Vec<ProcessRef>);

    impl ProcessProbe for BusyProbe {
        fn processes_in(&self, _dir: &Path) -> Vec<ProcessRef> {
            self.0.clone()
        }
        fn holders_of(&self, _file: &Path) -> Vec<ProcessRef> {
            self.0.clone()
        }
    }

    fn cfg() -> InUseConfig {
        InUseConfig::default()
    }

    /// Classify a path that need not exist. The in-use probe short-circuits on
    /// a missing directory, which is exactly what a pure path-rule test wants.
    fn classify_path(finding_type: &str, path: &str, size: u64) -> Classification {
        classify(
            finding_type,
            &PathBuf::from(path),
            size,
            &cfg(),
            &NoProcessProbe,
        )
    }

    // ── (a) In use ───────────────────────────────────────────────────

    #[test]
    fn a_target_dir_with_a_recent_write_is_in_use_not_safe() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("myrepo/target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::write(target.join("debug/app"), "just built").unwrap();

        let c = classify(
            "dev_cache",
            &target,
            133_079_642_112,
            &cfg(),
            &NoProcessProbe,
        );
        assert_eq!(c.category, CAT_IN_USE);
        assert_eq!(c.recommendation, "In use — do not remove");
        assert_ne!(c.recommendation, "Safe to remove");
        assert!(c.consequence.unwrap().contains("still writing"));
    }

    #[test]
    fn a_held_cargo_lock_names_the_live_compilers() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let lock = target.join(".cargo-lock");
        fs::write(&lock, "").unwrap();
        // Backdate the lock so only the lock-holder probe can fire, never the
        // recent-mtime one.
        let handle = fs::File::options().write(true).open(&lock).unwrap();
        handle
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(86_400),
            ))
            .unwrap();

        let busy = BusyProbe(
            (1..=5)
                .map(|pid| ProcessRef {
                    pid,
                    command: "rustc".into(),
                })
                .collect(),
        );
        let c = classify("dev_cache", &target, 1_000, &cfg(), &busy);
        assert_eq!(c.category, CAT_IN_USE);
        assert_eq!(
            c.consequence.as_deref(),
            Some("5 rustc processes are compiling here")
        );
    }

    #[test]
    fn in_use_beats_regenerable() {
        // ~/.npm is a costly cache, but if npm is installing into it right now
        // the answer is "in use", not "regenerable".
        let tmp = TempDir::new().unwrap();
        let npm = tmp.path().join(".npm");
        fs::create_dir_all(&npm).unwrap();
        fs::write(npm.join("_cacache"), "downloading").unwrap();

        let c = classify("app_cache", &npm, 821_067_776, &cfg(), &NoProcessProbe);
        assert_eq!(c.category, CAT_IN_USE);
    }

    #[test]
    fn in_use_is_never_bulk_trashable() {
        assert!(!bulk_trashable(CAT_IN_USE));
        assert!(!default_selected(CAT_IN_USE));
        assert!(needs_second_confirmation(CAT_IN_USE));
    }

    // ── (b) Regenerable but costly ───────────────────────────────────

    #[test]
    fn toolchain_and_package_caches_are_regenerable_not_safe() {
        // Every path here was trashed as "Safe to remove" on 2026-08-24.
        let cases = [
            ("/Users/j/Library/Caches/hermit", 812_900_352_u64),
            ("/Users/j/.cargo/registry", 1_341_743_104),
            ("/Users/j/.cargo/git", 100_000_000),
            ("/Users/j/.rustup", 4_000_000_000),
            ("/Users/j/.cache/uv", 931_364_864),
            ("/Users/j/.cache/huggingface", 925_380_608),
            ("/Users/j/.npm", 821_067_776),
            ("/Users/j/.cache/pip", 200_000_000),
            ("/Users/j/Library/Caches/ms-playwright", 581_394_432),
            ("/Users/j/Library/Caches/ms-playwright-mcp", 190_791_680),
            ("/Users/j/Library/Caches/ort.pyke.io", 154_128_384),
            ("/Users/j/Library/Caches/node-gyp", 132_317_184),
        ];
        for (path, size) in cases {
            let c = classify_path("app_cache", path, size);
            assert_eq!(c.category, CAT_REGENERABLE, "{} misclassified", path);
            assert!(
                c.recommendation
                    .starts_with("Regenerable but costly (re-downloads "),
                "{} recommendation was {:?}",
                path,
                c.recommendation
            );
            assert!(
                c.consequence
                    .as_deref()
                    .unwrap()
                    .starts_with("re-downloads "),
                "{} consequence was {:?}",
                path,
                c.consequence
            );
        }
    }

    #[test]
    fn a_regenerable_cache_states_its_download_size() {
        let c = classify_path("app_cache", "/Users/j/.cargo/registry", 1_341_743_104);
        assert_eq!(
            c.recommendation,
            "Regenerable but costly (re-downloads 1.2 GB)"
        );
    }

    #[test]
    fn regenerable_is_bulk_trashable_but_never_preselected() {
        assert!(bulk_trashable(CAT_REGENERABLE));
        assert!(
            !default_selected(CAT_REGENERABLE),
            "a costly cache must never be in the default 'safe' selection"
        );
    }

    #[test]
    fn a_path_below_a_costly_cache_classifies_the_same_way() {
        let c = classify_path(
            "app_cache",
            "/Users/j/.cargo/registry/cache/index",
            10_000_000,
        );
        assert_eq!(c.category, CAT_REGENERABLE);
    }

    // ── (c) Managed by macOS ─────────────────────────────────────────

    #[test]
    fn apple_system_caches_are_left_alone() {
        for path in [
            "/Users/j/Library/Caches/com.apple.callintelligenced",
            "/Users/j/Library/Caches/com.apple.textunderstandingd",
            "/Users/j/Library/Caches/SiriTTS",
            "/Users/j/Library/Caches/GeoServices",
        ] {
            let c = classify_path("app_cache", path, 315_318_272);
            assert_eq!(c.category, CAT_MACOS, "{} misclassified", path);
            assert_eq!(c.recommendation, "Managed by macOS — leave");
        }
    }

    #[test]
    fn a_non_apple_library_cache_is_not_macos_managed() {
        let c = classify_path("app_cache", "/Users/j/Library/Caches/Google", 129_404_928);
        assert_eq!(c.category, CAT_SAFE);
    }

    #[test]
    fn a_com_apple_dir_outside_library_caches_is_not_macos_managed() {
        assert!(!is_macos_managed(Path::new(
            "/Users/j/Documents/dev/com.apple.something"
        )));
    }

    #[test]
    fn macos_managed_is_never_bulk_trashable() {
        assert!(!bulk_trashable(CAT_MACOS));
        assert!(needs_second_confirmation(CAT_MACOS));
    }

    // ── (d) Xcode DerivedData ────────────────────────────────────────

    #[test]
    fn xcode_module_cache_stays_safe_but_states_its_rebuild_cost() {
        let c = classify_path(
            "build_artifact",
            "/Users/j/Library/Developer/Xcode/DerivedData/ModuleCache.noindex",
            972_607_488,
        );
        assert_eq!(c.category, CAT_SAFE);
        assert_eq!(
            c.consequence.as_deref(),
            Some("Xcode rebuilds its module cache on the next build")
        );
    }

    #[test]
    fn a_derived_data_project_names_the_project_it_rebuilds() {
        let c = classify_path(
            "build_artifact",
            "/Users/j/Library/Developer/Xcode/DerivedData/PermagentMobile-abhzgrkilkzbnpfpgepoavbwztwd",
            377_466_880,
        );
        assert_eq!(c.category, CAT_SAFE);
        assert_eq!(
            c.consequence.as_deref(),
            Some("the next Xcode build of PermagentMobile is a full rebuild")
        );
    }

    // ── Safe, with its price named ───────────────────────────────────

    #[test]
    fn an_idle_target_dir_is_safe_and_says_what_it_costs() {
        let c = classify_path(
            "dev_cache",
            "/Users/j/Documents/dev/spectral/target",
            1_839_452_160,
        );
        assert_eq!(c.category, CAT_SAFE);
        assert_eq!(
            c.consequence.as_deref(),
            Some("a full cargo rebuild of spectral")
        );
    }

    #[test]
    fn an_idle_node_modules_is_safe_and_says_what_it_costs() {
        let c = classify_path(
            "dev_cache",
            "/Users/j/Documents/dev/GetLadle/node_modules",
            451_121_152,
        );
        assert_eq!(c.category, CAT_SAFE);
        assert_eq!(
            c.consequence.as_deref(),
            Some("npm install re-fetches this package tree")
        );
    }

    #[test]
    fn a_large_user_file_still_needs_review() {
        let c = classify_path("large_file", "/Users/j/Downloads/movie.mov", 500_000_000);
        assert_eq!(c.category, CAT_REVIEW);
        assert_eq!(c.recommendation, "Review before removing");
    }

    #[test]
    fn every_category_key_has_a_label_and_a_bulk_rule() {
        for key in ALL_CATEGORIES {
            assert!(!category_label(key).is_empty());
            // Exactly the two dangerous categories are bulk-refused.
            let refused = matches!(*key, CAT_IN_USE | CAT_MACOS);
            assert_eq!(bulk_trashable(key), !refused, "{}", key);
        }
    }
}
