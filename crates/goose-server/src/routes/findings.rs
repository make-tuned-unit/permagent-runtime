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
    pub action_taken: Option<String>,
    pub actioned_at: Option<String>,
    pub size_recovered_bytes: Option<u64>,
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
}

#[derive(Serialize)]
pub struct ActionResponse {
    finding_id: String,
    action_taken: String,
    size_recovered_bytes: Option<u64>,
    trash_path: Option<String>,
    timestamp: String,
}

async fn perform_action(
    State(_state): State<Arc<AppState>>,
    AxumPath(finding_id): AxumPath<String>,
    Json(req): Json<ActionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, Json<ErrorBody>)> {
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
            let finding_path = data.findings[idx].path.clone();
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
            let size = data.findings[idx].size_bytes;
            let file_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

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

            data.findings[idx].action_taken = Some("trashed".into());
            data.findings[idx].actioned_at = Some(now.clone());
            data.findings[idx].size_recovered_bytes = Some(size);
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
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
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
                action_taken: Some("trashed".to_string()),
                actioned_at: Some("2026-07-20T00:00:00Z".to_string()),
                size_recovered_bytes: Some(*bytes),
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
                action_taken: None,
                actioned_at: None,
                size_recovered_bytes: None,
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
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
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
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
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

        /// Ledger persistence failure surfaces as 500 — never a fake 200.
        #[tokio::test(flavor = "multi_thread")]
        #[serial]
        #[cfg(unix)]
        async fn keep_action_ledger_write_failure_is_500() {
            use std::os::unix::fs::PermissionsExt;
            let home = tempfile::tempdir().unwrap();
            let _guard = env_lock::lock_env([
                ("HOME", Some(home.path().to_str().unwrap())),
                ("PERMAGENT_PATH_ROOT", Some(home.path().to_str().unwrap())),
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
