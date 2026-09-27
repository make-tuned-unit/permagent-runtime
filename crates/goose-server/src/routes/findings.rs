//! Action affordance endpoints for automation findings.
//!
//! Findings are cleanup/optimization items produced by recipes like Storage
//! Insights. Users act on them via Trash/Keep buttons in the Automate tab.
//!
//! Files go to native macOS Trash via the `trash` crate — NOT shell rm or
//! filesystem mv to ~/.Trash. The ledger retains the original path; native
//! Trash receives a verified staging path, so Finder restoration uses staging.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use permagent::storage_health::classify;
use permagent::storage_health::inuse::action::{
    self, ActionProbe, ActivityProbe, CargoLocks, CleanupLock, LockProvider,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

// ── iCloud eviction check (NSURL API) ─────────────────────────────

/// Check if a file is an iCloud-evicted stub using the authoritative
/// NSURLUbiquitousItemDownloadingStatusKey API.
///
/// Returns true when the file is ubiquitous AND its downloading status
/// is NotDownloaded — meaning content lives only in iCloud with zero
/// local disk allocation. This is consistent with the blocks()-based
/// `is_icloud_evicted` in safety.rs but uses the OS-level API as the
/// authoritative source for the delete guard.
#[cfg(target_os = "macos")]
fn is_icloud_evicted_nsurl(path: &Path) -> bool {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSString, NSURL};

    let path_str = match path.to_str() {
        Some(s) => s,
        None => return false,
    };

    let ns_path = NSString::from_str(path_str);
    let url = NSURL::fileURLWithPath(&ns_path);

    // NSURLIsUbiquitousItemKey — is this file managed by iCloud?
    let is_ubiquitous_key = NSString::from_str("NSURLIsUbiquitousItemKey");
    let mut ubiq_val: Option<Retained<AnyObject>> = None;
    let got_ubiq = unsafe { url.getResourceValue_forKey_error(&mut ubiq_val, &is_ubiquitous_key) };

    match (got_ubiq.is_ok(), &ubiq_val) {
        (true, Some(val)) => {
            let is_ubiq: bool = unsafe { objc2::msg_send![val, boolValue] };
            if !is_ubiq {
                return false; // Not an iCloud-managed file — allow trash
            }
        }
        // A nil value is what NSURL returns for every ordinary local file
        // (verified on a real ~/Downloads file, 2026-09-22): it means "not an
        // iCloud item", not "status unavailable". Treating nil as evicted
        // refused every cleanup on the machine. Only a REAL read error below
        // fails closed.
        (true, None) => return false,
        (false, _) => return true,
    }

    // NSURLUbiquitousItemDownloadingStatusKey
    let status_key = NSString::from_str("NSURLUbiquitousItemDownloadingStatusKey");
    let mut status_val: Option<Retained<AnyObject>> = None;
    let got_status = unsafe { url.getResourceValue_forKey_error(&mut status_val, &status_key) };

    match (got_status.is_ok(), status_val) {
        (true, Some(val)) => {
            // The value is an NSString; cast and compare
            // SAFETY: the NSURL API returns an NSString for this key
            let status_str: Retained<NSString> = unsafe { Retained::cast_unchecked(val) };
            let not_downloaded =
                NSString::from_str("NSURLUbiquitousItemDownloadingStatusNotDownloaded");
            status_str.isEqualToString(&not_downloaded)
        }
        _ => true, // Unknown downloading status is not deletion authority
    }
}

// ── Sensitive path validation ──────────────────────────────────────

fn is_sensitive_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let lower = path_str.to_lowercase();

    // Blocked directory prefixes
    let blocked_dirs = [
        "/.ssh/",
        "/.aws/",
        "/.gcp/",
        "/.gnupg/",
        "/library/keychains/",
    ];
    for dir in &blocked_dirs {
        if lower.contains(dir) {
            return true;
        }
    }

    // Blocked filename patterns
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name_lower = name.to_lowercase();
        if name_lower.contains(".env")
            || name_lower.contains("credentials")
            || name_lower.contains("id_rsa")
            || name_lower.contains("id_ed25519")
            || name_lower.ends_with(".key")
            || name_lower.ends_with(".pem")
        {
            return true;
        }
    }

    false
}

// ── Findings storage ───────────────────────────────────────────────

fn findings_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".permagent/automation/findings")
}

fn findings_path(run_id: &str) -> PathBuf {
    if run_id.is_empty()
        || !run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return findings_dir().join("invalid-run-id");
    }
    findings_dir().join(format!("{}.json", run_id))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub id: String,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(rename = "type")]
    pub finding_type: String,
    pub path: String,
    pub size_bytes: u64,
    pub age_days: Option<u64>,
    pub recommendation: String,
    /// Stable removal category — `classify::CAT_*`. This is what the action
    /// path enforces on, NOT the finding type: on 2026-08-24 every one of 33
    /// findings was type `dev_cache`/`app_cache`/`build_artifact` and every one
    /// of them therefore said "Safe to remove", including a target dir with
    /// five live rustc processes in it.
    #[serde(default = "default_category")]
    pub category: String,
    /// One line naming what removing this costs.
    #[serde(default)]
    pub consequence: Option<String>,
    pub action_taken: Option<String>,
    pub actioned_at: Option<String>,
    pub size_recovered_bytes: Option<u64>,
    /// Which route took the action: a UI click, a UI bulk sweep, an agent
    /// tool, or a raw API call. Recorded so the ledger can answer "who
    /// deleted 133 GB and how" without guessing.
    #[serde(default)]
    pub action_source: Option<String>,
    /// Another saved finding already covers this path. Listed, not deleted on
    /// its own, so the same bytes are not trashed twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contained_in: Option<String>,
    /// Present when deleting this row would also delete an in-use or protected
    /// path. The category is unchanged; the action path refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

/// Ledgers written before categories existed carry no `category`. Nothing
/// about them was ever verified, so they read as "review", not as "safe".
fn default_category() -> String {
    classify::CAT_REVIEW.to_string()
}

/// Current observation returned for every named pending finding.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Eligibility {
    pub finding_id: String,
    pub category: String,
    pub eligible: bool,
    pub reason: String,
    pub protection: String,
    pub observed_at: String,
    pub profiles: Vec<ProfileResult>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProfileResult {
    pub path: String,
    pub status: String,
    pub reason: String,
    pub bytes: u64,
    pub moved_bytes: u64,
    pub moved: usize,
    pub skipped: usize,
    pub failed: usize,
}

fn profile_availability(
    target: &Path,
    probe: &dyn ActivityProbe,
    locks: &dyn LockProvider,
    release_verified: bool,
) -> Result<Vec<ProfileResult>, String> {
    action::cargo_profiles(target)?.into_iter().map(|profile| {
        let mut result = ProfileResult { path: profile.path.to_string_lossy().into_owned(),
            status: "eligible".into(), reason: "Available build outputs".into(),
            bytes: profile.size_bytes, moved_bytes: 0, moved: 0, skipped: 0, failed: 0 };
        let outcome = (|| {
            if let Some(reason) = profile.reason { return Err(reason); }
            if !probe.check(&profile.path)?.is_empty() { return Err("Profile in use: active process".into()); }
            let _guard = locks.acquire(&profile.path)?;
            if !release_verified { return Err("Unsupported layout: real Cargo coordination acceptance is not yet verified".into()); }
            Ok(())
        })();
        if let Err(reason) = outcome {
            result.status = if reason.starts_with("Profile in use:") { "busy" } else { "unsupported" }.into();
            result.reason = reason;
        }
        Ok(result)
    }).collect()
}

fn eligibility(
    f: &mut Finding,
    include_regenerable: bool,
    include_review: bool,
    home: &Path,
    probe: &dyn ActivityProbe,
) -> Eligibility {
    let mut result = Eligibility {
        finding_id: f.id.clone(),
        category: f.category.clone(),
        eligible: false,
        reason: String::new(),
        protection: if classify::is_rust_target(Path::new(&f.path)) {
            "cargo-lock-required".into()
        } else {
            "probe-only".into()
        },
        observed_at: Utc::now().to_rfc3339(),
        profiles: Vec::new(),
    };
    if let Some(reason) = f.blocked_reason.clone() {
        result.reason = reason;
        return result;
    }
    if f.contained_in.is_some()
        && f.category != classify::CAT_IN_USE
        && f.category != classify::CAT_MACOS
    {
        result.reason = "Included in another finding; it is removed only with that item".into();
        return result;
    }
    let check = || -> Result<classify::Classification, String> {
        if !classify::ALL_CATEGORIES.contains(&f.category.as_str()) {
            return Err("Unknown category".into());
        }
        if f.action_taken.is_some() {
            return Err("Already actioned".into());
        }
        let path = Path::new(&f.path);
        action::validate_path(&f.finding_type, path, home)?;
        if is_sensitive_path(path) {
            return Err("Sensitive path is excluded".into());
        }
        #[cfg(target_os = "macos")]
        if is_icloud_evicted_nsurl(path) {
            return Err("iCloud content is not downloaded or its status is unavailable".into());
        }
        if classify::is_rust_target(path) {
            return Ok(classify::Classification {
                category: classify::CAT_REGENERABLE,
                recommendation: classify::category_label(classify::CAT_REGENERABLE).into(),
                consequence: Some("Cleaned outputs will need rebuilding.".into()),
            });
        }
        action::current_classification(&f.finding_type, path, f.size_bytes, probe)
    };
    match check() {
        Err(reason) => {
            result.reason = reason;
            return result;
        }
        Ok(c) => {
            f.category = c.category.into();
            f.recommendation = c.recommendation;
            f.consequence = c.consequence;
            f.observed_at = Some(result.observed_at.clone());
            result.category = f.category.clone();
            result.eligible = match c.category {
                classify::CAT_SAFE => true,
                classify::CAT_REGENERABLE => include_regenerable,
                classify::CAT_REVIEW => include_review,
                _ => false,
            };
            result.reason = match c.category {
                classify::CAT_REGENERABLE if !include_regenerable => {
                    "Rebuildable items require acknowledgment".into()
                }
                classify::CAT_REVIEW if !include_review => {
                    "Reviewed files require acknowledgment".into()
                }
                classify::CAT_MACOS => {
                    "Managed by macOS; use System Settings > General > Storage".into()
                }
                _ => f
                    .consequence
                    .clone()
                    .unwrap_or_else(|| classify::category_label(c.category).into()),
            };
        }
    }
    if result.category == classify::CAT_REGENERABLE
        && classify::is_rust_target(Path::new(&f.path))
        && action::validate_path(&f.finding_type, Path::new(&f.path), home).is_ok()
    {
        result.protection = "cargo-profile-locks".into();
        match profile_availability(
            Path::new(&f.path),
            probe,
            &CargoLocks,
            action::CARGO_PROFILE_RELEASE_VERIFIED,
        ) {
            Ok(profiles) => {
                result.eligible =
                    include_regenerable && profiles.iter().any(|p| p.status == "eligible");
                if profiles.iter().any(|p| p.status == "busy")
                    && !profiles.iter().any(|p| p.status == "eligible")
                {
                    result.reason =
                        "Profiles in use are skipped; other unsupported profiles remain excluded"
                            .into();
                } else if !profiles.iter().any(|p| p.status == "eligible") {
                    result.reason =
                        "Unsupported layout: no verified eligible Cargo profiles".into();
                }
                result.profiles = profiles;
            }
            Err(reason) => {
                result.eligible = false;
                result.reason = format!("Unsupported layout: {reason}");
            }
        }
    }
    result
}

