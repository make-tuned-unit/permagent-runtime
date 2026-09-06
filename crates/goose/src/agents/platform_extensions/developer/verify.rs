//! The `verify` tool — close the coding loop by running the project's checks.
//!
//! The coding harness's robustness bar is *verify-then-fix*: after editing,
//! RUN the project's tests/build/lint, feed the FULL errors back, fix, and
//! repeat — bounded, then escalate. The agent could already do this ad hoc
//! through the generic `shell` tool; what was missing is making it RELIABLE:
//!
//! - **Deterministic detection.** A pure `detect_checks` maps a directory's
//!   marker files (`Cargo.toml`, `package.json` + its scripts, `go.mod`,
//!   `pyproject.toml`/…, a `Makefile` with a `test`/`check` target) to the
//!   right ordered check command(s). No per-task guessing.
//! - **Full error capture.** Error-guided fixing needs the *real* errors, so a
//!   failing check returns its combined stdout+stderr intact (not the generic
//!   shell tool's blind 50-line tail). When output is genuinely huge it is
//!   bounded head-AND-tail so the first compiler errors and the final summary
//!   both survive.
//! - **Structured pass/fail.** Success is a clean `CallToolResult::success`;
//!   failure is a `CallToolResult::error` (so `is_error` is set) carrying the
//!   failing command, its exit code, and the captured errors.
//!
//! ## Bounding is REUSED, not reinvented
//!
//! There is deliberately **no repair-round counter here**. The run-loop bound
//! is the existing [`crate::tool_monitor`] `ProgressMonitor` (loop-safety
//! #715): its **verify-loop signal (S6)** counts CONSECUTIVE identical `verify`
//! failures for a goal *across the edits between them* — the real fix-loop shape
//! (`verify` → `edit` → `verify` → …), which the plain S1/S2 identical-call run
//! misses because each interleaved edit breaks that run. S6 nudges at 2 and
//! escalates at 3. For that floor to bind, the *same* failure must render
//! *identically*, so [`normalize`] neutralizes the spans that vary between two
//! otherwise-identical re-runs on one machine — tempdir paths, ephemeral
//! `host:port` binds, pids, heap addresses, UUIDs, ISO-8601 timestamps, and
//! reported durations. A failure whose errors GENUINELY change hashes
//! differently and is treated as progress (the monitor's state-change
//! discriminator), which is correct: real fixing must pass through. On the Nth
//! identical failure the verifier-driven escalation controller
//! ([`crate::cost_router::VerifyEscalation`]) climbs the fix to a stronger
//! model.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

/// Marker carried in `CallToolResult::structured_content` for the verifier's
/// machine-readable result. Consumers must use this field rather than parsing
/// the human-facing text content; the latter is intentionally free to change
/// and may contain untrusted command output.
pub const VERIFICATION_OBSERVATION_KIND: &str = "permagent.verification.v1";

/// Structured result emitted by the built-in `verify` tool. This is the
/// authoritative producer for harness verification telemetry.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct VerificationObservation {
    pub kind: String,
    pub command: Option<String>,
    pub verdict: Option<String>,
    pub evidence: Option<String>,
}

/// Return true only for a successful, structured verifier observation.
///
/// The textual content on a tool result is presentation, not an approval
/// protocol: a response that merely says "PASS" (or an RPC success with no
/// observation) must never advance a completion gate. Keeping this predicate
/// beside the producer gives every consumer one authoritative contract.
pub fn is_authoritative_pass(result: &CallToolResult) -> bool {
    if result.is_error == Some(true) {
        return false;
    }
    let Some(value) = result.structured_content.as_ref() else {
        return false;
    };
    let Ok(observation) = serde_json::from_value::<VerificationObservation>(value.clone()) else {
        return false;
    };
    observation.kind == VERIFICATION_OBSERVATION_KIND
        && observation.verdict.as_deref() == Some("pass")
}

/// The verification boundary requested by the caller. `Auto` is deliberately
/// conservative: it scopes checks to the changed files and never upgrades a
/// docs or package change into a workspace-wide Rust test.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationScope {
    Auto,
    Docs,
    Rust,
    Integration,
}

/// Risk is metadata for the selection decision, not a reason to broaden a
/// check. High-risk changes still require the explicit `integration` boundary
/// to run a workspace-wide test.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRisk {
    Low,
    Standard,
    High,
}

/// Per-check timeout when the caller does not specify one. Generous enough for
/// real builds/test suites, but bounded so a hung command can't wedge the
/// agent. A `timeout_secs` of 0 disables the timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Failure output is returned in full up to this many lines; beyond it we keep
/// the head and the tail (below) with an elision in between. Large, because
/// error-guided fixing needs the real errors — but still bounded.
const MAX_OUTPUT_LINES: usize = 400;
/// Lines kept from the top when clamping (compiler errors lead the output).
const HEAD_LINES: usize = 250;
/// Lines kept from the bottom when clamping (the final summary trails it).
const TAIL_LINES: usize = 120;
/// Documentation checks must never turn a large or special file into an
/// unbounded verifier read. Two MiB is ample for a source document while
/// keeping the check deterministic and cheap.
const MAX_MARKDOWN_BYTES: u64 = 2 * 1024 * 1024;

/// Directory markers that identify a Python project.
const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "tox.ini",
];

// Volatile spans that differ between two OTHERWISE-identical failing re-runs on
// the same machine. Neutralizing them is what lets the loop guard's same-failure
// cap bind to a genuinely-repeated verify failure (see the module docs). Every
// pattern is deliberately NARROW: it collapses only its volatile token, never
// stable text — so two DIFFERENT failures still hash differently instead of
// colliding into a false "same failure" that would escalate real progress.
//
// [`normalize`] applies these in declaration order. UUIDs and timestamps run
// before the coarser address/port patterns so their internal `-`/`:`/`.` are not
// half-consumed.

/// A UUID (any version), e.g. `550e8400-e29b-41d4-a716-446655440000`.
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
});
/// ISO-8601 / RFC-3339 timestamps, e.g. `2026-07-16T12:34:56.789Z` or
/// `2026-07-16 12:34:56+01:00`.
static TIMESTAMP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?").unwrap()
});
/// macOS per-process temp root `/var/folders/<xx>/<hash>` — collapse the two
/// random components while keeping any stable `/T/…` suffix, so distinct files
/// under it still differ.
static VAR_FOLDERS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/var/folders/[^/\s]+/[^/\s]+").unwrap());
/// The `tempfile` crate's random component (`.tmpXXXXXX`) — used under `/tmp/…`
/// and elsewhere. Collapsing just this token preserves any stable path suffix.
static TMP_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.tmp[A-Za-z0-9]{4,}").unwrap());
/// An ephemeral `host:port` bind — an IPv4 dotted-quad followed by a port.
/// Anchored on the four octets so it can never touch a `file.rs:12` line number.
static IPV4_PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}:\d{1,5}\b").unwrap());
/// A `localhost:<port>` bind (the port is the volatile half).
static LOCALHOST_PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\blocalhost:\d{1,5}\b").unwrap());
/// A pid, written `pid=1234`, `pid: 1234`, or `pid 1234`.
static PID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bpid[=:\s]\s*\d+").unwrap());
/// A heap/pointer address, e.g. `0x7ffee3b2a1c0`.
static HEX_ADDR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b0x[0-9a-fA-F]+\b").unwrap());
/// Reported durations — the residual timing that varies even when nothing else
/// does. Longest unit first so the alternation prefers "seconds" over a bare "s".
static DURATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+\.\d+\s*(seconds|secs|ms|s)\b").unwrap());

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VerifyParams {
    /// Explicit command to run instead of auto-detecting the project's checks,
    /// e.g. "cargo clippy -- -D warnings" or "npm run typecheck". When omitted,
    /// verify detects the project type and runs its build/test checks.
    pub command: Option<String>,
    /// Directory to verify. Relative paths resolve against the working
    /// directory; defaults to the working directory.
    pub path: Option<String>,
    /// Per-check timeout in seconds (default 600; 0 disables the timeout).
    pub timeout_secs: Option<u64>,
    /// Verification scope. Auto scopes checks from the changed files; use
    /// `integration` only at an intentional integration boundary.
    pub scope: Option<VerificationScope>,
    /// Optional risk annotation used in the selection rationale. It never
    /// silently broadens verification.
    pub risk: Option<VerificationRisk>,
    /// Changed paths supplied by the caller (relative to `path`). This avoids
    /// treating unrelated dirty worktree files as this task's scope. When
    /// omitted, tracked and untracked git changes under `path` are inspected.
    #[serde(alias = "files", alias = "changed_files")]
    pub changed_paths: Option<Vec<String>>,
}

