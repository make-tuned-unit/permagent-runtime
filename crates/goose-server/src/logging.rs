use anyhow::Result;
use tracing_appender::rolling::Rotation;
use tracing_subscriber::{
    filter::LevelFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
    Registry,
};

#[cfg(feature = "otel")]
use permagent::otel::otlp;
use permagent::tracing::langfuse_layer;

/// Sets up the logging infrastructure for the application.
/// This includes:
/// - File-based logging with JSON formatting (DEBUG level)
/// - Console output for development (INFO level)
/// - Optional Langfuse integration (DEBUG level)
pub fn setup_logging(name: Option<&str>) -> Result<()> {
    let log_dir = permagent::logging::prepare_log_directory("server", true)?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let log_filename = if let Some(n) = name {
        format!("{}-{}.log", timestamp, n)
    } else {
        format!("{}.log", timestamp)
    };
    let file_appender =
        tracing_appender::rolling::RollingFileAppender::new(Rotation::NEVER, log_dir, log_filename);

    // Create JSON file logging layer
    let file_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_writer(file_appender)
        .with_ansi(false)
        .with_file(true);

    let base_env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| default_env_filter());

    let console_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_level(true)
        .with_file(true)
        .with_ansi(false)
        .with_line_number(true)
        .pretty();

    let mut layers = vec![
        file_layer.with_filter(base_env_filter.clone()).boxed(),
        console_layer.with_filter(base_env_filter).boxed(),
    ];

    #[cfg(feature = "otel")]
    layers.extend(otlp::init_otlp_layers(permagent::config::Config::global()));

    if let Some(langfuse) = langfuse_layer::create_langfuse_observer() {
        layers.push(langfuse.with_filter(LevelFilter::DEBUG).boxed());
    }

    let subscriber = Registry::default().with(layers);

    subscriber.try_init()?;

    Ok(())
}

