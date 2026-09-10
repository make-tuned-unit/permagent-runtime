//! Worker availability probing and caching.
//!
//! Checks whether a worker is available on this machine based on its
//! `availability_check` string from agent.yaml.

use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::search_path::SearchPaths;

const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Cached probe result for a single worker.
#[derive(Debug, Clone)]
pub struct WorkerAvailability {
    pub available: bool,
    pub last_checked: Instant,
    pub reason: Option<String>,
}

/// Thread-safe cache of worker availability results.
pub struct ProbeCache {
    inner: Mutex<HashMap<String, WorkerAvailability>>,
}

impl Default for ProbeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Get a cached result if it exists and hasn't expired.
    pub fn get(&self, worker_key: &str) -> Option<WorkerAvailability> {
        let cache = self.inner.lock().unwrap();
        cache.get(worker_key).and_then(|entry| {
            if entry.last_checked.elapsed() < CACHE_TTL {
                Some(entry.clone())
            } else {
                None
            }
        })
    }

    /// Store a probe result.
    pub fn set(&self, worker_key: &str, available: bool, reason: Option<String>) {
        let mut cache = self.inner.lock().unwrap();
        cache.insert(
            worker_key.to_string(),
            WorkerAvailability {
                available,
                last_checked: Instant::now(),
                reason,
            },
        );
    }

    /// Clear all cached results (force re-probe on next access).
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Probe once per worker/configuration pair within the cache TTL.
    ///
    /// The availability check is part of the key, so editing agent.yaml does
    /// not leave the old answer live for five minutes. Callers that rebuild a
    /// prompt every model turn should use this instead of calling
    /// [`probe_worker`] directly: `model_loaded:` may otherwise add a two-
    /// second blocking HTTP timeout for every configured local worker on every
    /// turn.
    pub fn probe(&self, worker_key: &str, check: &str) -> (bool, Option<String>) {
        self.probe_with(worker_key, check, probe_worker)
    }

    fn probe_with<F>(&self, worker_key: &str, check: &str, probe: F) -> (bool, Option<String>)
    where
        F: FnOnce(&str) -> (bool, Option<String>),
    {
        let cache_key = format!("{worker_key}\0{check}");
        if let Some(cached) = self.get(&cache_key) {
            return (cached.available, cached.reason);
        }
        let (available, reason) = probe(check);
        self.set(&cache_key, available, reason.clone());
        (available, reason)
    }
}

/// Probe whether a worker is available based on its `availability_check` string.
///
/// Supported formats:
/// - `"always"` — always available
/// - `"bin_exists:<name>"` — check if binary is on Permagent's search path
///   (the inherited PATH widened with `~/.local/bin`, `/usr/local/bin`,
///   Homebrew and npm-global — see [`SearchPaths`])
/// - `"api_credential:<env_var>"` — check if environment variable is set
/// - `"model_loaded:<model>"` — check if Ollama model is pulled (HTTP call)
///
/// Returns `(available, reason)` where reason is `None` if available,
/// or a human-readable explanation if not.
///
/// Blocking: `model_loaded:` performs a local HTTP request with a two-second
/// timeout. It is safe to call from an async context — `probe_model_loaded`
/// isolates the blocking HTTP client on a thread of its own — but async callers
/// should prefer [`probe_worker_async`] so that wait never lands on a reactor
/// thread.
pub fn probe_worker(check: &str) -> (bool, Option<String>) {
    if check == "always" || check.is_empty() {
        return (true, None);
    }

    if let Some(bin_name) = check.strip_prefix("bin_exists:") {
        return probe_bin_exists(bin_name);
    }

    if let Some(env_var) = check.strip_prefix("api_credential:") {
        return probe_api_credential(env_var);
    }

    if let Some(model) = check.strip_prefix("model_loaded:") {
        return probe_model_loaded(model);
    }

    (
        false,
        Some(format!("Unknown availability_check format: {}", check)),
    )
}

/// [`probe_worker`], run on a blocking thread.
///
/// `probe_worker` can block for up to two seconds (`model_loaded:` does a local
/// HTTP call), so an async caller that invokes it directly stalls a reactor
/// thread for that long. Every other async caller in the tree already wraps it
/// in `spawn_blocking` — `goose-server`'s worker routes, `app_perception`,
/// `reply_parts` — and this is that step, named once so the orchestrator's tool
/// handlers do not have to open-code it.
///
/// Correctness does not depend on using this: `probe_model_loaded` keeps the
/// blocking HTTP client off the caller's thread whatever the caller's context.
pub async fn probe_worker_async(check: &str) -> (bool, Option<String>) {
    let check = check.to_string();
    tokio::task::spawn_blocking(move || probe_worker(&check))
        .await
        .unwrap_or_else(|e| {
            (
                false,
                Some(format!("Worker availability probe failed: {e}")),
            )
        })
}