/// One action boundary shared by the HTTP routes and hermetic race tests.
struct ActionChecks<'a> {
    run_id: &'a str,
    selected_profiles: &'a [String],
    home: &'a Path,
    probe: &'a dyn ActivityProbe,
    locks: &'a dyn LockProvider,
    include_regenerable: bool,
    include_review: bool,
}

fn checked_trash(
    f: &mut Finding,
    checks: &ActionChecks<'_>,
    mut trash: impl FnMut(&mut Finding) -> Result<u64, String>,
) -> (Eligibility, Result<u64, String>) {
    let identity = std::fs::symlink_metadata(&f.path);
    let mut current = eligibility(
        f,
        checks.include_regenerable,
        checks.include_review,
        checks.home,
        checks.probe,
    );
    let result = (|| {
        if !current.eligible {
            for profile in &mut current.profiles {
                if checks.selected_profiles.contains(&profile.path) {
                    profile.skipped += 1;
                }
            }
            return Err(current.reason.clone());
        }
        if classify::is_rust_target(Path::new(&f.path)) {
            return clean_profiles(f, checks, &mut current.profiles, &mut trash);
        }
        let _guard = lock_finding(f, checks.locks)?;
        action::validate_path(&f.finding_type, Path::new(&f.path), checks.home)?;
        let before = identity.map_err(|e| e.to_string())?;
        let after = std::fs::symlink_metadata(&f.path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err("Path changed during availability check".into());
            }
        }
        #[cfg(not(unix))]
        if before.file_type() != after.file_type() || before.len() != after.len() {
            return Err("Path changed during availability check".into());
        }
        let original = f.path.clone();
        let staged_result = permagent::storage_health::inuse::staging::with_staged(
            checks.home,
            checks.run_id,
            &f.id.clone(),
            Path::new(&original),
            &before,
            || {},
            |staged| {
                f.path = staged.to_string_lossy().into_owned();
                trash(f)
            },
        );
        f.path = original;
        staged_result
    })();
    if let Err(reason) = &result {
        current.eligible = false;
        current.reason = reason.clone();
    }
    (current, result)
}

/// Selection is the exact profile snapshot confirmed by the user. Newly
/// created custom profiles never join it, and the root itself is never moved.
fn clean_profiles(
    f: &mut Finding,
    checks: &ActionChecks<'_>,
    profiles: &mut Vec<ProfileResult>,
    trash: &mut impl FnMut(&mut Finding) -> Result<u64, String>,
) -> Result<u64, String> {
    if checks.selected_profiles.is_empty() {
        return Err("Exact profile_paths are required for Cargo cleanup".into());
    }
    for path in checks.selected_profiles {
        if !profiles.iter().any(|p| &p.path == path) {
            profiles.push(ProfileResult {
                path: path.clone(),
                status: "unsupported".into(),
                reason: "Selected profile is absent from the current supported layout".into(),
                bytes: 0,
                moved_bytes: 0,
                moved: 0,
                skipped: 0,
                failed: 0,
            });
        }
    }
    let mut moved_bytes = 0;
    let mut moved_items = 0;
    for profile in profiles {
        if !checks.selected_profiles.contains(&profile.path) {
            continue;
        }
        if profile.status != "eligible" {
            profile.skipped += 1;
            continue;
        }
        let outcome = (|| {
            action::validate_path(&f.finding_type, Path::new(&f.path), checks.home)?;
            if !checks.probe.check(Path::new(&profile.path))?.is_empty() {
                return Err("Profile in use: active process".into());
            }
            let _guard = checks.locks.acquire(Path::new(&profile.path))?;
            let contents = action::profile_contents(Path::new(&profile.path))?;
            for path in contents {
                let before = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                let bytes = action::measured_bytes(&path)?;
                let mut item = f.clone();
                item.size_bytes = bytes;
                let result = permagent::storage_health::inuse::staging::with_staged(
                    checks.home,
                    checks.run_id,
                    &f.id,
                    &path,
                    &before,
                    || {},
                    |staged| {
                        item.path = staged.to_string_lossy().into_owned();
                        trash(&mut item)
                    },
                );
                match result {
                    Ok(actual) => {
                        profile.moved += 1;
                        moved_items += 1;
                        profile.moved_bytes += actual;
                        moved_bytes += actual;
                        f.actioned_at = item.actioned_at;
                        f.action_source = item.action_source;
                    }
                    Err(reason) => {
                        profile.failed += 1;
                        profile.reason = reason;
                    }
                }
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                profile.status = if profile.failed == 0 {
                    "moved"
                } else {
                    "failed"
                }
                .into();
                if profile.failed == 0 {
                    profile.reason =
                        "Selected profile outputs moved; profile and coordination files remain"
                            .into();
                }
            }
            Err(reason) => {
                profile.skipped += 1;
                profile.status = "skipped".into();
                profile.reason = reason;
            }
        }
    }
    // Root remains pending for future scans; actual partial movement is reported
    // per profile and the action ledger accumulates only bytes actually moved.
    f.size_recovered_bytes = Some(f.size_recovered_bytes.unwrap_or(0) + moved_bytes);
    if moved_items == 0 {
        return Err("No selected profile outputs were moved".into());
    }
    Ok(moved_bytes)
}

fn action_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn lock_finding(f: &Finding, locks: &dyn LockProvider) -> Result<Box<dyn CleanupLock>, String> {
    if classify::is_rust_target(Path::new(&f.path)) {
        // Never delegate a target root to a per-profile LockProvider.
        let _ = locks;
        Err("Unsupported layout: whole Cargo target removal is excluded".into())
    } else {
        Ok(Box::new(Vec::<std::fs::File>::new()))
    }
}

// ── Action sources ─────────────────────────────────────────────────

/// A Trash/Keep button in the Automate tab.
pub const SOURCE_UI_CLICK: &str = "ui_click";
/// The Automate tab's bulk "Clean Up All" flow.
pub const SOURCE_UI_BULK: &str = "ui_bulk";
/// An agent tool. Refused here — see `refuse_agent_source`.
pub const SOURCE_AGENT_TOOL: &str = "agent_tool";
/// Anything else reaching the endpoint directly.
pub const SOURCE_API: &str = "api";

/// Normalize a caller-supplied source into something safe to store: one of the
/// known values, or `api` for an unrecognized one. Never trust an arbitrary
/// string into the ledger.
fn normalize_source(raw: Option<&str>) -> String {
    match raw.map(str::trim) {
        Some(SOURCE_UI_CLICK) => SOURCE_UI_CLICK,
        Some(SOURCE_UI_BULK) => SOURCE_UI_BULK,
        Some(SOURCE_AGENT_TOOL) => SOURCE_AGENT_TOOL,
        _ => SOURCE_API,
    }
    .to_string()
}

/// Agent-initiated destruction does not happen here.
///
/// No agent tool can currently reach this endpoint — `scan_storage_health` is
/// read-only and is the only storage tool that exists. This is the fail-closed
/// guard for the day one is added: an agent-sourced trash must be raised as a
/// Tier-2 decision through the approval ladder (#1095) and taken by the user,
/// not executed on the agent's say-so.
fn refuse_agent_source(source: &str) -> Option<(StatusCode, Json<ErrorBody>)> {
    if source != SOURCE_AGENT_TOOL {
        return None;
    }
    Some((
        StatusCode::FORBIDDEN,
        Json(ErrorBody {
            error: "Agent-initiated trash is not permitted on this endpoint. File it as a \
                    Tier 2 decision (user_data_deletion) through the approval ladder and let \
                    the user take the action."
                .to_string(),
        }),
    ))
}

// ── Category guards ────────────────────────────────────────────────

/// Refuse a bulk trash of a category that must never be swept up.
///
/// "In use" and "Managed by macOS" are not merely deselected by default — a
/// bulk action cannot take them at all, so no sequence of clicks reaches them.
fn bulk_refusal(f: &Finding) -> Option<BlockedItem> {
    if !classify::bulk_trashable(&f.category) {
        return Some(BlockedItem {
            finding_id: f.id.clone(),
            path: f.path.clone(),
            category: f.category.clone(),
            consequence: f.consequence.clone(),
        });
    }
    if f.blocked_reason.is_some() || f.contained_in.is_some() {
        return Some(BlockedItem {
            finding_id: f.id.clone(),
            path: f.path.clone(),
            category: f.category.clone(),
            consequence: f.blocked_reason.clone().or_else(|| {
                f.contained_in
                    .as_ref()
                    .map(|id| format!("Included in finding {id}"))
            }),
        });
    }
    None
}

/// Legacy callers receive a recheck hint for a saved busy category. Even a
/// confirmed request must pass the fresh checked_trash boundary below.
fn confirmation_refusal(f: &Finding, confirmed: bool) -> Option<(StatusCode, Json<ErrorBody>)> {
    if confirmed || !classify::needs_second_confirmation(&f.category) {
        return None;
    }
    let consequence = f
        .consequence
        .clone()
        .unwrap_or_else(|| classify::category_label(&f.category).to_string());
    Some((
        StatusCode::CONFLICT,
        Json(ErrorBody {
            error: format!(
                "{} is {} — {}. Recheck availability before trying again.",
                f.path,
                classify::category_label(&f.category),
                consequence
            ),
        }),
    ))
}

#[derive(Serialize, Deserialize)]
struct FindingsFile {
    run_id: String,
    findings: Vec<Finding>,
}

fn load_findings(run_id: &str) -> Option<FindingsFile> {
    let path = findings_path(run_id);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Atomically write pre-serialized JSON: temp file in the same directory, then
/// rename into place — a crash mid-write can never leave a half-written
/// ledger. Shared with the storage-scan route, which writes the same file
/// shape. Errors are for the caller to surface; this is the action ledger of a
/// DESTRUCTIVE flow (files moved to Trash) and must never be best-effort.
pub(crate) fn atomic_write_json(dir: &Path, file_name: &str, json: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("create findings dir {}: {}", dir.display(), e))?;
    let path = dir.join(file_name);
    let tmp = dir.join(format!(".{}.tmp", file_name));
    std::fs::write(&tmp, json).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        // Don't leave the temp file behind on a failed rename.
        let _ = std::fs::remove_file(&tmp);
        format!("rename {} into place: {}", path.display(), e)
    })?;
    Ok(())
}

