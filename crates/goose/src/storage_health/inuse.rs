//! "Is something using this right now?" for build and cache directories.
//!
//! # Why this exists
//!
//! On 2026-08-24 the storage-insights recipe ran at ~393 MiB free and a single
//! "Clean Up All" click trashed all 33 findings in about five seconds. One of
//! them was a 133 GB `target/` directory with five live `rustc` processes
//! compiling into it. Every finding said "Safe to remove", because the
//! recommendation was derived from the finding's *type* and nothing else — a
//! cargo target dir is a `dev_cache`, and `dev_cache` was hardcoded to "Safe to
//! remove" regardless of what was happening inside it.
//!
//! A directory being rebuildable is not the same as it being idle. This module
//! answers the second question.
//!
//! # Why not `lsof +D`
//!
//! `lsof +D <dir>` recurses the whole tree. On a 133 GB target directory that
//! takes minutes, which is far too slow to run per finding during a scan (and
//! a scan that takes minutes gets killed or ignored). Every probe here is
//! bounded:
//!
//! * `.cargo-lock` — cargo holds an advisory lock on `<target>/.cargo-lock`
//!   (and, per profile, `<target>/debug/.cargo-lock`) for the whole build.
//!   Checking two exact files is O(1) and is the single most reliable signal
//!   that a cargo build owns this directory.
//! * recent mtime — a bounded, early-exiting walk. An active build touches
//!   files constantly, so the first recently-modified file usually turns up
//!   immediately; the entry and depth caps stop the walk from ever becoming
//!   the slow thing it replaced.
//! * `lsof -F pcn +d <dir>` — `+d` is the TOP LEVEL ONLY (no recursion), so it
//!   returns in milliseconds. It catches a process whose cwd or open
//!   executables sit directly in the directory.
//!
//! Any one of the three is enough to refuse. False positives cost the user a
//! deferred cleanup; a false negative costs them five dead builds.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Default window for the recent-mtime probe, in minutes.
pub const DEFAULT_RECENT_MTIME_MINUTES: u64 = 30;

/// Config key overriding [`DEFAULT_RECENT_MTIME_MINUTES`].
pub const IN_USE_WINDOW_KEY: &str = "storage_in_use_window_minutes";

/// Tuning for the in-use probes.
#[derive(Clone, Copy, Debug)]
pub struct InUseConfig {
    /// A file modified more recently than this means something is still
    /// writing here.
    pub recent_mtime_minutes: u64,
    /// Hard cap on entries examined by the mtime walk, so the probe stays
    /// bounded on a directory with millions of files.
    pub max_entries_scanned: usize,
    /// Depth cap for the mtime walk. Cargo writes into `<target>/<profile>/`
    /// and `<target>/<profile>/{deps,build,incremental}/...`, which sits
    /// comfortably inside four levels.
    pub max_depth: usize,
}

impl Default for InUseConfig {
    fn default() -> Self {
        Self {
            recent_mtime_minutes: DEFAULT_RECENT_MTIME_MINUTES,
            max_entries_scanned: 20_000,
            max_depth: 5,
        }
    }
}

impl InUseConfig {
    /// Production config: the window comes from the user's config when set.
    ///
    /// Resolved HERE, at the impure edge, and never inside a probe — the same
    /// discipline the scanners follow so their tests never depend on global
    /// state (see the note above `scanner.rs`'s tests).
    pub fn from_config() -> Self {
        let mut cfg = Self::default();
        if let Ok(minutes) = crate::config::Config::global().get_param::<u64>(IN_USE_WINDOW_KEY) {
            if minutes > 0 {
                cfg.recent_mtime_minutes = minutes;
            }
        }
        cfg
    }
}

/// One process seen holding something in a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRef {
    pub pid: i32,
    pub command: String,
}

/// The process-listing side of the probe, behind a trait so the classifier
/// tests never shell out to `lsof` (and so they give identical answers on CI,
/// where no build is running and every real probe would say "idle").
pub trait ProcessProbe: Send + Sync {
    /// Processes with something open directly inside `dir` — top level only.
    fn processes_in(&self, dir: &Path) -> Vec<ProcessRef>;
    /// Processes holding the exact file `file` open.
    fn holders_of(&self, file: &Path) -> Vec<ProcessRef>;
}

/// The real probe: `lsof`, always bounded, never recursive.
pub struct LsofProbe;

impl ProcessProbe for LsofProbe {
    fn processes_in(&self, dir: &Path) -> Vec<ProcessRef> {
        run_lsof(&["-w", "-n", "-P", "-F", "pcn", "+d"], dir)
    }

