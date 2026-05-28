//! Worker availability probing and caching.
//!
//! Checks whether a worker is available on this machine based on its
//! `availability_check` string from agent.yaml.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
}

/// Probe whether a worker is available based on its `availability_check` string.
///
/// Supported formats:
/// - `"always"` — always available
/// - `"bin_exists:<name>"` — check if binary is on PATH
/// - `"api_credential:<env_var>"` — check if environment variable is set
/// - `"model_loaded:<model>"` — check if Ollama model is pulled (HTTP call)
///
/// Returns `(available, reason)` where reason is `None` if available,
/// or a human-readable explanation if not.
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

fn probe_bin_exists(name: &str) -> (bool, Option<String>) {
    match which::which(name) {
        Ok(_) => (true, None),
        Err(_) => (false, Some(format!("Binary '{}' not found on PATH", name))),
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

fn probe_model_loaded(model: &str) -> (bool, Option<String>) {
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
        assert!(reason.unwrap().contains("not found on PATH"));
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
}