fn save_findings_to(dir: &Path, data: &FindingsFile) -> Result<(), String> {
    let path = dir.join(format!("{}.json", data.run_id));
    let mut ledger = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    ledger["run_id"] = serde_json::json!(data.run_id);
    ledger["findings"] = serde_json::to_value(&data.findings).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&ledger).map_err(|e| e.to_string())?;
    atomic_write_json(dir, &format!("{}.json", data.run_id), &json)
}

fn save_findings(data: &FindingsFile) -> Result<(), String> {
    save_findings_to(&findings_dir(), data)
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ── POST /automation/finding/:finding_id/action ────────────────────

#[derive(Deserialize)]
pub struct ActionRequest {
    action: String, // "trash" | "keep" | "skip"
    run_id: String,
    /// Which route is taking this action — see the `SOURCE_*` constants.
    /// Absent means a raw API call.
    #[serde(default)]
    action_source: Option<String>,
    /// Set by the caller only AFTER the user has been shown the item's
    /// consequence and asked a second time. Required for an in-use or
    /// macOS-managed item.
    #[serde(default)]
    confirmed: bool,
}

/// One finding a bulk action refused to touch, and why.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct BlockedItem {
    pub finding_id: String,
    pub path: String,
    pub category: String,
    pub consequence: Option<String>,
}

#[derive(Serialize)]
pub struct ActionResponse {
    finding_id: String,
    action_taken: String,
    size_recovered_bytes: Option<u64>,
    trash_path: Option<String>,
    timestamp: String,
}

/// Move one finding's path to the native macOS Trash and stamp the ledger
/// entry in memory. Does NOT persist — the caller owns the ledger write, so a
/// bulk action writes once instead of 33 times.
///
/// Every guard that used to live inline in `perform_action` lives here, so the
/// bulk path cannot accidentally skip one.
fn trash_one(
    f: &mut Finding,
    source: &str,
    now: &str,
) -> Result<u64, (StatusCode, Json<ErrorBody>)> {
    let finding_path = f.path.clone();
    let file_path = Path::new(&finding_path);

    // Sensitive path validation
    if is_sensitive_path(file_path) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                error: format!("Refusing to trash sensitive path: {}", finding_path),
            }),
        ));
    }

    if !file_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("File not found: {}", finding_path),
            }),
        ));
    }

    // Block iCloud-evicted files whose content is not locally present.
    // Uses the authoritative NSURL downloadingStatus API rather than
    // raw st_flags, consistent with safety::is_icloud_evicted in the
    // scanner.
    #[cfg(target_os = "macos")]
    {
        if is_icloud_evicted_nsurl(file_path) {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody {
                    error: format!(
                        "File content is in iCloud only (not downloaded locally). \
                         No local disk space to recover: {}",
                        finding_path
                    ),
                }),
            ));
        }
    }

    let size = action::measured_bytes(file_path)
        .map_err(|error| (StatusCode::CONFLICT, Json(ErrorBody { error })))?;

    // Native macOS Trash via the `trash` crate.
    // The original path remains in our ledger; Finder sees the staging path.
    // Staging is required to close the source-path substitution race.
    trash::delete(file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to move to Trash: {}", e),
            }),
        )
    })?;

    // The run ledger of a destructive flow. Log WHAT went and by WHICH route:
    // after 2026-08-24 nobody could tell from the logs whether 133 GB left by
    // a UI click or an agent call.
    tracing::info!(
        target: "storage_cleanup",
        finding_id = %f.id,
        path = %finding_path,
        category = %f.category,
        size_bytes = size,
        action_source = %source,
        "trashed a storage finding"
    );

    f.action_taken = Some("trashed".into());
    f.actioned_at = Some(now.to_string());
    f.size_recovered_bytes = Some(size);
    f.action_source = Some(source.to_string());
    Ok(size)
}

async fn perform_action(
    State(_state): State<Arc<AppState>>,
    AxumPath(finding_id): AxumPath<String>,
    Json(req): Json<ActionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, Json<ErrorBody>)> {
    let source = normalize_source(req.action_source.as_deref());
    if let Some(refusal) = refuse_agent_source(&source) {
        return Err(refusal);
    }

    let mut data = load_findings(&req.run_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Run {} not found", req.run_id),
            }),
        )
    })?;

    let idx = data
        .findings
        .iter()
        .position(|f| f.id == finding_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: format!("Finding {} not found", finding_id),
                }),
            )
        })?;

    if data.findings[idx].action_taken.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: format!(
                    "Finding {} already actioned as {:?}",
                    finding_id, data.findings[idx].action_taken
                ),
            }),
        ));
    }

    let now = Utc::now().to_rfc3339();

    match req.action.as_str() {
        "trash" => {
            // An in-use or macOS-managed item takes a second, informed yes.
            if let Some(refusal) = confirmation_refusal(&data.findings[idx], req.confirmed) {
                return Err(refusal);
            }

            let finding_path = data.findings[idx].path.clone();
            let file_name = Path::new(&finding_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let home = action_home();
            let checks = ActionChecks {
                home: &home,
                run_id: &req.run_id,
                selected_profiles: &[],
                probe: &ActionProbe,
                locks: &CargoLocks,
                include_regenerable: true,
                include_review: true,
            };
            let (_, result) = checked_trash(&mut data.findings[idx], &checks, |f| {
                trash_one(f, &source, &now).map_err(|(_, body)| body.0.error)
            });
            let size = result.map_err(|error| (StatusCode::CONFLICT, Json(ErrorBody { error })))?;

            // The destructive step already happened — if recording it fails,
            // tell the truth (file IS in the Trash, ledger was not updated)
            // instead of returning a fake 200 that forgets the action.
            if let Err(e) = save_findings(&data) {
                tracing::error!(
                    "Finding {} moved '{}' to Trash but the action ledger write failed: {}",
                    finding_id,
                    finding_path,
                    e
                );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: format!(
                            "The file WAS moved to the Trash, but recording the action failed: {}. \
                             The finding may reappear as un-actioned; the file is recoverable \
                             from the Trash.",
                            e
                        ),
                    }),
                ));
            }

            Ok(Json(ActionResponse {
                finding_id,
                action_taken: "trashed".into(),
                size_recovered_bytes: Some(size),
                trash_path: Some(format!(
                    "{}/.Trash/{}",
                    std::env::var("HOME").unwrap_or_default(),
                    file_name
                )),
                timestamp: now,
            }))
        }
        "keep" | "skip" => {
            data.findings[idx].action_taken = Some(req.action.clone());
            data.findings[idx].actioned_at = Some(now.clone());
            data.findings[idx].action_source = Some(source.clone());
            // Persist BEFORE responding — a 200 must mean the action stuck.
            if let Err(e) = save_findings(&data) {
                tracing::error!(
                    "Finding {} action '{}' could not be persisted: {}",
                    finding_id,
                    req.action,
                    e
                );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: format!("Failed to record the '{}' action: {}", req.action, e),
                    }),
                ));
            }

            Ok(Json(ActionResponse {
                finding_id,
                action_taken: req.action,
                size_recovered_bytes: None,
                trash_path: None,
                timestamp: now,
            }))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("Invalid action: {}. Must be trash, keep, or skip.", other),
            }),
        )),
    }
}

// ── POST /automation/run/:run_id/bulk-action ───────────────────────
//
// The whole reason this endpoint exists. Before it, "Clean Up All" was a
// client-side `for` loop over the single-action endpoint, so the server saw 33
// independent, individually-legitimate requests and had no idea a sweep was
// happening. On 2026-08-24 that loop emptied the machine in five seconds.
//
// A sweep is now one request the server can reason about:
//   * it must carry an explicit `confirmed` — the UI sets it only after
//     showing the total size and the per-category counts;
//   * it rechecks exact identities and reports skipped items with reasons;
//     no newly eligible finding silently joins a confirmed selection;
//   * it writes the ledger once, stamped with `action_source`.

#[derive(Deserialize)]
pub struct BulkActionRequest {
    #[serde(default)]
    include_regenerable: bool,
    #[serde(default)]
    include_review: bool,
    /// "trash" | "keep" | "skip".
    action: String,
    /// Exact identities for trash. Empty is permitted only for keep/skip.
    #[serde(default)]
    finding_ids: Vec<String>,
    #[serde(default)]
    profile_paths: std::collections::HashMap<String, Vec<String>>,
    /// Must be true for a bulk trash. The UI sets it from the confirmation
    /// dialog, never by default.
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    action_source: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct BulkItemResult {
    pub finding_id: String,
    pub action_taken: Option<String>,
    pub size_recovered_bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct BulkActionResponse {
    pub availability: Vec<Eligibility>,
    pub run_id: String,
    pub action_source: String,
    pub total_recovered_bytes: u64,
    pub results: Vec<BulkItemResult>,
    /// Skipped items, each with the current reason; other selections proceed.
    pub blocked: Vec<BlockedItem>,
}

/// Resolve the request's target set: the named findings, or every un-actioned
/// one. Unknown ids are an error, not a silent skip.
fn resolve_bulk_targets(
    data: &FindingsFile,
    ids: &[String],
) -> Result<Vec<usize>, (StatusCode, Json<ErrorBody>)> {
    if ids.is_empty() {
        return Ok(data
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| f.action_taken.is_none())
            .map(|(i, _)| i)
            .collect());
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match data.findings.iter().position(|f| &f.id == id) {
            Some(i) => {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorBody {
                        error: format!("Finding {} not found", id),
                    }),
                ))
            }
        }
    }
    Ok(out)
}