/// How a check is executed: either a resolved program + args run directly (no
/// shell, no quoting — mirrors how the search tool runs ripgrep), or a raw
/// command string run through the platform shell (the explicit-`command` path).
#[derive(Debug, Clone, PartialEq)]
enum Exec {
    Direct { program: String, args: Vec<String> },
    Shell(String),
    Markdown { files: Vec<PathBuf> },
}

/// One check command to run: a human label plus how to execute it.
#[derive(Debug, Clone, PartialEq)]
struct Check {
    label: String,
    exec: Exec,
}

impl Check {
    /// A check run directly as `program arg arg …` (no shell).
    fn direct(program: &str, args: &[&str]) -> Self {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        Self::direct_owned(program, args)
    }

    fn direct_owned(program: &str, args: Vec<String>) -> Self {
        let label = if args.is_empty() {
            program.to_string()
        } else {
            format!("{program} {}", args.join(" "))
        };
        Self {
            label,
            exec: Exec::Direct {
                program: program.to_string(),
                args,
            },
        }
    }

    /// A check run through the platform shell (used for an explicit `command`).
    fn shell(command: &str) -> Self {
        Self {
            label: command.to_string(),
            exec: Exec::Shell(command.to_string()),
        }
    }

    fn markdown(files: Vec<PathBuf>) -> Self {
        Self {
            label: "bounded markdown hygiene checks".to_string(),
            exec: Exec::Markdown { files },
        }
    }

    /// The program whose absence would make this check unrunnable.
    fn program_name(&self) -> String {
        match &self.exec {
            Exec::Direct { program, .. } => program.clone(),
            // The missing program is the one the command line names, not the
            // shell that would have run it. Reporting "`shell` was not found on
            // PATH ... Install it" sent a local run looking for a tool called
            // `shell`; the tool actually missing was `npm`.
            Exec::Shell(command) => command
                .split_whitespace()
                .next()
                .unwrap_or("shell")
                .to_string(),
            Exec::Markdown { .. } => "markdown checker".to_string(),
        }
    }
}

/// The result of detecting a project's checks: either the ordered checks to run,
/// or a recognized project that simply has nothing runnable to verify yet (a
/// fresh scaffold). "No checks configured" is NOT a failure — surfacing it as one
/// would make a weak model chase a phantom failure on a brand-new project.
#[derive(Debug, Clone, PartialEq)]
enum DetectOutcome {
    Checks(Vec<Check>),
    NoChecksConfigured(String),
}

/// The outcome of running one check.
enum CheckOutcome {
    Passed,
    Failed {
        exit_code: Option<i32>,
        output: String,
    },
    TimedOut,
    ToolMissing,
    SpawnError(String),
}

pub struct VerifyTool;

impl VerifyTool {
    pub fn new() -> Self {
        Self
    }

    pub async fn verify_with_cwd(
        &self,
        params: VerifyParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        let base_dir = resolve_dir(working_dir, params.path.as_deref());

        // An explicit command overrides detection; otherwise detect the checks.
        let checks = match params.command.as_deref().map(str::trim) {
            Some("") => {
                return error_result(
                    "`command` cannot be empty — omit it to auto-detect the project's checks.",
                );
            }
            Some(command) => vec![Check::shell(command)],
            None => match select_checks(
                &base_dir,
                params.scope.unwrap_or(VerificationScope::Auto),
                params.risk.unwrap_or(VerificationRisk::Standard),
                params.changed_paths.as_deref(),
            ) {
                Ok(DetectOutcome::Checks(checks)) => checks,
                // A recognized project with nothing to verify yet is not a failure.
                Ok(DetectOutcome::NoChecksConfigured(note)) => return no_checks_result(&note),
                Err(message) => return error_result(&message),
            },
        };

        let timeout_secs = params.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);

        // Fail-fast: run checks in order, returning the first failure with its
        // full errors (you can't meaningfully test what won't build).
        for check in &checks {
            match run_check(check, &base_dir, timeout_secs).await {
                CheckOutcome::Passed => continue,
                CheckOutcome::Failed { exit_code, output } => {
                    // pytest exits 5 when it collected no tests — a fresh scaffold
                    // with no tests yet, not a real failure. Treat as no-checks so
                    // the model doesn't chase a phantom failure.
                    if is_no_tests_collected(check, exit_code) {
                        return no_checks_result(&format!(
                            "`{}` collected no tests (pytest exit 5) — there are no tests \
                             to run yet.",
                            check.label
                        ));
                    }
                    // jest/vitest exit 1 with "no tests found" when zero test files
                    // match — same empty-scaffold case as pytest exit 5.
                    if is_no_test_files_found(check, exit_code, &output) {
                        return no_checks_result(&format!(
                            "`{}` found no test files (the runner exited 1 with \"no tests \
                             found\") — there are no tests to run yet.",
                            check.label
                        ));
                    }
                    return fail_result(check, exit_code, &output);
                }
                CheckOutcome::TimedOut => {
                    return error_result(&format!(
                        "TIMEOUT - `{}` did not finish within {timeout_secs}s and was stopped. \
                         Re-run with a larger `timeout_secs`, or run a narrower check.",
                        check.label
                    ));
                }
                CheckOutcome::ToolMissing => {
                    return error_result(&format!(
                        "`{}` was not found on PATH, so `{}` could not run. Install it, or pass \
                         an explicit `command` that uses tools you have.",
                        check.program_name(),
                        check.label
                    ));
                }
                CheckOutcome::SpawnError(error) => {
                    return error_result(&format!("Could not run `{}`: {error}", check.label));
                }
            }
        }

        pass_result(&checks)
    }
}

impl Default for VerifyTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the directory to verify: an absolute `path` as-is, a relative `path`
/// against the working dir, or the working dir itself.
fn resolve_dir(working_dir: Option<&Path>, path: Option<&str>) -> PathBuf {
    let base = working_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    match path {
        Some(path) => {
            let candidate = Path::new(path);
            if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                base.join(candidate)
            }
        }
        None => base,
    }
}

// --- detection (pure over the directory's marker files) ---------------------

/// Detect the ordered check command(s) for the project rooted at `dir`.
/// Precedence is language-native first (unambiguous), then a Makefile fallback.
/// Returns an actionable `Err` when nothing runnable is found so the caller can
/// surface it instead of silently doing nothing.
fn detect_checks(dir: &Path) -> Result<DetectOutcome, String> {
    if dir.join("Cargo.toml").exists() {
        // `cargo test` compiles first, so it catches build errors and test
        // failures in one command.
        return Ok(DetectOutcome::Checks(vec![Check::direct(
            "cargo",
            &["test"],
        )]));
    }
    if dir.join("go.mod").exists() {
        return Ok(DetectOutcome::Checks(vec![
            Check::direct("go", &["build", "./..."]),
            Check::direct("go", &["test", "./..."]),
        ]));
    }
    if dir.join("package.json").exists() {
        return node_checks(dir);
    }
    if PYTHON_MARKERS
        .iter()
        .any(|marker| dir.join(marker).exists())
    {
        return Ok(DetectOutcome::Checks(vec![Check::direct("pytest", &[])]));
    }
    if let Some(target) = makefile_check_target(dir) {
        return Ok(DetectOutcome::Checks(vec![Check::direct(
            "make",
            &[target.as_str()],
        )]));
    }
    Err(
        "Could not detect the project type (looked for Cargo.toml, go.mod, \
         package.json, a Python project marker, or a Makefile with a test/check \
         target). Pass an explicit `command` to run this project's checks."
            .to_string(),
    )
}