/// The default filter used when `RUST_LOG` is unset. The global floor is
/// WARN, so any target that logs state at INFO must be listed here or its
/// lines silently vanish from every sink — the trap that hid the initiative
/// driver's ON/OFF startup lines behind a healthy-looking daemon.
fn default_env_filter() -> EnvFilter {
    EnvFilter::new("")
        .add_directive("mcp_client=info".parse().unwrap())
        // #580: the crate roots. `goose=info` / `goose_server=info` sat here
        // long after the crates were renamed to permagent / permagent-daemon —
        // so every MODULE-PATH info line in both crates (`permagent::…`,
        // `permagent_daemon::…`) was silently dropped while the filter looked
        // healthy. The registry test below now red-builds on that class.
        .add_directive("permagent=info".parse().unwrap())
        .add_directive("permagent_daemon=info".parse().unwrap())
        .add_directive("permagentd=info".parse().unwrap())
        .add_directive("tower_http=info".parse().unwrap())
        // #341: dedicated target for session-list latency instrumentation,
        // visible regardless of crate-name filter drift.
        .add_directive("session_perf=info".parse().unwrap())
        // #360: the initiative driver logs its ON/OFF state at startup under
        // this target; without the directive both lines fall below the WARN
        // floor and the "never silent" contract in initiative::driver::spawn
        // is defeated.
        .add_directive("initiative=info".parse().unwrap())
        // #169: SkillProposed is emitted on the in-process event bus, but the
        // proposal decision must also be visible in launchd's daemon.err.
        .add_directive("auto_skills=info".parse().unwrap())
        // #560: circuit-breaker trips / WAL checkpoints / watchdog. Its
        // error!/warn! lines cleared the WARN floor but the INFO heartbeats
        // ("WAL checkpoint ok") were in the trap.
        .add_directive("durability=info".parse().unwrap())
        .add_directive("steward=info".parse().unwrap())
        // #925: the recognition-instrumentation pruner logs its retention pass
        // under this target; without the directive the INFO line ("pruned N
        // rows") falls below the WARN floor (the #580 trap).
        .add_directive("recognition=info".parse().unwrap())
        // #746: the onboarding coach logs its once-a-day teach offer and its
        // usage/teachable updates under this target; without the directive the
        // INFO lines fall below the WARN floor (the #580 trap).
        .add_directive("onboarding=info".parse().unwrap())
        // Learning playbook: the synthesis worker logs its ON/OFF startup state
        // and per-project distillation, and the decompose consultation logs when
        // it injects hints (the A/B-observable signal). Without the directive
        // both fall below the WARN floor (the #580 trap).
        .add_directive("playbook=info".parse().unwrap())
        // Failure-learning return leg: decompose logs when it injects open
        // incidents (the A/B-observable signal, same contract as playbook).
        // Without the directive the INFO line falls below the WARN floor (#580).
        .add_directive("incidents=info".parse().unwrap())
        // Sovereignty: the egress-audit writer logs its failure modes (audit
        // row write failed / no db pool) under this target — an unlogged cloud
        // call is a lying audit, so these lines must never be dropped (#580).
        .add_directive("sovereignty=info".parse().unwrap())
        // Analytics drain: the outbound pull loop logs what it drained, what it
        // skipped, and why a drain failed. Without a directive the whole target
        // is silently dropped — and a drain that quietly stopped working looks
        // identical to a site with no traffic (#580, caught by the guard test
        // below rather than by anyone noticing missing events).
        .add_directive("analytics_drain=info".parse().unwrap())
        // Coding sessions → Brain: the summary endpoint logs each remembered
        // session under this target; without the directive the INFO line falls
        // below the WARN floor (the #580 trap) and a silently-failing memory
        // pipeline looks identical to nobody coding.
        .add_directive("coding_session=info".parse().unwrap())
        // Prompt caching: the prefix/tail split logs its hit and miss decisions
        // under this target. A cache that silently stopped hitting is the whole
        // failure mode worth watching, and without the directive those lines sit
        // below the WARN floor (the #580 trap) and never appear.
        .add_directive("prompt_cache=info".parse().unwrap())
        // Storage cleanup: the audit trail of a DESTRUCTIVE flow — every path
        // moved to the Trash, its size, its category, and the route that did
        // it (UI click vs bulk sweep vs API). On 2026-08-24 a bulk action
        // trashed 133 GB including five live builds and the logs could not say
        // by which route; without this directive those very lines would sit
        // below the WARN floor (the #580 trap) and the answer would still be
        // missing next time.
        .add_directive("storage_cleanup=info".parse().unwrap())
        .add_directive(LevelFilter::WARN.into())
}

/// Size cap for the launchd stderr/stdout capture files (#560 F5). launchd
/// appends to `daemon.err`/`daemon.log` forever with no rotation; the HTTP
/// access log increases their growth rate, so the daemon caps them itself.
const LAUNCHD_LOG_MAX_BYTES: u64 = 32 * 1024 * 1024;
const ROTATION_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

/// Spawn the size-cap rotation task for `~/.permagent/logs/daemon.{err,log}`.
/// Each file is copy-truncated to a single `.1` backup once it passes
/// [`LAUNCHD_LOG_MAX_BYTES`], so the worst-case footprint per file is 2× the
/// cap. First check runs immediately on boot.
pub fn spawn_launchd_log_rotation() {
    tokio::spawn(async {
        let logs = permagent::config::paths::Paths::in_state_dir("logs");
        let targets = [logs.join("daemon.err"), logs.join("daemon.log")];
        let mut interval = tokio::time::interval(ROTATION_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            for path in &targets {
                match rotate_if_oversize(path, LAUNCHD_LOG_MAX_BYTES) {
                    Ok(Some(bytes)) => {
                        tracing::info!("rotated {} ({} bytes) to .1 backup", path.display(), bytes)
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("log rotation failed for {}: {}", path.display(), e)
                    }
                }
            }
        }
    });
}

