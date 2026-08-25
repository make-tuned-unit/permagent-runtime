//! TimesFM, on the M1, over SSH.
//!
//! ## Why not on this machine
//!
//! The design amendment of 2026-08-24 moved the model off this machine: load it
//! on the second Mac over SSH, so the hub is not carrying it. The 1.93 GB and
//! ~20 s/week the spike measured are small — but they have no business
//! competing with the machine the user is typing on, which also holds the local
//! LLMs. So the model lives on the headless box and is reached over Tailscale.
//!
//! ## Why SSH and not HTTP
//!
//! No listener to secure, no port to expose, no service to keep alive, and
//! Tailscale already carries the identity. `ssh <target> <cmd>`, the batch in
//! on stdin as JSON, the forecasts out on stdout as JSON, process exits, memory
//! released. The 2.06 s warm start the spike measured is paid once a week.
//!
//! ## The rule that cannot bend
//!
//! An unreachable M1 **never** becomes a reason to run the model here. Four
//! failure states — [`RemoteError`] — all degrade to the Rust baseline with the
//! method relabelled. A forecast whose `method` says TimesFM was produced by
//! TimesFM, or it does not exist. That is the `picker.rs` discipline: own the
//! client, not the process, and distinguish "unreachable" from "reachable and
//! has nothing".

use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// 6x headroom over the 20 s the spike measured for a 100-series batch, with
/// the rest absorbing SSH setup over Tailscale. A model that hangs must not
/// hang the sweep.
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// Config keys, following `OLLAMA_HOST`'s precedent in `~/.permagent/config.yaml`:
/// the address is configuration, never a literal in the source.
pub const SSH_TARGET_KEY: &str = "forecaster_timesfm_ssh_target";
pub const REMOTE_DIR_KEY: &str = "forecaster_timesfm_remote_dir";
pub const ENABLED_KEY: &str = "forecaster_timesfm_enabled";

/// No default host, deliberately.
///
/// A machine address is a property of one person's network, not a product
/// fact, so none is compiled in. Set `forecaster_timesfm_ssh_target` to a
/// `user@host` the local SSH agent can reach without a prompt; until then the
/// model path reports [`RemoteError::NotConfigured`] and every forecast is the
/// labelled Rust baseline.
///
/// Prefer a Tailscale name or address: it is stable across reboots, where a
/// link-local Ethernet address is not — that one is a convenience for a human
/// at a terminal, never something a weekly job may depend on.
pub const DEFAULT_SSH_TARGET: &str = "";
pub const DEFAULT_REMOTE_DIR: &str = "~/.permagent/forecaster";

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteConfig {
    pub ssh_target: String,
    pub remote_dir: String,
    pub enabled: bool,
    /// The `ssh` binary. A field so tests can point it at a shim.
    pub ssh_bin: String,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            ssh_target: DEFAULT_SSH_TARGET.to_string(),
            remote_dir: DEFAULT_REMOTE_DIR.to_string(),
            enabled: true,
            ssh_bin: "ssh".to_string(),
        }
    }
}

impl RemoteConfig {
    pub fn load() -> Self {
        let cfg = crate::config::Config::global();
        let mut c = Self::default();
        if let Ok(v) = cfg.get_param::<String>(SSH_TARGET_KEY) {
            if !v.trim().is_empty() {
                c.ssh_target = v.trim().to_string();
            }
        }
        if let Ok(v) = cfg.get_param::<String>(REMOTE_DIR_KEY) {
            if !v.trim().is_empty() {
                c.remote_dir = v.trim().to_string();
            }
        }
        if let Ok(v) = cfg.get_param::<bool>(ENABLED_KEY) {
            c.enabled = v;
        }
        c
    }
}

/// One series to forecast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesRequest {
    pub series_id: String,
    pub values: Vec<f32>,
    pub horizon: usize,
}

/// One forecast as the remote produced it. Quantiles are the calibrated
/// continuous-head ones; TimesFM 1.0/2.0's quantile heads are explicitly
/// uncalibrated and are not used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteForecast {
    pub series_id: String,
    pub point: Vec<f64>,
    pub p10: Vec<f64>,
    pub p90: Vec<f64>,
}