/// Select checks for the current change set. This is intentionally separate
/// from marker-file detection: the latter remains a useful pure fallback and
/// its language behavior is stable, while the generic tool must not turn a
/// docs-only or narrow Rust edit into a workspace build.
fn select_checks(
    dir: &Path,
    scope: VerificationScope,
    risk: VerificationRisk,
    supplied_changes: Option<&[String]>,
) -> Result<DetectOutcome, String> {
    if scope == VerificationScope::Integration {
        if !dir.join("Cargo.toml").is_file() {
            return Err(
                "integration scope requires a Cargo workspace at the verification path".to_string(),
            );
        }
        return Ok(DetectOutcome::Checks(vec![Check::direct(
            "cargo",
            &["test", "--workspace", "--all-targets"],
        )]));
    }

    let changes = match supplied_changes {
        Some(paths) => normalize_supplied_changes(dir, paths)?,
        None => match git_changes_under(dir) {
            Some(changes) => changes,
            None if dir.join("Cargo.toml").is_file() => {
                return Ok(DetectOutcome::NoChecksConfigured(
                    "could not determine the changed files, so no Cargo check is inferred; pass an "
                        .to_string()
                        + "explicit command or provide `changed_paths`",
                ));
            }
            None => return detect_checks(dir),
        },
    };
    if changes.is_empty() {
        return Ok(DetectOutcome::NoChecksConfigured(
            "no changed files were found under this verification path — no scoped check is safe to infer"
                .to_string(),
        ));
    }

    let docs_only = changes.iter().all(|path| is_documentation_path(path));
    if scope == VerificationScope::Docs && !docs_only {
        return Err(
            "docs scope was requested, but the changed set includes non-documentation files; "
                .to_string(),
        );
    }
    if docs_only && matches!(scope, VerificationScope::Docs | VerificationScope::Auto) {
        // This check is deterministic, local, and does not compile anything.
        // It catches syntax-shaped defects without pretending to validate
        // prose meaning.
        return Ok(DetectOutcome::Checks(vec![markdown_check(dir, &changes)?]));
    }
    match scope {
        VerificationScope::Rust | VerificationScope::Auto => {
            if changes.iter().all(|path| is_rust_path(path)) {
                return narrow_rust_checks(dir, &changes);
            }
            if scope == VerificationScope::Rust {
                return Err(format!(
                    "scoped Rust verification cannot cover this change set (risk={risk:?}); \
                     pass `scope: \"integration\"` at the integration boundary or an explicit `command`",
                ));
            }
            if changes.iter().any(|path| is_rust_path(path)) {
                return Err("auto verification found a mixed Rust/non-Rust change set; provide `changed_paths` for this task or pass an explicit command (workspace-wide integration tests require `scope: \"integration\"`)".to_string());
            }
            // Preserve the existing language-native behavior for non-Rust
            // projects. A Cargo marker alone is not permission to run a broad
            // test when the changed set is unrelated to Rust.
            if dir.join("Cargo.toml").is_file() {
                return Ok(DetectOutcome::NoChecksConfigured("the changed files are not Rust, so no Cargo check is inferred; use the matching project check or an explicit command".to_string()));
            }
            detect_checks(dir)
        }
        VerificationScope::Docs => unreachable!("handled before scope dispatch"),
        VerificationScope::Integration => unreachable!(),
    }
}

/// Documentation extensions accepted by the lightweight markdown check.
fn is_documentation_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("docs/")
        || lower.starts_with(".docs/")
        || matches!(
            Path::new(&lower).extension().and_then(|e| e.to_str()),
            Some("md" | "mdx" | "markdown" | "rst" | "txt")
        )
}

fn is_rust_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".rs")
        || lower.ends_with("/cargo.toml")
        || lower == "cargo.toml"
        || lower.ends_with("/cargo.lock")
        || lower == "cargo.lock"
        || lower.ends_with("/.cargo/config")
        || lower.ends_with("/.cargo/config.toml")
        || lower == "build.rs"
        || lower.ends_with("/build.rs")
}

fn markdown_check(dir: &Path, paths: &[String]) -> Result<Check, String> {
    let root = git_root(dir).unwrap_or_else(|| dir.to_path_buf());
    let root = root.canonicalize().unwrap_or(root);
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let candidate = root.join(path);
        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(|error| format!("cannot inspect documentation path `{path}`: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!(
                "documentation path `{path}` must be a regular file inside the verification root"
            ));
        }
        if metadata.len() > MAX_MARKDOWN_BYTES {
            return Err(format!(
                "documentation path `{path}` exceeds the {MAX_MARKDOWN_BYTES}-byte verification limit"
            ));
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("cannot resolve documentation path `{path}`: {error}"))?;
        if !canonical.starts_with(&root) {
            return Err(format!(
                "documentation path `{path}` resolves outside the verification repository"
            ));
        }
        files.push(canonical);
    }
    Ok(Check::markdown(files))
}