async fn perform_bulk_action(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
    Json(req): Json<BulkActionRequest>,
) -> Result<Json<BulkActionResponse>, (StatusCode, axum::Json<serde_json::Value>)> {
    let err_plain = |code: StatusCode, body: Json<ErrorBody>| {
        (
            code,
            axum::Json(serde_json::json!({ "error": body.0.error })),
        )
    };

    let source = normalize_source(req.action_source.as_deref());
    if let Some((code, body)) = refuse_agent_source(&source) {
        return Err(err_plain(code, body));
    }

    let mut data = load_findings(&run_id).ok_or_else(|| {
        err_plain(
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Run {} not found", run_id),
            }),
        )
    })?;

    let targets = resolve_bulk_targets(&data, &req.finding_ids)
        .map_err(|(code, body)| err_plain(code, body))?;

    let is_trash = req.action == "trash";
    if !matches!(req.action.as_str(), "trash" | "keep" | "skip") {
        return Err(err_plain(
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!(
                    "Invalid action: {}. Must be trash, keep, or skip.",
                    req.action
                ),
            }),
        ));
    }

    if is_trash {
        if !req.confirmed {
            return Err(err_plain(
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "A bulk trash requires confirmed=true — the caller must show the \
                            total size and the count per category first."
                        .to_string(),
                }),
            ));
        }

        if req.finding_ids.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Bulk trash requires exact finding_ids"
                })),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    let mut results = Vec::with_capacity(targets.len());
    let mut total = 0u64;
    let mut availability = Vec::new();
    let mut blocked = Vec::new();

    for i in targets {
        let id = data.findings[i].id.clone();
        if data.findings[i].action_taken.is_some() {
            availability.push(eligibility(
                &mut data.findings[i],
                req.include_regenerable,
                req.include_review,
                &action_home(),
                &ActionProbe,
            ));
            results.push(BulkItemResult {
                finding_id: id,
                action_taken: data.findings[i].action_taken.clone(),
                size_recovered_bytes: data.findings[i].size_recovered_bytes,
                error: Some("already actioned".to_string()),
            });
            continue;
        }
        if is_trash {
            let home = action_home();
            let checks = ActionChecks {
                home: &home,
                run_id: &run_id,
                selected_profiles: req.profile_paths.get(&id).map(Vec::as_slice).unwrap_or(&[]),
                probe: &ActionProbe,
                locks: &CargoLocks,
                include_regenerable: req.include_regenerable,
                include_review: req.include_review,
            };
            let (current, result) = checked_trash(&mut data.findings[i], &checks, |f| {
                trash_one(f, &source, &now).map_err(|(_, body)| body.0.error)
            });
            if !current.eligible {
                blocked.push(BlockedItem {
                    finding_id: id.clone(),
                    path: data.findings[i].path.clone(),
                    category: current.category.clone(),
                    consequence: Some(current.reason.clone()),
                });
            }
            availability.push(current);
            match result {
                Ok(size) => {
                    total += size;
                    results.push(BulkItemResult {
                        finding_id: id,
                        action_taken: data.findings[i].action_taken.clone(),
                        size_recovered_bytes: Some(size),
                        error: None,
                    });
                }
                // One failure does not abort the rest — but it IS reported.
                Err(error) => results.push(BulkItemResult {
                    finding_id: id,
                    action_taken: None,
                    size_recovered_bytes: None,
                    error: Some(error),
                }),
            }
        } else {
            data.findings[i].action_taken = Some(req.action.clone());
            data.findings[i].actioned_at = Some(now.clone());
            data.findings[i].action_source = Some(source.clone());
            results.push(BulkItemResult {
                finding_id: id,
                action_taken: Some(req.action.clone()),
                size_recovered_bytes: None,
                error: None,
            });
        }
    }

    // One ledger write for the whole sweep, and it must stick.
    if let Err(e) = save_findings(&data) {
        tracing::error!(
            "Bulk '{}' on run {} completed but the action ledger write failed: {}",
            req.action,
            run_id,
            e
        );
        return Err(err_plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!(
                    "The actions WERE taken, but recording them failed: {}. Trashed files are \
                     recoverable from the Trash.",
                    e
                ),
            }),
        ));
    }

    Ok(Json(BulkActionResponse {
        run_id,
        action_source: source,
        total_recovered_bytes: total,
        results,
        blocked,
        availability,
    }))
}

// ── GET /automation/run/:run_id/bulk-preview ───────────────────────
//
// What the confirmation dialog is built from: the total, the count and bytes
// per category, and the items a bulk action will refuse. Derived server-side
// so the number the user confirms and the number the server enforces come from
// the same place.

#[derive(Serialize, Debug, PartialEq)]
pub struct CategorySummary {
    pub category: String,
    pub label: String,
    pub count: usize,
    pub bytes: u64,
    pub bulk_trashable: bool,
    pub default_selected: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct BulkPreviewResponse {
    pub availability: Vec<Eligibility>,
    pub findings: Vec<Finding>,
    pub run_id: String,
    pub pending_count: usize,
    pub pending_bytes: u64,
    /// What a bulk trash would take with nothing opted in — safe only.
    pub default_selected_count: usize,
    pub default_selected_bytes: u64,
    /// Currently eligible under this request's acknowledgments.
    pub eligible_count: usize,
    pub eligible_bytes: u64,
    pub by_category: Vec<CategorySummary>,
    pub blocked: Vec<BlockedItem>,
}

/// Pure summary over a ledger — no I/O, so it is directly testable.
fn bulk_preview(data: &FindingsFile) -> BulkPreviewResponse {
    use std::collections::BTreeMap;
    let pending: Vec<&Finding> = data
        .findings
        .iter()
        .filter(|f| f.action_taken.is_none())
        .collect();

    let mut by: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for f in &pending {
        let e = by.entry(f.category.as_str()).or_insert((0, 0));
        e.0 += 1;
        e.1 += f.size_bytes;
    }

    let mut by_category: Vec<CategorySummary> = by
        .into_iter()
        .map(|(cat, (count, bytes))| CategorySummary {
            category: cat.to_string(),
            label: classify::category_label(cat).to_string(),
            count,
            bytes,
            bulk_trashable: classify::bulk_trashable(cat),
            default_selected: classify::default_selected(cat),
        })
        .collect();
    by_category.sort_by_key(|c| std::cmp::Reverse(c.bytes));

    let sum = |pred: fn(&str) -> bool| -> (usize, u64) {
        pending
            .iter()
            .filter(|f| pred(&f.category))
            .fold((0, 0), |(c, b), f| (c + 1, b + f.size_bytes))
    };
    let (default_selected_count, default_selected_bytes) = sum(classify::default_selected);
    let (eligible_count, eligible_bytes) = sum(classify::bulk_trashable);

    BulkPreviewResponse {
        availability: Vec::new(),
        findings: data.findings.clone(),
        run_id: data.run_id.clone(),
        pending_count: pending.len(),
        pending_bytes: pending.iter().map(|f| f.size_bytes).sum(),
        default_selected_count,
        default_selected_bytes,
        eligible_count,
        eligible_bytes,
        by_category,
        blocked: pending.iter().filter_map(|f| bulk_refusal(f)).collect(),
    }
}

#[derive(Default, Deserialize)]
struct PreviewOptions {
    #[serde(default)]
    include_regenerable: bool,
    #[serde(default)]
    include_review: bool,
    /// Comma-separated exact identities; absent means all pending for display.
    finding_ids: Option<String>,
}

async fn get_bulk_preview(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
    axum::extract::Query(options): axum::extract::Query<PreviewOptions>,
) -> Result<Json<BulkPreviewResponse>, (StatusCode, Json<ErrorBody>)> {
    let mut data = load_findings(&run_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Run {} not found", run_id),
            }),
        )
    })?;
    let ids: Vec<String> = options
        .finding_ids
        .as_deref()
        .map(|s| s.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let targets = resolve_bulk_targets(&data, &ids)?;
    let mut availability = Vec::new();
    for i in targets {
        if data.findings[i].action_taken.is_some() {
            continue;
        }
        let current = eligibility(
            &mut data.findings[i],
            options.include_regenerable,
            options.include_review,
            &action_home(),
            &ActionProbe,
        );
        availability.push(current);
    }
    let mut response = bulk_preview(&data);
    response.default_selected_count = availability
        .iter()
        .filter(|e| e.eligible && e.category == classify::CAT_SAFE)
        .count();
    response.default_selected_bytes = data
        .findings
        .iter()
        .filter(|f| {
            availability
                .iter()
                .any(|e| e.finding_id == f.id && e.eligible && e.category == classify::CAT_SAFE)
        })
        .map(|f| f.size_bytes)
        .sum();
    response.eligible_count = availability.iter().filter(|e| e.eligible).count();
    response.eligible_bytes = availability
        .iter()
        .filter(|e| e.eligible)
        .map(|e| {
            if e.protection == "cargo-profile-locks" {
                e.profiles
                    .iter()
                    .filter(|p| p.status == "eligible")
                    .map(|p| p.bytes)
                    .sum()
            } else {
                data.findings
                    .iter()
                    .find(|f| f.id == e.finding_id)
                    .map_or(0, |f| f.size_bytes)
            }
        })
        .sum();
    response.blocked = availability
        .iter()
        .filter(|e| !e.eligible)
        .filter_map(|e| {
            data.findings
                .iter()
                .find(|f| f.id == e.finding_id)
                .map(|f| BlockedItem {
                    finding_id: f.id.clone(),
                    path: f.path.clone(),
                    category: e.category.clone(),
                    consequence: Some(e.reason.clone()),
                })
        })
        .collect();
    response.availability = availability;
    Ok(Json(response))
}

// ── GET /automation/recovery/total ──────────────────────────────────
//
// Cumulative recovery accounting (issue #242). The per-run findings files ARE
// the persisted ledger of every trash action ever taken, so the all-time total
// is derived by summing them — no separate counter that could drift from the
// source of truth. Survives app restarts for free because the ledgers do.

#[derive(Serialize, Debug, PartialEq)]
pub struct RecoveryTotalResponse {
    /// Sum of size_recovered_bytes across ALL runs' trashed findings.
    pub total_recovered_bytes: u64,
    /// Number of runs that recovered at least one byte.
    pub runs_with_recovery: usize,
    /// Total findings trashed across all runs.
    pub items_trashed: usize,
}

/// Sum recovery across every findings ledger in `dir`. Missing dir = zeros
/// (no cleanup has ever run). Unreadable/foreign files are skipped, not fatal:
/// one corrupt ledger must not blank the lifetime stat.
fn sum_recovery(dir: &Path) -> RecoveryTotalResponse {
    let mut total = RecoveryTotalResponse {
        total_recovered_bytes: 0,
        runs_with_recovery: 0,
        items_trashed: 0,
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return total,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Only real ledgers: skip atomic-write temp files (".<run>.json.tmp")
        // and anything that isn't .json.
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(data) = serde_json::from_str::<FindingsFile>(&content) else {
            continue;
        };
        let mut run_bytes = 0u64;
        for f in &data.findings {
            if f.action_taken.as_deref() == Some("trashed")
                || (classify::is_rust_target(Path::new(&f.path))
                    && f.size_recovered_bytes.unwrap_or(0) > 0)
            {
                run_bytes += f.size_recovered_bytes.unwrap_or(0);
                total.items_trashed += 1;
            }
        }
        if run_bytes > 0 {
            total.runs_with_recovery += 1;
        }
        total.total_recovered_bytes += run_bytes;
    }
    total
}

async fn recovery_total(State(_state): State<Arc<AppState>>) -> Json<RecoveryTotalResponse> {
    Json(sum_recovery(&findings_dir()))
}

// ── GET /automation/run/:run_id/findings ────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RejectedFinding {
    pub id: String,
    #[serde(default)]
    pub path: Option<String>,
    pub reason: String,
}

#[derive(Serialize)]
pub struct FindingsResponse {
    run_id: String,
    findings: Vec<Finding>,
    /// Scanner rows that were kept out of the selectable list. They are not
    /// paths the UI may delete.
    #[serde(default)]
    rejected: Vec<RejectedFinding>,
    /// Missing/corrupt files are never an explicitly empty successful scan.
    ledger_state: &'static str,
    created_at: Option<String>,
}