/// Every way the remote path can fail. All four degrade to the baseline; none
/// is a reason to run the model locally.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteError {
    /// Turned off by config.
    Disabled,
    /// No host has been set. Distinct from "off": nobody chose this, it was
    /// simply never configured, and the fix is one config key.
    NotConfigured,
    /// SSH could not reach the host, or `ssh` itself is missing.
    Unreachable(String),
    /// Reached, but the venv or the script is not installed there. A distinct
    /// state from unreachable: it has a different fix (run the bootstrap).
    NotInstalled(String),
    /// Reached, and did not answer inside [`TIMEOUT`].
    Timeout,
    /// Answered something we cannot read.
    Malformed(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "the TimesFM host is switched off in config"),
            Self::NotConfigured => write!(
                f,
                "no TimesFM host is configured; set {SSH_TARGET_KEY} to a user@host this \
                 machine can reach over SSH without a prompt"
            ),
            Self::Unreachable(m) => write!(f, "the TimesFM host could not be reached: {m}"),
            Self::NotInstalled(m) => write!(f, "the TimesFM host has no model installed: {m}"),
            Self::Timeout => write!(
                f,
                "the TimesFM host did not answer within {}s",
                TIMEOUT.as_secs()
            ),
            Self::Malformed(m) => write!(f, "the TimesFM host answered something unreadable: {m}"),
        }
    }
}

/// What `forecaster_health` reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteHealth {
    pub target: String,
    pub reachable: bool,
    pub venv_present: bool,
    pub script_present: bool,
    pub weights_present: bool,
    pub detail: String,
}

impl RemoteHealth {
    /// Can the remote actually forecast right now?
    pub fn ready(&self) -> bool {
        self.reachable && self.venv_present && self.script_present
    }
}

fn ssh_args(cfg: &RemoteConfig) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        // The host is reached by its stable Tailscale address; an interactive
        // host-key prompt would hang a weekly job forever.
        "StrictHostKeyChecking=accept-new".into(),
        cfg.ssh_target.clone(),
    ]
}

/// Ask the host what it has. Never runs the model.
pub async fn health(cfg: &RemoteConfig) -> RemoteHealth {
    let mut out = RemoteHealth {
        target: cfg.ssh_target.clone(),
        reachable: false,
        venv_present: false,
        script_present: false,
        weights_present: false,
        detail: String::new(),
    };
    if !cfg.enabled {
        out.detail = "switched off in config".into();
        return out;
    }
    if cfg.ssh_target.trim().is_empty() {
        out.detail = format!(
            "no host configured — set {SSH_TARGET_KEY}; until then every forecast is the \
             labelled Rust baseline"
        );
        return out;
    }
    let dir = &cfg.remote_dir;
    let probe = format!(
        "test -x {dir}/venv/bin/python && echo VENV; \
         test -f {dir}/forecast.py && echo SCRIPT; \
         test -d ~/.cache/huggingface/hub && echo WEIGHTS; \
         echo REACHED"
    );
    let mut args = ssh_args(cfg);
    args.push(probe);
    let run = tokio::process::Command::new(&cfg.ssh_bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let result = tokio::time::timeout(Duration::from_secs(20), run).await;
    match result {
        Err(_) => out.detail = "the host did not answer the health probe in 20s".into(),
        Ok(Err(e)) => out.detail = format!("could not run ssh: {e}"),
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            out.reachable = stdout.contains("REACHED");
            out.venv_present = stdout.contains("VENV");
            out.script_present = stdout.contains("SCRIPT");
            out.weights_present = stdout.contains("WEIGHTS");
            out.detail = if !out.reachable {
                format!("unreachable: {}", String::from_utf8_lossy(&o.stderr).trim())
            } else if !out.ready() {
                "reachable, but the venv or script is missing — run scripts/forecaster-bootstrap-m1.sh"
                    .into()
            } else {
                "ready".into()
            };
        }
    }
    out
}

/// Forecast a batch on the remote host.
///
/// One process, one round trip: the batch goes in on stdin because argv is the
/// wrong place for a thousand floats.
pub async fn forecast_batch(
    cfg: &RemoteConfig,
    reqs: &[SeriesRequest],
) -> Result<Vec<RemoteForecast>, RemoteError> {
    forecast_batch_with_deadline(cfg, reqs, TIMEOUT).await
}