/// Validate caller-supplied paths and normalize them to repository-root
/// relative paths, matching the representation returned by
/// [`git_changes_under`]. This keeps subdirectory verification honest and
/// rejects absolute/traversal inputs before any filesystem access.
fn normalize_supplied_changes(dir: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let root = git_root(dir).unwrap_or_else(|| dir.to_path_buf());
    let root = root.canonicalize().unwrap_or(root);
    let dir = dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve verification path: {error}"))?;
    if !dir.starts_with(&root) {
        return Err("verification path is outside the repository root".to_string());
    }
    let mut normalized = Vec::new();
    for raw in paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let supplied = Path::new(trimmed);
        if supplied.is_absolute()
            || supplied
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!(
                "changed path `{trimmed}` must be relative and cannot contain parent traversal"
            ));
        }
        let candidate = dir.join(supplied);
        let relative = candidate.strip_prefix(&root).map_err(|_| {
            format!("changed path `{trimmed}` resolves outside the verification repository")
        })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| format!("changed path `{trimmed}` is not valid UTF-8"))?
            .to_string();
        if !relative.is_empty() {
            normalized.push(relative);
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

/// Resolve the repository root and return tracked/untracked paths below `dir`.
/// A failure is surfaced rather than falling back to a broad Cargo command.
fn git_changes_under(dir: &Path) -> Option<Vec<String>> {
    let root = std::process::Command::new("git")
        .args(["-C", dir.to_str()?, "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))?;
    let relative_dir = dir
        .canonicalize()
        .ok()?
        .strip_prefix(&root)
        .ok()?
        .to_path_buf();

    let tracked = std::process::Command::new("git")
        .args(["-C", root.to_str()?, "diff", "--name-only", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let untracked = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str()?,
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let tracked_text = String::from_utf8_lossy(&tracked.stdout).into_owned();
    let untracked_text = String::from_utf8_lossy(&untracked.stdout).into_owned();
    let mut paths = tracked_text
        .lines()
        .chain(untracked_text.lines())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| {
            let path = Path::new(path);
            relative_dir.as_os_str().is_empty() || path.starts_with(&relative_dir)
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Some(paths)
}

/// Build package/target-scoped Cargo checks. Package names come from the
/// nearest manifest rather than directory spelling, and target files receive
/// a target selector so a single binary/integration test does not fan out to
/// every workspace member.
fn narrow_rust_checks(dir: &Path, changes: &[String]) -> Result<DetectOutcome, String> {
    let repo_root = git_root(dir)
        .or_else(|| dir.canonicalize().ok())
        .ok_or_else(|| "could not resolve the Cargo repository root".to_string())?;
    let mut packages = Vec::new();
    let mut targets = Vec::new();
    for changed in changes {
        let path = repo_root.join(changed);
        let Some(manifest) = nearest_manifest(&path, &repo_root) else {
            return Err(format!(
                "changed Rust path `{changed}` is not inside a Cargo package; pass `scope: \"integration\"` or an explicit `command`"
            ));
        };
        let package = cargo_package_name(&manifest)
            .ok_or_else(|| format!("could not read package name from {}", manifest.display()))?;
        if !packages.contains(&package) {
            packages.push(package);
        }
        if let Some(target) = cargo_target_selector(&path, &manifest) {
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
    if packages.is_empty() {
        return Err("no impacted Cargo package found for the Rust change".to_string());
    }
    let mut args = vec!["test".to_string()];
    for package in packages {
        args.extend(["-p".to_string(), package]);
    }
    // One target selector is safe and useful for a narrow target change. If a
    // package has several targets changed, package-level tests cover them all
    // without inventing an invalid multi-target Cargo command.
    if targets.len() == 1 {
        args.extend(targets.remove(0));
    }
    Ok(DetectOutcome::Checks(vec![Check::direct_owned(
        "cargo", args,
    )]))
}

fn git_root(dir: &Path) -> Option<PathBuf> {
    std::process::Command::new("git")
        .args(["-C", dir.to_str()?, "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
}

fn nearest_manifest(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut cursor = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        let manifest = cursor.join("Cargo.toml");
        if manifest.is_file() {
            return Some(manifest);
        }
        if cursor == root || !cursor.pop() {
            return None;
        }
    }
}

fn cargo_package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let name = value.split('#').next()?.trim().trim_matches('"');
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn cargo_target_selector(path: &Path, manifest: &Path) -> Option<Vec<String>> {
    let package_root = manifest.parent()?;
    let relative = path.strip_prefix(package_root).ok()?;
    let mut components = relative.components();
    match (components.next()?.as_os_str().to_str()?, components.next()?) {
        ("tests", name) if relative.extension().and_then(|e| e.to_str()) == Some("rs") => {
            Some(vec![
                "--test".to_string(),
                name.as_os_str()
                    .to_str()?
                    .trim_end_matches(".rs")
                    .to_string(),
            ])
        }
        ("examples", name) if relative.extension().and_then(|e| e.to_str()) == Some("rs") => {
            Some(vec![
                "--example".to_string(),
                name.as_os_str()
                    .to_str()?
                    .trim_end_matches(".rs")
                    .to_string(),
            ])
        }
        ("src", name) if name.as_os_str().to_str()? == "bin" => components
            .next()
            .and_then(|name| name.as_os_str().to_str())
            .map(|name| {
                vec![
                    "--bin".to_string(),
                    name.trim_end_matches(".rs").to_string(),
                ]
            }),
        _ => None,
    }
}

/// Node checks: run whichever of `build` then `test` scripts exist, via the
/// package manager the lockfile implies. The `npm init` placeholder test script
/// is NOT a real check (see [`is_placeholder_test`]).
fn node_checks(dir: &Path) -> Result<DetectOutcome, String> {
    let pm = detect_node_package_manager(dir);
    let scripts = read_package_scripts(dir);
    let mut checks = Vec::new();
    if scripts.contains_key("build") {
        checks.push(Check::direct(pm, &["run", "build"]));
    }
    // A real `test` script is a check; the `npm init` placeholder ("no test
    // specified" && exit 1) is not — running it always FAILs, which on a fresh
    // scaffold would trap the loop chasing a phantom failure.
    let placeholder_test = scripts
        .get("test")
        .is_some_and(|cmd| is_placeholder_test(cmd));
    if scripts.contains_key("test") && !placeholder_test {
        checks.push(Check::direct(pm, &["run", "test"]));
    }
    if !checks.is_empty() {
        return Ok(DetectOutcome::Checks(checks));
    }
    if placeholder_test {
        return Ok(DetectOutcome::NoChecksConfigured(
            "package.json has only the default `npm init` placeholder test script \
             (\"Error: no test specified\") and no build script — there are no real \
             checks to run yet."
                .to_string(),
        ));
    }
    Err(format!(
        "package.json defines no `build` or `test` script for {pm}. Pass an explicit \
         `command` (e.g. \"{pm} run <script>\")."
    ))
}

/// The `npm init` default test script — `echo "Error: no test specified" && exit 1`.
/// It is a placeholder, not a real check. Matched loosely on its two invariant
/// fragments so whitespace/quoting variations still classify.
fn is_placeholder_test(script: &str) -> bool {
    let s = script.to_ascii_lowercase();
    s.contains("no test specified") && s.contains("exit 1")
}

/// pytest exits 5 when it collected no tests — a fresh scaffold with no tests
/// yet, not a real failure. Exit 5 is pytest-specific, so this is gated to the
/// pytest check; another tool's exit 5 stays a genuine failure.
fn is_no_tests_collected(check: &Check, exit_code: Option<i32>) -> bool {
    exit_code == Some(5)
        && matches!(&check.exec, Exec::Direct { program, .. } if program.as_str() == "pytest")
}

/// jest and vitest exit 1 with "No tests found" / "No test files found" when
/// zero test files match — a fresh Node scaffold with no tests yet, not a real
/// failure. A genuine jest/vitest assertion failure ALSO exits 1, so the marker
/// string is the only discriminator; the program+args gate keeps another tool's
/// exit 1 from being swallowed. Narrow on purpose.
fn is_no_test_files_found(check: &Check, exit_code: Option<i32>, output: &str) -> bool {
    if exit_code != Some(1) {
        return false;
    }
    let is_node_test = matches!(
        &check.exec,
        Exec::Direct { program, args }
            if matches!(program.as_str(), "npm" | "pnpm" | "yarn" | "bun")
                && args.as_slice() == ["run", "test"]
    );
    if !is_node_test {
        return false;
    }
    let lower = output.to_ascii_lowercase();
    lower.contains("no tests found") || lower.contains("no test files found")
}

/// Package manager implied by the lockfile present (npm is the default).
fn detect_node_package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() {
        "bun"
    } else {
        "npm"
    }
}

/// Script name → command from `package.json`'s `scripts` object. Tolerant: a
/// missing or malformed file yields an empty map rather than an error. The
/// command text is needed to tell a real `test` script from the `npm init`
/// placeholder.
fn read_package_scripts(dir: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
        return HashMap::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    json.get("scripts")
        .and_then(|scripts| scripts.as_object())
        .map(|scripts| {
            scripts
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The Makefile target to run, preferring `test` over `check`. `None` when
/// there is no Makefile or it exposes neither target.
fn makefile_check_target(dir: &Path) -> Option<String> {
    let path = ["Makefile", "makefile", "GNUmakefile"]
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.exists())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let has_target = |name: &str| {
        let colon = format!("{name}:");
        let assign = format!("{name}:=");
        let spaced = format!("{name} :");
        text.lines().any(|line| {
            (line.starts_with(&colon) && !line.starts_with(&assign)) || line.starts_with(&spaced)
        })
    };
    if has_target("test") {
        Some("test".to_string())
    } else if has_target("check") {
        Some("check".to_string())
    } else {
        None
    }
}

// --- execution --------------------------------------------------------------

async fn run_check(check: &Check, dir: &Path, timeout_secs: u64) -> CheckOutcome {
    if let Exec::Markdown { files } = &check.exec {
        return run_markdown_checks(files, timeout_secs).await;
    }
    let mut command = match &check.exec {
        Exec::Direct { program, args } => {
            let mut command = tokio::process::Command::new(program);
            command.args(args);
            command
        }
        Exec::Shell(command_line) => shell_command(command_line),
        Exec::Markdown { .. } => unreachable!("markdown checks return before spawning a process"),
    };
    command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // If the timeout drops the wait future, the child is killed with it, so
        // a timed-out build can't linger.
        .kill_on_drop(true);

    // permagent-dispatch: seam=verifier_command_v1 class=excluded reason=deterministic_local authority=verification_contract
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CheckOutcome::ToolMissing;
        }
        Err(error) => return CheckOutcome::SpawnError(error.to_string()),
    };

    // `wait_with_output` drains stdout and stderr concurrently, so a check that
    // writes more than a pipe buffer can't deadlock.
    let output = if timeout_secs > 0 {
        match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return CheckOutcome::SpawnError(error.to_string()),
            Err(_) => return CheckOutcome::TimedOut,
        }
    } else {
        match child.wait_with_output().await {
            Ok(output) => output,
            Err(error) => return CheckOutcome::SpawnError(error.to_string()),
        }
    };

    if output.status.success() {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed {
            exit_code: output.status.code(),
            output: combine_streams(&output.stdout, &output.stderr),
        }
    }
}

/// Deterministic markdown hygiene for both tracked and newly-created files.
/// This intentionally checks syntax-shaped defects only: trailing whitespace,
/// empty inline destinations, and malformed reference definitions.
async fn run_markdown_checks(files: &[PathBuf], timeout_secs: u64) -> CheckOutcome {
    let reference = Regex::new(r"^\s*\[[^\]]+\]:\s*(\S.*)$").expect("static regex");
    let mut findings = Vec::new();
    for path in files {
        let path_for_read = path.clone();
        let read = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            let metadata = std::fs::symlink_metadata(&path_for_read)?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "not a regular non-symlink file",
                ));
            }
            if metadata.len() > MAX_MARKDOWN_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "file exceeds markdown verification size limit",
                ));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            std::fs::File::open(&path_for_read)?
                .take(MAX_MARKDOWN_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_MARKDOWN_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "file exceeds markdown verification size limit",
                ));
            }
            Ok(bytes)
        });
        let read = if timeout_secs > 0 {
            match tokio::time::timeout(Duration::from_secs(timeout_secs), read).await {
                Ok(result) => result,
                Err(_) => return CheckOutcome::TimedOut,
            }
        } else {
            read.await
        };
        let bytes = match read {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                findings.push(format!("{}: cannot read: {error}", path.display()));
                continue;
            }
            Err(error) => {
                findings.push(format!("{}: read task failed: {error}", path.display()));
                continue;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                findings.push(format!("{}: not valid UTF-8", path.display()));
                continue;
            }
        };
        for (line_number, line) in text.lines().enumerate() {
            if line.ends_with(' ') || line.ends_with('\t') {
                findings.push(format!(
                    "{}:{}: trailing whitespace",
                    path.display(),
                    line_number + 1
                ));
            }
            if let Some(start) = line.find("](") {
                let destination = &line[start + 2..];
                if !destination.contains(')')
                    || destination[..destination.find(')').unwrap_or(0)]
                        .trim()
                        .is_empty()
                {
                    findings.push(format!(
                        "{}:{}: inline link has an empty or unterminated destination",
                        path.display(),
                        line_number + 1
                    ));
                }
            }
            if line.contains("]: ") && !reference.is_match(line) {
                findings.push(format!(
                    "{}:{}: malformed link reference",
                    path.display(),
                    line_number + 1
                ));
            }
        }
    }
    if findings.is_empty() {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed {
            exit_code: Some(1),
            output: findings.join("\n"),
        }
    }
}