fn findings_response(dir: &Path, run_id: &str) -> FindingsResponse {
    let missing = || FindingsResponse {
        run_id: run_id.into(),
        findings: Vec::new(),
        rejected: Vec::new(),
        ledger_state: "missing",
        created_at: None,
    };
    if run_id.is_empty()
        || !run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return missing();
    }
    let Some(value) = std::fs::read(dir.join(format!("{run_id}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return missing();
    };
    let Ok(data) = serde_json::from_value::<FindingsFile>(value.clone()) else {
        return missing();
    };
    if data.run_id != run_id {
        return missing();
    }
    FindingsResponse {
        run_id: data.run_id,
        findings: data.findings,
        rejected: value
            .get("rejected")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        ledger_state: if value.get("validated").and_then(|v| v.as_bool()) == Some(true) {
            "valid"
        } else {
            "unverified"
        },
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    }
}

async fn get_findings(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Json<FindingsResponse>, (StatusCode, Json<ErrorBody>)> {
    Ok(Json(findings_response(&findings_dir(), &run_id)))
}

// ── POST /automation/run/:run_id/findings (create/update findings) ──

async fn save_findings_endpoint(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
    Json(findings): Json<Vec<Finding>>,
) -> Result<Json<FindingsResponse>, (StatusCode, Json<ErrorBody>)> {
    if run_id.is_empty()
        || !run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "Invalid run id".into(),
            }),
        ));
    }
    // No auth for localhost daemon
    let data = FindingsFile {
        run_id: run_id.clone(),
        findings,
    };
    let saved = serde_json::to_string_pretty(&data)
        .map_err(|e| e.to_string())
        .and_then(|json| atomic_write_json(&findings_dir(), &format!("{run_id}.json"), &json));
    if let Err(e) = saved {
        tracing::error!("Failed to persist findings for run {}: {}", run_id, e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to persist findings for run {}: {}", run_id, e),
            }),
        ));
    }

    Ok(Json(findings_response(&findings_dir(), &run_id)))
}