/// Probe for a binary against Permagent's augmented search path.
///
/// `which::which` resolves against the *process* PATH, and that was the bug
/// (reported 2026-08-18): the daemon is started by launchd, not by a login
/// shell, so it inherits a bare `/usr/bin:/bin:/usr/sbin:/sbin` with no
/// Homebrew on it. Every brew- or npm-installed tool in the roster — `docker`
/// for the Guard, `claude`, `codex`, `cursor-agent` — therefore probed as
/// missing on machines where the binary was plainly installed, and the user was
/// told "Binary docker not found on path" while `/opt/homebrew/bin/docker` sat
/// right there.
///
/// [`SearchPaths`] is the same widening the providers already use to *launch*
/// these binaries (`~/.local/bin`, `/usr/local/bin`, Homebrew, npm-global), so
/// probing through it makes "is it available" agree with "can we run it"
/// instead of contradicting it.
fn probe_bin_exists(name: &str) -> (bool, Option<String>) {
    let search = SearchPaths::builder()
        .with_npm()
        .path()
        // Assembling the path can only fail on a PATH entry containing the
        // separator itself. Fall back to the inherited PATH rather than
        // reporting a binary as absent because of it.
        .unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default());
    probe_bin_exists_in(name, &search)
}

/// The resolution step, with the search path passed in so a test can construct
/// the launchd condition (a minimal PATH plus an off-PATH install directory)
/// explicitly instead of relying on whatever PATH the test runner inherited.
fn probe_bin_exists_in(name: &str, search: &OsStr) -> (bool, Option<String>) {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    match which::which_in(name, Some(search), cwd) {
        Ok(_) => (true, None),
        // Still honest when the binary really is absent: the name is named, and
        // the message says where we looked so "not found" is checkable.
        Err(_) => (
            false,
            Some(format!(
                "Binary '{}' not found on PATH (searched PATH plus ~/.local/bin, \
                 /usr/local/bin, Homebrew and npm-global)",
                name
            )),
        ),
    }
}

fn probe_api_credential(env_var: &str) -> (bool, Option<String>) {
    if std::env::var(env_var).is_ok() {
        (true, None)
    } else {
        (
            false,
            Some(format!("Environment variable '{}' not set", env_var)),
        )
    }
}

/// Ollama's `/api/tags` check, run on a thread of its own.
///
/// The thread is not an optimisation, it is the correctness step. The body
/// below builds a `reqwest::blocking::Client`, and in a debug build
/// `reqwest::blocking::wait::enter` asserts it is not already inside a runtime
/// by *constructing a current-thread tokio runtime and immediately dropping it*
/// (`reqwest-0.13.4/src/blocking/wait.rs:80`). Dropping a runtime on a thread
/// that is inside an async context kills the process:
///
/// ```text
/// thread 'main' panicked at tokio/src/runtime/blocking/shutdown.rs:51:21:
/// Cannot drop a runtime in a context where blocking is not allowed.
/// ```
///
/// That is how a live harness run died on 2026-09-10 (fixture
/// `harness-cli-argparse-log-filter`): the coordinator called
/// `list_workers { refresh: true }`, the cleared cache forced a real probe, and
/// the orchestrator's async tool handler was calling [`probe_worker`] inline.
///
/// [`probe_worker`] is synchronous and has many callers — some async, some on
/// threads with no runtime at all, and one inside a `.map()` closure that
/// cannot `.await` — so the guarantee belongs here, at the single place that
/// owns the blocking client, rather than being re-derived at each call site.
fn probe_model_loaded(model: &str) -> (bool, Option<String>) {
    let model = model.to_string();
    let spawned = std::thread::Builder::new()
        .name("worker-probe-ollama".to_string())
        .spawn(move || probe_model_loaded_on_this_thread(&model));

    match spawned {
        Ok(handle) => handle.join().unwrap_or_else(|_| {
            (
                false,
                Some("Ollama availability probe panicked".to_string()),
            )
        }),
        Err(e) => (
            false,
            Some(format!("Ollama availability probe could not start: {e}")),
        ),
    }
}