#[cfg(not(windows))]
fn shell_command(command_line: &str) -> tokio::process::Command {
    let shell = if which::which("bash").is_ok() {
        "bash"
    } else {
        "sh"
    };
    let mut command = tokio::process::Command::new(shell);
    command.arg("-c").arg(command_line);
    command
}

#[cfg(windows)]
fn shell_command(command_line: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("cmd");
    command.arg("/C").arg(command_line);
    command
}

// --- rendering (pure) -------------------------------------------------------

/// Combine captured streams in a deterministic order (stdout then stderr) so
/// that an identical failure renders identically. Decoded lossily; empties are
/// skipped; wholly-empty output is labelled rather than blank.
fn combine_streams(stdout: &[u8], stderr: &[u8]) -> String {
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);
    let out = out.trim();
    let err = err.trim();
    match (out.is_empty(), err.is_empty()) {
        (true, true) => "(no output)".to_string(),
        (false, true) => out.to_string(),
        (true, false) => err.to_string(),
        (false, false) => format!("{out}\n{err}"),
    }
}

/// Neutralize the volatile spans that differ between two identical failing
/// re-runs (tempdir paths, `host:port`, pids, addresses, UUIDs, timestamps, and
/// durations) so the same failure hashes identically and the loop guard's
/// same-failure cap can bind. Conservative by construction: each pattern only
/// collapses its own volatile token, so two genuinely-different failures still
/// hash differently and real fixing stays visible as progress.
fn normalize(output: &str) -> String {
    let s = UUID_RE.replace_all(output, "<uuid>");
    let s = TIMESTAMP_RE.replace_all(&s, "<ts>");
    let s = VAR_FOLDERS_RE.replace_all(&s, "/var/folders/<tmp>");
    let s = TMP_SUFFIX_RE.replace_all(&s, ".tmp<rand>");
    let s = IPV4_PORT_RE.replace_all(&s, "<host:port>");
    let s = LOCALHOST_PORT_RE.replace_all(&s, "localhost:<port>");
    let s = PID_RE.replace_all(&s, "pid=<pid>");
    let s = HEX_ADDR_RE.replace_all(&s, "0x<addr>");
    DURATION_RE.replace_all(&s, "<t>${1}").into_owned()
}

/// Bound failure output to `MAX_OUTPUT_LINES`, keeping the head and the tail
/// when it is exceeded so both the leading errors and the trailing summary
/// survive — never a blind tail.
fn clamp_output(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_OUTPUT_LINES {
        return text.trim_end().to_string();
    }
    let head = lines[..HEAD_LINES].join("\n");
    let tail = lines[lines.len() - TAIL_LINES..].join("\n");
    let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
    format!("{head}\n... [{omitted} lines omitted] ...\n{tail}")
}

/// A clean success: no error dump, just which checks passed.
fn observation(
    mut result: CallToolResult,
    command: Option<&str>,
    verdict: Option<&str>,
    evidence: Option<&str>,
) -> CallToolResult {
    result.structured_content = Some(json!({
        "kind": VERIFICATION_OBSERVATION_KIND,
        "command": command,
        "verdict": verdict,
        "evidence": evidence,
    }));
    result
}

fn pass_result(checks: &[Check]) -> CallToolResult {
    let labels: Vec<&str> = checks.iter().map(|check| check.label.as_str()).collect();
    let message = format!("PASS - all checks passed: {}.", labels.join(", "));
    observation(
        CallToolResult::success(vec![Content::text(message.clone()).with_priority(0.0)]),
        Some(&labels.join(", ")),
        Some("pass"),
        Some(&message),
    )
}

