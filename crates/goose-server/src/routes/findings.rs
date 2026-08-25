//! Action affordance endpoints for automation findings.
//!
//! Findings are cleanup/optimization items produced by recipes like Storage
//! Insights. Users act on them via Trash/Keep buttons in the Automate tab.
//!
//! Files go to native macOS Trash via the `trash` crate — NOT shell rm or
//! filesystem mv to ~/.Trash. Native Trash preserves the original path for
//! restoration, integrates with iCloud/Time Machine, and supports Finder undo.

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
        _ => return false,
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
        _ => false, // Can't determine — allow trash
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
    findings_dir().join(format!("{}.json", run_id))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
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
}

/// Ledgers written before categories existed carry no `category`. Nothing
/// about them was ever verified, so they read as "review", not as "safe".
fn default_category() -> String {
    classify::CAT_REVIEW.to_string()
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
    if classify::bulk_trashable(&f.category) {
        return None;
    }
    Some(BlockedItem {
        finding_id: f.id.clone(),
        path: f.path.clone(),
        category: f.category.clone(),
        consequence: f.consequence.clone(),
    })
}

/// Refuse an individual trash of an in-use / macOS-managed item that has not
/// been confirmed a second time, and say why in the user's words.
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
                "{} is {} — {}. Re-send with confirmed=true to trash it anyway.",
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
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| format!("serialize findings for run {}: {}", data.run_id, e))?;
    atomic_write_json(dir, &format!("{}.json", data.run_id), &json)
}

fn save_findings(data: &FindingsFile) -> Result<(), String> {
    save_findings_to(&findings_dir(), data)
}

#[derive(Serialize)]
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

    // Use the scan-time size_bytes (recursive for directories) rather
    // than re-computing from metadata — metadata.len() on a directory
    // returns only the inode size (~256 bytes), not its content.
    let size = f.size_bytes;

    // Native macOS Trash via the `trash` crate.
    // This preserves original path for Finder restoration, integrates
    // with iCloud-synced Trash, and works with Time Machine. Do NOT
    // replace with shell rm or mv to ~/.Trash — those bypass native
    // Trash metadata and break restoration/undo.
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

            let size = trash_one(&mut data.findings[idx], &source, &now)?;

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
//   * it REFUSES outright, taking nothing, if any item is "In use" or
//     "Managed by macOS". Not "skips them" — refuses, so the caller has to
//     look at what it nearly did;
//   * it writes the ledger once, stamped with `action_source`.

#[derive(Deserialize)]
pub struct BulkActionRequest {
    /// "trash" | "keep" | "skip".
    action: String,
    /// The findings to act on. Empty means every un-actioned finding.
    #[serde(default)]
    finding_ids: Vec<String>,
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
    pub run_id: String,
    pub action_source: String,
    pub total_recovered_bytes: u64,
    pub results: Vec<BulkItemResult>,
    /// Always empty on success: a blocked item aborts the whole batch.
    pub blocked: Vec<BlockedItem>,
}

#[derive(Serialize)]
struct BulkRefusalBody {
    error: String,
    blocked: Vec<BlockedItem>,
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
            Some(i) => out.push(i),
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

