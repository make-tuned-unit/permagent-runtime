use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use utoipa::ToSchema;

/// Hosts the DownloadManager is willing to contact. Runtime-downloaded
/// artifacts (GGUF weights, ONNX models, voice packs) are parsed by native
/// code (llama.cpp, candle, onnxruntime), so a malicious artifact is
/// arbitrary-code-execution: every download must originate from a trusted,
/// explicitly listed host. Matching is exact (no subdomains) — this governs the
/// INITIAL request URL. Redirect hops are governed by [`validate_redirect_url`].
const ALLOWED_HOSTS: &[&str] = &["huggingface.co"];

/// CDN hosts our allowlisted origins hand the transfer off to. Both of them do:
/// a github.com release asset 302s to `release-assets.githubusercontent.com`,
/// and huggingface.co 302s to its Xet/LFS CDN (`us.aws.cdn.hf.co`,
/// `cas-bridge.xethub.hf.co`, `cdn-lfs*.hf.co`, …), whose exact subdomain varies
/// by region and account — so these are matched by domain SUFFIX rather than
/// exact host.
///
/// Every entry begins with a dot, which is what makes suffix matching safe: a
/// sibling registration like `evil-hf.co` does not end with `.hf.co`, so it
/// cannot slip through.
///
/// These apply to REDIRECT HOPS ONLY — an initial download URL must still name
/// an allowlisted origin, and content integrity is still the SHA-256 pin
/// verified as the bytes land.
const ALLOWED_REDIRECT_HOST_SUFFIXES: &[&str] =
    &[".githubusercontent.com", ".hf.co", ".huggingface.co"];

/// github.com is not generally trusted — only these exact release-asset path
/// prefixes are (third-party projects we intentionally pin, e.g. the
/// kokoro-onnx voice model release).
const ALLOWED_GITHUB_PATH_PREFIXES: &[&str] = &[
    "/thewh1teagle/kokoro-onnx/releases/download/",
    // sherpa-onnx pretrained-model releases (wake-word KWS zipformer).
    "/k2-fsa/sherpa-onnx/releases/download/kws-models/",
];

/// Validate that `raw` is an HTTPS URL pointing at an allowlisted download
/// host. Every URL handed to the DownloadManager must pass this check before
/// any network I/O happens.
pub fn validate_download_url(raw: &str) -> Result<()> {
    let url = reqwest::Url::parse(raw)
        .map_err(|e| anyhow::anyhow!("Invalid download URL '{}': {}", raw, e))?;

    if url.scheme() != "https" {
        anyhow::bail!(
            "Refusing non-HTTPS download URL '{}': model downloads must use https://",
            raw
        );
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Download URL '{}' has no host", raw))?
        .to_ascii_lowercase();

    if ALLOWED_HOSTS.contains(&host.as_str()) {
        return Ok(());
    }

    if host == "github.com" {
        if ALLOWED_GITHUB_PATH_PREFIXES
            .iter()
            .any(|prefix| url.path().starts_with(prefix))
        {
            return Ok(());
        }
        anyhow::bail!(
            "Refusing download from github.com path '{}': only pinned release paths are allowed",
            url.path()
        );
    }

    anyhow::bail!(
        "Refusing download from untrusted host '{}': allowed hosts are {:?} plus pinned github.com release paths",
        host,
        ALLOWED_HOSTS
    );
}

/// Validate a REDIRECT target. Accepts anything [`validate_download_url`] would
/// accept, plus the CDN suffixes in [`ALLOWED_REDIRECT_HOST_SUFFIXES`].
///
/// Applying the initial-URL policy verbatim to redirect hops made every real
/// download impossible: github.com release assets and huggingface.co both 302
/// to a CDN host that is not (and cannot sensibly be) in the origin allowlist,
/// so the voice models, the wake-word models and GGUF weights were all refused
/// at the first hop with "redirect to non-allowlisted URL".
pub fn validate_redirect_url(raw: &str) -> Result<()> {
    if validate_download_url(raw).is_ok() {
        return Ok(());
    }

    let url = reqwest::Url::parse(raw)
        .map_err(|e| anyhow::anyhow!("Invalid redirect URL '{}': {}", raw, e))?;

    if url.scheme() != "https" {
        anyhow::bail!(
            "Refusing non-HTTPS redirect to '{}': downloads must stay on https://",
            raw
        );
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Redirect URL '{}' has no host", raw))?
        .to_ascii_lowercase();

    if ALLOWED_REDIRECT_HOST_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
    {
        return Ok(());
    }

    anyhow::bail!(
        "Refusing redirect to untrusted host '{}': allowed CDN suffixes are {:?}",
        host,
        ALLOWED_REDIRECT_HOST_SUFFIXES
    );
}

/// Normalize an expected SHA-256 into lowercase hex, accepting an optional
/// "sha256:" prefix. Rejects anything that is not exactly 64 hex characters —
/// a malformed pin is a bug at the call site, never something to skip past.
fn normalize_sha256(expected: &str) -> Result<String> {
    let cleaned = expected
        .trim()
        .trim_start_matches("sha256:")
        .to_ascii_lowercase();
    if cleaned.len() == 64 && cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(cleaned)
    } else {
        anyhow::bail!(
            "Invalid expected SHA-256 '{}': must be 64 hex characters (optionally prefixed with 'sha256:')",
            expected
        )
    }
}

/// Compute the SHA-256 of a file as lowercase hex. Runs on the blocking pool —
/// model files are multiple GB.
async fn sha256_file_hex(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        // Chunked update instead of io::copy: the workspace's digest features
        // don't provide io::Write for Sha256, and model files run to ~800MB.
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|e| anyhow::anyhow!("SHA-256 hashing task failed: {}", e))?
}