/// Copy-truncate `path` to `<path>.1` if it exceeds `max_bytes`, returning
/// the rotated size. launchd holds an O_APPEND fd on these files, so rename
/// would not detach the writer — the contents are copied aside and the file
/// truncated in place (O_APPEND writes continue safely at the new EOF).
/// Lines appended between the copy and the truncate are lost; that is the
/// standard copytruncate trade-off.
fn rotate_if_oversize(path: &std::path::Path, max_bytes: u64) -> std::io::Result<Option<u64>> {
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if len <= max_bytes {
        return Ok(None);
    }

    let mut backup = path.as_os_str().to_owned();
    backup.push(".1");
    std::fs::copy(path, std::path::Path::new(&backup))?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .set_len(0)?;
    Ok(Some(len))
}

#[cfg(test)]
mod tests {
    use super::{default_env_filter, rotate_if_oversize};

    /// Targets that log meaningful state at INFO must be visible under the
    /// default filter — otherwise their lines vanish from every sink and the
    /// component looks like it never ran (the initiative-driver silence bug).
    #[test]
    fn default_filter_passes_dedicated_info_targets() {
        let directives = default_env_filter().to_string();
        for target in [
            "initiative",
            "auto_skills",
            "session_perf",
            "permagentd",
            "permagent",
            "permagent_daemon",
            "durability",
            "steward",
        ] {
            assert!(
                directives.contains(&format!("{target}=info")),
                "default env filter is missing `{target}=info`: {directives}"
            );
        }
    }

    /// #580 — the log-allowlist trap, made a red build instead of a silent
    /// drop: every explicit tracing target string used by a macro in the
    /// daemon or the permagent lib must have its ROOT covered by a directive
    /// in the default filter. Adding a new target without wiring it here now
    /// fails this test instead of shipping an invisible subsystem. (The
    /// scanner reads raw source, comments included — don't write the target
    /// key + quote pattern in prose.)
    #[test]
    fn every_tracing_target_in_source_has_a_default_directive() {
        fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_rs_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let roots = [manifest.join("src"), manifest.join("../goose/src")];
        let mut files = Vec::new();
        for root in &roots {
            assert!(root.exists(), "source root moved: {}", root.display());
            collect_rs_files(root, &mut files);
        }
        assert!(files.len() > 50, "suspiciously few source files scanned");

        let mut target_roots = std::collections::BTreeSet::new();
        for file in &files {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            for chunk in text.split("target: \"").skip(1) {
                if let Some(target) = chunk.split('"').next() {
                    let root = target.split("::").next().unwrap_or(target);
                    target_roots.insert(root.to_string());
                }
            }
        }
        assert!(
            !target_roots.is_empty(),
            "no explicit tracing targets found — scanner broken?"
        );

        let directives = default_env_filter().to_string();
        for root in &target_roots {
            assert!(
                directives.contains(&format!("{root}=")),
                "tracing target root `{root}` is used in source but has no \
                 directive in default_env_filter() — its lines are silently \
                 dropped (the #580 trap). Add `{root}=info` to logging.rs."
            );
        }
    }

    #[test]
    fn rotates_oversize_file_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.err");
        std::fs::write(&path, vec![b'x'; 2048]).unwrap();

        let rotated = rotate_if_oversize(&path, 1024).unwrap();
        assert_eq!(rotated, Some(2048));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

        let backup = dir.path().join("daemon.err.1");
        assert_eq!(std::fs::metadata(&backup).unwrap().len(), 2048);
    }

    #[test]
    fn leaves_small_and_missing_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.err");
        std::fs::write(&path, b"small").unwrap();

        assert_eq!(rotate_if_oversize(&path, 1024).unwrap(), None);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 5);
        assert!(!dir.path().join("daemon.err.1").exists());

        let missing = dir.path().join("nope.err");
        assert_eq!(rotate_if_oversize(&missing, 1024).unwrap(), None);
    }
}