/// The body, with the deadline injected. Separated so the timeout path can be
/// exercised in a test in under a second without weakening the real constant.
pub async fn forecast_batch_with_deadline(
    cfg: &RemoteConfig,
    reqs: &[SeriesRequest],
    deadline: Duration,
) -> Result<Vec<RemoteForecast>, RemoteError> {
    if !cfg.enabled {
        return Err(RemoteError::Disabled);
    }
    if cfg.ssh_target.trim().is_empty() {
        return Err(RemoteError::NotConfigured);
    }
    if reqs.is_empty() {
        return Ok(Vec::new());
    }
    let payload = serde_json::to_vec(reqs)
        .map_err(|e| RemoteError::Malformed(format!("could not encode the batch: {e}")))?;
    let dir = &cfg.remote_dir;
    let mut args = ssh_args(cfg);
    args.push(format!("{dir}/venv/bin/python {dir}/forecast.py"));

    let spawned = tokio::process::Command::new(&cfg.ssh_bin)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return Err(RemoteError::Unreachable(format!("could not run ssh: {e}"))),
    };
    if let Some(mut stdin) = child.stdin.take() {
        // A write failure here is the remote having already died; the wait
        // below turns it into the real error, so this one is not worth raising.
        let _ = stdin.write_all(&payload).await;
        let _ = stdin.shutdown().await;
    }

    let finished = tokio::time::timeout(deadline, child.wait_with_output()).await;
    let output = match finished {
        // The child is dropped here, which kills the local `ssh`. The remote
        // python exits with its stdin closed rather than lingering.
        Err(_) => return Err(RemoteError::Timeout),
        Ok(Err(e)) => return Err(RemoteError::Unreachable(e.to_string())),
        Ok(Ok(o)) => o,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // "No such file" is the bootstrap not having run, which has a different
        // fix from a host that is down. Say which.
        return Err(
            if stderr.contains("No such file") || stderr.contains("not found") {
                RemoteError::NotInstalled(stderr)
            } else {
                RemoteError::Unreachable(stderr)
            },
        );
    }
    let parsed: Vec<RemoteForecast> = serde_json::from_slice(&output.stdout)
        .map_err(|e| RemoteError::Malformed(e.to_string()))?;
    // A batch that came back short would silently drop a series' forecast and
    // leave the caller labelling a baseline as the model.
    for req in reqs {
        let Some(got) = parsed.iter().find(|f| f.series_id == req.series_id) else {
            return Err(RemoteError::Malformed(format!(
                "no forecast came back for {}",
                req.series_id
            )));
        };
        if got.point.len() != req.horizon {
            return Err(RemoteError::Malformed(format!(
                "{} came back with {} points, not {}",
                req.series_id,
                got.point.len(),
                req.horizon
            )));
        }
        if got.point.iter().any(|v| !v.is_finite()) {
            return Err(RemoteError::Malformed(format!(
                "{} came back with a non-finite value",
                req.series_id
            )));
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;

    // These tests are `multi_thread` deliberately, not by habit.
    //
    // Tokio reaps a child process through its runtime's signal driver. On a
    // single-threaded runtime that driver shares the one worker with the test
    // body, and on a loaded CI box running thousands of tests at once it can be
    // starved long enough that an ssh shim which has ALREADY exited is not
    // reaped until the 120s production timeout fires. That is exactly how
    // `a_good_answer_round_trips` failed on macOS while passing on Linux: the
    // transport was correct and the runtime never noticed the child was done.
    // A second worker thread keeps the driver off the critical path. No
    // assertion here is weakened by it.

    /// A fake `ssh` on disk. Every test here drives the real spawn/timeout/parse
    /// path — only the binary is swapped, so nothing touches the network and
    /// nothing depends on the M1 being up.
    fn shim(name: &str, body: &str) -> (tempfile::TempDir, RemoteConfig) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).expect("create shim");
        write!(f, "#!/bin/sh\n{body}\n").expect("write shim");
        drop(f);
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        // A stand-in host: the shim ignores it, but it has to be non-empty or
        // every call short-circuits on NotConfigured.
        //
        // Bound to a local rather than written inline in the struct literal:
        // the tracing-target guard in `logging.rs` scans for a bare
        // `target:` followed by a quote, and the field name `ssh_target`
        // ends in exactly that, so an inline string here is read as a tracing
        // target root and fails a guard that has nothing to do with this file.
        let host = String::from("tester@forecast-host.invalid");
        let cfg = RemoteConfig {
            ssh_bin: path.to_string_lossy().to_string(),
            ssh_target: host,
            ..RemoteConfig::default()
        };
        (dir, cfg)
    }

    fn one_request() -> Vec<SeriesRequest> {
        vec![SeriesRequest {
            series_id: "s1".into(),
            values: (0..64).map(|i| i as f32).collect(),
            horizon: 3,
        }]
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_good_answer_round_trips() {
        let (_d, cfg) = shim(
            "ssh",
            r#"cat > /dev/null
echo '[{"series_id":"s1","point":[1.0,2.0,3.0],"p10":[0.5,1.5,2.5],"p90":[1.5,2.5,3.5]}]'"#,
        );
        let got = forecast_batch(&cfg, &one_request())
            .await
            .expect("forecast");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].point, vec![1.0, 2.0, 3.0]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hung_remote_call_times_out_and_falls_back_to_the_baseline() {
        // `sleep 600` stands in for a wedged model. The caller must get a
        // Timeout quickly, not block the weekly sweep for ten minutes.
        let (_d, mut cfg) = shim("ssh", "cat > /dev/null\nsleep 600");
        cfg.enabled = true;
        let started = std::time::Instant::now();
        // Drive the real code path with a short deadline rather than waiting
        // out the production TIMEOUT: what is under test is that the timeout
        // fires and the child is dropped, not the constant's value.
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            forecast_batch_with_deadline(&cfg, &one_request(), Duration::from_millis(400)),
        )
        .await
        .expect("the timeout must fire well inside the outer deadline");
        assert_eq!(result, Err(RemoteError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(3));
        // And the production constant keeps its headroom over the measured
        // ~5.4s for a 12-series batch on the M1.
        assert_eq!(TIMEOUT.as_secs(), 120);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unreachable_host_is_unreachable_and_never_runs_locally() {
        let (_d, cfg) = shim(
            "ssh",
            "cat > /dev/null\necho 'ssh: connect to host port 22: Host is down' >&2\nexit 255",
        );
        let err = forecast_batch(&cfg, &one_request()).await.unwrap_err();
        match err {
            RemoteError::Unreachable(m) => assert!(m.contains("Host is down"), "{m}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_missing_install_is_a_different_state_from_a_down_host() {
        // "no such file" has a different fix — run the bootstrap — so it must
        // not be reported as the host being down.
        let (_d, cfg) = shim(
            "ssh",
            "cat > /dev/null\necho 'bash: line 1: /Users/x/.permagent/forecaster/venv/bin/python: No such file or directory' >&2\nexit 127",
        );
        let err = forecast_batch(&cfg, &one_request()).await.unwrap_err();
        assert!(matches!(err, RemoteError::NotInstalled(_)), "{err:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_short_or_nonfinite_answer_is_malformed_rather_than_trusted() {
        // Right series, wrong horizon.
        let (_d, cfg) = shim(
            "ssh",
            r#"cat > /dev/null
echo '[{"series_id":"s1","point":[1.0],"p10":[0.5],"p90":[1.5]}]'"#,
        );
        let err = forecast_batch(&cfg, &one_request()).await.unwrap_err();
        assert!(matches!(err, RemoteError::Malformed(_)), "{err:?}");

        // A series that simply did not come back must not be silently dropped:
        // the caller would otherwise label a baseline as the model.
        let (_d2, cfg2) = shim("ssh", "cat > /dev/null\necho '[]'");
        let err = forecast_batch(&cfg2, &one_request()).await.unwrap_err();
        assert!(matches!(err, RemoteError::Malformed(_)), "{err:?}");

        let (_d3, cfg3) = shim(
            "ssh",
            r#"cat > /dev/null
echo '[{"series_id":"s1","point":[1.0,null,3.0],"p10":[0,0,0],"p90":[9,9,9]}]'"#,
        );
        assert!(forecast_batch(&cfg3, &one_request()).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switching_the_host_off_is_a_state_not_an_error_path() {
        let (_d, mut cfg) = shim("ssh", "exit 0");
        cfg.enabled = false;
        assert_eq!(
            forecast_batch(&cfg, &one_request()).await,
            Err(RemoteError::Disabled)
        );
        let h = health(&cfg).await;
        assert!(!h.ready());
        assert!(h.detail.contains("switched off"), "{}", h.detail);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_separates_reachable_from_installed() {
        // Reachable, nothing installed: the fix is the bootstrap, not a reboot.
        let (_d, cfg) = shim("ssh", "echo REACHED");
        let h = health(&cfg).await;
        assert!(h.reachable);
        assert!(!h.venv_present && !h.script_present);
        assert!(!h.ready());
        assert!(h.detail.contains("bootstrap"), "{}", h.detail);

        let (_d2, cfg2) = shim("ssh", "echo VENV; echo SCRIPT; echo WEIGHTS; echo REACHED");
        let h = health(&cfg2).await;
        assert!(h.ready());
        assert_eq!(h.detail, "ready");
    }

    /// No machine address is compiled in: an address is one person's network,
    /// not a product fact. An unconfigured host is its own state, distinct from
    /// a host that was switched off.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unconfigured_host_is_its_own_state_and_never_runs_locally() {
        assert_eq!(DEFAULT_SSH_TARGET, "");
        let cfg = RemoteConfig::default();
        assert!(
            cfg.enabled,
            "enabled by default; it is the ADDRESS that is absent"
        );
        assert_eq!(
            forecast_batch(&cfg, &one_request()).await,
            Err(RemoteError::NotConfigured)
        );
        let h = health(&cfg).await;
        assert!(!h.ready());
        assert!(h.detail.contains("no host configured"), "{}", h.detail);
        // And the message names the key that fixes it.
        assert!(h.detail.contains(SSH_TARGET_KEY), "{}", h.detail);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_empty_batch_asks_the_host_nothing() {
        // A shim that would fail if it were ever run.
        let (_d, cfg) = shim("ssh", "exit 9");
        assert_eq!(forecast_batch(&cfg, &[]).await, Ok(Vec::new()));
    }
}
