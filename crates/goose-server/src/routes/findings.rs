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

fn save_findings(data: &FindingsFile) {
    let dir = findings_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = findings_path(&data.run_id);
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(data).unwrap_or_default(),
    );
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
            save_findings(&data);

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
            save_findings(&data);

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
    save_findings(&data);

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
}