    fn holders_of(&self, file: &Path) -> Vec<ProcessRef> {
        run_lsof(&["-w", "-n", "-P", "-F", "pcn", "--"], file)
    }
}

/// A probe that always reports an idle machine. Used by tests, and as the
/// fallback on platforms without `lsof`.
pub struct NoProcessProbe;

impl ProcessProbe for NoProcessProbe {
    fn processes_in(&self, _dir: &Path) -> Vec<ProcessRef> {
        Vec::new()
    }
    fn holders_of(&self, _file: &Path) -> Vec<ProcessRef> {
        Vec::new()
    }
}

/// Run `lsof` and parse its `-F` field output.
///
/// `lsof` exits non-zero when it finds nothing, and non-zero again when it
/// merely warns — so the exit code is not a usable signal. Parse stdout and
/// let an empty parse mean "nothing found".
fn run_lsof(args: &[&str], target: &Path) -> Vec<ProcessRef> {
    let output = match Command::new("lsof").args(args).arg(target).output() {
        Ok(o) => o,
        // No lsof on this machine (or it could not be spawned): report nothing
        // rather than pretending. The mtime and lockfile probes still apply.
        Err(_) => return Vec::new(),
    };
    parse_lsof_fields(&String::from_utf8_lossy(&output.stdout))
}