/// The probe body. Only ever called on the dedicated thread that
/// [`probe_model_loaded`] spawns, which is what makes `reqwest::blocking` safe
/// here; call it from anywhere else and the panic above comes back.
fn probe_model_loaded_on_this_thread(model: &str) -> (bool, Option<String>) {
    // Synchronous HTTP check against Ollama's local API.
    // Uses a short timeout since Ollama should be local.
    let url = "http://localhost:11434/api/tags";
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return (
                false,
                Some("Failed to create HTTP client for Ollama check".to_string()),
            )
        }
    };

    let resp = match client.get(url).send() {
        Ok(r) => r,
        Err(_) => {
            return (
                false,
                Some("Ollama not reachable at localhost:11434".to_string()),
            )
        }
    };

    let body = match resp.text() {
        Ok(b) => b,
        Err(_) => return (false, Some("Failed to read Ollama response".to_string())),
    };

    // Ollama /api/tags returns {"models": [{"name": "model:tag", ...}, ...]}
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return (false, Some("Failed to parse Ollama response".to_string())),
    };

    let empty = vec![];
    let models = parsed
        .get("models")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);

    let found = models.iter().any(|m| {
        m.get("name")
            .and_then(|n| n.as_str())
            .is_some_and(|n| n == model || n.starts_with(&format!("{}:", model)))
    });

    if found {
        (true, None)
    } else {
        (
            false,
            Some(format!("Model '{}' not found in Ollama", model)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_is_available() {
        let (ok, reason) = probe_worker("always");
        assert!(ok);
        assert!(reason.is_none());
    }

    #[test]
    fn empty_string_is_available() {
        let (ok, reason) = probe_worker("");
        assert!(ok);
        assert!(reason.is_none());
    }

    #[test]
    fn bin_exists_finds_sh() {
        // /bin/sh exists on macOS and Linux
        let (ok, reason) = probe_worker("bin_exists:sh");
        assert!(ok, "sh should be on PATH: {:?}", reason);
        assert!(reason.is_none());
    }

    #[test]
    fn bin_exists_missing_binary() {
        let (ok, reason) = probe_worker("bin_exists:__nonexistent_binary_xyz__");
        assert!(!ok);
        let reason = reason.unwrap();
        // A genuinely absent binary must still say so, and name itself.
        assert!(reason.contains("not found on PATH"), "{}", reason);
        assert!(reason.contains("__nonexistent_binary_xyz__"), "{}", reason);
    }

    /// Regression (reported 2026-08-18): the Guard reported "Binary docker not
    /// found on path" on a machine with `/opt/homebrew/bin/docker` installed,
    /// because the probe used `which::which`, which resolves against the
    /// *process* PATH — and the daemon is started by launchd with a bare
    /// `/usr/bin:/bin:/usr/sbin:/sbin`.
    ///
    /// This builds that condition rather than inheriting it: a PATH containing
    /// exactly one empty directory, and the binary installed in a second
    /// directory that stands in for `/opt/homebrew/bin`. Under the old
    /// `which::which` the probe cannot see the binary at all; it passes only if
    /// resolution honours the augmented search path.
    #[cfg(unix)]
    #[test]
    fn bin_exists_finds_binary_outside_a_launchd_style_path() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let bare_path = tmp.path().join("bare-path");
        let brew_bin = tmp.path().join("brew/bin");
        std::fs::create_dir_all(&bare_path).unwrap();
        std::fs::create_dir_all(&brew_bin).unwrap();

        let tool = brew_bin.join("permagent-probe-fixture");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The launchd condition: the install directory is NOT on PATH.
        let minimal_path = bare_path.clone().into_os_string();
        let (ok, reason) = probe_bin_exists_in("permagent-probe-fixture", &minimal_path);
        assert!(
            !ok,
            "fixture must be invisible to the bare PATH, or the test proves nothing: {:?}",
            reason
        );

        // The fix: search the augmented path, and the installed binary is found.
        let augmented = env::join_paths([brew_bin.as_path(), bare_path.as_path()]).unwrap();
        let (ok, reason) = probe_bin_exists_in("permagent-probe-fixture", &augmented);
        assert!(
            ok,
            "binary is installed and must probe as present: {:?}",
            reason
        );
        assert!(reason.is_none());

        // And a binary that is absent from every searched directory still
        // reports absent — the fix must not turn the probe into a rubber stamp.
        let (ok, reason) = probe_bin_exists_in("permagent-probe-fixture-absent", &augmented);
        assert!(!ok);
        assert!(reason.unwrap().contains("not found on PATH"));
    }

    /// The other half of the regression: `probe_bin_exists` must hand
    /// `probe_bin_exists_in` a path that actually contains the directories
    /// package managers install into. Without this, the test above could pass
    /// against a search path the production probe never builds.
    #[test]
    fn probe_search_path_covers_the_directories_tools_install_into() {
        let search = SearchPaths::builder().with_npm().path().unwrap();
        let entries: Vec<PathBuf> = env::split_paths(&search).collect();
        let ends_with = |suffix: &str| entries.iter().any(|d| d.ends_with(suffix));
        let has = |p: &str| entries.iter().any(|d| d == std::path::Path::new(p));

        // pipx installs the Guard's own scanner here; npm-global is where
        // `claude` and `codex` land.
        assert!(ends_with(".local/bin"), "search path was {:?}", search);
        assert!(ends_with(".npm-global/bin"), "search path was {:?}", search);

        #[cfg(unix)]
        assert!(has("/usr/local/bin"), "search path was {:?}", search);

        if cfg!(target_os = "macos") {
            assert!(
                has("/opt/homebrew/bin"),
                "Homebrew is where `docker`, `claude` and `codex` live on macOS; \
                 search path was {:?}",
                search
            );
        }

        // And the inherited PATH is still honoured, not replaced.
        if let Some(inherited) = env::var_os("PATH") {
            for dir in env::split_paths(&inherited) {
                assert!(
                    entries.contains(&dir),
                    "inherited PATH entry {:?} was dropped from {:?}",
                    dir,
                    search
                );
            }
        }
    }

    #[test]
    fn api_credential_set_env() {
        // PATH is always set
        let (ok, reason) = probe_worker("api_credential:PATH");
        assert!(ok);
        assert!(reason.is_none());
    }

    #[test]
    fn api_credential_unset_env() {
        let (ok, reason) = probe_worker("api_credential:__NONEXISTENT_PERMAGENT_TEST_VAR__");
        assert!(!ok);
        assert!(reason.unwrap().contains("not set"));
    }

    #[test]
    fn unknown_format_returns_error() {
        let (ok, reason) = probe_worker("magic_check:foo");
        assert!(!ok);
        assert!(reason.unwrap().contains("Unknown availability_check"));
    }

    // model_loaded tests are skipped in CI since Ollama may not be running.
    // Run manually: cargo test -p permagent --lib -- worker_probe::tests::model_loaded --ignored
    #[test]
    #[ignore]
    fn model_loaded_ollama_not_running() {
        // This test assumes Ollama is NOT running — will fail if it is.
        let (ok, reason) = probe_worker("model_loaded:nonexistent-model");
        assert!(!ok);
        assert!(reason.is_some());
    }

    /// Regression (2026-09-10, harness fixture
    /// `harness-cli-argparse-log-filter`): the coordinator called
    /// `list_workers { refresh: true }`, the cleared cache forced a real probe,
    /// and the CLI died with
    ///
    /// ```text
    /// thread 'main' panicked at tokio/src/runtime/blocking/shutdown.rs:51:21:
    /// Cannot drop a runtime in a context where blocking is not allowed.
    /// ```
    ///
    /// because `probe_model_loaded` built a `reqwest::blocking` client — and so
    /// built and dropped that client's private runtime — on a tokio worker
    /// thread.
    ///
    /// This is that call, from inside a runtime. It panicked before the fix.
    /// Unlike `model_loaded_ollama_not_running` it is not `#[ignore]`d, because
    /// it does not care which verdict comes back: it asserts only that a
    /// verdict comes back at all, which holds whether or not Ollama is running
    /// on the machine running the test.
    #[tokio::test]
    async fn model_loaded_probe_does_not_panic_inside_a_runtime() {
        let (ok, reason) = probe_worker("model_loaded:permagent-probe-fixture-no-such-model");
        assert_eq!(
            ok,
            reason.is_none(),
            "a probe reports a reason exactly when it reports unavailable: {reason:?}"
        );
    }

    #[test]
    fn cache_returns_within_ttl() {
        let cache = ProbeCache::new();
        cache.set("test_worker", true, None);

        let result = cache.get("test_worker");
        assert!(result.is_some());
        assert!(result.unwrap().available);
    }

    #[test]
    fn cache_miss_for_unknown_key() {
        let cache = ProbeCache::new();
        assert!(cache.get("unknown").is_none());
    }

    #[test]
    fn cache_clear_works() {
        let cache = ProbeCache::new();
        cache.set("worker", true, None);
        assert!(cache.get("worker").is_some());

        cache.clear();
        assert!(cache.get("worker").is_none());
    }

    #[test]
    fn cached_probe_runs_once_and_configuration_changes_miss() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let cache = ProbeCache::new();
        let calls = AtomicUsize::new(0);
        let first = cache.probe_with("worker", "bin_exists:first", |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            (true, None)
        });
        let second = cache.probe_with("worker", "bin_exists:first", |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            (false, Some("must not run".into()))
        });
        let changed = cache.probe_with("worker", "bin_exists:second", |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            (false, Some("changed".into()))
        });

        assert!(first.0);
        assert!(second.0);
        assert!(!changed.0);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