/// One file to download: source URL, final destination, and (whenever the
/// artifact is known in advance) the expected SHA-256 of its contents.
///
/// When `expected_sha256` is present the downloaded bytes are hashed and MUST
/// match before the file is promoted from its temporary `.part` path to
/// `destination`; on mismatch the partial file is deleted and the download
/// fails. When `None`, no content pin is enforced — the URL still has to pass
/// the HTTPS + trusted-host checks. Prefer pinning wherever a trustworthy
/// digest exists (static artifact tables, the HuggingFace API's `lfs.sha256`).
#[derive(Debug, Clone)]
pub struct DownloadFile {
    pub url: String,
    pub destination: PathBuf,
    pub expected_sha256: Option<String>,
}

impl DownloadFile {
    pub fn new(
        url: impl Into<String>,
        destination: impl Into<PathBuf>,
        expected_sha256: Option<String>,
    ) -> Self {
        Self {
            url: url.into(),
            destination: destination.into(),
            expected_sha256,
        }
    }
}

fn partial_path_for(destination: &Path) -> PathBuf {
    destination.with_extension(
        destination
            .extension()
            .map(|e| format!("{}.part", e.to_string_lossy()))
            .unwrap_or_else(|| "part".to_string()),
    )
}

/// Remove orphaned `.part` files in the given directory (and one level of subdirectories).
/// Preserves `.part` files whose final destination is in `registered_paths` so that
/// in-progress shard downloads can resume after a restart.
pub fn cleanup_partial_downloads(
    dir: &Path,
    registered_paths: &std::collections::HashSet<PathBuf>,
) {
    let should_keep = |part_path: &Path| -> bool {
        // Derive the final path by stripping the trailing ".part" extension
        let final_path = part_path.with_extension("");
        registered_paths.contains(&final_path)
    };

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "part") && !should_keep(&path) {
                let _ = std::fs::remove_file(&path);
            }
            if path.is_dir() {
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.extension().is_some_and(|e| e == "part")
                            && !should_keep(&sub_path)
                        {
                            let _ = std::fs::remove_file(&sub_path);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownloadProgress {
    /// Model ID being downloaded
    pub model_id: String,
    /// Download status
    pub status: DownloadStatus,
    /// Bytes downloaded so far
    pub bytes_downloaded: u64,
    /// Total bytes to download
    pub total_bytes: u64,
    /// Download progress percentage (0-100)
    pub progress_percent: f32,
    /// Download speed in bytes per second
    pub speed_bps: Option<u64>,
    /// Estimated time remaining in seconds
    pub eta_seconds: Option<u64>,
    /// Error message if failed
    pub error: Option<String>,
    /// Whether the background download task has exited
    #[serde(skip)]
    pub task_exited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

type DownloadMap = Arc<Mutex<HashMap<String, DownloadProgress>>>;

pub struct DownloadManager {
    downloads: DownloadMap,
    /// Destinations with a live download task. Two model_ids can resolve to
    /// the same destination file; concurrent writers on one shared `.part`
    /// could slip unverified bytes past the digest check between the hash
    /// pass and the rename, so a destination is exclusive while claimed.
    active_destinations: Arc<Mutex<HashSet<PathBuf>>>,
    /// Test-only escape hatch: allow plain-HTTP loopback URLs so tests can
    /// drive the full download path against a local mock server. Always false
    /// for the production singleton.
    allow_insecure_loopback: bool,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            downloads: Arc::new(Mutex::new(HashMap::new())),
            active_destinations: Arc::new(Mutex::new(HashSet::new())),
            allow_insecure_loopback: false,
        }
    }

    #[cfg(test)]
    fn new_with_insecure_loopback_for_tests() -> Self {
        Self {
            downloads: Arc::new(Mutex::new(HashMap::new())),
            active_destinations: Arc::new(Mutex::new(HashSet::new())),
            allow_insecure_loopback: true,
        }
    }

    /// Validate a download URL under this manager's policy: strict HTTPS +
    /// host-allowlist rules, with a loopback exemption only for test managers.
    fn validate_url(&self, raw: &str) -> Result<()> {
        if self.allow_insecure_loopback {
            if let Ok(url) = reqwest::Url::parse(raw) {
                if matches!(url.host_str(), Some("127.0.0.1") | Some("localhost")) {
                    return Ok(());
                }
            }
        }
        validate_download_url(raw)
    }

    pub fn get_progress(&self, model_id: &str) -> Option<DownloadProgress> {
        self.downloads.lock().ok()?.get(model_id).cloned()
    }

    pub fn cancel_download(&self, model_id: &str) -> Result<()> {
        let mut downloads = self
            .downloads
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire lock"))?;

        if let Some(progress) = downloads.get_mut(model_id) {
            progress.status = DownloadStatus::Cancelled;
            Ok(())
        } else {
            anyhow::bail!("Download not found")
        }
    }

    pub async fn download_model(
        &self,
        model_id: String,
        url: String,
        destination: PathBuf,
        expected_sha256: Option<String>,
        on_complete: Option<Box<dyn FnOnce() + Send + 'static>>,
    ) -> Result<()> {
        self.download_model_sharded(
            model_id,
            vec![DownloadFile::new(url, destination, expected_sha256)],
            0,
            on_complete,
        )
        .await
    }

    pub async fn download_model_sharded(
        &self,
        model_id: String,
        mut files: Vec<DownloadFile>,
        total_size_hint: u64,
        on_complete: Option<Box<dyn FnOnce() + Send + 'static>>,
    ) -> Result<()> {
        info!(model_id = %model_id, file_count = files.len(), "Starting model download");

        // Reject untrusted URLs and malformed digest pins up front, before any
        // download state is registered or any network I/O happens.
        for file in &mut files {
            self.validate_url(&file.url)?;
            if let Some(expected) = &file.expected_sha256 {
                file.expected_sha256 = Some(normalize_sha256(expected)?);
            }
        }

        {
            let mut downloads = self
                .downloads
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire lock"))?;

            if let Some(existing) = downloads.get(&model_id) {
                if existing.status == DownloadStatus::Downloading {
                    anyhow::bail!("Download already in progress");
                }
                if existing.status == DownloadStatus::Cancelled && !existing.task_exited {
                    anyhow::bail!(
                        "Download is being cancelled; wait for it to finish before restarting"
                    );
                }
            }

            downloads.insert(
                model_id.clone(),
                DownloadProgress {
                    model_id: model_id.clone(),
                    status: DownloadStatus::Downloading,
                    bytes_downloaded: 0,
                    total_bytes: total_size_hint,
                    progress_percent: 0.0,
                    speed_bps: None,
                    eta_seconds: None,
                    error: None,
                    task_exited: false,
                },
            );
        }

        // Create parent directories for all files
        for file in &files {
            if let Some(parent) = file.destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to create directory: {}", e))?;
            }
        }

        // Claim every destination for the task's lifetime. Two model_ids can
        // resolve to one destination (catalog entries sharing a filename); a
        // second writer appending to the shared `.part` between the first
        // task's hash pass and its rename could promote unverified bytes.
        let files_for_cleanup: Vec<PathBuf> = files.iter().map(|f| f.destination.clone()).collect();
        let clash: Option<PathBuf> = {
            let mut active = self
                .active_destinations
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire lock"))?;
            let mut claimed: Vec<&PathBuf> = Vec::with_capacity(files_for_cleanup.len());
            let mut clash = None;
            for dest in &files_for_cleanup {
                if !active.insert(dest.clone()) {
                    clash = Some(dest.clone());
                    break;
                }
                claimed.push(dest);
            }
            if clash.is_some() {
                for dest in claimed {
                    active.remove(dest);
                }
            }
            clash
        };
        if let Some(dest) = clash {
            let msg = format!(
                "Destination '{}' is already being written by another download",
                dest.display()
            );
            if let Ok(mut downloads) = self.downloads.lock() {
                if let Some(progress) = downloads.get_mut(&model_id) {
                    progress.status = DownloadStatus::Failed;
                    progress.error = Some(msg.clone());
                    progress.task_exited = true;
                }
            }
            anyhow::bail!(msg);
        }

        let downloads = self.downloads.clone();
        let model_id_clone = model_id.clone();
        let active_destinations = self.active_destinations.clone();
        let allow_insecure_loopback = self.allow_insecure_loopback;

        tokio::spawn(async move {
            let result = Self::download_files_sequentially(
                &files,
                &downloads,
                &model_id_clone,
                allow_insecure_loopback,
            )
            .await;

            // Release the destination claims before recording the outcome so
            // a follow-up download can start as soon as the task is done.
            if let Ok(mut active) = active_destinations.lock() {
                for dest in &files_for_cleanup {
                    active.remove(dest);
                }
            }

            match result {
                Ok(_) => {
                    info!(model_id = %model_id_clone, "Download completed successfully");
                    if let Ok(mut downloads) = downloads.lock() {
                        if let Some(progress) = downloads.get_mut(&model_id_clone) {
                            progress.status = DownloadStatus::Completed;
                            progress.progress_percent = 100.0;
                            progress.task_exited = true;
                        }
                    }

                    if let Some(callback) = on_complete {
                        callback();
                    }
                }
                Err(e) => {
                    for dest in &files_for_cleanup {
                        let partial = partial_path_for(dest);
                        let _ = tokio::fs::remove_file(&partial).await;
                    }

                    if let Ok(mut downloads) = downloads.lock() {
                        if let Some(progress) = downloads.get_mut(&model_id_clone) {
                            if progress.status != DownloadStatus::Cancelled {
                                progress.status = DownloadStatus::Failed;
                            }
                            progress.error = Some(e.to_string());
                            progress.task_exited = true;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    const MAX_RETRIES: u32 = 10;
    const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
    const RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

    async fn cancellable_sleep(
        delay: std::time::Duration,
        downloads: &DownloadMap,
        model_id: &str,
    ) -> Result<(), anyhow::Error> {
        let check_interval = std::time::Duration::from_millis(500);
        let start = std::time::Instant::now();
        while start.elapsed() < delay {
            if Self::is_cancelled(downloads, model_id) {
                anyhow::bail!("Download cancelled");
            }
            let remaining = delay.saturating_sub(start.elapsed());
            tokio::time::sleep(std::cmp::min(check_interval, remaining)).await;
        }
        Ok(())
    }

    fn is_cancelled(downloads: &DownloadMap, model_id: &str) -> bool {
        if let Ok(downloads) = downloads.lock() {
            if let Some(progress) = downloads.get(model_id) {
                return progress.status == DownloadStatus::Cancelled;
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    /// Download multiple files sequentially, tracking cumulative progress under one model_id.
    ///
    /// Files whose destination already exists are skipped (resume support) and
    /// are NOT re-verified — integrity is enforced at download time, when the
    /// bytes cross the trust boundary.
    async fn download_files_sequentially(
        files: &[DownloadFile],
        downloads: &DownloadMap,
        model_id: &str,
        allow_insecure_loopback: bool,
    ) -> Result<(), anyhow::Error> {
        // Re-validate every redirect hop: reqwest's default policy follows
        // cross-host and https->http redirects, which would let an allowlisted
        // host hand the transfer to an arbitrary origin — fatal for files
        // downloaded without a pin. Hops use the slightly wider
        // `validate_redirect_url` (origin allowlist + the CDN suffixes those
        // origins actually redirect to), because the initial-URL policy applied
        // verbatim here refused every real download.
        let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 5 {
                return attempt.error("too many redirects");
            }
            let loopback_ok = allow_insecure_loopback
                && matches!(
                    attempt.url().host_str(),
                    Some("127.0.0.1") | Some("localhost")
                );
            if loopback_ok || validate_redirect_url(attempt.url().as_str()).is_ok() {
                attempt.follow()
            } else {
                let msg = format!("redirect to non-allowlisted URL '{}'", attempt.url());
                attempt.error(msg)
            }
        });
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(120))
            .redirect(redirect_policy)
            .build()?;

        // HEAD each file to get accurate total size. Only replace the hint if
        // every file returned a size; partial results would underestimate.
        let mut total: u64 = 0;
        let mut all_resolved = true;
        for file in files {
            let size = client
                .head(&file.url)
                .send()
                .await
                .ok()
                .and_then(|r| r.content_length())
                .unwrap_or(0);
            if size == 0 {
                all_resolved = false;
            }
            total += size;
        }
        if all_resolved && total > 0 {
            if let Ok(mut dl) = downloads.lock() {
                if let Some(progress) = dl.get_mut(model_id) {
                    progress.total_bytes = total;
                }
            }
        }

        let start_time = std::time::Instant::now();
        let mut cumulative_bytes: u64 = 0;
        // Account for already-downloaded shards
        for file in files {
            let partial = partial_path_for(&file.destination);
            if file.destination.exists() {
                if let Ok(meta) = tokio::fs::metadata(&file.destination).await {
                    cumulative_bytes += meta.len();
                }
            } else if partial.exists() {
                if let Ok(meta) = tokio::fs::metadata(&partial).await {
                    cumulative_bytes += meta.len();
                }
            }
        }
        let bytes_at_start = cumulative_bytes;

        for file in files {
            if Self::is_cancelled(downloads, model_id) {
                anyhow::bail!("Download cancelled");
            }

            // Skip already-completed shards
            if file.destination.exists() {
                continue;
            }

            Self::download_one_file(
                &client,
                file,
                downloads,
                model_id,
                &mut cumulative_bytes,
                start_time,
                bytes_at_start,
            )
            .await?;
        }

        Ok(())
    }

    /// Verify the completed `.part` file against `file.expected_sha256` (when
    /// pinned) and promote it to its final destination. On digest mismatch the
    /// partial is deleted and an error is returned — a corrupted or tampered
    /// artifact must never land at a path the loaders trust.
    async fn verify_and_promote(
        file: &DownloadFile,
        partial_path: &Path,
        model_id: &str,
    ) -> Result<(), anyhow::Error> {
        if let Some(expected) = &file.expected_sha256 {
            let actual = sha256_file_hex(partial_path).await?;
            if &actual != expected {
                let _ = tokio::fs::remove_file(partial_path).await;
                anyhow::bail!(
                    "SHA-256 mismatch for {}: expected {}, got {} — the downloaded file was discarded",
                    file.url,
                    expected,
                    actual
                );
            }
            info!(model_id = %model_id, sha256 = %actual, "Download integrity verified");
        }
        tokio::fs::rename(partial_path, &file.destination).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_one_file(
        client: &reqwest::Client,
        file: &DownloadFile,
        downloads: &DownloadMap,
        model_id: &str,
        cumulative_bytes: &mut u64,
        start_time: std::time::Instant,
        bytes_at_start: u64,
    ) -> Result<(), anyhow::Error> {
        let url: &str = &file.url;
        let destination: &Path = &file.destination;
        let partial_path = partial_path_for(destination);
        let mut retries = 0u32;

        let mut file_bytes: u64 = if partial_path.exists() {
            tokio::fs::metadata(&partial_path).await?.len()
        } else {
            0
        };

        // Get this file's total size
        let mut file_total: u64 = client
            .head(url)
            .send()
            .await
            .ok()
            .and_then(|r| r.content_length())
            .unwrap_or(0);

        // If partial matches expected size exactly, verify and promote it.
        // A digest mismatch here may just be a corrupted resume from a previous
        // run — discard the partial and fall through to a fresh download (whose
        // result is verified again before promotion).
        if file_total > 0 && file_bytes == file_total {
            match Self::verify_and_promote(file, &partial_path, model_id).await {
                Ok(()) => {
                    // cumulative_bytes already accounts for this file from the pre-scan
                    return Ok(());
                }
                Err(e) => {
                    warn!(model_id = %model_id, error = %e, "Complete partial failed verification, re-downloading from scratch");
                    *cumulative_bytes = cumulative_bytes.saturating_sub(file_bytes);
                    file_bytes = 0;
                    let _ = tokio::fs::remove_file(&partial_path).await;
                }
            }
        }

        // If partial is oversized or remote changed, discard and re-download
        if file_total > 0 && file_bytes > file_total {
            info!(model_id = %model_id, file_bytes, file_total, "Partial file oversized, re-downloading");
            *cumulative_bytes = cumulative_bytes.saturating_sub(file_bytes);
            file_bytes = 0;
            let _ = tokio::fs::remove_file(&partial_path).await;
        }

        loop {
            if Self::is_cancelled(downloads, model_id) {
                let _ = tokio::fs::remove_file(&partial_path).await;
                anyhow::bail!("Download cancelled");
            }

            let mut request = client.get(url);
            if file_bytes > 0 {
                request = request.header("Range", format!("bytes={}-", file_bytes));
            }

            let response = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    // A redirect the policy refused is deterministic — the
                    // server will answer with the same disallowed Location on
                    // every retry. Fail immediately, naming the policy reason.
                    if e.is_redirect() {
                        let detail = std::error::Error::source(&e)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| e.to_string());
                        anyhow::bail!("Download refused: {}", detail);
                    }
                    if retries >= Self::MAX_RETRIES {
                        anyhow::bail!("Download failed after {} retries: {}", retries, e);
                    }
                    retries += 1;
                    let delay = std::cmp::min(
                        Self::RETRY_BASE_DELAY * 2u32.saturating_pow(retries - 1),
                        Self::RETRY_MAX_DELAY,
                    );
                    info!(model_id = %model_id, retry = retries, delay_secs = ?delay.as_secs(), error = %e, "Retrying download after connection error");
                    Self::cancellable_sleep(delay, downloads, model_id).await?;
                    continue;
                }
            };

            let status = response.status();
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                if file_total > 0 && file_bytes == file_total {
                    break;
                }
                *cumulative_bytes = cumulative_bytes.saturating_sub(file_bytes);
                file_bytes = 0;
                let _ = tokio::fs::remove_file(&partial_path).await;
                continue;
            }

            if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                let is_transient = status.is_server_error()
                    || status == reqwest::StatusCode::REQUEST_TIMEOUT
                    || status == reqwest::StatusCode::TOO_MANY_REQUESTS;

                if !is_transient || retries >= Self::MAX_RETRIES {
                    anyhow::bail!("Failed to download: HTTP {}", status);
                }
                retries += 1;
                let delay = std::cmp::min(
                    Self::RETRY_BASE_DELAY * 2u32.saturating_pow(retries - 1),
                    Self::RETRY_MAX_DELAY,
                );
                info!(model_id = %model_id, retry = retries, http_status = %status, "Retrying download after transient HTTP error");
                Self::cancellable_sleep(delay, downloads, model_id).await?;
                continue;
            }

            if file_bytes > 0 && status == reqwest::StatusCode::OK {
                info!(model_id = %model_id, "Server ignored Range header, restarting file from scratch");
                // Subtract already-counted partial bytes from cumulative
                *cumulative_bytes = cumulative_bytes.saturating_sub(file_bytes);
                file_bytes = 0;
                let _ = tokio::fs::remove_file(&partial_path).await;
            }

            // If HEAD didn't return this file's size, learn it from the GET response.
            // This block only fires once per file (file_total stays non-zero after),
            // so retries don't double-count. Since download_files_sequentially's HEAD
            // pass contributed 0 for this file, we add the discovered size to the
            // shared total so progress/ETA are accurate.
            if file_total == 0 {
                let new_file_total = if file_bytes > 0 {
                    response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.rsplit('/').next())
                        .and_then(|s| s.parse::<u64>().ok())
                } else {
                    response.content_length()
                };
                if let Some(t) = new_file_total {
                    file_total = t;
                    if let Ok(mut dl) = downloads.lock() {
                        if let Some(progress) = dl.get_mut(model_id) {
                            progress.total_bytes = progress.total_bytes.saturating_add(t);
                        }
                    }
                }
            }

            let mut part_file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&partial_path)
                .await?;

            let file_len = tokio::fs::metadata(&partial_path).await?.len();
            if file_len != file_bytes {
                part_file.set_len(file_bytes).await?;
            }

            let mut stream_error = false;
            let mut resp = response;

            loop {
                let chunk_result = resp.chunk().await;
                match chunk_result {
                    Ok(Some(chunk)) => {
                        if Self::is_cancelled(downloads, model_id) {
                            let _ = tokio::fs::remove_file(&partial_path).await;
                            anyhow::bail!("Download cancelled");
                        }

                        part_file.write_all(&chunk).await?;
                        let chunk_len = chunk.len() as u64;
                        file_bytes += chunk_len;
                        *cumulative_bytes += chunk_len;

                        let elapsed = start_time.elapsed().as_secs_f64();
                        let bytes_this_session = cumulative_bytes.saturating_sub(bytes_at_start);
                        let speed_bps = if elapsed > 0.0 {
                            Some((bytes_this_session as f64 / elapsed) as u64)
                        } else {
                            None
                        };

                        let current_total = if let Ok(dl) = downloads.lock() {
                            dl.get(model_id).map(|p| p.total_bytes).unwrap_or(0)
                        } else {
                            0
                        };

                        let eta_seconds = if let Some(speed) = speed_bps {
                            if speed > 0 && current_total > 0 {
                                Some(current_total.saturating_sub(*cumulative_bytes) / speed)
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Ok(mut dl) = downloads.lock() {
                            if let Some(progress) = dl.get_mut(model_id) {
                                progress.bytes_downloaded = *cumulative_bytes;
                                progress.progress_percent = if current_total > 0 {
                                    (*cumulative_bytes as f64 / current_total as f64 * 100.0) as f32
                                } else {
                                    0.0
                                };
                                progress.speed_bps = speed_bps;
                                progress.eta_seconds = eta_seconds;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        info!(model_id = %model_id, bytes = *cumulative_bytes, error = %e, "Download stream interrupted, will retry");
                        stream_error = true;
                        break;
                    }
                }
            }

            part_file.flush().await?;
            drop(part_file);

            if stream_error {
                if retries >= Self::MAX_RETRIES {
                    anyhow::bail!(
                        "Download failed after {} retries due to stream interruption",
                        retries
                    );
                }
                retries += 1;
                let delay = std::cmp::min(
                    Self::RETRY_BASE_DELAY * 2u32.saturating_pow(retries - 1),
                    Self::RETRY_MAX_DELAY,
                );
                info!(model_id = %model_id, retry = retries, delay_secs = ?delay.as_secs(), "Retrying download with resume");
                Self::cancellable_sleep(delay, downloads, model_id).await?;
                continue;
            }

            break;
        }

        // Stream complete: enforce the digest pin (when present) before the
        // file is promoted to the path the model loaders trust. A mismatch at
        // this point is a hard failure — the server sent us the wrong bytes.
        Self::verify_and_promote(file, &partial_path, model_id).await?;
        Ok(())
    }

    pub fn clear_completed(&self, model_id: &str) {
        if let Ok(mut downloads) = self.downloads.lock() {
            if let Some(progress) = downloads.get(model_id) {
                let is_terminal = progress.status == DownloadStatus::Completed
                    || progress.status == DownloadStatus::Failed
                    || progress.status == DownloadStatus::Cancelled;
                if is_terminal && progress.task_exited {
                    downloads.remove(model_id);
                }
            }
        }
    }
}

static DOWNLOAD_MANAGER: once_cell::sync::Lazy<DownloadManager> =
    once_cell::sync::Lazy::new(DownloadManager::new);

pub fn get_download_manager() -> &'static DownloadManager {
    &DOWNLOAD_MANAGER
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sha256_hex_of(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    // ── Redirect-hop policy ────────────────────────────────────────────────

    /// The regression this policy exists for: both allowlisted origins hand the
    /// transfer to a CDN, so applying the initial-URL rule to hops refused every
    /// real download ("redirect to non-allowlisted URL"). These are the exact
    /// hosts observed in the wild on 2026-08-11.
    #[test]
    fn allows_redirect_to_origin_cdns() {
        for url in [
            // github.com release asset -> GitHub's release-asset CDN
            "https://release-assets.githubusercontent.com/github-production-release-asset/911666237/abc",
            "https://objects.githubusercontent.com/github-production-release-asset/1/2",
            // huggingface.co -> Xet / LFS CDN (subdomain varies by region)
            "https://us.aws.cdn.hf.co/xet-bridge-us/697c69e5/64cd2138",
            "https://cas-bridge.xethub.hf.co/xet-bridge-us/abc",
            "https://cdn-lfs.huggingface.co/repos/aa/bb/model.gguf",
        ] {
            assert!(
                validate_redirect_url(url).is_ok(),
                "redirect hop should be allowed: {url}"
            );
        }
    }

    /// The leading dot on every suffix is load-bearing: a sibling registration
    /// must not satisfy `.hf.co`.
    #[test]
    fn refuses_redirect_to_lookalike_sibling_domain() {
        for url in [
            "https://evil-hf.co/model.bin",
            "https://hf.co.evil.example/model.bin",
            "https://notgithubusercontent.com/model.bin",
            "https://evil.example/model.bin",
        ] {
            assert!(
                validate_redirect_url(url).is_err(),
                "redirect hop must be refused: {url}"
            );
        }
    }

    #[test]
    fn refuses_non_https_redirect_even_to_allowed_suffix() {
        assert!(validate_redirect_url("http://us.aws.cdn.hf.co/model.bin").is_err());
    }

    /// The widened list is for HOPS only — a download must still *start* at an
    /// allowlisted origin, never at a CDN host directly.
    #[test]
    fn cdn_suffix_does_not_widen_initial_url_policy() {
        assert!(validate_download_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2"
        )
        .is_err());
        assert!(validate_download_url("https://us.aws.cdn.hf.co/xet-bridge-us/abc").is_err());
    }

    // ── URL allowlist / scheme policy ──────────────────────────────────────

    #[test]
    fn allows_huggingface_https() {
        assert!(validate_download_url(
            "https://huggingface.co/oxide-lab/whisper-base-GGUF/resolve/main/whisper-base-q8_0.gguf"
        )
        .is_ok());
    }

    #[test]
    fn allows_pinned_github_release_path() {
        assert!(validate_download_url(
            "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx"
        )
        .is_ok());
    }

    #[test]
    fn allows_sherpa_kws_release_path() {
        assert!(validate_download_url(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2"
        )
        .is_ok());
    }

    #[test]
    fn rejects_non_pinned_github_path() {
        let err = validate_download_url(
            "https://github.com/attacker/evil-repo/releases/download/v1/model.onnx",
        )
        .unwrap_err();
        assert!(err.to_string().contains("pinned release paths"));
    }

    #[test]
    fn rejects_http_scheme() {
        let err = validate_download_url(
            "http://huggingface.co/oxide-lab/whisper-base-GGUF/resolve/main/model.gguf",
        )
        .unwrap_err();
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn rejects_untrusted_host() {
        let err = validate_download_url("https://evil.example.com/model.gguf").unwrap_err();
        assert!(err.to_string().contains("untrusted host"));
    }

    #[test]
    fn rejects_host_lookalikes() {
        // Exact-match host policy: neither subdomain-forging nor suffix tricks pass.
        assert!(validate_download_url("https://huggingface.co.evil.com/model.gguf").is_err());
        assert!(validate_download_url("https://evilhuggingface.co/model.gguf").is_err());
        assert!(validate_download_url("https://cdn.huggingface.co/model.gguf").is_err());
    }

    #[test]
    fn rejects_invalid_url() {
        assert!(validate_download_url("not a url").is_err());
        assert!(validate_download_url("file:///etc/passwd").is_err());
    }

    // ── Digest pin normalization ───────────────────────────────────────────

    #[test]
    fn normalizes_sha256_forms() {
        let digest = "a".repeat(64);
        assert_eq!(normalize_sha256(&digest).unwrap(), digest);
        assert_eq!(
            normalize_sha256(&format!("sha256:{}", digest)).unwrap(),
            digest
        );
        assert_eq!(normalize_sha256(&digest.to_uppercase()).unwrap(), digest);
    }

    #[test]
    fn rejects_malformed_sha256() {
        assert!(normalize_sha256("").is_err());
        assert!(normalize_sha256("abc123").is_err());
        assert!(normalize_sha256(&"g".repeat(64)).is_err());
        assert!(normalize_sha256(&"a".repeat(63)).is_err());
        assert!(normalize_sha256(&"a".repeat(65)).is_err());
    }

    // ── Manager-level rejection (before any network I/O) ───────────────────

    #[tokio::test]
    async fn download_rejects_http_url_immediately() {
        let dm = DownloadManager::new();
        let dir = tempfile::tempdir().unwrap();
        let err = dm
            .download_model(
                "http-test".to_string(),
                "http://huggingface.co/some/model.gguf".to_string(),
                dir.path().join("model.gguf"),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("https"));
        assert!(dm.get_progress("http-test").is_none());
    }

    #[tokio::test]
    async fn download_rejects_untrusted_host_immediately() {
        let dm = DownloadManager::new();
        let dir = tempfile::tempdir().unwrap();
        let err = dm
            .download_model(
                "host-test".to_string(),
                "https://evil.example.com/model.gguf".to_string(),
                dir.path().join("model.gguf"),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("untrusted host"));
        assert!(dm.get_progress("host-test").is_none());
    }

    #[tokio::test]
    async fn download_rejects_malformed_expected_hash_immediately() {
        let dm = DownloadManager::new();
        let dir = tempfile::tempdir().unwrap();
        let err = dm
            .download_model(
                "hash-format-test".to_string(),
                "https://huggingface.co/some/model.gguf".to_string(),
                dir.path().join("model.gguf"),
                Some("not-a-real-digest".to_string()),
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid expected SHA-256"));
        assert!(dm.get_progress("hash-format-test").is_none());
    }

    // ── End-to-end verification against a local mock server ───────────────

    async fn wait_for_terminal(dm: &DownloadManager, id: &str) -> DownloadProgress {
        let start = std::time::Instant::now();
        loop {
            if let Some(p) = dm.get_progress(id) {
                if p.task_exited {
                    return p;
                }
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(30),
                "download {id} did not reach a terminal state in time"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    #[tokio::test]
    async fn matching_hash_promotes_to_destination() {
        let body = b"tiny model fixture bytes for integrity testing".to_vec();
        let expected = sha256_hex_of(&body);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model(
            "good-hash".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            Some(expected),
            None,
        )
        .await
        .unwrap();

        let progress = wait_for_terminal(&dm, "good-hash").await;
        assert_eq!(progress.status, DownloadStatus::Completed, "{:?}", progress);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!partial_path_for(&dest).exists());
    }

    #[tokio::test]
    async fn hash_mismatch_fails_and_leaves_no_files() {
        let body = b"bytes an attacker swapped in".to_vec();
        // Expect the digest of DIFFERENT content.
        let expected = sha256_hex_of(b"the artifact we actually pinned");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model(
            "bad-hash".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            Some(expected),
            None,
        )
        .await
        .unwrap();

        let progress = wait_for_terminal(&dm, "bad-hash").await;
        assert_eq!(progress.status, DownloadStatus::Failed, "{:?}", progress);
        assert!(
            progress
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("SHA-256 mismatch"),
            "error should name the digest mismatch: {:?}",
            progress.error
        );
        assert!(!dest.exists(), "destination must not exist after mismatch");
        assert!(
            !partial_path_for(&dest).exists(),
            "partial must be cleaned up after mismatch"
        );
    }

    #[tokio::test]
    async fn download_without_pin_still_works() {
        let body = b"unpinned artifact".to_vec();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model(
            "no-pin".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            None,
            None,
        )
        .await
        .unwrap();

        let progress = wait_for_terminal(&dm, "no-pin").await;
        assert_eq!(progress.status, DownloadStatus::Completed, "{:?}", progress);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    #[tokio::test]
    async fn redirect_to_non_allowlisted_url_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model.bin"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", "https://evil.example/model.bin"),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model(
            "redirected".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            None,
            None,
        )
        .await
        .unwrap();

        let progress = wait_for_terminal(&dm, "redirected").await;
        assert_eq!(progress.status, DownloadStatus::Failed, "{:?}", progress);
        assert!(
            progress
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("redirect"),
            "error should name the refused redirect: {:?}",
            progress.error
        );
        assert!(!dest.exists(), "no bytes may land via a refused redirect");
        assert!(!partial_path_for(&dest).exists());
    }

    #[tokio::test]
    async fn concurrent_downloads_to_same_destination_are_refused() {
        let body = b"slow artifact".to_vec();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(body.clone())
                    .set_delay(std::time::Duration::from_millis(750)),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model(
            "writer-a".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            None,
            None,
        )
        .await
        .unwrap();

        // Distinct model_id, same destination: refused while A holds the
        // claim, with the failure recorded on B's own progress entry.
        let err = dm
            .download_model(
                "writer-b".to_string(),
                format!("{}/model.bin", server.uri()),
                dest.clone(),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("already being written"),
            "unexpected error: {err}"
        );
        let b = dm.get_progress("writer-b").unwrap();
        assert_eq!(b.status, DownloadStatus::Failed);
        assert!(b.task_exited);

        let a = wait_for_terminal(&dm, "writer-a").await;
        assert_eq!(a.status, DownloadStatus::Completed, "{:?}", a);

        // Claim released after A's task exit: a new download to the same
        // destination is accepted (and completes via the existing-file skip).
        dm.download_model(
            "writer-c".to_string(),
            format!("{}/model.bin", server.uri()),
            dest.clone(),
            None,
            None,
        )
        .await
        .unwrap();
        let c = wait_for_terminal(&dm, "writer-c").await;
        assert_eq!(c.status, DownloadStatus::Completed, "{:?}", c);
    }

    #[tokio::test]
    async fn sharded_download_verifies_every_file() {
        let good = b"first shard bytes".to_vec();
        let evil = b"second shard tampered".to_vec();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/shard1.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(good.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shard2.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(evil))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest1 = dir.path().join("shard1.bin");
        let dest2 = dir.path().join("shard2.bin");
        let dm = DownloadManager::new_with_insecure_loopback_for_tests();
        dm.download_model_sharded(
            "sharded".to_string(),
            vec![
                DownloadFile::new(
                    format!("{}/shard1.bin", server.uri()),
                    dest1.clone(),
                    Some(sha256_hex_of(&good)),
                ),
                DownloadFile::new(
                    format!("{}/shard2.bin", server.uri()),
                    dest2.clone(),
                    Some(sha256_hex_of(b"what shard2 should have been")),
                ),
            ],
            0,
            None,
        )
        .await
        .unwrap();

        let progress = wait_for_terminal(&dm, "sharded").await;
        assert_eq!(progress.status, DownloadStatus::Failed, "{:?}", progress);
        // Shard 1 verified and landed; shard 2 was rejected and cleaned up.
        assert_eq!(std::fs::read(&dest1).unwrap(), good);
        assert!(!dest2.exists());
        assert!(!partial_path_for(&dest2).exists());
    }
}