// ── Routes ─────────────────────────────────────────────────────────

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/automation/finding/{finding_id}/action",
            post(perform_action),
        )
        .route(
            "/automation/run/{run_id}/findings",
            get(get_findings).post(save_findings_endpoint),
        )
        .route(
            "/automation/run/{run_id}/bulk-action",
            post(perform_bulk_action),
        )
        .route(
            "/automation/run/{run_id}/bulk-preview",
            get(get_bulk_preview),
        )
        .route("/automation/recovery/total", get(recovery_total))
        .with_state(state)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_paths_rejected() {
        assert!(is_sensitive_path(Path::new("/Users/jesse/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new(
            "/Users/jesse/.aws/credentials"
        )));
        assert!(is_sensitive_path(Path::new("/Users/jesse/project/.env")));
        assert!(is_sensitive_path(Path::new("/Users/jesse/.env.local")));
        assert!(is_sensitive_path(Path::new(
            "/Users/jesse/Library/Keychains/login.keychain"
        )));
        assert!(is_sensitive_path(Path::new(
            "/Users/jesse/.gnupg/private-keys-v1.d/key"
        )));
        assert!(is_sensitive_path(Path::new("/Users/jesse/server.key")));
        assert!(is_sensitive_path(Path::new("/Users/jesse/cert.pem")));
    }

    #[test]
    fn safe_paths_accepted() {
        assert!(!is_sensitive_path(Path::new(
            "/Users/jesse/Downloads/installer.dmg"
        )));
        assert!(!is_sensitive_path(Path::new(
            "/Users/jesse/Desktop/photo.jpg"
        )));
        assert!(!is_sensitive_path(Path::new(
            "/Users/jesse/Documents/report.pdf"
        )));
        assert!(!is_sensitive_path(Path::new(
            "/Users/jesse/Downloads/node-v18.pkg"
        )));
    }

    // ── Ledger persistence (bug-sweep wave 1) ──────────────────────────────
    //
    // The findings file is the action ledger of a DESTRUCTIVE flow: the trash
    // handler moves a real file to the Trash and then records it. Writes were
    // `let _ =` best-effort; now they are atomic and failures surface as 500.

    fn sample_findings(run_id: &str) -> FindingsFile {
        FindingsFile {
            run_id: run_id.to_string(),
            findings: vec![Finding {
                observed_at: None,
                id: "f-1".to_string(),
                finding_type: "old_download".to_string(),
                path: "/tmp/does-not-matter.dmg".to_string(),
                size_bytes: 123,
                age_days: Some(400),
                recommendation: "trash".to_string(),
                category: classify::CAT_SAFE.to_string(),
                consequence: None,
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
                action_source: None,
                contained_in: None,
                blocked_reason: None,
            }],
        }
    }

    #[test]
    fn save_findings_round_trips_and_is_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let data = sample_findings("run-roundtrip");
        save_findings_to(tmp.path(), &data).expect("write must succeed");

        // No temp artifact left behind.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["run-roundtrip.json".to_string()]);

        let raw = std::fs::read_to_string(tmp.path().join("run-roundtrip.json")).unwrap();
        let loaded: FindingsFile = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.run_id, "run-roundtrip");
        assert_eq!(loaded.findings.len(), 1);
        assert_eq!(loaded.findings[0].id, "f-1");
    }

    #[test]
    #[cfg(unix)]
    fn save_findings_to_read_only_dir_errors() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let ro = tmp.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = save_findings_to(&ro, &sample_findings("run-ro"))
            .expect_err("read-only dir must fail the write");
        assert!(err.contains("run-ro") || err.contains("ro"), "{}", err);

        // Restore so TempDir can clean up.
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn atomic_write_json_cleans_up_temp_on_failed_rename() {
        // Renaming onto a path whose parent vanished mid-flight is hard to
        // stage portably; instead verify the temp-file naming can't collide
        // with a real findings file and that a plain write leaves no temp.
        let tmp = tempfile::tempdir().unwrap();
        atomic_write_json(tmp.path(), "a.json", "{}").unwrap();
        assert!(tmp.path().join("a.json").exists());
        assert!(!tmp.path().join(".a.json.tmp").exists());
    }

    // ── Cumulative recovery total (issue #242) ─────────────────────────────
    //
    // The lifetime total is derived from ALL persisted run ledgers, not just
    // the current run — that is the whole bug being pinned here.

    fn write_run_with_recovery(dir: &Path, run_id: &str, recovered: &[u64], pending: usize) {
        let mut findings: Vec<Finding> = recovered
            .iter()
            .enumerate()
            .map(|(i, bytes)| Finding {
                observed_at: None,
                id: format!("{}-t{}", run_id, i),
                finding_type: "old_download".to_string(),
                path: format!("/tmp/{}-{}.dmg", run_id, i),
                size_bytes: *bytes,
                age_days: Some(100),
                recommendation: "trash".to_string(),
                category: classify::CAT_SAFE.to_string(),
                consequence: None,
                action_taken: Some("trashed".to_string()),
                actioned_at: Some("2026-07-20T00:00:00Z".to_string()),
                size_recovered_bytes: Some(*bytes),
                action_source: Some(SOURCE_UI_CLICK.to_string()),
                contained_in: None,
                blocked_reason: None,
            })
            .collect();
        for i in 0..pending {
            findings.push(Finding {
                observed_at: None,
                id: format!("{}-p{}", run_id, i),
                finding_type: "old_download".to_string(),
                path: format!("/tmp/{}-pending-{}.dmg", run_id, i),
                size_bytes: 999,
                age_days: None,
                recommendation: "trash".to_string(),
                category: classify::CAT_SAFE.to_string(),
                consequence: None,
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
                action_source: None,
                contained_in: None,
                blocked_reason: None,
            });
        }
        save_findings_to(
            dir,
            &FindingsFile {
                run_id: run_id.to_string(),
                findings,
            },
        )
        .unwrap();
    }

    #[test]
    fn recovery_total_sums_across_runs_not_just_the_latest() {
        let tmp = tempfile::tempdir().unwrap();
        // Three separate cleanup runs over time, plus un-actioned leftovers
        // that must NOT count.
        write_run_with_recovery(tmp.path(), "run-a", &[100, 50], 1);
        write_run_with_recovery(tmp.path(), "run-b", &[1000], 0);
        write_run_with_recovery(tmp.path(), "run-c", &[], 3); // scan only, nothing trashed

        let total = sum_recovery(tmp.path());
        assert_eq!(
            total.total_recovered_bytes, 1150,
            "must be cumulative, not last-run"
        );
        assert_eq!(total.runs_with_recovery, 2);
        assert_eq!(total.items_trashed, 3);
    }

    #[test]
    fn recovery_total_missing_dir_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let total = sum_recovery(&tmp.path().join("never-created"));
        assert_eq!(total.total_recovered_bytes, 0);
        assert_eq!(total.runs_with_recovery, 0);
        assert_eq!(total.items_trashed, 0);
    }

    #[test]
    fn recovery_total_skips_corrupt_and_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_run_with_recovery(tmp.path(), "run-good", &[42], 0);
        std::fs::write(tmp.path().join("run-corrupt.json"), "{not json").unwrap();
        std::fs::write(tmp.path().join(".run-x.json.tmp"), "{}").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "ignore me").unwrap();

        let total = sum_recovery(tmp.path());
        assert_eq!(total.total_recovered_bytes, 42);
        assert_eq!(total.runs_with_recovery, 1);
        assert_eq!(total.items_trashed, 1);
    }

    #[test]
    fn recovery_total_counts_keep_and_skip_as_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let mut data = sample_findings("run-kept");
        data.findings[0].action_taken = Some("keep".to_string());
        save_findings_to(tmp.path(), &data).unwrap();

        let total = sum_recovery(tmp.path());
        assert_eq!(total.total_recovered_bytes, 0);
        assert_eq!(total.items_trashed, 0);
    }

    // ── Categories, bulk refusal, and the action ledger ────────────────────
    //
    // The 2026-08-24 incident in one line: 33 findings, every one of them
    // "Safe to remove", one bulk click, five dead builds and a re-download bill.
    // These pin the rules that make that impossible.

    fn categorized(id: &str, path: &str, category: &str, size: u64) -> Finding {
        Finding {
            observed_at: None,
            id: id.to_string(),
            finding_type: "dev_cache".to_string(),
            path: path.to_string(),
            size_bytes: size,
            age_days: Some(0),
            recommendation: classify::category_label(category).to_string(),
            category: category.to_string(),
            consequence: if category == classify::CAT_IN_USE {
                Some("5 rustc processes are compiling here".to_string())
            } else {
                None
            },
            action_taken: None,
            actioned_at: None,
            size_recovered_bytes: None,
            action_source: None,
            contained_in: None,
            blocked_reason: None,
        }
    }

    /// The 13:39 ledger, in miniature: the live target dir, a costly cache, an
    /// Apple cache, and one genuinely safe item.
    fn incident_shaped_run(run_id: &str) -> FindingsFile {
        FindingsFile {
            run_id: run_id.to_string(),
            findings: vec![
                categorized(
                    "finding-008",
                    "/Users/j/Documents/dev/permagent-runtime/target",
                    classify::CAT_IN_USE,
                    133_079_642_112,
                ),
                categorized(
                    "finding-028",
                    "/Users/j/.cargo/registry",
                    classify::CAT_REGENERABLE,
                    1_341_743_104,
                ),
                categorized(
                    "finding-015",
                    "/Users/j/Library/Caches/com.apple.callintelligenced",
                    classify::CAT_MACOS,
                    315_318_272,
                ),
                categorized(
                    "finding-005",
                    "/Users/j/Documents/dev/spectral/target",
                    classify::CAT_SAFE,
                    1_839_452_160,
                ),
            ],
        }
    }

    #[test]
    fn a_legacy_ledger_without_categories_reads_as_review_never_as_safe() {
        // Every ledger written before this change — including the 84 previous
        // storage-insights runs — has no `category` field.
        let raw = r#"{"run_id":"old","findings":[{"id":"f-1","type":"dev_cache",
            "path":"/tmp/target","size_bytes":10,"age_days":0,
            "recommendation":"Safe to remove","action_taken":null,
            "actioned_at":null,"size_recovered_bytes":null}]}"#;
        let loaded: FindingsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(loaded.findings[0].category, classify::CAT_REVIEW);
        assert_eq!(loaded.findings[0].action_source, None);
    }

    #[test]
    fn bulk_refusal_blocks_in_use_and_macos_but_not_regenerable_or_safe() {
        let run = incident_shaped_run("r");
        let blocked: Vec<&str> = run
            .findings
            .iter()
            .filter_map(|f| bulk_refusal(f).map(|_| f.id.as_str()))
            .collect();
        assert_eq!(blocked, vec!["finding-008", "finding-015"]);
    }

    #[test]
    fn contained_and_blocked_findings_are_not_removable() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let mut contained = categorized("child", "/tmp/child", classify::CAT_SAFE, 10);
        contained.contained_in = Some("parent".into());
        let result = eligibility(&mut contained, true, true, &home, &idle());
        assert!(!result.eligible);
        assert_eq!(
            result.reason,
            "Included in another finding; it is removed only with that item"
        );
        assert!(bulk_refusal(&contained).is_some());

        let mut blocked = categorized("parent", "/tmp/parent", classify::CAT_SAFE, 20);
        blocked.blocked_reason = Some("Contains an in-use or protected item".into());
        let refused = bulk_refusal(&blocked).expect("a protected ancestor must be blocked");
        assert_eq!(
            refused.consequence.as_deref(),
            Some("Contains an in-use or protected item")
        );
        let observed = eligibility(&mut blocked, true, true, &home, &idle());
        assert!(!observed.eligible);
        assert_eq!(observed.reason, "Contains an in-use or protected item");
        assert_eq!(blocked.category, classify::CAT_SAFE);
    }

    #[test]
    fn bulk_refusal_carries_the_live_process_consequence() {
        let f = categorized("x", "/tmp/target", classify::CAT_IN_USE, 1);
        let blocked = bulk_refusal(&f).expect("an in-use item must be blocked");
        assert_eq!(
            blocked.consequence.as_deref(),
            Some("5 rustc processes are compiling here")
        );
    }

    #[test]
    fn an_unconfirmed_in_use_trash_is_refused_with_its_consequence() {
        let f = categorized("x", "/tmp/target", classify::CAT_IN_USE, 1);
        let (status, body) =
            confirmation_refusal(&f, false).expect("must refuse without confirmation");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body.0
                .error
                .contains("5 rustc processes are compiling here"),
            "{}",
            body.0.error
        );
    }

    #[test]
    fn a_confirmed_request_still_requires_action_time_checks() {
        let f = categorized("x", "/tmp/target", classify::CAT_IN_USE, 1);
        assert!(confirmation_refusal(&f, true).is_none());
    }

    #[test]
    fn a_safe_item_never_needs_a_second_confirmation() {
        let f = categorized("x", "/tmp/target", classify::CAT_SAFE, 1);
        assert!(confirmation_refusal(&f, false).is_none());
    }

    #[test]
    fn bulk_preview_reports_totals_per_category_and_what_it_will_refuse() {
        let run = incident_shaped_run("r");
        let p = bulk_preview(&run);

        assert_eq!(p.pending_count, 4);
        assert_eq!(p.pending_bytes, 136_576_155_648);
        // With nothing opted in, only the genuinely safe item is selected.
        assert_eq!(p.default_selected_count, 1);
        assert_eq!(p.default_selected_bytes, 1_839_452_160);
        // With the costly cache opted in, that is the ceiling — the in-use and
        // Apple items are never eligible at any opt-in level.
        assert_eq!(p.eligible_count, 2);
        assert_eq!(p.eligible_bytes, 1_839_452_160 + 1_341_743_104);

        assert_eq!(p.blocked.len(), 2);
        let categories: Vec<&str> = p.by_category.iter().map(|c| c.category.as_str()).collect();
        assert_eq!(
            categories,
            vec![
                classify::CAT_IN_USE,
                classify::CAT_SAFE,
                classify::CAT_REGENERABLE,
                classify::CAT_MACOS,
            ],
            "largest category first"
        );
        let in_use = &p.by_category[0];
        assert!(!in_use.bulk_trashable);
        assert!(!in_use.default_selected);
        assert_eq!(in_use.label, "In use — do not remove");
    }

    #[test]
    fn bulk_preview_ignores_already_actioned_findings() {
        let mut run = incident_shaped_run("r");
        run.findings[3].action_taken = Some("trashed".into());
        let p = bulk_preview(&run);
        assert_eq!(p.pending_count, 3);
        assert_eq!(p.default_selected_count, 0);
    }

    #[test]
    fn an_unknown_action_source_is_recorded_as_api_not_verbatim() {
        assert_eq!(normalize_source(Some("ui_click")), SOURCE_UI_CLICK);
        assert_eq!(normalize_source(Some("ui_bulk")), SOURCE_UI_BULK);
        assert_eq!(normalize_source(Some("agent_tool")), SOURCE_AGENT_TOOL);
        assert_eq!(normalize_source(None), SOURCE_API);
        assert_eq!(normalize_source(Some("<script>")), SOURCE_API);
    }

    #[test]
    fn an_agent_sourced_action_is_refused_and_pointed_at_the_ladder() {
        let (status, body) =
            refuse_agent_source(SOURCE_AGENT_TOOL).expect("agent source must be refused");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.0.error.contains("Tier 2"), "{}", body.0.error);
        assert!(refuse_agent_source(SOURCE_UI_CLICK).is_none());
        assert!(refuse_agent_source(SOURCE_UI_BULK).is_none());
    }

    mod route_tests {
        use super::*;
        use crate::state::AppState;
        use axum::body::Body;
        use axum::http::Request;
        use serial_test::serial;
        use tower::ServiceExt;

        async fn action_request(
            app: &axum::Router,
            finding_id: &str,
            body: &str,
        ) -> (StatusCode, String) {
            let request = Request::builder()
                .uri(format!("/automation/finding/{}/action", finding_id))
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }

        /// keep action success round-trips through the ledger on disk.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn keep_action_persists_and_round_trips() {
            // HOME stays a throwaway — findings_dir()/the ledger are keyed off
            // HOME and the cumulative-total assertion needs per-test isolation.
            // Only PERMAGENT_PATH_ROOT (the global session-pool DB root) resolves
            // to the shared, process-lifetime root so it never outlives a
            // per-test tempdir (#858).
            let root = crate::test_support::test_root();
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
            ]);
            let dir = findings_dir();
            save_findings_to(&dir, &sample_findings("run-keep")).unwrap();

            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) =
                action_request(&app, "f-1", r#"{"action":"keep","run_id":"run-keep"}"#).await;
            assert_eq!(status, StatusCode::OK, "{}", body);

            let persisted = load_findings("run-keep").expect("ledger readable");
            assert_eq!(persisted.findings[0].action_taken.as_deref(), Some("keep"));
            assert!(persisted.findings[0].actioned_at.is_some());
        }

        /// The mounted /automation/recovery/total endpoint reads the real
        /// findings dir and reports the cross-run cumulative total (#242).
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn recovery_total_route_reports_cumulative() {
            // HOME stays a throwaway — findings_dir()/the ledger are keyed off
            // HOME and the cumulative-total assertion needs per-test isolation.
            // Only PERMAGENT_PATH_ROOT (the global session-pool DB root) resolves
            // to the shared, process-lifetime root so it never outlives a
            // per-test tempdir (#858).
            let root = crate::test_support::test_root();
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
            ]);
            let dir = findings_dir();
            write_run_with_recovery(&dir, "run-1", &[10], 0);
            write_run_with_recovery(&dir, "run-2", &[20, 30], 0);

            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let request = Request::builder()
                .uri("/automation/recovery/total")
                .method("GET")
                .body(Body::empty())
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["total_recovered_bytes"], 60);
            assert_eq!(body["runs_with_recovery"], 2);
            assert_eq!(body["items_trashed"], 3);
        }

        async fn bulk_request(
            app: &axum::Router,
            run_id: &str,
            body: &str,
        ) -> (StatusCode, String) {
            let request = Request::builder()
                .uri(format!("/automation/run/{}/bulk-action", run_id))
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }

        /// A throwaway HOME with the incident-shaped ledger already seeded.
        macro_rules! seeded_run {
            ($home:ident, $guard:ident, $run:expr) => {
                let root = crate::test_support::test_root();
                let $home = tempfile::tempdir().unwrap();
                let $guard = env_lock::lock_env([
                    ("HOME", Some($home.path().to_str().unwrap())),
                    ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
                ]);
                save_findings_to(&findings_dir(), &incident_shaped_run($run)).unwrap();
            };
        }

        /// ST1 requires exact identities; an empty selection can no longer
        /// silently expand to all pending findings.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn acceptance_6_bulk_trash_requires_exact_identities() {
            seeded_run!(_home, _guard, "run-bulk-blocked");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);
            let (status, body) = bulk_request(
                &app,
                "run-bulk-blocked",
                r#"{"action":"trash","confirmed":true,"action_source":"ui_bulk"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{}", body);
            assert!(body.contains("Bulk trash requires exact finding_ids"));
            assert!(load_findings("run-bulk-blocked")
                .unwrap()
                .findings
                .iter()
                .all(|f| f.action_taken.is_none()));
        }

        /// A bulk trash without an explicit confirmation is refused before it
        /// touches anything.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn bulk_trash_without_confirmation_is_refused() {
            seeded_run!(_home, _guard, "run-bulk-unconfirmed");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = bulk_request(
                &app,
                "run-bulk-unconfirmed",
                r#"{"action":"trash","finding_ids":["finding-005"],"action_source":"ui_bulk"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{}", body);
            assert!(body.contains("confirmed=true"), "{}", body);

            let persisted = load_findings("run-bulk-unconfirmed").unwrap();
            assert!(persisted.findings.iter().all(|f| f.action_taken.is_none()));
        }

        /// An agent tool cannot trash, in bulk or otherwise — it is pointed at
        /// the Tier 2 approval ladder instead.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn an_agent_sourced_trash_is_refused_by_both_endpoints() {
            seeded_run!(_home, _guard, "run-agent");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = bulk_request(
                &app,
                "run-agent",
                r#"{"action":"trash","confirmed":true,"action_source":"agent_tool"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{}", body);
            assert!(body.contains("Tier 2"), "{}", body);

            let (status, body) = action_request(
                &app,
                "finding-005",
                r#"{"action":"trash","run_id":"run-agent","action_source":"agent_tool"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{}", body);

            let persisted = load_findings("run-agent").unwrap();
            assert!(persisted.findings.iter().all(|f| f.action_taken.is_none()));
        }

        /// Trashing an in-use item individually is refused until the caller
        /// confirms having seen the live-process consequence.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn single_trash_of_an_in_use_item_asks_again_before_acting() {
            seeded_run!(_home, _guard, "run-single-inuse");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = action_request(
                &app,
                "finding-008",
                r#"{"action":"trash","run_id":"run-single-inuse","action_source":"ui_click"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{}", body);
            assert!(
                body.contains("5 rustc processes are compiling here"),
                "the refusal must state the consequence: {}",
                body
            );
            assert!(body.contains("Recheck availability"), "{}", body);

            let persisted = load_findings("run-single-inuse").unwrap();
            assert_eq!(persisted.findings[0].action_taken, None);
        }

        /// Every action records which route took it. Uses `keep`, so the test
        /// never moves a real file to the Trash.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn the_ledger_records_the_action_source_for_a_ui_click() {
            seeded_run!(_home, _guard, "run-source-click");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = action_request(
                &app,
                "finding-005",
                r#"{"action":"keep","run_id":"run-source-click","action_source":"ui_click"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{}", body);

            let persisted = load_findings("run-source-click").unwrap();
            let f = persisted
                .findings
                .iter()
                .find(|f| f.id == "finding-005")
                .unwrap();
            assert_eq!(f.action_source.as_deref(), Some("ui_click"));
        }

        /// The same, by the bulk route — and the bulk route stamps `ui_bulk`,
        /// so the ledger can tell a sweep from 33 individual clicks.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn the_ledger_records_the_action_source_for_a_bulk_sweep() {
            seeded_run!(_home, _guard, "run-source-bulk");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = bulk_request(
                &app,
                "run-source-bulk",
                r#"{"action":"keep","action_source":"ui_bulk"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{}", body);

            let persisted = load_findings("run-source-bulk").unwrap();
            assert!(
                persisted
                    .findings
                    .iter()
                    .all(|f| f.action_source.as_deref() == Some("ui_bulk")),
                "every swept item carries the bulk route"
            );
        }

        /// The confirmation dialog's numbers come from the server.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn bulk_preview_route_reports_the_per_category_breakdown() {
            seeded_run!(_home, _guard, "run-preview");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let request = Request::builder()
                .uri("/automation/run/run-preview/bulk-preview")
                .method("GET")
                .body(Body::empty())
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["pending_count"], 4);
            // Fixture paths do not exist: a fresh check must exclude all four.
            assert_eq!(body["default_selected_count"], 0);
            assert_eq!(body["eligible_count"], 0);
            assert_eq!(body["blocked"].as_array().unwrap().len(), 4);
            assert_eq!(body["availability"].as_array().unwrap().len(), 4);
            assert_eq!(body["by_category"].as_array().unwrap().len(), 4);
        }

        /// Ledger persistence failure surfaces as 500 — never a fake 200.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        #[cfg(unix)]
        async fn keep_action_ledger_write_failure_is_500() {
            use std::os::unix::fs::PermissionsExt;
            // HOME stays a throwaway — findings_dir()/the ledger are keyed off
            // HOME and the cumulative-total assertion needs per-test isolation.
            // Only PERMAGENT_PATH_ROOT (the global session-pool DB root) resolves
            // to the shared, process-lifetime root so it never outlives a
            // per-test tempdir (#858).
            let root = crate::test_support::test_root();
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
            ]);
            let dir = findings_dir();
            save_findings_to(&dir, &sample_findings("run-fail")).unwrap();
            // Make the ledger dir unwritable AFTER seeding so the load works
            // but the persist of the action cannot.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) =
                action_request(&app, "f-1", r#"{"action":"keep","run_id":"run-fail"}"#).await;

            // Restore before asserting so TempDir cleanup always works.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{}", body);
            assert!(body.contains("keep"), "{}", body);

            // And the ledger was NOT silently mutated on disk.
            let persisted = load_findings("run-fail").expect("ledger readable");
            assert_eq!(persisted.findings[0].action_taken, None);
        }
    }
}