/// Parse `lsof -F pcn` output.
///
/// The format is one field per line, tagged by its first character: `p<pid>`
/// opens a process record, `c<command>` names it, and `n<path>` lines follow
/// for each open file. A process with no `n` line under it has nothing open in
/// the target and is dropped — `+d` can emit a bare process record.
pub(crate) fn parse_lsof_fields(stdout: &str) -> Vec<ProcessRef> {
    let mut out: Vec<ProcessRef> = Vec::new();
    let mut pid: Option<i32> = None;
    let mut command = String::new();
    let mut recorded_current = false;

    for line in stdout.lines() {
        let mut chars = line.chars();
        let Some(tag) = chars.next() else {
            continue;
        };
        let value = chars.as_str();
        match tag {
            'p' => {
                pid = value.trim().parse::<i32>().ok();
                command.clear();
                recorded_current = false;
            }
            'c' => command = value.trim().to_string(),
            'n' | 'f' => {
                // Any file record under the current process confirms it.
                if !recorded_current {
                    if let Some(p) = pid {
                        out.push(ProcessRef {
                            pid: p,
                            command: if command.is_empty() {
                                "process".to_string()
                            } else {
                                command.clone()
                            },
                        });
                        recorded_current = true;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Why a directory is considered in use, in the user's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InUseReason {
    /// One line, ready to show: "5 rustc processes are compiling here".
    pub consequence: String,
}

/// Decide whether `dir` is actively in use, and say why.
///
/// Returns `None` when every bounded probe comes back idle. Probes run
/// cheapest-first and short-circuit.
pub fn in_use_reason(
    dir: &Path,
    cfg: &InUseConfig,
    probe: &dyn ProcessProbe,
) -> Option<InUseReason> {
    if !dir.is_dir() {
        return None;
    }

    // 1. A cargo build's lock file. Two stat calls, and the most specific
    //    signal there is: if it is held, a build owns this target directory.
    if let Some(reason) = cargo_lock_reason(dir, probe) {
        return Some(reason);
    }

    // 2. Something wrote here in the last N minutes. A zero window disables
    //    this probe outright — the scanner's own tests use it so a fixture
    //    written a millisecond ago doesn't read as a live build.
    if let Some(minutes) = recently_modified_minutes(dir, cfg) {
        return Some(InUseReason {
            consequence: match minutes {
                0 => "files here changed less than a minute ago — a build or install is still writing to it".to_string(),
                1 => "files here changed 1 minute ago — a build or install is still writing to it".to_string(),
                m => format!(
                    "files here changed {} minutes ago — a build or install is still writing to it",
                    m
                ),
            },
        });
    }

    // 3. A process with something open at the top level (cwd, an executable).
    let procs = probe.processes_in(dir);
    if !procs.is_empty() {
        return Some(InUseReason {
            consequence: summarize_processes(&procs),
        });
    }

    None
}

/// `<target>/.cargo-lock` and `<target>/<profile>/.cargo-lock`, held.
fn cargo_lock_reason(dir: &Path, probe: &dyn ProcessProbe) -> Option<InUseReason> {
    let candidates = [
        dir.join(".cargo-lock"),
        dir.join("debug/.cargo-lock"),
        dir.join("release/.cargo-lock"),
    ];
    for lock in &candidates {
        if !lock.is_file() {
            continue;
        }
        let holders = probe.holders_of(lock);
        if !holders.is_empty() {
            return Some(InUseReason {
                consequence: summarize_processes(&holders),
            });
        }
    }
    None
}

/// Age in whole minutes of the most recently modified file under `dir`, if
/// that age is inside the configured window. Bounded and early-exiting.
fn recently_modified_minutes(dir: &Path, cfg: &InUseConfig) -> Option<u64> {
    if cfg.recent_mtime_minutes == 0 {
        return None;
    }
    let window = Duration::from_secs(cfg.recent_mtime_minutes.saturating_mul(60));
    let now = SystemTime::now();

    let mut seen = 0usize;

    let walker = ignore::WalkBuilder::new(dir)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .max_depth(Some(cfg.max_depth))
        .build();

    for entry in walker.flatten() {
        seen += 1;
        if seen > cfg.max_entries_scanned {
            break;
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        // A clock skew that puts mtime in the future reads as age zero, which
        // is the cautious answer: treat it as active.
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age <= window {
            // Early exit — one recent file is the whole answer.
            return Some(age.as_secs() / 60);
        }
    }
    None
}

/// Turn a process list into one plain line.
pub(crate) fn summarize_processes(procs: &[ProcessRef]) -> String {
    if procs.is_empty() {
        return "a process is using this directory".to_string();
    }
    // Report the dominant command name — during a cargo build that is `rustc`,
    // many times over, and "5 rustc processes" is the sentence that makes the
    // consequence obvious.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for p in procs {
        *counts.entry(p.command.as_str()).or_insert(0) += 1;
    }
    let (command, count) = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(c, n)| (*c, *n))
        .unwrap_or(("process", procs.len()));

    let plural = if count == 1 { "" } else { "es" };
    let verb = if count == 1 { "is" } else { "are" };
    if is_compiler(command) {
        format!(
            "{} {} process{} {} compiling here",
            count, command, plural, verb
        )
    } else {
        format!(
            "{} {} process{} {} using this directory",
            count, command, plural, verb
        )
    }
}

fn is_compiler(command: &str) -> bool {
    matches!(
        command,
        "rustc" | "cargo" | "cc" | "clang" | "clang++" | "swiftc" | "ld" | "tsc" | "node" | "go"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A probe that answers from a fixed script, so tests never shell out.
    struct ScriptedProbe {
        in_dir: Vec<ProcessRef>,
        holders: Vec<(PathBuf, Vec<ProcessRef>)>,
    }

    impl ScriptedProbe {
        fn idle() -> Self {
            Self {
                in_dir: Vec::new(),
                holders: Vec::new(),
            }
        }
    }

    impl ProcessProbe for ScriptedProbe {
        fn processes_in(&self, _dir: &Path) -> Vec<ProcessRef> {
            self.in_dir.clone()
        }
        fn holders_of(&self, file: &Path) -> Vec<ProcessRef> {
            self.holders
                .iter()
                .find(|(p, _)| p == file)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        }
    }

    fn rustc(pid: i32) -> ProcessRef {
        ProcessRef {
            pid,
            command: "rustc".into(),
        }
    }

    /// Backdate a file's mtime. No extra crate needed: `File::set_times` has
    /// been stable since 1.75 and this repo builds on 1.91.
    fn set_mtime(path: &Path, ago: Duration) {
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_times(fs::FileTimes::new().set_modified(SystemTime::now() - ago))
            .unwrap();
    }

    /// Backdate a file well outside any test window.
    fn make_old(path: &Path) {
        set_mtime(path, Duration::from_secs(60 * 60 * 24 * 30));
    }

    #[test]
    fn recent_file_makes_a_target_dir_in_use() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        fs::write(target.join("debug/deps/libfoo.rlib"), "fresh").unwrap();

        let reason = in_use_reason(&target, &InUseConfig::default(), &ScriptedProbe::idle())
            .expect("a file written just now must read as in use");
        assert!(
            reason.consequence.contains("still writing"),
            "unexpected consequence: {}",
            reason.consequence
        );
    }

    #[test]
    fn an_idle_target_dir_is_not_in_use() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        let f = target.join("debug/libfoo.rlib");
        fs::write(&f, "cold").unwrap();
        make_old(&f);

        assert!(
            in_use_reason(&target, &InUseConfig::default(), &ScriptedProbe::idle()).is_none(),
            "a directory whose only file is a month old must not read as in use"
        );
    }

    #[test]
    fn a_held_cargo_lock_makes_a_target_dir_in_use() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let lock = target.join(".cargo-lock");
        fs::write(&lock, "").unwrap();
        make_old(&lock);

        let probe = ScriptedProbe {
            in_dir: Vec::new(),
            holders: vec![(lock, vec![rustc(1), rustc(2), rustc(3), rustc(4), rustc(5)])],
        };
        let reason = in_use_reason(&target, &InUseConfig::default(), &probe)
            .expect("a held .cargo-lock must read as in use");
        assert_eq!(reason.consequence, "5 rustc processes are compiling here");
    }

    #[test]
    fn an_unheld_cargo_lock_is_not_in_use() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let lock = target.join(".cargo-lock");
        fs::write(&lock, "").unwrap();
        make_old(&lock);

        assert!(
            in_use_reason(&target, &InUseConfig::default(), &ScriptedProbe::idle()).is_none(),
            "a .cargo-lock nobody holds is just a leftover file"
        );
    }

    #[test]
    fn a_profile_cargo_lock_is_checked_too() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        let lock = target.join("debug/.cargo-lock");
        fs::write(&lock, "").unwrap();
        make_old(&lock);

        let probe = ScriptedProbe {
            in_dir: Vec::new(),
            holders: vec![(lock, vec![rustc(9)])],
        };
        let reason = in_use_reason(&target, &InUseConfig::default(), &probe).unwrap();
        assert_eq!(reason.consequence, "1 rustc process is compiling here");
    }

    #[test]
    fn a_process_with_the_dir_open_makes_it_in_use() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("node_modules");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("marker");
        fs::write(&f, "x").unwrap();
        make_old(&f);

        let probe = ScriptedProbe {
            in_dir: vec![ProcessRef {
                pid: 42,
                command: "vite".into(),
            }],
            holders: Vec::new(),
        };
        let reason = in_use_reason(&dir, &InUseConfig::default(), &probe).unwrap();
        assert_eq!(reason.consequence, "1 vite process is using this directory");
    }

    #[test]
    fn a_missing_dir_is_never_in_use() {
        assert!(in_use_reason(
            Path::new("/nonexistent/storage-safety-xyz"),
            &InUseConfig::default(),
            &ScriptedProbe::idle()
        )
        .is_none());
    }

    #[test]
    fn window_is_configurable() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("target");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("out");
        fs::write(&f, "x").unwrap();
        // 10 minutes old.
        set_mtime(&f, Duration::from_secs(10 * 60));

        let narrow = InUseConfig {
            recent_mtime_minutes: 5,
            ..InUseConfig::default()
        };
        assert!(
            in_use_reason(&dir, &narrow, &ScriptedProbe::idle()).is_none(),
            "a 10-minute-old file is outside a 5-minute window"
        );

        let wide = InUseConfig {
            recent_mtime_minutes: 30,
            ..InUseConfig::default()
        };
        assert!(
            in_use_reason(&dir, &wide, &ScriptedProbe::idle()).is_some(),
            "the same file is inside the default 30-minute window"
        );
    }

    #[test]
    fn lsof_field_output_parses_into_processes() {
        // Real `lsof -F pcn` shape: a process record, then its open files.
        let out = "p1234\ncrustc\nfcwd\nn/Users/j/dev/repo/target\np5678\nccargo\nftxt\nn/Users/j/dev/repo/target/.cargo-lock\n";
        let procs = parse_lsof_fields(out);
        assert_eq!(
            procs,
            vec![
                ProcessRef {
                    pid: 1234,
                    command: "rustc".into()
                },
                ProcessRef {
                    pid: 5678,
                    command: "cargo".into()
                },
            ]
        );
    }

    #[test]
    fn lsof_output_with_no_open_files_yields_nothing() {
        assert!(parse_lsof_fields("").is_empty());
        // A bare process record with no file lines is not evidence.
        assert!(parse_lsof_fields("p1234\ncrustc\n").is_empty());
    }

    #[test]
    fn summaries_name_the_dominant_process() {
        assert_eq!(
            summarize_processes(&[rustc(1), rustc(2)]),
            "2 rustc processes are compiling here"
        );
        assert_eq!(
            summarize_processes(&[ProcessRef {
                pid: 3,
                command: "Finder".into()
            }]),
            "1 Finder process is using this directory"
        );
    }
}