        // Refuse the WHOLE batch if it contains anything a bulk action must
        // never touch. Silently dropping them would let a sweep look clean
        // while the user never learns what it nearly deleted.
        let blocked: Vec<BlockedItem> = targets
            .iter()
            .filter_map(|i| bulk_refusal(&data.findings[*i]))
            .collect();
        if !blocked.is_empty() {
            let names: Vec<&str> = blocked.iter().map(|b| b.path.as_str()).take(3).collect();
            let body = BulkRefusalBody {
                error: format!(
                    "Refusing the whole batch: {} item(s) cannot be trashed in bulk ({}{}). \
                     Trash them one at a time if you really mean to.",
                    blocked.len(),
                    names.join(", "),
                    if blocked.len() > names.len() {
                        ", …"
                    } else {
                        ""
                    }
                ),
                blocked,
            };
            return Err((
                StatusCode::CONFLICT,
                axum::Json(serde_json::to_value(body).unwrap_or_else(|_| serde_json::json!({}))),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    let mut results = Vec::with_capacity(targets.len());
    let mut total = 0u64;

    for i in targets {
        let id = data.findings[i].id.clone();
        if data.findings[i].action_taken.is_some() {
            results.push(BulkItemResult {
                finding_id: id,
                action_taken: data.findings[i].action_taken.clone(),
                size_recovered_bytes: data.findings[i].size_recovered_bytes,
                error: Some("already actioned".to_string()),
            });
            continue;
        }
        if is_trash {
            match trash_one(&mut data.findings[i], &source, &now) {
                Ok(size) => {
                    total += size;
                    results.push(BulkItemResult {
                        finding_id: id,
                        action_taken: Some("trashed".into()),
                        size_recovered_bytes: Some(size),
                        error: None,
                    });
                }
                // One failure does not abort the rest — but it IS reported.
                Err((_, body)) => results.push(BulkItemResult {
                    finding_id: id,
                    action_taken: None,
                    size_recovered_bytes: None,
                    error: Some(body.0.error),
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
        blocked: Vec::new(),
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
    pub run_id: String,
    pub pending_count: usize,
    pub pending_bytes: u64,
    /// What a bulk trash would take with nothing opted in — safe only.
    pub default_selected_count: usize,
    pub default_selected_bytes: u64,
    /// What a bulk trash could take at most, with regenerable caches opted in.
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

async fn get_bulk_preview(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Json<BulkPreviewResponse>, (StatusCode, Json<ErrorBody>)> {
    let data = load_findings(&run_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Run {} not found", run_id),
            }),
        )
    })?;
    Ok(Json(bulk_preview(&data)))
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
            if f.action_taken.as_deref() == Some("trashed") {
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

#[derive(Serialize)]
pub struct FindingsResponse {
    run_id: String,
    findings: Vec<Finding>,
}

async fn get_findings(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Json<FindingsResponse>, (StatusCode, Json<ErrorBody>)> {
    // No auth required for read-only findings (localhost daemon)
    match load_findings(&run_id) {
        Some(data) => Ok(Json(FindingsResponse {
            run_id: data.run_id,
            findings: data.findings,
        })),
        None => Ok(Json(FindingsResponse {
            run_id,
            findings: vec![],
        })),
    }
}

// ── POST /automation/run/:run_id/findings (create/update findings) ──

async fn save_findings_endpoint(
    State(_state): State<Arc<AppState>>,
    AxumPath(run_id): AxumPath<String>,
    Json(findings): Json<Vec<Finding>>,
) -> Result<Json<FindingsResponse>, (StatusCode, Json<ErrorBody>)> {
    // No auth for localhost daemon
    let data = FindingsFile {
        run_id: run_id.clone(),
        findings,
    };
    if let Err(e) = save_findings(&data) {
        tracing::error!("Failed to persist findings for run {}: {}", run_id, e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to persist findings for run {}: {}", run_id, e),
            }),
        ));
    }

    Ok(Json(FindingsResponse {
        run_id: data.run_id,
        findings: data.findings,
    }))
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
            })
            .collect();
        for i in 0..pending {
            findings.push(Finding {
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
    fn a_confirmed_in_use_trash_is_allowed_through() {
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

        /// THE regression. A bulk trash that includes the live target dir and
        /// an Apple cache takes NOTHING — not "skips them", refuses the batch.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        async fn bulk_trash_refuses_the_whole_batch_when_an_in_use_item_is_included() {
            seeded_run!(_home, _guard, "run-bulk-blocked");
            let state = AppState::new(true).await.unwrap();
            let app = routes(state);

            let (status, body) = bulk_request(
                &app,
                "run-bulk-blocked",
                r#"{"action":"trash","confirmed":true,"action_source":"ui_bulk"}"#,
            )
            .await;

            assert_eq!(status, StatusCode::CONFLICT, "{}", body);
            let json: serde_json::Value = serde_json::from_str(&body).unwrap();
            let blocked = json["blocked"].as_array().expect("blocked list");
            assert_eq!(blocked.len(), 2, "{}", body);
            assert_eq!(blocked[0]["finding_id"], "finding-008");
            assert_eq!(
                blocked[0]["consequence"],
                "5 rustc processes are compiling here"
            );
            assert_eq!(blocked[1]["category"], classify::CAT_MACOS);

            // And nothing was touched.
            let persisted = load_findings("run-bulk-blocked").unwrap();
            assert!(
                persisted.findings.iter().all(|f| f.action_taken.is_none()),
                "a refused batch must take no action at all"
            );
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
            assert!(body.contains("confirmed=true"), "{}", body);

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
            assert_eq!(body["default_selected_count"], 1);
            assert_eq!(body["eligible_count"], 2);
            assert_eq!(body["blocked"].as_array().unwrap().len(), 2);
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