/// A structured failure: the failing command, its exit code, the (normalized,
/// bounded) errors, and the self-governed bound. `CallToolResult::error` sets
/// `is_error`, which is what the loop guard reads to classify a same-failure
/// loop.
fn fail_result(check: &Check, exit_code: Option<i32>, output: &str) -> CallToolResult {
    let code = match exit_code {
        Some(code) => code.to_string(),
        None => "killed".to_string(),
    };
    let body = clamp_output(&normalize(output));
    let message = format!(
        "FAIL - `{label}` failed (exit {code}).\n\n{body}\n\n\
         Fix what these errors point to, then run verify again. If the same failure \
         persists after two or three focused attempts, stop and hand it to the user \
         with these errors rather than repeating - identical retries make no progress \
         and are blocked by the runaway-loop guard.",
        label = check.label,
    );
    observation(
        CallToolResult::error(vec![Content::text(message).with_priority(0.0)]),
        Some(&check.label),
        Some("fail"),
        Some(&body),
    )
}

fn error_result(message: &str) -> CallToolResult {
    observation(
        CallToolResult::error(vec![Content::text(message.to_string()).with_priority(0.0)]),
        None,
        Some("error"),
        Some(message),
    )
}

/// A "no checks configured" outcome: the project type was recognized but has
/// nothing runnable to verify yet (a fresh scaffold, a placeholder-only test, or
/// pytest collecting nothing). This is NOT a failure — it is a clean success, so
/// neither the loop guard nor the model treats it as a phantom failure to chase.
fn no_checks_result(reason: &str) -> CallToolResult {
    let message = format!(
        "NO CHECKS - {reason} Nothing to verify yet. Add tests or a build/test script and run \
         verify again, or pass an explicit `command` to run a specific check."
    );
    observation(
        CallToolResult::success(vec![Content::text(message.clone()).with_priority(0.0)]),
        None,
        Some("pass"),
        Some(&message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;
    use serde_json::json;

    fn text_of(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(text) => &text.text,
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn verifier_results_carry_authoritative_structured_observations() {
        let result = pass_result(&[Check::direct("cargo", &["test"])]);
        let observation: VerificationObservation =
            serde_json::from_value(result.structured_content.unwrap()).unwrap();
        assert_eq!(observation.kind, VERIFICATION_OBSERVATION_KIND);
        assert_eq!(observation.command.as_deref(), Some("cargo test"));
        assert_eq!(observation.verdict.as_deref(), Some("pass"));
        assert!(observation
            .evidence
            .as_deref()
            .is_some_and(|e| e.contains("PASS")));

        let failed = fail_result(
            &Check::direct("cargo", &["test"]),
            Some(101),
            "error: test failed",
        );
        let observation: VerificationObservation =
            serde_json::from_value(failed.structured_content.unwrap()).unwrap();
        assert_eq!(observation.verdict.as_deref(), Some("fail"));
        assert_eq!(observation.command.as_deref(), Some("cargo test"));
        assert_eq!(observation.evidence.as_deref(), Some("error: test failed"));
    }

    /// Unwrap a detection that must have produced runnable checks.
    fn checks_of(dir: &Path) -> Vec<Check> {
        match detect_checks(dir) {
            Ok(DetectOutcome::Checks(checks)) => checks,
            other => panic!("expected runnable checks, got {other:?}"),
        }
    }

    // ── Detection → the correct check command ──

    #[test]
    fn shell_check_names_the_missing_program_not_the_shell() {
        // A local harness run was told "`shell` was not found on PATH, so `npm
        // run test` could not run. Install it" — there is no program called
        // `shell`, and the advice was unfollowable.
        assert_eq!(Check::shell("npm run test").program_name(), "npm");
        assert_eq!(Check::shell("cargo test --lib").program_name(), "cargo");
    }

    #[test]
    fn detects_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("cargo", &["test"])]
        );
    }

    #[test]
    fn auto_docs_scope_uses_deterministic_diff_check_without_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/guide.md"), "# Guide\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Readme\n").unwrap();
        let changes = ["docs/guide.md".to_string(), "README.md".to_string()];
        assert_eq!(
            select_checks(
                dir.path(),
                VerificationScope::Auto,
                VerificationRisk::Low,
                Some(&changes),
            )
            .unwrap(),
            DetectOutcome::Checks(vec![markdown_check(
                dir.path(),
                &["README.md".to_string(), "docs/guide.md".to_string()],
            )
            .unwrap(),])
        );
    }

    #[test]
    fn supplied_rust_paths_are_relative_to_the_requested_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("crates/widget");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::write(package.join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        std::fs::write(package.join("src/lib.rs"), "pub fn widget() {}\n").unwrap();
        let changes = ["src/lib.rs".to_string()];
        assert_eq!(
            select_checks(
                &package,
                VerificationScope::Rust,
                VerificationRisk::Standard,
                Some(&changes),
            )
            .unwrap(),
            DetectOutcome::Checks(vec![Check::direct("cargo", &["test", "-p", "widget"])])
        );
    }

    #[test]
    fn supplied_changes_reject_absolute_and_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["../outside.md", "/tmp/outside.md"] {
            let error = normalize_supplied_changes(dir.path(), &[path.to_string()]).unwrap_err();
            assert!(error.contains("must be relative"), "got: {error}");
        }
    }

    #[test]
    fn auto_does_not_turn_non_rust_changes_into_workspace_cargo_tests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        let changes = ["ui/app.ts".to_string()];
        assert!(matches!(
            select_checks(
                dir.path(),
                VerificationScope::Auto,
                VerificationRisk::Standard,
                Some(&changes),
            )
            .unwrap(),
            DetectOutcome::NoChecksConfigured(_)
        ));
    }

    #[test]
    fn integration_scope_is_the_only_auto_selection_for_workspace_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = []").unwrap();
        assert_eq!(
            select_checks(
                dir.path(),
                VerificationScope::Integration,
                VerificationRisk::High,
                None,
            )
            .unwrap(),
            DetectOutcome::Checks(vec![Check::direct(
                "cargo",
                &["test", "--workspace", "--all-targets"],
            )])
        );
    }

    #[test]
    fn target_selector_keeps_rust_verification_narrow() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"x\"").unwrap();
        let target = cargo_target_selector(&dir.path().join("tests/parser.rs"), &manifest);
        assert_eq!(
            target,
            Some(vec!["--test".to_string(), "parser".to_string()])
        );
    }

    #[test]
    fn cargo_package_name_only_reads_the_package_section() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[dependencies]\nname = \"wrong\"\n\n[package]\nname = \"right\"\n",
        )
        .unwrap();
        assert_eq!(cargo_package_name(&manifest).as_deref(), Some("right"));
    }

    #[tokio::test]
    async fn docs_scope_checks_the_supplied_untracked_file() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("notes.md");
        std::fs::write(&docs, "[broken]()  \n").unwrap();
        let result = VerifyTool::new()
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: Some(30),
                    scope: Some(VerificationScope::Docs),
                    risk: Some(VerificationRisk::Low),
                    changed_paths: Some(vec!["notes.md".to_string()]),
                },
                Some(dir.path()),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("bounded markdown hygiene checks"));
    }

    #[test]
    fn detects_go_build_then_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module x").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![
                Check::direct("go", &["build", "./..."]),
                Check::direct("go", &["test", "./..."]),
            ]
        );
    }

    #[test]
    fn detects_node_npm_build_then_test_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"jest","build":"tsc"}}"#,
        )
        .unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![
                Check::direct("npm", &["run", "build"]),
                Check::direct("npm", &["run", "test"]),
            ]
        );
    }

    #[test]
    fn detects_node_package_manager_from_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("pnpm", &["run", "test"])]
        );
    }

    #[test]
    fn node_without_build_or_test_script_is_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        let error = detect_checks(dir.path()).unwrap_err();
        assert!(
            error.contains("no `build` or `test` script"),
            "got: {error}"
        );
    }

    #[test]
    fn detects_python_pytest_from_any_marker() {
        for marker in ["pyproject.toml", "setup.py", "requirements.txt", "tox.ini"] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(marker), "").unwrap();
            assert_eq!(
                checks_of(dir.path()),
                vec![Check::direct("pytest", &[])],
                "marker {marker} should map to pytest"
            );
        }
    }

    #[test]
    fn detects_make_test_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile"),
            ".PHONY: test\ntest:\n\tcargo test\n",
        )
        .unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("make", &["test"])]
        );
    }

    #[test]
    fn detects_make_check_when_no_test_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "check:\n\truff check .\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("make", &["check"])]
        );
    }

    #[test]
    fn makefile_without_test_or_check_is_not_detected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "build:\n\tgo build\n").unwrap();
        assert!(detect_checks(dir.path()).is_err());
    }

    #[test]
    fn language_marker_takes_precedence_over_makefile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("Makefile"), "test:\n\techo hi\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("cargo", &["test"])]
        );
    }

    #[test]
    fn rust_takes_precedence_over_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"x"}}"#,
        )
        .unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("cargo", &["test"])]
        );
    }

    #[test]
    fn unknown_project_returns_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = detect_checks(dir.path()).unwrap_err();
        assert!(error.contains("Could not detect"), "got: {error}");
        assert!(error.contains("command"), "should point at the override");
    }

    // ── F2.5: fresh-scaffold "no checks" is not a failure (regression guard) ──

    #[test]
    fn npm_init_placeholder_test_is_no_checks_not_a_check() {
        // A brand-new `npm init` scaffold: only the placeholder test, no build.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        match detect_checks(dir.path()) {
            Ok(DetectOutcome::NoChecksConfigured(note)) => {
                assert!(note.contains("placeholder"), "got: {note}")
            }
            other => panic!("placeholder scaffold must be no-checks, got {other:?}"),
        }
    }

    #[test]
    fn npm_placeholder_test_with_a_real_build_runs_only_build() {
        // A real build script IS a check; the placeholder test is skipped.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"tsc","test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("npm", &["run", "build"])],
            "placeholder test must be dropped, real build kept"
        );
    }

    #[test]
    fn is_placeholder_test_matches_npm_init_default_only() {
        assert!(is_placeholder_test(
            "echo \"Error: no test specified\" && exit 1"
        ));
        // Quoting/whitespace variations still classify.
        assert!(is_placeholder_test("echo 'no test specified' && exit 1"));
        // A real test script must NOT be misclassified.
        assert!(!is_placeholder_test("jest"));
        assert!(!is_placeholder_test("vitest run"));
    }

    #[test]
    fn pytest_exit_5_is_no_tests_collected_but_only_for_pytest() {
        assert!(is_no_tests_collected(
            &Check::direct("pytest", &[]),
            Some(5)
        ));
        // A real pytest failure (exit 1) is still a failure.
        assert!(!is_no_tests_collected(
            &Check::direct("pytest", &[]),
            Some(1)
        ));
        // Exit 5 from a different tool is left as a genuine failure.
        assert!(!is_no_tests_collected(
            &Check::direct("cargo", &["test"]),
            Some(5)
        ));
        // An explicit shell check is never reclassified.
        assert!(!is_no_tests_collected(&Check::shell("pytest"), Some(5)));
    }

    #[test]
    fn jest_vitest_no_test_files_is_empty_not_broken() {
        let npm_test = Check::direct("npm", &["run", "test"]);
        assert!(is_no_test_files_found(
            &npm_test,
            Some(1),
            "No tests found, exiting with code 1"
        ));
        assert!(is_no_test_files_found(
            &npm_test,
            Some(1),
            "No test files found, exiting with code 1"
        ));
    }

    /// A real failing assertion also exits 1; without this guard the empty-suite
    /// exemption would report a broken suite as "no tests" — the mirror bug.
    #[test]
    fn a_real_jest_failure_still_fails() {
        assert!(!is_no_test_files_found(
            &Check::direct("npm", &["run", "test"]),
            Some(1),
            "FAIL  src/foo.test.js\n  ● adds numbers › 1 + 1 = 2\n    Expected: 2\n    Received: 3"
        ));
    }

    #[test]
    fn no_test_files_marker_is_gated_to_node_test_scripts() {
        let marker = "No tests found, exiting with code 1";
        assert!(!is_no_test_files_found(
            &Check::direct("cargo", &["test"]),
            Some(1),
            marker
        ));
        assert!(!is_no_test_files_found(
            &Check::shell("npm run test"),
            Some(1),
            marker
        ));
    }

    #[tokio::test]
    async fn verify_on_a_placeholder_scaffold_is_a_clean_non_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        let tool = VerifyTool::new();
        let result = tool
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: None,
                    scope: None,
                    risk: None,
                    changed_paths: None,
                },
                Some(dir.path()),
            )
            .await;
        // NOT a failure — is_error must be unset/false so the loop guard and the
        // model don't chase a phantom failure on a fresh scaffold.
        assert_eq!(result.is_error, Some(false));
        assert!(text_of(&result).starts_with("NO CHECKS"));
    }

    // ── Full error capture (not truncated) ──

    #[test]
    fn clamp_returns_full_output_when_under_limit() {
        assert_eq!(clamp_output("a\nb\nc"), "a\nb\nc");
    }

    #[test]
    fn clamp_keeps_head_and_tail_when_over_limit() {
        let lines: Vec<String> = (0..(MAX_OUTPUT_LINES + 200))
            .map(|i| format!("line{i}"))
            .collect();
        let clamped = clamp_output(&lines.join("\n"));
        // An early line survives — this is not a blind tail.
        assert!(clamped.contains("line0"), "head must be kept");
        assert!(clamped.contains("line5"), "early context must survive");
        // The final summary line survives.
        assert!(
            clamped.contains(&format!("line{}", MAX_OUTPUT_LINES + 199)),
            "tail must be kept"
        );
        assert!(clamped.contains("lines omitted"), "elision marker present");
    }

    #[test]
    fn fail_result_carries_the_real_errors() {
        let check = Check::direct("cargo", &["test"]);
        let result = fail_result(&check, Some(101), "error[E0425]: cannot find value `x`");
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.starts_with("FAIL"));
        assert!(text.contains("cargo test"), "the failing command is named");
        assert!(text.contains("exit 101"), "the exit code is shown");
        assert!(text.contains("E0425"), "the real compiler error survives");
    }

    #[test]
    fn combine_streams_is_deterministic_stdout_then_stderr() {
        assert_eq!(combine_streams(b"out", b"err"), "out\nerr");
        assert_eq!(combine_streams(b"", b"err"), "err");
        assert_eq!(combine_streams(b"out", b""), "out");
        assert_eq!(combine_streams(b"", b""), "(no output)");
    }

    // ── Clean exit on pass ──

    #[test]
    fn pass_result_is_a_clean_success() {
        let result = pass_result(&[
            Check::direct("go", &["build", "./..."]),
            Check::direct("go", &["test", "./..."]),
        ]);
        assert_eq!(result.is_error, Some(false));
        let text = text_of(&result);
        assert!(text.starts_with("PASS"));
        assert!(text.contains("go build ./..."));
        assert!(text.contains("go test ./..."));
    }

    // ── The bounded-repair contract: REUSE the ProgressMonitor cap ──

    #[test]
    fn identical_failures_render_identically_so_the_loop_guard_can_bind() {
        // The same underlying failure, differing only in reported durations,
        // must render to identical tool output — otherwise the loop guard's
        // same-failure cap (which compares result hashes) could never bind to a
        // verify loop.
        let check = Check::direct("cargo", &["test"]);
        let run_1 = fail_result(
            &check,
            Some(101),
            "error[E0425]: cannot find `x`\ntest result: FAILED. finished in 3.24s",
        );
        let run_2 = fail_result(
            &check,
            Some(101),
            "error[E0425]: cannot find `x`\ntest result: FAILED. finished in 9.90s",
        );
        assert_eq!(
            text_of(&run_1),
            text_of(&run_2),
            "identical failures must render identically"
        );

        // A genuinely different failure must render differently — progress is
        // visible to the guard, which then lets real fixing through.
        let run_3 = fail_result(
            &check,
            Some(101),
            "error[E0433]: failed to resolve\ntest result: FAILED. finished in 3.24s",
        );
        assert_ne!(
            text_of(&run_1),
            text_of(&run_3),
            "a different error must render differently"
        );
    }

    #[test]
    fn repeated_identical_verify_failure_is_bounded_and_escalates() {
        // The bound is REUSED, not reinvented: a verify call that keeps failing
        // identically is the existing ProgressMonitor's S2 "same failure" — it
        // escalates to the Decision Inbox at the third identical failure rather
        // than looping forever.
        use crate::tool_monitor::{assess_tool_call, is_mutating, LoopAction, Signal, ToolEvent};

        let name = "developer__verify";
        let args = json!({});
        let failure = |result_hash: u64| ToolEvent {
            name: name.to_string(),
            args: args.clone(),
            result_hash,
            is_error: true,
            is_mutating: is_mutating(name),
        };

        // Two prior identical failures (same normalized output → same hash),
        // then the third identical failing verify.
        let same_failure = vec![failure(0xF00D), failure(0xF00D)];
        assert_eq!(
            assess_tool_call(&same_failure, name, &args),
            LoopAction::Escalate(Signal::SameFailure),
            "a 3rd identical verify failure must escalate via the loop guard"
        );

        // A changing failure (each attempt hashes differently) is progress and
        // must NOT be escalated as a same-failure loop.
        let changing = vec![failure(1), failure(2)];
        assert_ne!(
            assess_tool_call(&changing, name, &args),
            LoopAction::Escalate(Signal::SameFailure),
            "changing failure output is progress and must not escalate"
        );
    }

    // ── normalize ──

    #[test]
    fn normalize_neutralizes_durations_only() {
        assert_eq!(normalize("finished in 3.24s"), "finished in <t>s");
        assert_eq!(normalize("took 0.03 seconds"), "took <t>seconds");
        assert_eq!(normalize("Time: 2.5 s"), "Time: <t>s");
        assert_eq!(normalize("ran in 125.0ms"), "ran in <t>ms");
        // Same failure, different timing → equal after normalize.
        assert_eq!(normalize("FAILED in 1.11s"), normalize("FAILED in 8.88s"));
        // Different errors → still different.
        assert_ne!(normalize("error A in 1.0s"), normalize("error B in 1.0s"));
        // No duration and version-like numbers are left untouched.
        assert_eq!(normalize("plain error"), "plain error");
        assert_eq!(normalize("Compiling foo v0.1.0"), "Compiling foo v0.1.0");
    }

    #[test]
    fn normalize_stabilizes_volatile_spans_so_repeats_hash_equal() {
        // Two runs of the SAME underlying failure differing only in the volatile
        // spans the loop guard must see through: a tempdir path, a host:port
        // bind, a pid, a heap address, a UUID, and an ISO-8601 timestamp. After
        // normalization they must be byte-identical (→ equal result hash → the
        // same-failure cap binds).
        let run_a = "thread 'main' panicked at src/x.rs: assertion failed\n\
             temp=/var/folders/ab/9x8y7z/T/.tmpAA11BB bound 127.0.0.1:54321 \
             pid=1234 ptr=0x7ffee3b2a1c0 id=550e8400-e29b-41d4-a716-446655440000 \
             at 2026-07-16T12:34:56.789Z";
        let run_b = "thread 'main' panicked at src/x.rs: assertion failed\n\
             temp=/var/folders/qp/1a2b3c/T/.tmpZZ99QQ bound 127.0.0.1:12345 \
             pid=9876 ptr=0x00abcdef1234 id=11112222-3333-4444-5555-666677778888 \
             at 2026-07-15T01:02:03.000Z";
        assert_eq!(
            normalize(run_a),
            normalize(run_b),
            "a repeated failure differing only in volatile spans must normalize equal"
        );
        // localhost:<port> is stabilized too.
        assert_eq!(
            normalize("connect localhost:8080"),
            normalize("connect localhost:9090")
        );

        // Guardrail: over-collapsing would make DIFFERENT failures falsely equal.
        // A different error under the same tempdir must still differ …
        assert_ne!(
            normalize("error[E0425] at /var/folders/ab/9x8y7z/T/a.rs"),
            normalize("error[E0433] at /var/folders/ab/9x8y7z/T/a.rs"),
            "distinct errors must not collide"
        );
        // … and a distinct file under the same tempdir stays distinct (only the
        // random root collapses; the stable suffix is preserved).
        assert_ne!(
            normalize("fail at /var/folders/ab/9x/T/alpha.rs"),
            normalize("fail at /var/folders/ab/9x/T/beta.rs")
        );
        // A source line number (file.rs:NN) is NOT a port and must survive.
        assert_eq!(normalize("src/main.rs:42: oops"), "src/main.rs:42: oops");
    }

    // ── params ──

    #[test]
    fn params_parse_with_defaults_and_overrides() {
        let empty: VerifyParams = serde_json::from_value(json!({})).unwrap();
        assert!(empty.command.is_none());
        assert!(empty.path.is_none());
        assert!(empty.timeout_secs.is_none());
        assert!(empty.scope.is_none());
        assert!(empty.risk.is_none());
        assert!(empty.changed_paths.is_none());

        let full: VerifyParams = serde_json::from_value(
            json!({"command":"cargo clippy","path":"crates/x","timeout_secs":30}),
        )
        .unwrap();
        assert_eq!(full.command.as_deref(), Some("cargo clippy"));
        assert_eq!(full.path.as_deref(), Some("crates/x"));
        assert_eq!(full.timeout_secs, Some(30));
    }

    #[tokio::test]
    async fn empty_command_is_rejected_with_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let tool = VerifyTool::new();
        let result = tool
            .verify_with_cwd(
                VerifyParams {
                    command: Some("   ".to_string()),
                    path: None,
                    timeout_secs: None,
                    scope: None,
                    risk: None,
                    changed_paths: None,
                },
                Some(dir.path()),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("auto-detect"));
    }

    #[tokio::test]
    async fn undetected_project_reports_actionably() {
        let dir = tempfile::tempdir().unwrap();
        let tool = VerifyTool::new();
        let result = tool
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: None,
                    scope: None,
                    risk: None,
                    changed_paths: None,
                },
                Some(dir.path()),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("Could not detect"));
    }

    // ── Integration smoke: actually run a command (shell path). Skipped where
    // no POSIX shell is available so the pure suite above stays the gate. ──

    #[cfg(not(windows))]
    #[tokio::test]
    async fn verify_passes_on_a_zero_exit_command() {
        let dir = tempfile::tempdir().unwrap();
        let tool = VerifyTool::new();
        let result = tool
            .verify_with_cwd(
                VerifyParams {
                    command: Some("exit 0".to_string()),
                    path: None,
                    timeout_secs: Some(30),
                    scope: None,
                    risk: None,
                    changed_paths: None,
                },
                Some(dir.path()),
            )
            .await;
        assert_eq!(result.is_error, Some(false));
        assert!(text_of(&result).starts_with("PASS"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn verify_fails_and_captures_both_streams() {
        let dir = tempfile::tempdir().unwrap();
        let tool = VerifyTool::new();
        let result = tool
            .verify_with_cwd(
                VerifyParams {
                    command: Some("echo EARLY_MARKER; echo LATE_MARKER 1>&2; exit 1".to_string()),
                    path: None,
                    timeout_secs: Some(30),
                    scope: None,
                    risk: None,
                    changed_paths: None,
                },
                Some(dir.path()),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.starts_with("FAIL"));
        assert!(text.contains("exit 1"));
        assert!(text.contains("EARLY_MARKER"), "stdout captured");
        assert!(text.contains("LATE_MARKER"), "stderr captured");
    }
}