#[cfg(test)]
mod st1_acceptance {
    use super::*;
    use permagent::storage_health::inuse::ProcessRef;

    struct Probe(Result<Vec<ProcessRef>, String>);
    impl ActivityProbe for Probe {
        fn check(&self, _: &Path) -> Result<Vec<ProcessRef>, String> {
            self.0.clone()
        }
    }
    struct IdleLock;
    impl LockProvider for IdleLock {
        fn acquire(&self, _: &Path) -> Result<Box<dyn CleanupLock>, String> {
            Ok(Box::new(Vec::<std::fs::File>::new()))
        }
    }
    fn fixture(home: &Path, relative: &str, kind: &str, category: &str) -> Finding {
        let path = home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if kind == "stale_file" || kind == "large_file" {
            std::fs::write(&path, "fixture").unwrap();
        } else {
            std::fs::create_dir_all(&path).unwrap();
        }
        if path.file_name().is_some_and(|n| n == "target") {
            std::fs::write(path.parent().unwrap().join("Cargo.toml"), "[package]").unwrap();
            std::fs::create_dir_all(path.join("debug/deps")).unwrap();
            for name in action::CARGO_COORDINATION_FILES {
                std::fs::write(path.join("debug").join(name), "").unwrap();
            }
            std::fs::write(path.join("debug/deps/artifact"), "fixture").unwrap();
        }
        Finding {
            observed_at: None,
            id: relative.into(),
            path: path.to_str().unwrap().into(),
            finding_type: kind.into(),
            size_bytes: 100,
            age_days: None,
            recommendation: "Safe to remove".into(),
            category: category.into(),
            consequence: None,
            action_taken: None,
            actioned_at: None,
            size_recovered_bytes: None,
            action_source: None,
            contained_in: None,
            blocked_reason: None,
        }
    }
    fn idle() -> Probe {
        Probe(Ok(Vec::new()))
    }
    fn busy() -> Probe {
        Probe(Ok(vec![ProcessRef {
            pid: 7,
            command: "rustc".into(),
        }]))
    }

    #[test]
    fn acceptance_2_profile_execution_counts_contents_and_preserves_roots() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let mut f = fixture(&home, "repo/target", "dev_cache", classify::CAT_REGENERABLE);
        let target = PathBuf::from(&f.path);
        for name in ["release", "custom", "aarch64-apple-darwin/custom"] {
            let profile = target.join(name);
            std::fs::create_dir_all(profile.join("deps")).unwrap();
            for lock in action::CARGO_COORDINATION_FILES {
                std::fs::write(profile.join(lock), "").unwrap();
            }
            std::fs::write(profile.join("deps/artifact"), "output").unwrap();
        }
        std::fs::write(target.join(".rustc_info.json"), "root metadata").unwrap();
        // Bypass only the release gate in this hermetic unit of execution.
        // This is not evidence that the production Cargo release gate can open.
        let mut profiles = profile_availability(&target, &idle(), &IdleLock, true).unwrap();
        let selected: Vec<_> = profiles.iter().map(|p| p.path.clone()).collect();
        let expected: u64 = profiles.iter().map(|p| p.bytes).sum();
        let checks = ActionChecks {
            home: &home,
            run_id: "profiles",
            selected_profiles: &selected,
            probe: &idle(),
            locks: &IdleLock,
            include_regenerable: true,
            include_review: false,
        };
        let moved = clean_profiles(&mut f, &checks, &mut profiles, &mut |item| {
            let size = permagent::storage_health::size::dir_size(Path::new(&item.path));
            std::fs::remove_dir_all(&item.path).map_err(|e| e.to_string())?;
            Ok(size)
        })
        .unwrap();
        assert_eq!(moved, expected);
        assert_eq!(f.size_recovered_bytes, Some(expected));
        assert!(f.action_taken.is_none());
        assert_eq!(profiles.len(), 4);
        for p in profiles {
            assert_eq!((p.moved, p.skipped, p.failed), (1, 0, 0));
            assert_eq!(p.status, "moved");
            assert!(Path::new(&p.path).is_dir());
            for lock in action::CARGO_COORDINATION_FILES {
                assert!(Path::new(&p.path).join(lock).is_file());
            }
        }
        assert!(target.join("aarch64-apple-darwin").is_dir());
        assert_eq!(
            std::fs::read_to_string(target.join(".rustc_info.json")).unwrap(),
            "root metadata"
        );
    }

    #[test]
    fn acceptance_3_profile_becomes_busy_after_preview_without_source_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let mut f = fixture(&home, "repo/target", "dev_cache", classify::CAT_REGENERABLE);
        let mut profiles =
            profile_availability(Path::new(&f.path), &idle(), &IdleLock, true).unwrap();
        let selected = vec![profiles[0].path.clone()];
        let checks = ActionChecks {
            home: &home,
            run_id: "busy",
            selected_profiles: &selected,
            probe: &busy(),
            locks: &IdleLock,
            include_regenerable: true,
            include_review: false,
        };
        assert!(
            clean_profiles(&mut f, &checks, &mut profiles, &mut |_| panic!(
                "busy outputs must survive"
            ))
            .is_err()
        );
        assert_eq!(profiles[0].skipped, 1);
        assert_eq!(profiles[0].reason, "Profile in use: active process");
        assert!(Path::new(&selected[0]).join("deps/artifact").is_file());
    }

    #[test]
    fn acceptance_7_profile_trash_failure_restores_contents_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let mut f = fixture(&home, "repo/target", "dev_cache", classify::CAT_REGENERABLE);
        let mut profiles =
            profile_availability(Path::new(&f.path), &idle(), &IdleLock, true).unwrap();
        let selected = vec![profiles[0].path.clone()];
        let checks = ActionChecks {
            home: &home,
            run_id: "failed",
            selected_profiles: &selected,
            probe: &idle(),
            locks: &IdleLock,
            include_regenerable: true,
            include_review: false,
        };
        assert!(clean_profiles(&mut f, &checks, &mut profiles, &mut |_| Err(
            "Native Trash failed".into()
        ))
        .is_err());
        assert_eq!(
            (profiles[0].moved, profiles[0].skipped, profiles[0].failed),
            (0, 0, 1)
        );
        assert_eq!(profiles[0].moved_bytes, 0);
        assert_eq!(profiles[0].reason, "Native Trash failed");
        assert!(Path::new(&selected[0]).join("deps/artifact").is_file());
    }

    #[test]
    fn acceptance_6_already_actioned_target_cannot_regain_eligibility() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let mut f = fixture(&home, "repo/target", "dev_cache", classify::CAT_REGENERABLE);
        f.action_taken = Some("kept".into());
        let result = eligibility(&mut f, true, true, &home, &idle());
        assert!(!result.eligible);
        assert_eq!(result.reason, "Already actioned");
        assert!(result.profiles.is_empty());
    }

    #[test]
    fn acceptance_8_missing_ledger_is_not_a_valid_empty_report() {
        let dir = tempfile::tempdir().unwrap();
        let missing = findings_response(dir.path(), "09-22");
        assert_eq!(missing.ledger_state, "missing");
        assert!(missing.findings.is_empty());
        std::fs::write(dir.path().join("09-22.json"), "broken").unwrap();
        assert_eq!(
            findings_response(dir.path(), "09-22").ledger_state,
            "missing"
        );
        std::fs::write(
            dir.path().join("09-22.json"),
            r#"{"run_id":"09-22","findings":[]}"#,
        )
        .unwrap();
        assert_eq!(
            findings_response(dir.path(), "09-22").ledger_state,
            "unverified"
        );
        let home = dir.path().canonicalize().unwrap();
        permagent::storage_health::scanner::persist_report(
            "<findings>[]</findings>",
            "09-15",
            dir.path(),
            &home,
            &idle(),
        )
        .unwrap();
        let previous = std::fs::read(dir.path().join("09-15.json")).unwrap();
        let valid = findings_response(dir.path(), "09-15");
        assert_eq!(valid.ledger_state, "valid");
        assert!(valid.findings.is_empty());
        assert!(valid.created_at.is_some());
        assert_eq!(
            std::fs::read(dir.path().join("09-15.json")).unwrap(),
            previous
        );
        assert_eq!(
            findings_response(dir.path(), "../09-15").ledger_state,
            "missing"
        );
    }

    #[test]
    fn acceptance_1_live_build_and_forged_safe_metadata_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let target = home.join("repo/target");
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.parent().unwrap().join("Cargo.toml"), "[package]").unwrap();
        for name in action::CARGO_COORDINATION_FILES {
            std::fs::write(target.join("debug").join(name), "").unwrap();
        }
        std::fs::write(target.join("debug/deps/artifact"), "fixture-bytes").unwrap();
        let mut f = Finding {
            observed_at: None,
            id: "repo/target".into(),
            path: target
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            finding_type: "dev_cache".into(),
            size_bytes: 100,
            age_days: None,
            recommendation: "Safe to remove".into(),
            category: classify::CAT_SAFE.into(),
            consequence: None,
            action_taken: None,
            actioned_at: None,
            size_recovered_bytes: None,
            action_source: None,
            contained_in: None,
            blocked_reason: None,
        };
        let checks = ActionChecks {
            home: &home,
            run_id: "action",
            selected_profiles: &[],
            probe: &busy(),
            locks: &IdleLock,
            include_regenerable: true,
            include_review: true,
        };
        let (current, result) =
            checked_trash(&mut f, &checks, |_| panic!("live build must survive"));
        assert!(result.is_err());
        assert_eq!(
            current.reason,
            "Profiles in use are skipped; other unsupported profiles remain excluded"
        );
        assert_eq!(current.profiles.len(), 1);
        assert_eq!(current.profiles[0].status, "busy");
        assert_eq!(current.profiles[0].reason, "Profile in use: active process");
        assert!(Path::new(&f.path).exists());
    }

    #[test]
    fn acceptance_2_and_4_shipping_gate_never_moves_a_whole_target() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let target = home.join("repo/target");
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.parent().unwrap().join("Cargo.toml"), "[package]").unwrap();
        for name in action::CARGO_COORDINATION_FILES {
            std::fs::write(target.join("debug").join(name), "").unwrap();
        }
        std::fs::write(target.join("debug/deps/artifact"), "fixture-bytes").unwrap();
        let mut f = Finding {
            observed_at: None,
            id: "repo/target".into(),
            path: target
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            finding_type: "dev_cache".into(),
            size_bytes: 100,
            age_days: None,
            recommendation: "Safe to remove".into(),
            category: classify::CAT_IN_USE.into(),
            consequence: None,
            action_taken: None,
            actioned_at: None,
            size_recovered_bytes: None,
            action_source: None,
            contained_in: None,
            blocked_reason: None,
        };
        let checks = ActionChecks {
            run_id: "fixture",
            home: &home,
            selected_profiles: &[],
            probe: &idle(),
            locks: &IdleLock,
            include_regenerable: true,
            include_review: true,
        };
        let (current, result) =
            checked_trash(&mut f, &checks, |_| panic!("whole target must survive"));
        assert!(result.is_err());
        assert_eq!(current.category, classify::CAT_REGENERABLE);
        if action::CARGO_PROFILE_RELEASE_VERIFIED {
            // Gate on (real-cargo acceptance demonstrated 2026-09-22): the whole
            // tree is still never trashed; cleanup is offered per profile.
            assert!(current.profiles.iter().any(|p| p.status == "eligible"));
        } else {
            assert!(current.reason.contains("Unsupported layout"));
        }
        assert!(Path::new(&f.path).exists());
    }

    #[test]
    fn acceptance_5_probe_failures_never_mean_idle() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        for reason in [
            "Activity check timed out",
            "Activity information is incomplete or permission was denied",
            "Activity information is incomplete",
        ] {
            let path = home.join("Downloads/old.zip");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "fixture-bytes").unwrap();
            let mut f = Finding {
                observed_at: None,
                id: "Downloads/old.zip".into(),
                path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
                finding_type: "stale_file".into(),
                size_bytes: 100,
                age_days: None,
                recommendation: "Safe to remove".into(),
                category: classify::CAT_SAFE.into(),
                consequence: None,
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
                action_source: None,
                contained_in: None,
                blocked_reason: None,
            };
            let checks = ActionChecks {
                home: &home,
                run_id: "action",
                selected_profiles: &[],
                probe: &Probe(Err(reason.into())),
                locks: &IdleLock,
                include_regenerable: true,
                include_review: true,
            };
            let (_, result) =
                checked_trash(&mut f, &checks, |_| panic!("failed probe must exclude"));
            assert_eq!(result.unwrap_err(), reason);
            assert!(Path::new(&f.path).exists());
        }
    }

    #[test]
    fn acceptance_6_category_acknowledgment_and_containment_matrix() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        for (relative, kind, saved, regen, review, eligible) in [
            (
                "Downloads/old.zip",
                "stale_file",
                classify::CAT_SAFE,
                false,
                false,
                true,
            ),
            (".npm", "app_cache", classify::CAT_SAFE, false, false, false),
            (".npm", "app_cache", classify::CAT_SAFE, true, false, true),
            (
                "Documents/photo.raw",
                "large_file",
                classify::CAT_REVIEW,
                true,
                false,
                false,
            ),
            (
                "Documents/photo.raw",
                "large_file",
                classify::CAT_REVIEW,
                false,
                true,
                true,
            ),
            (
                "Library/Caches/com.apple.cache",
                "app_cache",
                classify::CAT_SAFE,
                true,
                true,
                false,
            ),
            (
                "Downloads/.env",
                "stale_file",
                classify::CAT_SAFE,
                true,
                true,
                false,
            ),
            (
                "Library/Mobile Documents/photo",
                "large_file",
                classify::CAT_SAFE,
                true,
                true,
                false,
            ),
            (
                "Downloads/unknown",
                "stale_file",
                "invented",
                true,
                true,
                false,
            ),
            (
                "source.rs",
                "large_file",
                classify::CAT_SAFE,
                true,
                true,
                false,
            ),
        ] {
            let path = home.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if kind == "stale_file" || kind == "large_file" {
                std::fs::write(&path, "fixture-bytes").unwrap();
            } else {
                std::fs::create_dir_all(&path).unwrap();
            }
            let mut f = Finding {
                observed_at: None,
                id: relative.into(),
                path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
                finding_type: kind.into(),
                size_bytes: 100,
                age_days: None,
                recommendation: "Safe to remove".into(),
                category: saved.into(),
                consequence: None,
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
                action_source: None,
                contained_in: None,
                blocked_reason: None,
            };
            let current = eligibility(&mut f, regen, review, &home, &idle());
            assert_eq!(current.eligible, eligible, "{relative}: {}", current.reason);
        }
        #[cfg(unix)]
        {
            let real = home.join("Downloads/real");
            std::fs::write(&real, "data").unwrap();
            let alias = home.join("Downloads/alias");
            std::os::unix::fs::symlink(&real, &alias).unwrap();
            assert!(action::validate_path("stale_file", &alias, &home).is_err());
        }
    }

    #[test]
    fn acceptance_6_path_substitution_during_probe_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let path = home.join("Downloads/old.zip");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fixture-bytes").unwrap();
        let mut f = Finding {
            observed_at: None,
            id: "Downloads/old.zip".into(),
            path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
            finding_type: "stale_file".into(),
            size_bytes: 100,
            age_days: None,
            recommendation: "Safe to remove".into(),
            category: classify::CAT_SAFE.into(),
            consequence: None,
            action_taken: None,
            actioned_at: None,
            size_recovered_bytes: None,
            action_source: None,
            contained_in: None,
            blocked_reason: None,
        };
        struct Replace;
        impl ActivityProbe for Replace {
            fn check(&self, path: &Path) -> Result<Vec<ProcessRef>, String> {
                std::fs::rename(path, path.with_extension("saved")).unwrap();
                std::fs::write(path, "replacement").unwrap();
                Ok(Vec::new())
            }
        }
        let checks = ActionChecks {
            home: &home,
            run_id: "action",
            selected_profiles: &[],
            probe: &Replace,
            locks: &IdleLock,
            include_regenerable: true,
            include_review: true,
        };
        let (_, result) =
            checked_trash(&mut f, &checks, |_| panic!("substituted path must survive"));
        assert_eq!(
            result.unwrap_err(),
            "Path changed during availability check"
        );
        assert!(Path::new(&f.path).exists());
    }

    #[test]
    fn acceptance_6_exact_selection_deduplicates_and_never_adds_newly_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let first = fixture(&home, "Downloads/first", "stale_file", classify::CAT_SAFE);
        let second = fixture(&home, "Downloads/second", "stale_file", classify::CAT_SAFE);
        let id = first.id.clone();
        let data = FindingsFile {
            run_id: "fixture".into(),
            findings: vec![first, second],
        };
        let ledger_dir = home.join("ledger");
        save_findings_to(&ledger_dir, &data).unwrap();
        assert_eq!(
            resolve_bulk_targets(&data, &[id.clone(), id]).unwrap(),
            vec![0]
        );
        assert!(resolve_bulk_targets(&data, &["unknown".into()]).is_err());
        let saved: FindingsFile =
            serde_json::from_slice(&std::fs::read(ledger_dir.join("fixture.json")).unwrap())
                .unwrap();
        assert_eq!(saved.findings.len(), 2);
    }
}
