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
    /// X-verify-2: when `verdict` is `"pass"`, why the pass does not count as
    /// real coverage — a syntax-only smoke check (`py_compile`/`node
    /// --check`, see [`Check::smoke_only`]), a "nothing to verify yet"
    /// no-checks pass, or a real check that nonetheless reported zero
    /// tests/assertions (see [`zero_test_marker`]). `None` for a pass that
    /// genuinely exercised something, and always `None` for a non-pass
    /// verdict. `#[serde(default)]` so an observation built before this field
    /// existed (a hand-built test fixture included) deserializes as `None` —
    /// a genuine pass — rather than failing to parse.
    #[serde(default)]
    pub vacuous_reason: Option<String>,
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

/// X-verify-2: why an authoritative PASS does not count as real coverage, if
/// any reason applies (see [`VerificationObservation::vacuous_reason`]).
/// `None` for a result that is not an authoritative pass at all (a failure is
/// never "vacuous" — it is just a failure, and is handled by ordinary
/// verify-loop logic) and for a genuine, non-vacuous pass.
pub fn vacuous_pass_reason(result: &CallToolResult) -> Option<String> {
    if !is_authoritative_pass(result) {
        return None;
    }
    let value = result.structured_content.as_ref()?;
    let observation: VerificationObservation = serde_json::from_value(value.clone()).ok()?;
    observation.vacuous_reason
}

/// G5 finding 1: the ONE predicate that means "this verify result is real
/// evidence a completion gate may act on" — an authoritative pass (see
/// [`is_authoritative_pass`]) that is ALSO not vacuous (see
/// [`vacuous_pass_reason`]). `messages_have_successful_verify`
/// (`orchestrator.rs`) is the sole caller and, through it, both
/// `maybe_hold_review` (the Review-transition gate) and
/// `auto_land_headless_goal` (the unattended landing path) — a smoke-only or
/// zero-tests pass must refuse both exactly as if verify had never run at
/// all, never just one of them.
pub fn is_genuine_pass(result: &CallToolResult) -> bool {
    is_authoritative_pass(result) && vacuous_pass_reason(result).is_none()
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
    /// R21c: this goal's acceptance criteria, when known — pass them along so
    /// a project with no detectable test suite is not stuck with a
    /// syntax-only smoke check (or an honest NO CHECKS) as its only possible
    /// verdict. When the checks this tool would otherwise run are all smoke
    /// (or there are none at all), verify tries to compile each criterion
    /// into a small, mechanically-run probe: a plain file-existence claim
    /// ("`fizzbuzz.py` exists"), a function/class-definition claim ("defines
    /// a function named fizzbuzz"), or a backtick-quoted command's exit code
    /// ("`pytest -k smoke` exits 0"). A synthesized probe runs as a REAL
    /// check and can genuinely pass or fail; a criterion that matches none of
    /// these closed shapes is left alone — never approximated. See
    /// [`synthesize_probe_checks`].
    ///
    /// R21c-b: omitting this is FINE inside a dispatched goal worker session
    /// — a cheap model will not reliably think to pass it itself, so
    /// `verify_with_cwd` loads the calling session's own bound goal card's
    /// acceptance criteria automatically when this is `None`. See
    /// [`load_bound_goal_acceptance_criteria`].
    #[serde(alias = "acceptance_criteria_text")]
    pub acceptance_criteria: Option<Vec<String>>,
}

/// How a check is executed: either a resolved program + args run directly (no
/// shell, no quoting — mirrors how the search tool runs ripgrep), or a raw
/// command string run through the platform shell (the explicit-`command` path).
#[derive(Debug, Clone, PartialEq)]
enum Exec {
    Direct {
        program: String,
        args: Vec<String>,
    },
    Shell(String),
    Markdown {
        files: Vec<PathBuf>,
    },
    /// R21c: an acceptance-criterion probe, evaluated in-process (or, for
    /// [`ProbeKind::CommandExitsZero`], by delegating to the ordinary
    /// `Shell` path) rather than treated as its own subprocess shape. See
    /// [`run_probe_check`].
    Probe(ProbeKind),
}

/// R21c: a mechanically-checkable condition synthesized from one acceptance
/// criterion. A small, CLOSED, documented set — extending it means adding a
/// variant here, never a best-effort string match at a call site. Never
/// constructed for a criterion this module cannot check for real; see
/// [`synthesize_probe_checks`].
#[derive(Debug, Clone, PartialEq)]
enum ProbeKind {
    /// A file exists at this path, relative to the verification root.
    FileExists(String),
    /// A function/class of this name is defined. `file`, when known (R21b's
    /// structured `probe:symbol_defined path=… symbol=… kind=…` grammar
    /// names it explicitly), narrows the search to exactly that file —
    /// otherwise every top-level file in the verification directory is
    /// scanned. See [`find_symbol_definition_in_file`] /
    /// [`find_symbol_definition`].
    SymbolDefined {
        symbol: String,
        kind: SymbolKind,
        file: Option<String>,
    },
    /// Running this command (through the platform shell) exits 0.
    CommandExitsZero(String),
}

/// The kind of symbol a [`ProbeKind::SymbolDefined`] probe looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolKind {
    Function,
    Class,
}

impl SymbolKind {
    fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
        }
    }
}

/// One check command to run: a human label plus how to execute it.
#[derive(Debug, Clone, PartialEq)]
struct Check {
    label: String,
    exec: Exec,
    /// X-verify-2: true only for a syntax-only smoke check
    /// (`py_compile`/`node --check`, see [`manifestless_checks`]) that never
    /// actually executes the code under test. `false` for every ordinary
    /// constructor below — set via [`Check::smoke_only`] at the two call sites
    /// that build one, so a check's "did this run real tests" status is
    /// explicit at construction rather than sniffed from its label text.
    is_smoke: bool,
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
            is_smoke: false,
        }
    }

    /// A check run through the platform shell (used for an explicit `command`).
    fn shell(command: &str) -> Self {
        Self {
            label: command.to_string(),
            exec: Exec::Shell(command.to_string()),
            is_smoke: false,
        }
    }

    fn markdown(files: Vec<PathBuf>) -> Self {
        Self {
            label: "bounded markdown hygiene checks".to_string(),
            exec: Exec::Markdown { files },
            is_smoke: false,
        }
    }

    /// R21c: a probe synthesized from an acceptance criterion (see
    /// [`synthesize_probe_checks`]). Deliberately NOT smoke-only: a probe
    /// mechanically checks something the criterion actually asked for (a
    /// file's presence, a symbol's definition, a command's exit code), so a
    /// pass here is real coverage, not a stand-in for it.
    fn probe(label: String, kind: ProbeKind) -> Self {
        Self {
            label,
            exec: Exec::Probe(kind),
            is_smoke: false,
        }
    }

    /// Mark this check as a syntax-only smoke (see [`Check::is_smoke`]).
    fn smoke_only(mut self) -> Self {
        self.is_smoke = true;
        self
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
            Exec::Probe(ProbeKind::CommandExitsZero(command)) => command
                .split_whitespace()
                .next()
                .unwrap_or("shell")
                .to_string(),
            Exec::Probe(_) => "verify's probe checker".to_string(),
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
    /// X-verify-2: carries the check's combined stdout+stderr even on
    /// success — previously discarded, which made it impossible to tell a
    /// real test pass from one that reported running zero tests
    /// (`zero_test_marker` inspects this).
    Passed {
        output: String,
    },
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

    /// R21c-b: `session_id` is the CALLING session (the worker's own session,
    /// not a goal card id) — when `params.acceptance_criteria` is absent AND
    /// this session is a live, in-process goal worker (see
    /// [`load_bound_goal_acceptance_criteria`]), its bound goal card's own
    /// acceptance criteria are loaded and probed instead. `None` (an
    /// interactive session, a best-of-N candidate, or a session this process
    /// never dispatched as a goal worker) behaves exactly as before this
    /// lane: no criteria unless the caller supplies them explicitly.
    pub async fn verify_with_cwd(
        &self,
        params: VerifyParams,
        working_dir: Option<&Path>,
        session_id: Option<&str>,
    ) -> CallToolResult {
        let base_dir = resolve_dir(working_dir, params.path.as_deref());

        // R35b: set only when detection left nothing but a syntax-only smoke
        // check AND no acceptance-criteria probe could stand in for a real
        // one — the sole remaining path that used to score a smoke-only run
        // as a `pass_result` (and increment `vacuous_passes`). Carries the
        // honest [`no_real_checks_reason`] text so the run loop below can
        // report [`NO_CHECKS_VERDICT`] instead of a fabricated PASS, while a
        // genuine smoke FAILURE (a real syntax error) still surfaces as
        // `fail_result` exactly as before — this flag is only consulted on
        // the success path.
        let mut smoke_without_real_check: Option<String> = None;

        // An explicit command overrides detection; otherwise detect the checks.
        let checks = match params.command.as_deref().map(str::trim) {
            Some("") => {
                return error_result(
                    "`command` cannot be empty — omit it to auto-detect the project's checks.",
                );
            }
            Some(command) => vec![Check::shell(command)],
            None => {
                let outcome = select_checks(
                    &base_dir,
                    params.scope.unwrap_or(VerificationScope::Auto),
                    params.risk.unwrap_or(VerificationRisk::Standard),
                    params.changed_paths.as_deref(),
                );
                // R21c-b: acceptance criteria are only ever CONSULTED when
                // detection would otherwise leave nothing genuine to run —
                // an all-smoke `Checks` outcome or `NoChecksConfigured` —
                // so the (possibly DB-backed, see
                // `load_bound_goal_acceptance_criteria`) load only happens
                // on that path, never on every ordinary auto-detected
                // `cargo test`/`npm test`/etc. call.
                let would_use_criteria = matches!(
                    outcome,
                    Ok(DetectOutcome::NoChecksConfigured(_))
                ) || matches!(&outcome, Ok(DetectOutcome::Checks(checks)) if checks.iter().all(|c| c.is_smoke));
                let loaded_criteria: Vec<String> =
                    if would_use_criteria && params.acceptance_criteria.is_none() {
                        match session_id {
                            Some(session_id) => load_bound_goal_acceptance_criteria(session_id)
                                .await
                                .unwrap_or_default(),
                            None => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                let criteria: &[String] = params
                    .acceptance_criteria
                    .as_deref()
                    .unwrap_or(&loaded_criteria);
                match outcome {
                    Ok(DetectOutcome::Checks(checks)) => {
                        // R21c: a syntax-only smoke check is the only
                        // coverage a manifestless single-file project gets
                        // today (X-verify-2) — the code is never actually
                        // executed, so this pass is always vacuous. If the
                        // acceptance criteria (explicit, or R21c-b's bound-
                        // goal fallback) carry conditions this tool can
                        // check mechanically, run THOSE as real checks
                        // instead — a probe is strictly more informative
                        // than "the file parses", so it replaces the smoke
                        // check rather than merely joining it.
                        if checks.iter().all(|check| check.is_smoke) {
                            let probes = synthesize_probe_checks(&base_dir, criteria);
                            if probes.is_empty() {
                                // R35b: a smoke-only run is never scored as a
                                // PASS — run it anyway (a real syntax error
                                // must still surface as `fail_result` below),
                                // but a *successful* smoke check now reports
                                // honest `NO_CHECKS_VERDICT` instead of
                                // `pass_result`'s fabricated PASS.
                                smoke_without_real_check =
                                    Some(no_real_checks_reason(criteria.len()));
                                checks
                            } else {
                                probes
                            }
                        } else {
                            checks
                        }
                    }
                    // A recognized project with nothing to verify yet is not
                    // a failure — but prefer running mechanically-checkable
                    // acceptance criteria as real checks over an honest, but
                    // uninformative, NO CHECKS.
                    Ok(DetectOutcome::NoChecksConfigured(note)) => {
                        let probes = synthesize_probe_checks(&base_dir, criteria);
                        if probes.is_empty() {
                            return no_checks_result(&note);
                        }
                        probes
                    }
                    Err(message) => return error_result(&message),
                }
            }
        };

        let timeout_secs = params.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);

        // Fail-fast: run checks in order, returning the first failure with its
        // full errors (you can't meaningfully test what won't build).
        // X-verify-2: each check's own (successful) output is kept so
        // `pass_result` can tell a real test pass from one that reports zero
        // tests ran.
        let mut passed_outputs: Vec<String> = Vec::with_capacity(checks.len());
        for check in &checks {
            match run_check(check, &base_dir, timeout_secs).await {
                CheckOutcome::Passed { output } => {
                    passed_outputs.push(output);
                    continue;
                }
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
                    // The program that ran (e.g. `python3`, `node`) was found and
                    // started, but a package it tried to import/require is not
                    // installed for THAT interpreter — a dependency-provisioning
                    // gap, not a defect in the code under test. See
                    // `missing_import_dependency`.
                    if let Some(dep) = missing_import_dependency(&output) {
                        return environment_dependency_result(check, exit_code, &output, &dep);
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

        // R35b: every check that ran passed. A smoke-only run with no
        // acceptance-criteria probe to back it up is never reported as a
        // PASS (see `smoke_without_real_check` above) — report the honest
        // `NO_CHECKS_VERDICT` instead. Every other outcome (a real check
        // passed, or a probe stood in for the smoke check) is an unchanged,
        // genuine `pass_result`.
        match smoke_without_real_check {
            Some(reason) => no_checks_result(&reason),
            None => pass_result(&checks, &passed_outputs),
        }
    }
}

/// R35b: the honest verdict text for "detection left nothing but a
/// syntax-only smoke check, and none of the `unparsed` supplied acceptance
/// criteria (explicit, or R21c-b's bound-goal fallback) compiled into a
/// mechanically-checkable probe" — the sole remaining path that used to let
/// `pass_result` score a smoke-only run as a PASS (X-verify-2/R21c/A32 root
/// cause 2). `unparsed` is `0` when no criteria were supplied at all (still
/// honestly "nothing to run"), matching [`NO_CHECKS_VERDICT`]'s own contract:
/// neither a pass nor a fail, just an honest account of what was tried.
fn no_real_checks_reason(unparsed: usize) -> String {
    format!(
        "no executable checks derived from the supplied criteria ({unparsed} unparsed); run \
         the test suite or name a check"
    )
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

/// R21c-b: when a worker calls `verify` without `acceptance_criteria`, load
/// its bound goal card's own criteria so a cheap model — which will not
/// reliably think to pass the param itself — still gets probe checks for a
/// manifestless goal. Cheap and correct-by-construction for every NON-goal
/// caller: [`crate::agents::platform_extensions::orchestrator::
/// live_goal_worker_card_for_session`] is an in-memory registry lookup
/// (`GOAL_WORKERS`, populated at dispatch by `register_goal_worker`) that
/// returns `None` immediately for an interactive session, a best-of-N
/// candidate, or any session this PROCESS never dispatched as a goal worker
/// — no DB round trip happens unless this really is a live goal worker.
/// `None` on any failure (no bound card, DB unavailable, card gone, no
/// criteria on the card) — this is a best-effort enrichment, never a reason
/// to fail or change `verify`'s behavior; the caller falls back to today's
/// unenriched detection exactly as if this function did not exist.
async fn load_bound_goal_acceptance_criteria(session_id: &str) -> Option<Vec<String>> {
    let card_id =
        crate::agents::platform_extensions::orchestrator::live_goal_worker_card_for_session(
            session_id,
        )?;
    let pool = crate::session::SessionManager::instance()
        .pool_clone()
        .await
        .ok()?;
    let card = match crate::cards::get_card(&pool, &card_id).await {
        Ok(Some(card)) => card,
        _ => return None,
    };
    let criteria: Vec<String> = card
        .metadata_json
        .get("acceptance_criteria")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if criteria.is_empty() {
        None
    } else {
        Some(criteria)
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
    // A Python project marker alone is not evidence pytest is the test
    // runner — `requirements.txt`/`setup.py`/etc. exist on plenty of Python
    // projects that use `unittest`, a bespoke runner, or nothing yet.
    // Confidently proposing pytest anyway is how a bare-assert test module
    // with a `sys.exit()` at import time (a real shape seen live) ends in a
    // 40-line pytest INTERNALERROR instead of an actionable result. Only
    // commit to pytest when the project actually declares it.
    if PYTHON_MARKERS
        .iter()
        .any(|marker| dir.join(marker).exists())
        && has_pytest_marker(dir)
    {
        return Ok(DetectOutcome::Checks(vec![Check::direct("pytest", &[])]));
    }
    if let Some(target) = makefile_check_target(dir) {
        return Ok(DetectOutcome::Checks(vec![Check::direct(
            "make",
            &[target.as_str()],
        )]));
    }
    // Manifestless shapes: no Cargo/go/npm/Python/Makefile marker at all (or a
    // Python marker with no pytest evidence), but a single runnable file or an
    // obviously-a-test file is sitting right there. Run THAT, directly, with
    // the right runtime — never guess at a framework that might not even be
    // installed, and never require a manifest just to run one script.
    if let Some(checks) = manifestless_checks(dir) {
        return Ok(DetectOutcome::Checks(checks));
    }
    Err(
        "Could not detect the project type (looked for Cargo.toml, go.mod, \
         package.json, a Python project marker, or a Makefile with a test/check \
         target). Pass an explicit `command` to run this project's checks."
            .to_string(),
    )
}

/// The ordered check-command labels `detect_checks` would run for `dir`
/// (e.g. `["cargo test"]`), exposed for callers outside this module that need
/// to ground a claim in what this repo can actually verify (Z1, harness loop
/// Round C — `decompose_roadmap`'s acceptance criteria previously referenced
/// a command or file this repo does not have; see
/// `orchestrator::RepoFacts`). Deliberately the same detector `verify` itself
/// runs, so the two surfaces cannot drift apart. Empty when nothing could be
/// detected — the caller decides how to degrade, never guesses a command
/// here.
pub(crate) fn detect_check_labels(dir: &Path) -> Vec<String> {
    match detect_checks(dir) {
        Ok(DetectOutcome::Checks(checks)) => checks.into_iter().map(|c| c.label).collect(),
        Ok(DetectOutcome::NoChecksConfigured(_)) | Err(_) => Vec::new(),
    }
}

/// True when the project actually declares pytest as its test runner:
/// `pytest.ini`, a `[tool.pytest...]` table in `pyproject.toml`, or a
/// `conftest.py` anywhere at the root. A bare Python marker
/// (`requirements.txt`, `setup.py`, ...) is not enough on its own.
fn has_pytest_marker(dir: &Path) -> bool {
    if dir.join("pytest.ini").exists() || dir.join("conftest.py").exists() {
        return true;
    }
    std::fs::read_to_string(dir.join("pyproject.toml"))
        .map(|content| content.contains("[tool.pytest"))
        .unwrap_or(false)
}

/// A file name that is obviously a test/check file by naming convention,
/// independent of any manifest: `test_*.py`, `*_test.py`, `check.*`,
/// `test*.mjs`, or anything with a `.test.` segment (`foo.test.js`).
fn is_test_like_filename(name: &str) -> bool {
    if name.starts_with("check.") {
        return true;
    }
    if name.ends_with(".py") {
        return name.starts_with("test_") || name.ends_with("_test.py");
    }
    if name.starts_with("test") && name.ends_with(".mjs") {
        return true;
    }
    name.contains(".test.")
}

/// The path (relative to `dir` when possible) of the first manifestless test
/// file found: preferring a `tests/` directory's own matches, falling back to
/// the project root. Entries are sorted so the result is deterministic.
fn find_manifestless_test_file(dir: &Path) -> Option<PathBuf> {
    let matches_in = |scan_dir: &Path| -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(scan_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.is_file()
                            && path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(is_test_like_filename)
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    };

    let tests_dir = dir.join("tests");
    let candidates = if tests_dir.is_dir() {
        matches_in(&tests_dir)
    } else {
        Vec::new()
    };
    let candidates = if candidates.is_empty() {
        matches_in(dir)
    } else {
        candidates
    };
    candidates.into_iter().next()
}

/// `path` made relative to `dir` for a clean, non-absolute check label —
/// falls back to the path as given when it is not actually under `dir`.
fn relative_to(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// The direct-run command for a manifestless file, chosen by extension. `None`
/// for an extension with no safe default runtime (the caller falls back to
/// the single-file heuristics below).
fn direct_run_check(dir: &Path, path: &Path) -> Option<Check> {
    let rel = relative_to(dir, path);
    match path.extension().and_then(|e| e.to_str()) {
        Some("py") => Some(Check::direct_owned("python3", vec![rel])),
        Some("mjs") | Some("js") => Some(Check::direct_owned("node", vec![rel])),
        _ => None,
    }
}

/// Every file directly in `dir` (non-recursive) whose extension is `ext`.
fn files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file() && path.extension().and_then(|e| e.to_str()) == Some(ext)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The manifestless-project fallback: no Cargo/go/npm/Python/Makefile check
/// applies, but the directory shape still has something obviously runnable.
/// Tried in order: (a) a `tests/`-dir or test-named file, run directly; (b) a
/// single lone `.py` file, syntax-smoke-checked (never run — it might not be
/// a script meant to execute cleanly on its own); (c) a single lone `.mjs`/
/// `.js` file, syntax-checked the same way. `None` when none of these shapes
/// match, so the caller's actionable "could not detect" error still fires.
fn manifestless_checks(dir: &Path) -> Option<Vec<Check>> {
    if let Some(test_file) = find_manifestless_test_file(dir) {
        if let Some(check) = direct_run_check(dir, &test_file) {
            return Some(vec![check]);
        }
    }

    let py_files = files_with_extension(dir, "py");
    if py_files.len() == 1 {
        let rel = relative_to(dir, &py_files[0]);
        // X-verify-2: `py_compile` only parses the file — it never executes a
        // line of it, so a PASS here must never be reported (or trusted by a
        // consumer) as if real tests ran.
        return Some(vec![Check::direct_owned(
            "python3",
            vec!["-m".to_string(), "py_compile".to_string(), rel],
        )
        .smoke_only()]);
    }

    let js_files: Vec<PathBuf> = files_with_extension(dir, "mjs")
        .into_iter()
        .chain(files_with_extension(dir, "js"))
        .collect();
    if js_files.len() == 1 {
        let rel = relative_to(dir, &js_files[0]);
        // X-verify-2: `node --check` is the JS equivalent of `py_compile` —
        // syntax-only, never executed.
        return Some(vec![Check::direct_owned(
            "node",
            vec!["--check".to_string(), rel],
        )
        .smoke_only()]);
    }

    None
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

    let mut git_context = None;
    let changes = match supplied_changes {
        Some(paths) => normalize_supplied_changes(dir, paths)?,
        None => match git_changes_under(dir) {
            Some(changes) => {
                git_context = Some((changes.root, changes.relative_dir));
                changes.paths
            }
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
    tracing::debug!(
        ?changes,
        supplied = supplied_changes.is_some(),
        "verify initial change set"
    );
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
                if let Some((root, relative_dir)) = git_context {
                    let committed = changes_since_branch_fork_point(&root, &relative_dir);
                    if let Some(committed) = committed {
                        let mut union = changes;
                        union.extend(committed);
                        union.sort();
                        union.dedup();
                        tracing::debug!(?union, "verify using working-tree and fork-point union");
                        // Retry scoped selection on recognized paths in the union.
                        // Unmapped paths must not hide committed Rust work, nor
                        // authorize a workspace-wide check. Never filter by name.
                        let rust_paths: Vec<_> = union
                            .into_iter()
                            .filter(|path| is_rust_path(path))
                            .collect();
                        if !rust_paths.is_empty() {
                            return narrow_rust_checks(dir, &rust_paths);
                        }
                    }
                }
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

struct GitChanges {
    root: PathBuf,
    relative_dir: PathBuf,
    paths: Vec<String>,
}

/// Resolve the repository root and return tracked/untracked paths below `dir`.
/// Retain the resolved context for a later fork-point fallback.
/// A failure is surfaced rather than falling back to a broad Cargo command.
fn git_changes_under(dir: &Path) -> Option<GitChanges> {
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
    // Y10e: `diff HEAD` + `ls-files --others` describes the WORKING TREE, so
    // the instant a goal worker commits its work the change set goes empty and
    // `select_checks` answers "no changed files were found under this
    // verification path" — M15 fx05, fx08, fx09 and fx10 each burned a turn on
    // exactly that, having just committed on their `goal/<session>` branch.
    // The work is not gone, it is one commit down. Only in that case (nothing
    // uncommitted to describe, so today's answer is a useless NO CHECKS) widen
    // the base to this branch's fork point. See [`branch_fork_point`].
    if paths.is_empty() {
        if let Some(committed) = changes_since_branch_fork_point(&root, &relative_dir) {
            tracing::debug!(
                ?committed,
                "verify using fork-point diff for empty working tree"
            );
            paths = committed;
        }
    }
    Some(GitChanges {
        root,
        relative_dir,
        paths,
    })
}

/// Run `git -C root <args>` and return its stdout, or `None` on any failure.
fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(output.stdout).ok()
}

/// The local branch this repo's goal work eventually lands on, detected
/// without any card/DB context — `verify.rs` runs as a tool inside the
/// worktree only, and every `goal/<run-id>` branch (see
/// `goal_engine::goal_branch_name`) forks from this one, directly or
/// through a chain of redispatch attempts. Tried in order: the remote's own
/// recorded default branch (`refs/remotes/origin/HEAD`, set by `git remote
/// set-head`/a fresh clone), then the conventional local names `main` and
/// `master`. `None` when nothing matches, letting [`branch_fork_point`]
/// fall back to its pre-R54a approximation below.
fn detect_trunk_branch(root: &Path) -> Option<String> {
    if let Some(sym) = git_stdout(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let sym = sym.trim();
        if !sym.is_empty() {
            return Some(sym.to_string());
        }
    }
    for candidate in ["main", "master"] {
        let refname = format!("refs/heads/{candidate}");
        if git_stdout(root, &["rev-parse", "--verify", "--quiet", &refname]).is_some() {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Y10e: the commit this branch forked from.
///
/// This is what "the work this goal branch has done" means once the worker has
/// committed: a goal worktree checks out `goal/<session>` off the trunk, so the
/// trunk's own tip is reachable from some other ref and every commit the worker
/// made is not.
///
/// R54a (A51 analysis, "branch_fork_point"): prefer a TRUNK-relative fork
/// point — `git merge-base <trunk> HEAD` — over the old `--exclude`-based
/// approximation below whenever [`detect_trunk_branch`] can name the trunk.
/// The old walk treats ANY other local/remote branch as "already covering"
/// HEAD's commits, including this SAME goal's own prior-attempt sibling
/// branches (`goal/<run-id>` — Y16b mints a fresh one per redispatch and
/// never deletes the last one's), so a redispatched goal whose final
/// attempt makes only a small/no-op commit on top of real earlier progress
/// sees its own earlier commits treated as "covered by
/// goal/<earlier-session>" and narrows to just that last commit — the A51
/// Run 2 defect (verified diff = the empty checkpoint commit → NO CHECKS →
/// an unverified reviewer hallucinated a rejection of complete work).
/// `merge-base(trunk, HEAD)` sidesteps the ambiguity entirely: it is the
/// point this branch actually diverged from the project's landing branch,
/// unaffected by how many sibling attempt branches happen to still exist.
///
/// Falls back to the original `--exclude`-based walk only when no trunk can
/// be named (no remote configured and no local `main`/`master`) — itself
/// narrowed by this lane to also `--exclude=goal/*`: every `goal/<run-id>`
/// branch is, by the repo's own naming convention, ALWAYS a goal-attempt
/// branch (this goal's own prior attempts, or another goal's) and never a
/// long-lived integration branch, so none of them can legitimately be
/// "already covering" this branch's commits either. Deliberately narrow in
/// both paths — `None` on a detached HEAD (no branch to scope to), on a
/// branch with no unique commits (nothing was committed here), and when the
/// fork point would be a root commit with nothing to diff against.
fn branch_fork_point(root: &Path) -> Option<String> {
    let branch = git_stdout(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }

    if let Some(trunk) = detect_trunk_branch(root) {
        if trunk != branch {
            if let Some(base) = git_stdout(root, &["merge-base", &trunk, "HEAD"]) {
                let base = base.trim().to_string();
                if !base.is_empty() {
                    return Some(base);
                }
            }
        }
    }

    // `--exclude` is matched against the ref name with the `refs/heads/`
    // prefix ALREADY STRIPPED when it feeds `--branches`, so the pattern is
    // the bare branch name. Spelled `refs/heads/<branch>` it matches nothing,
    // `--branches` re-includes this branch, every commit on it is excluded by
    // `--not`, and the walk returns empty — no fork point, no fallback.
    // `--exclude` patterns accumulate and apply to the `--branches`/
    // `--remotes` glob that follows, so `goal/*` drops every goal-attempt
    // branch from consideration alongside this branch itself.
    let exclude_self = format!("--exclude={branch}");
    let unique = git_stdout(
        root,
        &[
            "rev-list",
            "HEAD",
            "--not",
            &exclude_self,
            "--exclude=goal/*",
            "--branches",
            "--remotes",
        ],
    )?;
    // `rfind`, not `last()`: `rev-list` prints newest first, so the oldest
    // unique commit is the last non-empty line. Walking from the back is what
    // clippy's `double_ended_iterator_last` / `filter_next` pair insists on;
    // the element chosen is identical.
    let oldest = unique
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())?;
    // A root commit has no parent: `rev-parse --verify` fails and we stop,
    // rather than diffing the whole history and calling it a scoped check.
    let parent = format!("{oldest}^");
    let sha = git_stdout(root, &["rev-parse", "--verify", "--quiet", &parent])?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        return None;
    }
    Some(sha)
}

/// The paths changed since [`branch_fork_point`], filtered to `relative_dir`
/// exactly as the working-tree scan is. `None` when there is no fork point to
/// measure from, so the caller keeps its existing (empty) answer.
fn changes_since_branch_fork_point(root: &Path, relative_dir: &Path) -> Option<Vec<String>> {
    let base = branch_fork_point(root)?;
    let text = git_stdout(root, &["diff", "--name-only", &base])?;
    let mut paths = text
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| {
            let path = Path::new(path);
            relative_dir.as_os_str().is_empty() || path.starts_with(relative_dir)
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return None;
    }
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

/// Matches a check's failure output against the handful of standard ways an
/// interpreter/runtime reports "I started fine, but a package you asked me to
/// import/require is not installed for me" — as opposed to a defect in the
/// code under test. Two Python shapes both occur in the wild for the exact
/// same underlying gap and must both be caught:
/// - `python3 -m X` when `X` isn't importable prints
///   `<interpreter path>: No module named X` (no exception, no quotes).
/// - code that does `import X` prints a traceback ending
///   `ModuleNotFoundError: No module named 'X'` (quoted).
///
/// Node's `require`/`import` resolution failure and Ruby's `LoadError` follow
/// the same "found the runtime, not the package" shape and are cheap to cover
/// alongside Python since `verify` already special-cases pytest/jest/vitest
/// elsewhere in this file. Captures the missing package name for the hint.
static MISSING_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)No module named '?([\w][\w.\-]*)'?|Cannot find module '([^']+)'|cannot load such file -- (\S+)"#,
    )
    .unwrap()
});

/// Returns the missing package/module name when `output` looks like a check
/// failed because the interpreter/runtime that ran it could not IMPORT a
/// dependency — see [`MISSING_IMPORT_RE`] — distinct from
/// [`CheckOutcome::ToolMissing`] (the executable itself was never found on
/// PATH, e.g. no `pytest` binary at all: that case already gets its own clear
/// "install it" message in `verify_with_cwd`). This is the sibling gap: the
/// interpreter (e.g. `python3`) IS on PATH and DID run, but a package it
/// tried to import is missing for that specific interpreter — often because a
/// machine has more than one `python3` and only one of them has the package.
/// A model that treats this as an ordinary test failure and starts editing
/// application code to "fix" it is chasing an environment gap, not a bug.
fn missing_import_dependency(output: &str) -> Option<String> {
    let caps = MISSING_IMPORT_RE.captures(output)?;
    caps.iter()
        .skip(1)
        .flatten()
        .next()
        .map(|m| m.as_str().to_string())
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
    if let Exec::Probe(kind) = &check.exec {
        return run_probe_check(kind, dir, timeout_secs).await;
    }
    let mut command = match &check.exec {
        Exec::Direct { program, args } => {
            let mut command = tokio::process::Command::new(program);
            command.args(args);
            command
        }
        Exec::Shell(command_line) => shell_command(command_line),
        Exec::Markdown { .. } => unreachable!("markdown checks return before spawning a process"),
        Exec::Probe(_) => unreachable!("probe checks return before spawning a process"),
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
        CheckOutcome::Passed {
            output: combine_streams(&output.stdout, &output.stderr),
        }
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
                let destination = line.get(start + 2..).unwrap_or("");
                let inside_parens = destination
                    .split_once(')')
                    .map(|(inside, _)| inside)
                    .unwrap_or("");
                if !destination.contains(')') || inside_parens.trim().is_empty() {
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
        CheckOutcome::Passed {
            output: String::new(),
        }
    } else {
        CheckOutcome::Failed {
            exit_code: Some(1),
            output: findings.join("\n"),
        }
    }
}

// ── R21c: acceptance-criteria probe checks ──────────────────────────────────
//
// When `select_checks` can offer nothing but a syntax-only smoke check (or
// nothing at all), the only way `verify` can produce a GENUINE, non-vacuous
// pass for a manifestless project is to check something the acceptance
// criteria actually asked for. This mirrors the file_exists/command_exit_zero
// vocabulary `orchestrator::checks_from_acceptance` already compiles from
// acceptance criteria for a different purpose (crediting a sibling goal's
// committed diff) — evaluated locally here for the same reason that code is
// evaluated locally there: this crate has no dependency on goose-server's
// `verification::checks`. `SymbolDefined` is new: neither that vocabulary nor
// checks.rs has a "does this symbol exist" check today, and the fizzbuzz-
// shaped defect this lane fixes needs exactly that.

/// Validate a probe-supplied path before any filesystem access (R44a/#A39):
/// reject an absolute path outright (a probe only ever checks inside the goal
/// worktree — the compiler that seeds these paths is supposed to have
/// relativised or dropped an absolute one already, but a probe must never
/// trust that and fall back to a silent host filesystem lookup) and reject
/// any path that could escape the worktree via `..` or another non-plain
/// component, mirroring the same guard [`normalize_supplied_changes`] already
/// applies to caller-supplied changed-file paths. Does not require the target
/// to exist — `FileExists` probes a path that may legitimately be absent —
/// so this only joins and validates, it never canonicalizes the candidate.
fn safe_probe_path(dir: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let supplied = Path::new(rel_path);
    if supplied.is_absolute() {
        return Err(
            "is an absolute path — a probe can only check paths inside the goal worktree, \
             not the host filesystem"
                .to_string(),
        );
    }
    if supplied
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(
            "must be relative and cannot contain parent-directory traversal (`..`)".to_string(),
        );
    }
    Ok(dir.join(supplied))
}

/// Run one synthesized probe. `FileExists`/`SymbolDefined` are plain,
/// synchronous filesystem reads — cheap enough not to need the timeout.
/// `CommandExitsZero` delegates to the ordinary `Shell` path (same spawn,
/// timeout, and output-capture behavior as any other check) rather than
/// duplicating it.
async fn run_probe_check(kind: &ProbeKind, dir: &Path, timeout_secs: u64) -> CheckOutcome {
    match kind {
        ProbeKind::FileExists(rel_path) => {
            let full_path = match safe_probe_path(dir, rel_path) {
                Ok(full_path) => full_path,
                Err(reason) => {
                    return CheckOutcome::SpawnError(format!(
                        "PROBE: cannot check `{rel_path}`: {reason}"
                    ));
                }
            };
            if full_path.is_file() {
                CheckOutcome::Passed {
                    output: format!("PROBE: `{rel_path}` exists."),
                }
            } else {
                CheckOutcome::Failed {
                    exit_code: None,
                    output: format!(
                        "PROBE: `{rel_path}` does not exist under `{}`.",
                        dir.display()
                    ),
                }
            }
        }
        ProbeKind::SymbolDefined { symbol, kind, file } => {
            let found = match file {
                Some(file) => {
                    if let Err(reason) = safe_probe_path(dir, file) {
                        return CheckOutcome::SpawnError(format!(
                            "PROBE: cannot check `{file}`: {reason}"
                        ));
                    }
                    find_symbol_definition_in_file(dir, file, symbol, *kind)
                }
                None => find_symbol_definition(dir, symbol, *kind),
            };
            match found {
                Some(location) => CheckOutcome::Passed {
                    output: format!(
                        "PROBE: {} `{symbol}` is defined at {location}.",
                        kind.label()
                    ),
                },
                None => CheckOutcome::Failed {
                    exit_code: None,
                    output: match file {
                        Some(file) => format!(
                            "PROBE: no {} named `{symbol}` was found in `{file}`.",
                            kind.label()
                        ),
                        None => format!(
                            "PROBE: no {} named `{symbol}` was found in the scanned source \
                             files under `{}`.",
                            kind.label(),
                            dir.display()
                        ),
                    },
                },
            }
        }
        ProbeKind::CommandExitsZero(command_line) => {
            // `run_check` -> `run_probe_check` -> `run_check` is a genuine
            // async cycle (a probe may resolve to an ordinary shell check),
            // so this edge must be boxed or the future would be infinitely
            // sized (E0733). One `Box::pin` anywhere in the cycle is enough;
            // put it here, on the rarer probe path, rather than taxing every
            // ordinary check.
            Box::pin(run_check(&Check::shell(command_line), dir, timeout_secs)).await
        }
    }
}

/// Bounded, best-effort symbol lookup (X-verify-3): a small, documented set of
/// per-language regexes for function/class definitions, scanned over the
/// verification directory's TOP-LEVEL files only — the same non-recursive
/// scope [`manifestless_checks`] itself uses, because the shape this probe
/// exists for is a small, manifestless scaffold, not a full repository walk.
/// Returns the first `relative/path:line` match, or `None`. A probe that
/// finds nothing here FAILS rather than being silently skipped: the scan
/// scope is part of what the probe mechanically checked, not a reason to call
/// the criterion unverifiable.
fn find_symbol_definition(dir: &Path, symbol: &str, kind: SymbolKind) -> Option<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    for path in files {
        // A file whose extension has no pattern for `kind` is skipped, not a
        // reason to abandon the whole scan — `?` here would incorrectly bail
        // out of the entire function on the first unrecognized-extension
        // file (e.g. a README.md sorting before the actual source file).
        let Some(pattern) = symbol_pattern(&path, symbol, kind) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (line_number, line) in text.lines().enumerate() {
            if pattern.is_match(line) {
                return Some(format!("{}:{}", relative_to(dir, &path), line_number + 1));
            }
        }
    }
    None
}

/// The same lookup as [`find_symbol_definition`], but scoped to exactly one
/// named file (R21b's structured `probe:symbol_defined path=… symbol=…
/// kind=…` grammar names the file explicitly, so there is no need to guess
/// via a directory scan — and no risk of matching a same-named symbol in an
/// unrelated file).
fn find_symbol_definition_in_file(
    dir: &Path,
    file: &str,
    symbol: &str,
    kind: SymbolKind,
) -> Option<String> {
    let path = dir.join(file);
    let pattern = symbol_pattern(&path, symbol, kind)?;
    let text = std::fs::read_to_string(&path).ok()?;
    for (line_number, line) in text.lines().enumerate() {
        if pattern.is_match(line) {
            return Some(format!("{file}:{}", line_number + 1));
        }
    }
    None
}

/// The per-language definition pattern for `symbol`, or `None` when the
/// file's extension has no recognized pattern for `kind` — the caller then
/// simply moves to the next file rather than matching a wrong language's
/// syntax.
fn symbol_pattern(path: &Path, symbol: &str, kind: SymbolKind) -> Option<Regex> {
    let escaped = regex::escape(symbol);
    let ext = path.extension().and_then(|e| e.to_str())?;
    let pattern = match (ext, kind) {
        ("py", SymbolKind::Function) => format!(r"^\s*(?:async\s+)?def\s+{escaped}\s*\("),
        ("py", SymbolKind::Class) => format!(r"^\s*class\s+{escaped}\s*[:\(]"),
        ("js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx", SymbolKind::Function) => format!(
            r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+{escaped}\s*\(|^\s*(?:export\s+)?(?:const|let|var)\s+{escaped}\s*=\s*(?:async\s*)?(?:\(|function\b)"
        ),
        ("js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx", SymbolKind::Class) => {
            format!(r"^\s*(?:export\s+)?(?:default\s+)?class\s+{escaped}\b")
        }
        ("rs", SymbolKind::Function) => {
            format!(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{escaped}\s*[<(]")
        }
        ("rs", SymbolKind::Class) => {
            format!(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+{escaped}\b")
        }
        ("go", SymbolKind::Function) => format!(r"^\s*func\s+(?:\([^)]*\)\s*)?{escaped}\s*\("),
        ("go", SymbolKind::Class) => format!(r"^\s*type\s+{escaped}\s+struct\b"),
        ("rb", SymbolKind::Function) => format!(r"^\s*def\s+(?:self\.)?{escaped}\b"),
        ("rb", SymbolKind::Class) => format!(r"^\s*class\s+{escaped}\b"),
        _ => return None,
    };
    Regex::new(&pattern).ok()
}

/// The marker `orchestrator::decompose_roadmap` prefixes a criterion with
/// when no repo check command grounds it — stripped, when present, before
/// probe matching so both the raw criterion and the marked form work.
const UNVERIFIED_MARKER_PREFIX: &str =
    "UNVERIFIED (no detected check command in this repo) — kept for human review: ";

/// "`<path>` exists" / "<path> exists" — a plain file-existence claim.
static FILE_EXISTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)`?([\w./-]+\.[A-Za-z0-9_]{1,10})`?\s+exists\b").unwrap());

/// "`<command>` exits 0" / "`<command>` exits with code 0".
static COMMAND_EXIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)`([^`]{1,300})`\s+exits?(?:\s+with)?(?:\s+code)?\s+0\b").unwrap()
});

/// "defines a function named X" / "defines class X" / "function X is defined".
static SYMBOL_DEFINED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bdefines?\s+(?:a\s+)?(function|method|class)\s+(?:named\s+|called\s+)?`?(\w+)`?\b|\b(function|method|class)\s+(?:named\s+|called\s+)?`?(\w+)`?\s+is\s+defined\b",
    )
    .unwrap()
});

fn file_exists_probe(text: &str) -> Option<Check> {
    let caps = FILE_EXISTS_RE.captures(text)?;
    let path = caps.get(1)?.as_str().to_string();
    Some(Check::probe(
        format!("acceptance criterion: `{path}` exists"),
        ProbeKind::FileExists(path),
    ))
}

fn command_exit_probe(text: &str) -> Option<Check> {
    let caps = COMMAND_EXIT_RE.captures(text)?;
    let command = caps.get(1)?.as_str().trim().to_string();
    if command.is_empty() {
        return None;
    }
    Some(Check::probe(
        format!("acceptance criterion: `{command}` exits 0"),
        ProbeKind::CommandExitsZero(command),
    ))
}

fn symbol_defined_probe(text: &str) -> Option<Check> {
    let caps = SYMBOL_DEFINED_RE.captures(text)?;
    let (kind_word, symbol) = match (caps.get(1), caps.get(2), caps.get(3), caps.get(4)) {
        (Some(k), Some(s), _, _) => (k.as_str(), s.as_str()),
        (_, _, Some(k), Some(s)) => (k.as_str(), s.as_str()),
        _ => return None,
    };
    let kind = if kind_word.eq_ignore_ascii_case("class") {
        SymbolKind::Class
    } else {
        SymbolKind::Function
    };
    Some(Check::probe(
        format!(
            "acceptance criterion: {} `{symbol}` is defined",
            kind.label()
        ),
        ProbeKind::SymbolDefined {
            symbol: symbol.to_string(),
            kind,
            file: None,
        },
    ))
}

/// R35b (A32 root cause 2): "calling `f(args)` returns `value`" / "`f(args)`
/// == `value`" — the shape a model most often writes for a manifestless
/// single-file goal. Backtick-delimited on both sides so the call expression
/// and the expected value are unambiguous.
static CALL_RETURNS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)`([A-Za-z_][\w.]*\([^()`]*\))`\s*(?:returns?|==)\s*`([^`]*)`").unwrap()
});

/// The same shape without backticks: "f(args) should return value", or
/// plainly "f(args) returns value" — the exact `"fizzbuzz(3) returns
/// 'Fizz'"` text A32 root cause 2 traced as the freeform criterion that fell
/// through to a vacuous smoke-only PASS.
static CALL_RETURNS_BARE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([A-Za-z_][\w.]*\([^()\n]*\))\s+(?:should\s+return|returns?)\s+(.+)$")
        .unwrap()
});

/// A single, flat call expression — no nested calls or parens in the
/// argument list. [`call_returns_probe`] rejects anything else rather than
/// risk splicing something unexpected into the synthesized Python check.
static CALL_EXPR_SHAPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][\w.]*\([^()]*\)$").unwrap());

/// "running `<cmd>` prints `<text>`" / "running `<cmd>` outputs `<text>`".
static RUN_STDOUT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)running\s+`([^`]{1,300})`\s+(?:prints?|outputs?)\s+`([^`]*)`").unwrap()
});

/// Normalize a captured value token into valid Python source: an
/// already-quoted string, `True`/`False`/`None`, a list/tuple literal, or a
/// number are used as-is; a bare word or phrase (the common unquoted-string
/// shape, e.g. "returns Fizz") is wrapped as a Python string literal via
/// Rust's `Debug` formatting — close enough to Python string-literal syntax
/// for the plain ASCII text these criteria are written in. Anything else
/// (unrecognized punctuation, characters that would make the shape
/// ambiguous) is rejected rather than guessed at.
fn python_literal_for_value(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        return Some(trimmed.to_string());
    }
    if trimmed == "True" || trimmed == "False" || trimmed == "None" {
        return Some(trimmed.to_string());
    }
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('(') && trimmed.ends_with(')'))
    {
        return Some(trimmed.to_string());
    }
    let digits = trimmed.trim_start_matches(['-', '+']);
    if !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().any(|c| c.is_ascii_digit())
    {
        return Some(trimmed.to_string());
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ')
    {
        return Some(format!("{trimmed:?}"));
    }
    None
}

/// Wrap `text` as one POSIX shell single-quoted argument, safe regardless of
/// its contents (quotes, `$`, backticks, newlines): the standard
/// close-quote/escaped-quote/reopen-quote technique. Used to embed
/// criterion-supplied text into a generated `ProbeKind::CommandExitsZero`
/// command without needing to reason about shell metacharacters in that text.
fn shell_single_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The primary language(s) [`call_returns_probe`]/[`node_call_returns_probe`]
/// may safely synthesize a real execution probe for, detected from the same
/// marker files [`detect_checks`] already keys off. R54a (A51 analysis,
/// "call_returns_probe"): `call_returns_probe` used to compile a Python
/// `exec`/assert script for ANY repo whose criterion text merely looked
/// call-shaped, including a TypeScript-only fixture with no `.py` file
/// anywhere — a guaranteed `NameError`, for any model, forever. Checked in
/// the same precedence [`detect_checks`] uses (Cargo.toml, then a JS/TS
/// marker, then a Python marker or a bare top-level `.py` file — the exact
/// manifestless shape [`call_returns_probe`] was written for and that carries
/// no `pyproject.toml`/`setup.py` of its own).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeLanguage {
    Python,
    JavaScript,
    Rust,
    Other,
}

fn detect_probe_language(dir: &Path) -> ProbeLanguage {
    if dir.join("Cargo.toml").exists() {
        return ProbeLanguage::Rust;
    }
    if dir.join("package.json").exists() || dir.join("tsconfig.json").exists() {
        return ProbeLanguage::JavaScript;
    }
    if PYTHON_MARKERS
        .iter()
        .any(|marker| dir.join(marker).exists())
        || has_top_level_py_file(dir)
    {
        return ProbeLanguage::Python;
    }
    ProbeLanguage::Other
}

/// A bare top-level `.py` file with no `pyproject.toml`/`setup.py` of its
/// own — the manifestless single-file shape A32 root cause 2 motivated
/// [`call_returns_probe`] for in the first place, which none of
/// [`PYTHON_MARKERS`] would otherwise catch.
fn has_top_level_py_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("py"))
}

/// Parse "calling `f(args)` returns `value`" / "`f(args)` == `value`" (see
/// [`CALL_RETURNS_RE`]) or "f(args) returns value" / "f(args) should return
/// value" (see [`CALL_RETURNS_BARE_RE`]) into `(call_expression,
/// expected_value_text)`. Shared by every per-language call/return probe
/// below so the two closed text shapes are parsed exactly once.
fn call_returns_captures(text: &str) -> Option<(String, String)> {
    if let Some(caps) = CALL_RETURNS_RE.captures(text) {
        return Some((
            caps.get(1)?.as_str().trim().to_string(),
            caps.get(2)?.as_str().trim().to_string(),
        ));
    }
    let caps = CALL_RETURNS_BARE_RE.captures(text)?;
    Some((
        caps.get(1)?.as_str().trim().to_string(),
        caps.get(2)?
            .as_str()
            .trim()
            .trim_end_matches(['.', '!'])
            .trim()
            .to_string(),
    ))
}

/// R35b: "calling `f(args)` returns `value`" (see [`CALL_RETURNS_RE`]) or
/// "f(args) should return value" (see [`CALL_RETURNS_BARE_RE`]) becomes a
/// [`ProbeKind::CommandExitsZero`] that `exec`s every top-level `.py` file in
/// the verification directory (the same manifestless, non-recursive scope
/// [`manifestless_checks`]/[`find_symbol_definition`] already use) and
/// asserts the call's actual result equals the expected value — real
/// execution, not a syntax-only smoke check.
///
/// R54a: gated on [`detect_probe_language`] returning `Python` — `None`
/// immediately otherwise, so a JS/TS/Rust/undetected repo never gets a
/// Python script compiled against it (see [`node_call_returns_probe`] for
/// the JS/TS counterpart). `None` also when the call expression isn't a
/// single flat call ([`CALL_EXPR_SHAPE_RE`]) or the value doesn't normalize
/// into valid Python source ([`python_literal_for_value`]) — left honestly
/// unverified rather than approximated, same as every other probe shape.
fn call_returns_probe(dir: &Path, text: &str) -> Option<Check> {
    if detect_probe_language(dir) != ProbeLanguage::Python {
        return None;
    }
    let (expr, value_raw) = call_returns_captures(text)?;
    if !CALL_EXPR_SHAPE_RE.is_match(&expr) {
        return None;
    }
    let value_literal = python_literal_for_value(&value_raw)?;
    let script = format!(
        "import glob\nfor _f in sorted(glob.glob('*.py')):\n    exec(open(_f).read())\n\
         _actual = {expr}\n_expected = {value_literal}\nassert _actual == _expected, \
         f\"expected {{_expected!r}}, got {{_actual!r}}\"\n"
    );
    let command = format!("python3 -c {}", shell_single_quote(&script));
    Some(Check::probe(
        format!("acceptance criterion: `{expr}` returns `{value_raw}`"),
        ProbeKind::CommandExitsZero(command),
    ))
}

/// Normalize a captured value token into valid JS source — the same closed
/// shapes [`python_literal_for_value`] accepts, spelled with JS's own
/// literal keywords (`true`/`false`/`null` rather than
/// `True`/`False`/`None`); a bare word/phrase is still wrapped via Rust's
/// `Debug` formatting, which produces a double-quoted string valid in both
/// languages for the plain ASCII text these criteria are written in.
fn js_literal_for_value(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        return Some(trimmed.to_string());
    }
    if trimmed == "true" || trimmed == "false" || trimmed == "null" {
        return Some(trimmed.to_string());
    }
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
    {
        return Some(trimmed.to_string());
    }
    let digits = trimmed.trim_start_matches(['-', '+']);
    if !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().any(|c| c.is_ascii_digit())
    {
        return Some(trimmed.to_string());
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ')
    {
        return Some(format!("{trimmed:?}"));
    }
    None
}

/// R54a: the JS/TS counterpart to [`call_returns_probe`], gated the same way
/// ([`detect_probe_language`] must return `JavaScript`) but additionally
/// requiring the call's function name to resolve to a REAL, locatable
/// top-level definition ([`find_symbol_definition`], the same lookup
/// [`symbol_defined_probe`] already trusts) in a plain `.js`/`.mjs`/`.cjs`
/// file specifically — a `.ts`/`.tsx` source cannot be `require()`d by a
/// bare `node -e` without a compiler, and guessing at one would risk
/// exactly the wrong-language guaranteed failure this lane exists to
/// prevent. `None` (criterion left honestly UNVERIFIED, its marker intact)
/// whenever any of that doesn't hold — never approximated, same closed-set
/// discipline as every other probe shape.
fn node_call_returns_probe(dir: &Path, text: &str) -> Option<Check> {
    if detect_probe_language(dir) != ProbeLanguage::JavaScript {
        return None;
    }
    let (expr, value_raw) = call_returns_captures(text)?;
    if !CALL_EXPR_SHAPE_RE.is_match(&expr) {
        return None;
    }
    let fn_name = expr.split('(').next()?.trim();
    if fn_name.is_empty()
        || !fn_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let location = find_symbol_definition(dir, fn_name, SymbolKind::Function)?;
    let file = location.split(':').next()?.to_string();
    if !matches!(
        Path::new(&file).extension().and_then(|e| e.to_str()),
        Some("js" | "mjs" | "cjs")
    ) {
        return None;
    }
    let value_literal = js_literal_for_value(&value_raw)?;
    let script = format!(
        "const _m = require('./{file}');\n\
         const {fn_name} = (typeof _m === 'function') ? _m : \
         ((_m && typeof _m.{fn_name} === 'function') ? _m.{fn_name} : undefined);\n\
         if (typeof {fn_name} !== 'function') {{ \
         throw new Error('{fn_name} is not an exported function of {file}'); }}\n\
         const _actual = {expr};\nconst _expected = {value_literal};\n\
         if (JSON.stringify(_actual) !== JSON.stringify(_expected)) {{ \
         throw new Error('expected ' + JSON.stringify(_expected) + ', got ' + \
         JSON.stringify(_actual)); }}\n"
    );
    let command = format!("node -e {}", shell_single_quote(&script));
    Some(Check::probe(
        format!("acceptance criterion: `{expr}` returns `{value_raw}`"),
        ProbeKind::CommandExitsZero(command),
    ))
}

/// R35b: "running `<cmd>` prints `<text>`" / "running `<cmd>` outputs
/// `<text>`" (see [`RUN_STDOUT_RE`]) becomes a [`ProbeKind::CommandExitsZero`]
/// that pipes the command's real output through `grep -qF` for the expected
/// text — reuses the existing `CommandExitsZero` → `Shell` delegation
/// ([`run_probe_check`]) exactly like [`command_exit_probe`] does, just with
/// a synthesized pipeline instead of the criterion's own command verbatim.
/// `cmd` is spliced in unescaped, same trust boundary as
/// [`command_exit_probe`]'s own backtick-delimited command text; `text` is
/// shell-quoted via [`shell_single_quote`] so it is matched literally
/// regardless of its contents.
fn run_stdout_probe(text: &str) -> Option<Check> {
    let caps = RUN_STDOUT_RE.captures(text)?;
    let cmd = caps.get(1)?.as_str().trim().to_string();
    let expected = caps.get(2)?.as_str();
    if cmd.is_empty() || expected.is_empty() {
        return None;
    }
    let command = format!("{cmd} | grep -qF -- {}", shell_single_quote(expected));
    Some(Check::probe(
        format!("acceptance criterion: `{cmd}` prints `{expected}`"),
        ProbeKind::CommandExitsZero(command),
    ))
}

/// R21b's structured probe-check grammar (`orchestrator::{
/// PROBE_FILE_EXISTS_PREFIX, PROBE_SYMBOL_DEFINED_PREFIX,
/// PROBE_COMMAND_PREFIX}` — the sole producer, `rewrite_ungrounded_named_
/// criteria`'s no-detected-check fallback), parsed here as plain TEXT
/// rather than by importing R21b's Rust types: the two lanes land on
/// separate branches and are not always merged together, so the wire
/// format (not a shared Rust type) is the contract.
///
/// ```text
/// probe:file_exists path=<path>
/// probe:symbol_defined path=<path> symbol=<name> kind=<function|method|class>
/// probe:command cmd="<command>"
/// ```
///
/// optionally followed by `" — "` (space, em dash, space) plus the original
/// criterion text, which is informational only and ignored here.
fn structured_probe(text: &str) -> Option<Check> {
    let marker = text.split(" — ").next().unwrap_or(text).trim();
    if let Some(rest) = marker.strip_prefix("probe:file_exists") {
        let path = probe_kv(rest, "path")?;
        return Some(Check::probe(
            format!("acceptance criterion: `{path}` exists"),
            ProbeKind::FileExists(path),
        ));
    }
    if let Some(rest) = marker.strip_prefix("probe:symbol_defined") {
        let path = probe_kv(rest, "path")?;
        let symbol = probe_kv(rest, "symbol")?;
        let kind_word = probe_kv(rest, "kind")?;
        let kind = if kind_word.eq_ignore_ascii_case("class") {
            SymbolKind::Class
        } else {
            SymbolKind::Function
        };
        return Some(Check::probe(
            format!(
                "acceptance criterion: {} `{symbol}` is defined in `{path}`",
                kind.label()
            ),
            ProbeKind::SymbolDefined {
                symbol,
                kind,
                file: Some(path),
            },
        ));
    }
    if let Some(rest) = marker.strip_prefix("probe:command") {
        let cmd = probe_quoted_kv(rest, "cmd")?;
        if cmd.trim().is_empty() {
            return None;
        }
        return Some(Check::probe(
            format!("acceptance criterion: `{cmd}` exits 0"),
            ProbeKind::CommandExitsZero(cmd),
        ));
    }
    None
}

/// Extract `key=<token>` (ends at the next whitespace, or the end of the
/// string) from a `probe:*` grammar tail. `None` when the key is absent or
/// its value is empty.
// Boundary-safe by construction: `start` is `str::find`'s match index plus the
// marker's own byte length (the marker's end boundary), and `end` comes from a
// `find` on the remainder or its `len()`. No index can split a UTF-8 character.
#[allow(clippy::string_slice)]
fn probe_kv(text: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let value = &rest[..end];
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Extract `key="<value>"` (double-quoted, may itself contain whitespace)
/// from a `probe:*` grammar tail.
// Boundary-safe by construction: `start` is `str::find`'s match index plus the
// marker's own byte length (the marker's end boundary), and `end` comes from a
// `find` on the remainder or its `len()`. No index can split a UTF-8 character.
#[allow(clippy::string_slice)]
fn probe_quoted_kv(text: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Compile ONE criterion into a probe check, trying each closed shape in
/// order — R21b's structured `probe:*` marker first (unambiguous when
/// present), then the free-text heuristics. `None` when the criterion
/// matches none of them — left honestly unverified, never approximated.
///
/// R54a: `dir` is the verification directory, threaded through so
/// [`call_returns_probe`]/[`node_call_returns_probe`] can gate themselves on
/// [`detect_probe_language`] — the other shapes here (`file_exists`,
/// `command_exit`, `symbol_defined`, `run_stdout`, the structured `probe:*`
/// grammar) are language-agnostic and ignore it.
fn probe_for_criterion(dir: &Path, criterion: &str) -> Option<Check> {
    let text = criterion
        .strip_prefix(UNVERIFIED_MARKER_PREFIX)
        .unwrap_or(criterion)
        .trim();
    structured_probe(text)
        .or_else(|| file_exists_probe(text))
        .or_else(|| command_exit_probe(text))
        .or_else(|| symbol_defined_probe(text))
        .or_else(|| call_returns_probe(dir, text))
        .or_else(|| node_call_returns_probe(dir, text))
        .or_else(|| run_stdout_probe(text))
}

/// Synthesize probe checks for every criterion that mechanically supports
/// one; every other criterion is silently dropped (not fabricated as a
/// check). See the module comment above for why this is a separate, local
/// implementation from `orchestrator::checks_from_acceptance` rather than a
/// shared one.
fn synthesize_probe_checks(dir: &Path, criteria: &[String]) -> Vec<Check> {
    criteria
        .iter()
        .filter_map(|criterion| probe_for_criterion(dir, criterion))
        .collect()
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
    vacuous_reason: Option<&str>,
) -> CallToolResult {
    result.structured_content = Some(json!({
        "kind": VERIFICATION_OBSERVATION_KIND,
        "command": command,
        "verdict": verdict,
        "evidence": evidence,
        "vacuous_reason": vacuous_reason,
    }));
    result
}

/// X-verify-2: a "the test command ran but exercised nothing" marker across a
/// small, curated set of common runners. Anchored with `\b` so a real "120
/// passed"/"20 passed" can never falsely match "0 passed". Intentionally
/// narrow — a false negative here (a genuine zero-test run using different
/// wording) only misses a vacuous pass, while a false positive would block a
/// real, useful one; the same conservative bias `after_turn::changed_lines`/
/// `changed_paths` document for their own leniency.
static ZERO_TESTS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\b0 passed\b|\bno tests ran\b|\bno test files\b|\bran 0 tests\b|\bcollected 0 items\b|\b0 examples, 0 failures\b)",
    )
    .unwrap()
});

/// Return the matched "zero tests" marker text, if the combined check output
/// contains one. See [`ZERO_TESTS_RE`].
fn zero_test_marker(output: &str) -> Option<String> {
    ZERO_TESTS_RE.find(output).map(|m| m.as_str().to_string())
}

/// Y10a: the counter-marker to [`ZERO_TESTS_RE`] — "at least one test actually
/// ran here". `cargo test` prints one `test result:` block PER TARGET, so a
/// crate whose real tests all pass still emits an empty doc-test block beside
/// them:
///
/// ```text
/// running 6 tests ... test result: ok. 6 passed; 0 failed; ...
///    Doc-tests bench_rs
/// running 0 tests ... test result: ok. 0 passed; 0 failed; ...
/// ```
///
/// [`ZERO_TESTS_RE`] scans the whole joined output, so it matched that trailing
/// `0 passed` and judged every multi-target `cargo test` vacuous. A non-zero
/// pass count (or a `running N tests` header with N > 0) anywhere in the SAME
/// output proves the run exercised something, which makes the zero marker a
/// sibling target rather than an empty run. Same conservative bias the
/// [`ZERO_TESTS_RE`] comment declares: missing a vacuous pass costs a great
/// deal less than blocking a real one.
static NONZERO_TESTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\b[1-9]\d* passed\b|\brunning [1-9]\d* tests?\b)").unwrap());

/// Return the matched "at least one test ran" marker text, if the combined
/// check output contains one. See [`NONZERO_TESTS_RE`].
fn nonzero_test_marker(output: &str) -> Option<String> {
    NONZERO_TESTS_RE
        .find(output)
        .map(|m| m.as_str().to_string())
}

/// X-verify-2: why a set of PASSED checks does not count as real coverage, if
/// any reason applies. Checked in order: (1) every check that ran was a
/// syntax-only smoke ([`Check::is_smoke`]) — the code was never executed at
/// all; (2) the combined output of the checks that actually ran contains a
/// recognized "zero tests" marker ([`ZERO_TESTS_RE`]) AND no counter-marker
/// ([`NONZERO_TESTS_RE`]) proving some other target in the same output did run
/// tests — a real test command that nonetheless collected/ran nothing. `None`
/// when neither applies: a genuine pass.
fn vacuous_pass_reason_for_checks(checks: &[Check], outputs: &[String]) -> Option<String> {
    if !checks.is_empty() && checks.iter().all(|check| check.is_smoke) {
        return Some(
            "only a syntax-only smoke check ran (e.g. `py_compile`/`node --check`) — the code \
             was never executed, so this is not a test pass"
                .to_string(),
        );
    }
    let combined = outputs.join("\n");
    // Y10a: a non-zero pass count anywhere in the same output means tests DID
    // run, so any "0 passed" beside it belongs to a sibling target (cargo's
    // doc-tests) and this pass is not vacuous. See [`NONZERO_TESTS_RE`].
    if nonzero_test_marker(&combined).is_some() {
        return None;
    }
    zero_test_marker(&combined)
        .map(|marker| format!("the check's own output reports zero tests ran (\"{marker}\")"))
}

fn pass_result(checks: &[Check], outputs: &[String]) -> CallToolResult {
    let labels: Vec<&str> = checks.iter().map(|check| check.label.as_str()).collect();
    let message = format!("PASS - all checks passed: {}.", labels.join(", "));
    let vacuous_reason = vacuous_pass_reason_for_checks(checks, outputs);
    if vacuous_reason.is_some() {
        crate::receipts::record("verification_integrity", "vacuous_passes");
    }
    observation(
        CallToolResult::success(vec![Content::text(message.clone()).with_priority(0.0)]),
        Some(&labels.join(", ")),
        Some("pass"),
        Some(&message),
        vacuous_reason.as_deref(),
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
        None,
    )
}

/// A check failed because the interpreter/runtime it ran under could not
/// IMPORT a dependency (see [`missing_import_dependency`]) — reported as its
/// own ENVIRONMENT class, distinct from [`fail_result`]'s generic "FAIL", so
/// the model does not spend its bounded fix-and-reverify attempts editing
/// application code for a gap that editing code cannot close. Still
/// `CallToolResult::error` (so `is_error` is set and the existing runaway-loop
/// guard still bounds a genuinely stuck run) — only the message and the
/// `verdict` tag change.
fn environment_dependency_result(
    check: &Check,
    exit_code: Option<i32>,
    output: &str,
    missing: &str,
) -> CallToolResult {
    let code = match exit_code {
        Some(code) => code.to_string(),
        None => "killed".to_string(),
    };
    let body = clamp_output(&normalize(output));
    let message = format!(
        "ENVIRONMENT - `{label}` failed (exit {code}), but not because of a defect in the \
         code under test: the interpreter/runtime it ran under could not import `{missing}`.\n\n\
         {body}\n\n\
         This is a missing dependency for THIS interpreter/runtime, not a bug to fix by \
         editing application code — a machine can have more than one interpreter (e.g. \
         several `python3`s) where only some have `{missing}` installed. To resolve it: \
         install `{missing}` for the interpreter this check actually used (e.g. \
         `python3 -m pip install {missing}`, `npm install {missing}`, or the project's own \
         package manager), point the project at its existing virtualenv/lockfile if it has \
         one, or pass `verify` an explicit `command` that runs the check with an interpreter \
         that already has `{missing}`. Do not add or change application code to work around \
         a missing test dependency. Then run verify again.",
        label = check.label,
    );
    observation(
        CallToolResult::error(vec![Content::text(message).with_priority(0.0)]),
        Some(&check.label),
        Some("environment"),
        Some(&body),
        None,
    )
}

fn error_result(message: &str) -> CallToolResult {
    observation(
        CallToolResult::error(vec![Content::text(message.to_string()).with_priority(0.0)]),
        None,
        Some("error"),
        Some(message),
        None,
    )
}

/// R21c: the verdict [`no_checks_result`] reports — deliberately NEITHER
/// `"pass"` NOR `"fail"`. Zero checks ran, so there is nothing to grade;
/// claiming `"pass"` (as this used to) was itself the defect (M23 dschat: a
/// fizzbuzz-shaped goal with no detectable test suite got a `PASS` from
/// verify and a `vacuous_passes` count against it — verify CLAIMED a pass
/// with nothing run, when the honest answer was that nothing COULD run).
/// `is_authoritative_pass` reads only `verdict == "pass"`, so this verdict is
/// never mistaken for one — no consumer needs to special-case it.
pub const NO_CHECKS_VERDICT: &str = "no_checks";

/// A "no checks configured" outcome: the project type was recognized but has
/// nothing runnable to verify yet (a fresh scaffold, a placeholder-only test,
/// or pytest collecting nothing) — and no acceptance-criteria probe could
/// stand in for a real check (see [`synthesize_probe_checks`]). This is NOT a
/// failure, so neither the loop guard nor the model treats it as a phantom
/// failure to chase — but as of R21c it is also, honestly, NOT a pass:
/// [`NO_CHECKS_VERDICT`] reports plainly that zero checks ran, rather than
/// dressing that up as a `"pass"` a consumer might trust. Because
/// `is_authoritative_pass` requires `verdict == "pass"`, this result now
/// automatically reads as "no verify evidence" to every consumer that
/// predicate feeds (`after_turn::scan_verify_positions`,
/// `orchestrator::verify_evidence_in_messages`) — the done-claim path treats
/// it exactly like an unverified edit, which still routes to Review for the
/// cross-family reviewer to inspect, rather than auto-landing on an
/// unearned green.
fn no_checks_result(reason: &str) -> CallToolResult {
    let message = format!(
        "NO CHECKS - {reason} Nothing to verify yet. Add tests or a build/test script and run \
         verify again, or pass an explicit `command` to run a specific check."
    );
    observation(
        CallToolResult::success(vec![Content::text(message.clone()).with_priority(0.0)]),
        None,
        Some(NO_CHECKS_VERDICT),
        Some(&message),
        None,
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
        let result = pass_result(
            &[Check::direct("cargo", &["test"])],
            &["test result: ok. 3 passed; 0 failed; 0 ignored".to_string()],
        );
        let observation: VerificationObservation =
            serde_json::from_value(result.structured_content.unwrap()).unwrap();
        assert_eq!(observation.kind, VERIFICATION_OBSERVATION_KIND);
        assert_eq!(observation.command.as_deref(), Some("cargo test"));
        assert_eq!(observation.verdict.as_deref(), Some("pass"));
        assert!(observation
            .evidence
            .as_deref()
            .is_some_and(|e| e.contains("PASS")));
        assert_eq!(
            observation.vacuous_reason, None,
            "a real pass with 3 tests run must not be flagged vacuous"
        );

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

    // ── X-verify-2: vacuous passes ─────────────────────────────────────────

    /// The gap this closes: a pass whose only check was a syntax-only smoke
    /// (`py_compile`) used to be indistinguishable, on the wire, from a real
    /// test pass — `is_authoritative_pass` (and every consumer that trusted
    /// it, e.g. `after_turn::scan_verify_positions`) would advance past it.
    #[test]
    fn a_smoke_only_pass_is_authoritative_but_flagged_vacuous() {
        let result = pass_result(
            &[Check::direct_owned(
                "python3",
                vec![
                    "-m".to_string(),
                    "py_compile".to_string(),
                    "a.py".to_string(),
                ],
            )
            .smoke_only()],
            &[String::new()],
        );
        assert!(
            is_authoritative_pass(&result),
            "a smoke check still reports a real, structured pass"
        );
        let reason = vacuous_pass_reason(&result);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains("smoke")),
            "expected a smoke-labelled vacuous reason, got {reason:?}"
        );
    }

    /// R21c: `no_checks_result` used to report `verdict: "pass"` with a
    /// vacuous reason attached — an authoritative pass by
    /// `is_authoritative_pass`'s own definition, just one flagged vacuous.
    /// That was the defect (M23 dschat fizzbuzz): verify CLAIMING a pass with
    /// zero checks run. It is now neither a pass nor a fail — see
    /// [`NO_CHECKS_VERDICT`].
    #[test]
    fn a_no_checks_result_honestly_reports_neither_pass_nor_fail() {
        let result = no_checks_result("nothing to run yet.");
        assert!(
            !is_authoritative_pass(&result),
            "zero checks ran — this must never read as an authoritative pass"
        );
        assert!(
            !is_genuine_pass(&result),
            "and therefore never as a genuine one either"
        );
        assert_eq!(
            vacuous_pass_reason(&result),
            None,
            "a vacuous pass is verify CLAIMING a pass with nothing run — NO CHECKS never claims \
             a pass at all, so it must not be counted as one"
        );
        let observation: VerificationObservation =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(observation.verdict.as_deref(), Some(NO_CHECKS_VERDICT));
        assert_eq!(
            observation.vacuous_reason, None,
            "vacuous_reason is defined only for a verdict of pass"
        );
        // NOT a failure either — the loop guard and the model must not chase
        // a phantom failure on a fresh scaffold.
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn a_real_test_run_reporting_zero_tests_is_vacuous() {
        let result = pass_result(
            &[Check::direct("cargo", &["test"])],
            &["test result: ok. 0 passed; 0 failed; 0 ignored".to_string()],
        );
        assert!(is_authoritative_pass(&result));
        let reason = vacuous_pass_reason(&result);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains("zero tests")),
            "expected a zero-tests vacuous reason, got {reason:?}"
        );
    }

    #[test]
    fn a_real_multi_test_run_is_not_vacuous() {
        let result = pass_result(
            &[Check::direct("cargo", &["test"])],
            &["test result: ok. 120 passed; 0 failed; 0 ignored".to_string()],
        );
        assert_eq!(
            vacuous_pass_reason(&result),
            None,
            "'120 passed' must never be misread as '0 passed'"
        );
    }

    // ── Y10a: the multi-target counter-marker ──────────────────────────────

    /// The M15 regression this closes, verbatim from the fx10 worker's own
    /// `cargo test` output: a crate with six real passing tests and an empty
    /// doc-test target. `ZERO_TESTS_RE` matched the doc-test block's
    /// `0 passed`, so verify stamped `vacuous` on a genuine pass, the goal was
    /// held, and fx03/fx10 (and zai fx01) failed on it.
    #[test]
    fn a_cargo_run_with_an_empty_doc_test_target_is_not_vacuous() {
        let output = "\
running 6 tests
test coupons::tests::trims ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

   Doc-tests bench_rs

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        let result = pass_result(&[Check::direct("cargo", &["test"])], &[output.to_string()]);
        assert!(is_authoritative_pass(&result));
        assert_eq!(
            vacuous_pass_reason(&result),
            None,
            "6 real passing tests beside an empty doc-test target is a genuine pass"
        );
        assert!(
            is_genuine_pass(&result),
            "the goal engine must be able to land this verdict"
        );
    }

    /// The other half of the same regression: with nothing but zero-count
    /// blocks, the marker must still fire.
    #[test]
    fn a_cargo_run_with_only_empty_targets_is_still_vacuous() {
        let output = "\
running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

   Doc-tests bench_rs

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        let result = pass_result(&[Check::direct("cargo", &["test"])], &[output.to_string()]);
        let reason = vacuous_pass_reason(&result);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains("zero tests")),
            "an all-empty cargo run is still vacuous, got {reason:?}"
        );
    }

    /// Separate checks, separate output strings: the counter-marker is scanned
    /// over the same joined output the zero marker is, so a real `cargo test`
    /// beside a clippy run that happens to print `0 passed` still counts.
    #[test]
    fn the_counter_marker_is_scanned_across_all_check_outputs() {
        let result = pass_result(
            &[
                Check::direct("cargo", &["test"]),
                Check::direct("cargo", &["test", "--doc"]),
            ],
            &[
                "test result: ok. 6 passed; 0 failed; 0 ignored".to_string(),
                "test result: ok. 0 passed; 0 failed; 0 ignored".to_string(),
            ],
        );
        assert_eq!(vacuous_pass_reason(&result), None);
    }

    /// pytest and jest keep their existing behaviour in both directions: a
    /// genuine run is not vacuous, an empty collection still is.
    #[test]
    fn pytest_and_jest_equivalents_are_unchanged_by_the_counter_marker() {
        for genuine in [
            "===== 12 passed, 1 skipped in 0.42s =====",
            "Tests:       3 passed, 3 total\nSnapshots:   0 total",
        ] {
            let result = pass_result(&[Check::direct("pytest", &["-q"])], &[genuine.to_string()]);
            assert_eq!(
                vacuous_pass_reason(&result),
                None,
                "a genuine run must not be vacuous: {genuine:?}"
            );
        }
        for empty in [
            "collected 0 items\n\n===== no tests ran in 0.01s =====",
            "No tests found, exiting with code 0\nNo test files",
            "===== 0 passed in 0.01s =====",
        ] {
            let result = pass_result(&[Check::direct("pytest", &["-q"])], &[empty.to_string()]);
            assert!(
                vacuous_pass_reason(&result).is_some(),
                "an empty run must stay vacuous: {empty:?}"
            );
        }
    }

    #[test]
    fn nonzero_marker_matches_only_real_counts() {
        for text in [
            "test result: ok. 6 passed; 0 failed",
            "running 6 tests",
            "running 1 test",
            "12 passed, 1 skipped",
        ] {
            assert!(
                nonzero_test_marker(text).is_some(),
                "expected a counter-marker in {text:?}"
            );
        }
        for text in [
            "test result: ok. 0 passed; 0 failed",
            "running 0 tests",
            "collected 0 items",
            "no tests ran",
        ] {
            assert!(
                nonzero_test_marker(text).is_none(),
                "must not claim tests ran in {text:?}"
            );
        }
    }

    #[test]
    fn a_failing_result_is_never_reported_as_vacuous() {
        // Vacuousness is a property of a PASS; a failure is handled by the
        // ordinary verify-loop guard and must not also be flagged vacuous.
        let failed = fail_result(&Check::direct("cargo", &["test"]), Some(1), "0 passed");
        assert!(!is_authoritative_pass(&failed));
        assert_eq!(vacuous_pass_reason(&failed), None);
    }

    // ── G5 finding 1: the shared genuine-pass predicate ─────────────────────

    #[test]
    fn is_genuine_pass_rejects_a_smoke_only_pass() {
        let result = pass_result(
            &[Check::direct_owned(
                "python3",
                vec![
                    "-m".to_string(),
                    "py_compile".to_string(),
                    "a.py".to_string(),
                ],
            )
            .smoke_only()],
            &[String::new()],
        );
        assert!(
            is_authoritative_pass(&result),
            "the underlying wire result is still an authoritative pass"
        );
        assert!(
            !is_genuine_pass(&result),
            "a smoke-only pass must never read as a genuine, actionable pass"
        );
    }

    #[test]
    fn is_genuine_pass_rejects_a_zero_tests_pass() {
        let result = pass_result(
            &[Check::direct("cargo", &["test"])],
            &["test result: ok. 0 passed; 0 failed; 0 ignored".to_string()],
        );
        assert!(!is_genuine_pass(&result));
    }

    #[test]
    fn is_genuine_pass_accepts_a_real_multi_test_pass() {
        let result = pass_result(
            &[Check::direct("cargo", &["test"])],
            &["test result: ok. 120 passed; 0 failed; 0 ignored".to_string()],
        );
        assert!(is_genuine_pass(&result));
    }

    #[test]
    fn is_genuine_pass_rejects_a_failure() {
        let failed = fail_result(&Check::direct("cargo", &["test"]), Some(1), "boom");
        assert!(!is_genuine_pass(&failed));
    }

    #[test]
    fn zero_tests_marker_matches_common_runner_phrasings() {
        for text in [
            "test result: ok. 0 passed; 0 failed; 0 ignored",
            "No tests ran in 0.01s",
            "no test files",
            "Ran 0 tests in 0.000s",
            "collected 0 items",
            "0 examples, 0 failures",
        ] {
            assert!(
                zero_test_marker(text).is_some(),
                "expected a match in {text:?}"
            );
        }
    }

    #[test]
    fn zero_tests_marker_does_not_false_positive_on_a_larger_count() {
        for text in [
            "test result: ok. 120 passed; 0 failed",
            "20 passed, 1 failed",
            "ran 10 tests",
        ] {
            assert!(
                zero_test_marker(text).is_none(),
                "must not match a non-zero count in {text:?}"
            );
        }
    }

    // ── Y10e: a committed goal branch still has a change set ───────────────

    /// `git -C dir <args>`, panicking with the real stderr so a fixture that
    /// fails to build says why.
    fn git_y10e(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git must be on PATH for this test");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A repo seeded exactly like a bench fixture: one commit on `main`, then
    /// a `goal/<session>` branch carrying the worker's committed change and a
    /// CLEAN working tree — the state every M15 worker was in when it called
    /// `verify` right after committing.
    fn y10e_committed_goal_branch() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize: `git rev-parse --show-toplevel` resolves symlinks
        // (/var → /private/var on macOS) and `git_changes_under` compares the
        // two, so the fixture must hand back the resolved path.
        let dir = tmp.path().canonicalize().unwrap();
        let dir = dir.as_path();
        git_y10e(dir, &["init", "-b", "main"]);
        git_y10e(dir, &["config", "user.email", "t@example.com"]);
        git_y10e(dir, &["config", "user.name", "T"]);
        git_y10e(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"fx\"\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "seed"]);

        git_y10e(dir, &["checkout", "-b", "goal/session-1"]);
        std::fs::write(dir.join("src/coupons.rs"), "pub fn trim() {}\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "worker: add coupons"]);
        let resolved = dir.to_path_buf();
        (tmp, resolved)
    }

    /// The M15 regression: with the work COMMITTED the working tree is clean,
    /// so `diff HEAD` + `ls-files --others` return nothing and verify answered
    /// "no changed files were found under this verification path". fx05, fx08,
    /// fx09 and fx10 each burned a turn on it. The branch's fork point still
    /// knows what this goal changed.
    #[test]
    fn a_committed_goal_branch_still_reports_its_changed_files() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        let dir = dir.as_path();

        assert_eq!(
            git_y10e(dir, &["status", "--porcelain"]),
            "",
            "the fixture's working tree must be clean, or it proves nothing"
        );

        let changes = git_changes_under(dir).expect("the repo resolves").paths;
        assert_eq!(
            changes,
            vec!["src/coupons.rs".to_string()],
            "a committed goal branch must still describe what it changed"
        );
    }

    /// The narrowing that follows: because the change set is no longer empty,
    /// `select_checks` infers a real scoped Rust check instead of NO CHECKS.
    #[test]
    fn a_committed_goal_branch_infers_a_real_check_instead_of_no_checks() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        let outcome = select_checks(
            dir.as_path(),
            VerificationScope::Auto,
            VerificationRisk::Low,
            None,
        )
        .expect("auto scope must not error on an all-Rust change set");
        match outcome {
            DetectOutcome::Checks(checks) => assert!(
                checks.iter().any(|check| check.label.starts_with("cargo ")),
                "expected a Cargo check, got {:?}",
                checks.iter().map(|c| &c.label).collect::<Vec<_>>()
            ),
            other => panic!("expected runnable checks, got {other:?}"),
        }
    }

    #[test]
    fn select_checks_uses_fork_point_with_only_untracked_non_rust_files() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        std::fs::write(dir.join("runner.sh"), "exit 0\n").unwrap();
        std::fs::create_dir(dir.join("scratch")).unwrap();
        std::fs::write(dir.join("scratch/result.json"), "{}\n").unwrap();
        assert_eq!(
            git_changes_under(&dir).unwrap().paths,
            vec!["runner.sh".to_string(), "scratch/result.json".to_string()]
        );
        assert_eq!(
            select_checks(&dir, VerificationScope::Auto, VerificationRisk::Low, None).unwrap(),
            DetectOutcome::Checks(vec![Check::direct("cargo", &["test", "-p", "fx"])])
        );
        // Explicit caller scope remains authoritative even on this branch.
        let supplied = ["runner.sh".to_string()];
        assert!(matches!(
            select_checks(
                &dir,
                VerificationScope::Auto,
                VerificationRisk::Low,
                Some(&supplied),
            )
            .unwrap(),
            DetectOutcome::NoChecksConfigured(_)
        ));
    }

    #[test]
    fn select_checks_uses_fork_point_with_docs_and_script_only() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        std::fs::write(dir.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        git_y10e(&dir, &["add", "src/lib.rs"]);
        git_y10e(&dir, &["commit", "-m", "worker: update lib"]);
        std::fs::write(dir.join("README.md"), "# Fixture\n").unwrap();
        std::fs::write(dir.join("oracle_check.sh"), "exit 0\n").unwrap();
        assert_eq!(
            git_changes_under(&dir).unwrap().paths,
            vec!["README.md".to_string(), "oracle_check.sh".to_string()]
        );
        assert_eq!(
            select_checks(&dir, VerificationScope::Auto, VerificationRisk::Low, None).unwrap(),
            DetectOutcome::Checks(vec![Check::direct("cargo", &["test", "-p", "fx"])])
        );
    }

    #[test]
    fn git_changes_retains_subdirectory_context_for_fork_point() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        std::fs::write(dir.join("src/oracle_check.sh"), "exit 0\n").unwrap();
        let changes = git_changes_under(&dir.join("src")).unwrap();
        assert_eq!(changes.root, dir);
        assert_eq!(changes.relative_dir, PathBuf::from("src"));
        assert_eq!(changes.paths, vec!["src/oracle_check.sh".to_string()]);
        assert_eq!(
            changes_since_branch_fork_point(&changes.root, &changes.relative_dir),
            Some(vec!["src/coupons.rs".to_string()])
        );
    }

    #[test]
    fn select_checks_keeps_untracked_new_rust_source() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        std::fs::write(dir.join("src/new.rs"), "pub fn new() {}\n").unwrap();
        assert_eq!(
            git_changes_under(&dir).unwrap().paths,
            vec!["src/new.rs".to_string()]
        );
        assert_eq!(
            select_checks(&dir, VerificationScope::Auto, VerificationRisk::Low, None).unwrap(),
            DetectOutcome::Checks(vec![Check::direct("cargo", &["test", "-p", "fx"])])
        );
    }

    #[test]
    fn select_checks_with_no_changes_anywhere_still_has_no_checks() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        git_y10e(&dir, &["checkout", "main"]);
        assert_eq!(git_changes_under(&dir).unwrap().paths, Vec::<String>::new());
        assert!(matches!(
            select_checks(&dir, VerificationScope::Auto, VerificationRisk::Low, None).unwrap(),
            DetectOutcome::NoChecksConfigured(reason) if reason.contains("no changed files")
        ));
        std::fs::write(dir.join("runner.sh"), "exit 0\n").unwrap();
        assert!(matches!(
            select_checks(&dir, VerificationScope::Auto, VerificationRisk::Low, None).unwrap(),
            DetectOutcome::NoChecksConfigured(reason) if reason.contains("not Rust")
        ));
    }

    /// The fallback must stay a FALLBACK: while there is uncommitted work, the
    /// working tree is still the answer — widening to the fork point there
    /// would silently broaden every ordinary verify call.
    #[test]
    fn uncommitted_work_still_wins_over_the_fork_point() {
        let (_tmp, dir) = y10e_committed_goal_branch();
        let dir = dir.as_path();
        std::fs::write(dir.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();

        let changes = git_changes_under(dir).expect("the repo resolves").paths;
        assert_eq!(
            changes,
            vec!["src/lib.rs".to_string()],
            "with a dirty tree the working-tree scan must be untouched by Y10e"
        );
    }

    /// A branch with nothing unique to it (the trunk itself, clean) has no
    /// fork point to widen to, and must keep answering "nothing changed"
    /// rather than diffing its whole history.
    #[test]
    fn a_clean_trunk_has_no_fork_point_to_widen_to() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let dir = dir.as_path();
        git_y10e(dir, &["init", "-b", "main"]);
        git_y10e(dir, &["config", "user.email", "t@example.com"]);
        git_y10e(dir, &["config", "user.name", "T"]);
        git_y10e(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.rs"), "// seed\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "seed"]);

        assert!(
            branch_fork_point(dir).is_none(),
            "a single root commit has no parent to diff against"
        );
        assert_eq!(git_changes_under(dir).unwrap().paths, Vec::<String>::new());
    }

    /// R54a (A51 analysis, "branch_fork_point"): the A51 Run 2 defect,
    /// reproduced directly. A goal redispatched twice leaves BOTH prior
    /// attempts' `goal/<session>` branches alive (Y16b never deletes them),
    /// each a parent of the next; the final attempt's only new commit is an
    /// empty checkpoint. Before this lane, `--branches` counted
    /// `goal/s1`/`goal/s2` as "already covering" their own commits, so the
    /// verified diff narrowed to just the empty checkpoint (`git_changes_
    /// under` reporting no changed files at all). After it, the fork point
    /// is measured against trunk (`main`) via `merge-base`, so the diff
    /// spans every commit since trunk — both real attempts' work, not just
    /// the last one's no-op commit.
    #[test]
    fn fork_point_spans_every_commit_since_trunk_despite_live_sibling_attempt_branches() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let dir = dir.as_path();
        git_y10e(dir, &["init", "-b", "main"]);
        git_y10e(dir, &["config", "user.email", "t@example.com"]);
        git_y10e(dir, &["config", "user.name", "T"]);
        git_y10e(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.rs"), "// seed\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "seed"]);
        let seed = git_y10e(dir, &["rev-parse", "HEAD"]).trim().to_string();

        // Attempt 1, still alive as `goal/s1`.
        git_y10e(dir, &["checkout", "-q", "-b", "goal/s1"]);
        std::fs::write(dir.join("b.rs"), "pub fn one() {}\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "attempt 1"]);

        // Attempt 2, redispatched from attempt 1's tip, still alive as `goal/s2`.
        git_y10e(dir, &["checkout", "-q", "-b", "goal/s2"]);
        std::fs::write(dir.join("c.rs"), "pub fn two() {}\n").unwrap();
        git_y10e(dir, &["add", "-A"]);
        git_y10e(dir, &["commit", "-m", "attempt 2"]);

        // Attempt 3, the current goal worktree: an empty checkpoint commit
        // on top of real earlier progress.
        git_y10e(dir, &["checkout", "-q", "-b", "goal/s3"]);
        git_y10e(dir, &["commit", "--allow-empty", "-m", "checkpoint"]);

        assert_eq!(
            branch_fork_point(dir).as_deref(),
            Some(seed.as_str()),
            "the fork point must be trunk-relative (merge-base with main), \
             not narrowed by goal/s1 and goal/s2 still existing"
        );
        let mut changed = changes_since_branch_fork_point(dir, Path::new("")).unwrap();
        changed.sort();
        assert_eq!(
            changed,
            vec!["b.rs".to_string(), "c.rs".to_string()],
            "the verified diff must span every commit since trunk, not just \
             attempt 3's empty checkpoint"
        );
        assert_eq!(
            git_changes_under(dir).unwrap().paths,
            vec!["b.rs".to_string(), "c.rs".to_string()],
            "select_checks must see both attempts' real work, never NO CHECKS"
        );
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
    fn detect_check_labels_matches_detect_checks_for_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        assert_eq!(detect_check_labels(dir.path()), vec!["cargo test"]);
    }

    #[test]
    fn detect_check_labels_is_empty_for_an_undetectable_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_check_labels(dir.path()).is_empty());
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
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
    fn detects_python_pytest_only_when_a_pytest_marker_also_exists() {
        // A Python marker on its own is not evidence of pytest (see
        // `python_marker_alone_does_not_propose_pytest` below) — pytest is only
        // proposed once the project also declares it one of the three ways.
        for (python_marker, pytest_marker, pytest_content) in [
            ("requirements.txt", "pytest.ini", ""),
            ("setup.py", "conftest.py", ""),
            ("tox.ini", "pyproject.toml", "[tool.pytest.ini_options]\n"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(python_marker), "").unwrap();
            std::fs::write(dir.path().join(pytest_marker), pytest_content).unwrap();
            assert_eq!(
                checks_of(dir.path()),
                vec![Check::direct("pytest", &[])],
                "marker {python_marker} + {pytest_marker} should map to pytest"
            );
        }
    }

    /// F3(d): `requirements.txt`/`setup.py`/etc. establish "this is a Python
    /// project", not "this project uses pytest" — plenty of Python projects
    /// use `unittest`, a bespoke runner, or nothing yet. Confidently running
    /// pytest anyway is how a bare-assert test module with a `sys.exit()` at
    /// import time ends in an INTERNALERROR instead of an actionable result
    /// (see `HARNESS_LOOP_LOG.md`'s `harness-testfix-inventory-suite-green`
    /// evidence). Before this fix every one of these markers alone mapped
    /// straight to `pytest`.
    #[test]
    fn python_marker_alone_does_not_propose_pytest() {
        // Some of these markers (`setup.py`) are ALSO the only `.py` file in
        // the directory, so the manifestless single-file fallback (b) can
        // legitimately still return a `py_compile` syntax-smoke check — the
        // invariant under test is narrower than "always errors": pytest must
        // never be the answer without real evidence it is configured.
        for marker in ["pyproject.toml", "setup.py", "requirements.txt", "tox.ini"] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(marker), "").unwrap();
            match detect_checks(dir.path()) {
                Err(error) => assert!(
                    !error.to_ascii_lowercase().contains("pytest"),
                    "marker {marker} alone must not propose pytest, got: {error}"
                ),
                Ok(DetectOutcome::Checks(checks)) => assert!(
                    !checks.iter().any(|c| c.label.contains("pytest")),
                    "marker {marker} alone must not propose pytest, got: {checks:?}"
                ),
                Ok(other) => panic!("unexpected outcome for marker {marker}: {other:?}"),
            }
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

    // ── F3: manifestless shapes (no Cargo/go/npm/Python/Makefile marker) ──
    //
    // Live evidence (HARNESS_LOOP_LOG.md, Lane A2): a lone `.py` script, a
    // vanilla `.mjs`+`.html` workspace, and a `tests/test_inventory.py` with no
    // package manifest at all each wasted a turn on "Could not detect the
    // project type ... Pass an explicit `command`" before this fix.

    #[test]
    fn manifestless_test_file_in_a_tests_dir_runs_directly() {
        let dir = tempfile::tempdir().unwrap();
        let tests_dir = dir.path().join("tests");
        std::fs::create_dir(&tests_dir).unwrap();
        std::fs::write(tests_dir.join("test_inventory.py"), "assert 1 + 1 == 2\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("python3", &["tests/test_inventory.py"])]
        );
    }

    #[test]
    fn manifestless_test_file_at_the_root_runs_directly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test_inventory.py"), "assert True\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("python3", &["test_inventory.py"])]
        );
    }

    #[test]
    fn manifestless_check_dot_star_file_runs_directly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("check.py"), "assert True\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("python3", &["check.py"])]
        );
    }

    #[test]
    fn manifestless_test_mjs_runs_directly_with_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.mjs"), "console.log('ok');\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("node", &["test.mjs"])]
        );
    }

    #[test]
    fn manifestless_dot_test_dot_file_runs_directly_with_node() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("logfilter.test.js"), "console.log('ok');\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("node", &["logfilter.test.js"])]
        );
    }

    #[test]
    fn manifestless_single_py_file_gets_an_honestly_named_syntax_smoke() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fizzbuzz.py"), "def fizzbuzz(): pass\n").unwrap();
        let checks = checks_of(dir.path());
        assert_eq!(
            checks,
            vec![Check::direct("python3", &["-m", "py_compile", "fizzbuzz.py"]).smoke_only()]
        );
        // Honest about what it is: a syntax check, never described as a test run.
        assert!(checks[0].label.contains("py_compile"));
        // X-verify-2: and never SCORED as one either.
        assert!(checks[0].is_smoke);
    }

    #[test]
    fn manifestless_single_mjs_file_gets_a_node_check() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.mjs"), "export const x = 1;\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("node", &["--check", "app.mjs"]).smoke_only()]
        );
    }

    #[test]
    fn manifestless_single_js_file_gets_a_node_check() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.js"), "const x = 1;\n").unwrap();
        assert_eq!(
            checks_of(dir.path()),
            vec![Check::direct("node", &["--check", "app.js"]).smoke_only()]
        );
    }

    #[test]
    fn manifestless_shapes_never_propose_pytest_for_a_bare_assert_script() {
        // The exact live crash this guards against: `python3 -m pytest` on a
        // plain-assert test file with a `sys.exit()` at import time produced a
        // 40-line INTERNALERROR. The manifestless path must never choose a
        // framework — only ever a direct interpreter invocation.
        let dir = tempfile::tempdir().unwrap();
        let tests_dir = dir.path().join("tests");
        std::fs::create_dir(&tests_dir).unwrap();
        std::fs::write(
            tests_dir.join("test_inventory.py"),
            "import sys\nassert 1 == 1\nsys.exit(0)\n",
        )
        .unwrap();
        let checks = checks_of(dir.path());
        assert_eq!(checks.len(), 1);
        assert!(
            !checks[0].label.contains("pytest"),
            "must never propose pytest for a manifestless test file, got: {:?}",
            checks[0]
        );
    }

    #[test]
    fn ambiguous_manifestless_shape_with_multiple_py_files_is_still_not_detected() {
        // More than one lone `.py` file with none of them test-named is not a
        // safe single-file guess — this must fall through to the same
        // actionable error as before, not pick one arbitrarily.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
        std::fs::write(dir.path().join("b.py"), "y = 2\n").unwrap();
        assert!(detect_checks(dir.path()).is_err());
    }

    /// Regression guard for the F3 change: a real manifest still wins outright
    /// and is never diverted into the new manifestless fallback, even when the
    /// manifestless heuristics would also technically match something in the
    /// same directory (e.g. a `.py` file living next to a `Cargo.toml`).
    #[test]
    fn existing_manifest_detections_are_unchanged_by_the_manifestless_fallback() {
        let cargo_dir = tempfile::tempdir().unwrap();
        std::fs::write(cargo_dir.path().join("Cargo.toml"), "[package]\nname=\"x\"").unwrap();
        std::fs::write(cargo_dir.path().join("stray.py"), "x = 1\n").unwrap();
        assert_eq!(
            checks_of(cargo_dir.path()),
            vec![Check::direct("cargo", &["test"])]
        );

        let go_dir = tempfile::tempdir().unwrap();
        std::fs::write(go_dir.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(
            checks_of(go_dir.path()),
            vec![
                Check::direct("go", &["build", "./..."]),
                Check::direct("go", &["test", "./..."]),
            ]
        );

        let node_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            node_dir.path().join("package.json"),
            r#"{"scripts":{"test":"jest"}}"#,
        )
        .unwrap();
        assert_eq!(
            checks_of(node_dir.path()),
            vec![Check::direct("npm", &["run", "test"])]
        );

        let make_dir = tempfile::tempdir().unwrap();
        std::fs::write(make_dir.path().join("Makefile"), "test:\n\techo hi\n").unwrap();
        assert_eq!(
            checks_of(make_dir.path()),
            vec![Check::direct("make", &["test"])]
        );
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
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

    // ── Missing-dependency environment classification (R18h) ──

    #[test]
    fn missing_import_dependency_catches_python_dash_m_style() {
        // `python3 -m X` when X isn't importable: no exception, no quotes,
        // just `<interpreter>: No module named X` — the shape actually seen
        // reproducing harness-cli-argparse-log-filter's persistent failure.
        let out = "/opt/homebrew/opt/python@3.14/bin/python3.14: No module named pytest";
        assert_eq!(missing_import_dependency(out).as_deref(), Some("pytest"));
    }

    #[test]
    fn missing_import_dependency_catches_python_traceback_style() {
        let out = "Traceback (most recent call last):\n  File \"tests/test_x.py\", line 5, in <module>\n    import pytest\nModuleNotFoundError: No module named 'pytest'";
        assert_eq!(missing_import_dependency(out).as_deref(), Some("pytest"));
    }

    #[test]
    fn missing_import_dependency_catches_node_and_ruby() {
        assert_eq!(
            missing_import_dependency(
                "Error: Cannot find module 'lodash'\nRequire stack:\n- /a/b.js"
            )
            .as_deref(),
            Some("lodash")
        );
        assert_eq!(
            missing_import_dependency(
                "x.rb:1:in `require': cannot load such file -- nokogiri (LoadError)"
            )
            .as_deref(),
            Some("nokogiri")
        );
    }

    #[test]
    fn missing_import_dependency_does_not_flag_an_ordinary_test_failure() {
        assert_eq!(
            missing_import_dependency("AssertionError: expected 2, got 3"),
            None
        );
        assert_eq!(
            missing_import_dependency("error[E0425]: cannot find value `x` in this scope"),
            None
        );
    }

    #[test]
    fn environment_dependency_result_labels_the_gap_and_hints_a_fix_not_a_code_edit() {
        let check = Check::direct("pytest", &[]);
        let output = "/opt/homebrew/opt/python@3.14/bin/python3.14: No module named pytest";
        let result = environment_dependency_result(&check, Some(1), output, "pytest");
        assert_eq!(
            result.is_error,
            Some(true),
            "still is_error for the loop guard"
        );
        let text = text_of(&result);
        assert!(
            text.starts_with("ENVIRONMENT"),
            "labeled distinctly from FAIL: {text}"
        );
        assert!(
            text.contains("pip install pytest"),
            "gives a concrete remediation: {text}"
        );
        assert!(
            text.contains("more than one interpreter"),
            "explains the multi-interpreter trap: {text}"
        );
        assert!(
            text.contains("Do not add or change application code"),
            "tells the model not to chase this as a code bug: {text}"
        );
        let observation: VerificationObservation =
            serde_json::from_value(result.structured_content.clone().unwrap()).unwrap();
        assert_eq!(observation.verdict.as_deref(), Some("environment"));
    }

    #[test]
    fn a_check_failure_with_a_missing_import_routes_to_environment_not_fail() {
        // Same failure shape as `fail_result_carries_the_real_errors` above,
        // but with output indicating a missing dependency instead of a real
        // code defect — the dispatch loop in `verify_with_cwd` must not
        // report this as a generic FAIL.
        let output = "/opt/homebrew/opt/python@3.14/bin/python3.14: No module named pytest";
        assert!(missing_import_dependency(output).is_some());
        let check = Check::direct("pytest", &[]);
        let env_result = environment_dependency_result(&check, Some(1), output, "pytest");
        let generic_fail = fail_result(&check, Some(1), output);
        let fail_result_text = text_of(&generic_fail);
        let env_result_text = text_of(&env_result);
        assert_ne!(
            env_result_text, fail_result_text,
            "the environment-classified message must differ from the generic FAIL message"
        );
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
        let result = pass_result(
            &[
                Check::direct("go", &["build", "./..."]),
                Check::direct("go", &["test", "./..."]),
            ],
            &[String::new(), "ok  \tpkg\t0.003s".to_string()],
        );
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
            is_guard_block: false,
            // R52a: this fixture models a plain verify failure, not the
            // AC-validation retry gate.
            is_ac_validation_error: false,
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
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
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.starts_with("FAIL"));
        assert!(text.contains("exit 1"));
        assert!(text.contains("EARLY_MARKER"), "stdout captured");
        assert!(text.contains("LATE_MARKER"), "stderr captured");
    }

    // ── R21c: acceptance-criteria probe checks ──────────────────────────────

    /// `VerifyParams` with only `acceptance_criteria` set — every other field
    /// at its auto-detect default, mirroring the plain literals above.
    fn params_with_criteria(criteria: &[&str]) -> VerifyParams {
        VerifyParams {
            command: None,
            path: None,
            timeout_secs: Some(30),
            scope: None,
            risk: None,
            changed_paths: None,
            acceptance_criteria: Some(criteria.iter().map(|s| s.to_string()).collect()),
        }
    }

    /// An empty directory with no language marker at all
    /// (`detect_probe_language` reports `Other`) — fine for every probe
    /// shape that doesn't consult the language gate.
    fn no_language_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// A directory `detect_probe_language` reports as `Python`: a bare
    /// top-level `.py` file, the exact manifestless shape A32 root cause 2
    /// motivated `call_returns_probe` for.
    fn python_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def fizzbuzz(n):\n    return n\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn file_exists_criterion_compiles_to_a_probe() {
        let dir = no_language_dir();
        let check = probe_for_criterion(dir.path(), "fizzbuzz.py exists").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::FileExists("fizzbuzz.py".to_string()))
        );
        assert!(!check.is_smoke);
    }

    #[test]
    fn file_exists_criterion_strips_the_unverified_marker() {
        let dir = no_language_dir();
        let marked = format!("{UNVERIFIED_MARKER_PREFIX}fizzbuzz.py exists");
        let check = probe_for_criterion(dir.path(), &marked).unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::FileExists("fizzbuzz.py".to_string()))
        );
    }

    #[test]
    fn function_definition_criterion_compiles_to_a_probe() {
        let dir = no_language_dir();
        let check = probe_for_criterion(dir.path(), "defines a function named fizzbuzz").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::SymbolDefined {
                symbol: "fizzbuzz".to_string(),
                kind: SymbolKind::Function,
                file: None,
            })
        );
    }

    #[test]
    fn class_definition_criterion_compiles_to_a_probe() {
        let dir = no_language_dir();
        let check = probe_for_criterion(dir.path(), "class FizzBuzzer is defined").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::SymbolDefined {
                symbol: "FizzBuzzer".to_string(),
                kind: SymbolKind::Class,
                file: None,
            })
        );
    }

    #[test]
    fn command_exit_criterion_compiles_to_a_probe() {
        let dir = no_language_dir();
        let check = probe_for_criterion(dir.path(), "`pytest -k smoke` exits 0").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::CommandExitsZero("pytest -k smoke".to_string()))
        );
    }

    // ── R35b: call/return and run/stdout-contains probe shapes ──────────────
    // ── R54a: language-gated — see the `call_returns_probe`/
    //    `node_call_returns_probe` tests below for the JS/TS/Python split ──

    /// A32 root cause 2's exact motivating text: no backticks, plain
    /// "returns", a quoted string value — this is the specific freeform
    /// criterion that used to fall through to a vacuous smoke-only PASS.
    /// R54a: only synthesizes for a repo `detect_probe_language` reports as
    /// Python — probe as before, now on an explicitly Python-shaped dir.
    #[test]
    fn call_returns_criterion_bare_compiles_to_a_probe() {
        let dir = python_dir();
        let check = probe_for_criterion(dir.path(), "fizzbuzz(3) returns 'Fizz'").unwrap();
        assert!(!check.is_smoke);
        let Exec::Probe(ProbeKind::CommandExitsZero(command)) = &check.exec else {
            panic!("expected a CommandExitsZero probe, got {:?}", check.exec);
        };
        assert!(command.starts_with("python3 -c "), "got: {command}");
        assert!(command.contains("fizzbuzz(3)"), "got: {command}");
        assert!(command.contains("Fizz"), "got: {command}");
        assert_eq!(
            check.label,
            "acceptance criterion: `fizzbuzz(3)` returns `'Fizz'`"
        );
    }

    /// The backtick-delimited "==" phrasing, and an unquoted bare-word value
    /// (the common unquoted-string shape, e.g. "returns FizzBuzz") which
    /// `python_literal_for_value` must wrap into a valid Python string
    /// literal rather than treat as a bare (undefined) identifier.
    #[test]
    fn call_returns_criterion_with_backticks_and_bare_value_compiles_to_a_probe() {
        let dir = python_dir();
        let check = probe_for_criterion(dir.path(), "`fizzbuzz(15)` == `FizzBuzz`").unwrap();
        let Exec::Probe(ProbeKind::CommandExitsZero(command)) = &check.exec else {
            panic!("expected a CommandExitsZero probe, got {:?}", check.exec);
        };
        assert!(command.contains("fizzbuzz(15)"), "got: {command}");
        assert!(command.contains(r#""FizzBuzz""#), "got: {command}");
    }

    /// R54a (A51 analysis, "call_returns_probe"): the exact defect — a
    /// TypeScript-only repo with the same call/return criterion text must
    /// NEVER get a Python probe compiled against it (a guaranteed
    /// `NameError`, for any model, forever). The named function is placed at
    /// the repo's top level (in `find_symbol_definition`'s scan scope) and
    /// genuinely defined there, so this pins the REAL reason no probe is
    /// synthesized: the `.ts` source can't be `require()`d by a bare `node
    /// -e` without a compiler (see `node_call_returns_probe`'s own doc
    /// comment) — not merely that the file wasn't found. The criterion falls
    /// through exactly as it did before any probe grammar existed: marker
    /// intact, left for NO_CHECKS/R43b-c to handle.
    #[test]
    fn typescript_repo_call_returns_criterion_synthesizes_no_python_probe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        std::fs::write(
            dir.path().join("pricing.ts"),
            "export function computeGoldDiscount(cents: number): number {\n  \
             return Math.min(Math.round(cents * 0.2), 5000);\n}\n",
        )
        .unwrap();
        assert_eq!(detect_probe_language(dir.path()), ProbeLanguage::JavaScript);
        assert!(
            find_symbol_definition(dir.path(), "computeGoldDiscount", SymbolKind::Function)
                .is_some(),
            "the function must be genuinely locatable, so the TS extension is what blocks it"
        );
        let marked =
            format!("{UNVERIFIED_MARKER_PREFIX}computeGoldDiscount(100_000) returns 20_000");
        assert!(
            probe_for_criterion(dir.path(), &marked).is_none(),
            "a TS-only repo must never get a wrong-language probe compiled for it"
        );
    }

    /// R54a: the JS counterpart — a repo `detect_probe_language` reports as
    /// JavaScript, with the named function actually defined (and locatable
    /// via `find_symbol_definition`) in a plain, requireable `.js` file,
    /// synthesizes a real `node -e` probe instead of leaving the criterion
    /// unverified.
    #[test]
    fn javascript_repo_call_returns_criterion_compiles_to_a_node_probe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"fx"}"#).unwrap();
        std::fs::write(
            dir.path().join("math.js"),
            "function computeDouble(n) {\n  return n * 2;\n}\n\
             module.exports = { computeDouble };\n",
        )
        .unwrap();
        assert_eq!(detect_probe_language(dir.path()), ProbeLanguage::JavaScript);
        let check = probe_for_criterion(dir.path(), "computeDouble(21) returns 42").unwrap();
        assert!(!check.is_smoke);
        let Exec::Probe(ProbeKind::CommandExitsZero(command)) = &check.exec else {
            panic!("expected a CommandExitsZero probe, got {:?}", check.exec);
        };
        assert!(command.starts_with("node -e "), "got: {command}");
        assert!(command.contains("computeDouble(21)"), "got: {command}");
        assert!(command.contains("require("), "got: {command}");
        assert!(command.contains("math.js"), "got: {command}");
        assert!(command.contains("42"), "got: {command}");
        assert_eq!(
            check.label,
            "acceptance criterion: `computeDouble(21)` returns `42`"
        );
    }

    /// The closed-set contract, narrowed to this shape: text that merely
    /// LOOKS call-shaped but names no return/equality phrasing must never be
    /// approximated into a probe.
    #[test]
    fn call_returns_criterion_without_a_return_phrase_synthesizes_no_probe() {
        let dir = python_dir();
        assert!(probe_for_criterion(dir.path(), "fizzbuzz(3) is nice").is_none());
        assert!(probe_for_criterion(dir.path(), "the fizzbuzz function is well tested").is_none());
    }

    /// "running `<cmd>` prints/outputs `<text>`" compiles to a real
    /// `CommandExitsZero` probe that pipes the command's own output through
    /// `grep -qF` for the expected text.
    #[test]
    fn run_stdout_criterion_compiles_to_a_probe() {
        let dir = no_language_dir();
        let check = probe_for_criterion(
            dir.path(),
            "running `python3 main.py` prints `Hello, world!`",
        )
        .unwrap();
        assert!(!check.is_smoke);
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::CommandExitsZero(
                "python3 main.py | grep -qF -- 'Hello, world!'".to_string()
            ))
        );
        assert_eq!(
            check.label,
            "acceptance criterion: `python3 main.py` prints `Hello, world!`"
        );

        let check =
            probe_for_criterion(dir.path(), "running `node app.js` outputs `ready`").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::CommandExitsZero(
                "node app.js | grep -qF -- 'ready'".to_string()
            ))
        );
    }

    /// The closed-set contract, narrowed to this shape: prose that describes
    /// running something without the backtick-delimited `<cmd>`/`<text>`
    /// shape is left honestly unverified, never guessed at.
    #[test]
    fn run_stdout_criterion_without_backticks_synthesizes_no_probe() {
        let dir = no_language_dir();
        assert!(probe_for_criterion(dir.path(), "running main.py prints Hello").is_none());
        assert!(probe_for_criterion(dir.path(), "the program prints a greeting").is_none());
    }

    /// The closed-set contract, root-cause level: freeform criteria the
    /// probe grammar cannot parse must never be silently approximated —
    /// verified at the `verify_with_cwd` level (never a smoke-only PASS) by
    /// `unparseable_acceptance_criteria_reports_no_checks_not_a_smoke_pass`.
    #[test]
    fn an_unmechanical_criterion_synthesizes_no_probe_even_when_call_shaped() {
        let dir = python_dir();
        assert!(synthesize_probe_checks(
            dir.path(),
            &[
                "the output feels right".to_string(),
                "fizzbuzz(3) is basically Fizz".to_string(),
            ]
        )
        .is_empty());
    }

    /// The closed-set contract: a criterion that names no file, no command,
    /// and no symbol is left completely alone — never approximated into a
    /// check that would silently fabricate a verdict.
    #[test]
    fn an_unmechanical_criterion_synthesizes_no_probe() {
        let dir = no_language_dir();
        assert!(probe_for_criterion(dir.path(), "the design feels polished").is_none());
        assert!(probe_for_criterion(dir.path(), "the API is intuitive to use").is_none());
        assert_eq!(
            synthesize_probe_checks(
                dir.path(),
                &[
                    "the design feels polished".to_string(),
                    "fizzbuzz.py exists".to_string(),
                ]
            )
            .len(),
            1,
            "only the mechanically-checkable criterion becomes a probe"
        );
    }

    #[test]
    fn find_symbol_definition_locates_a_present_python_function() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def fizzbuzz(n):\n    return n\n",
        )
        .unwrap();
        let found = find_symbol_definition(dir.path(), "fizzbuzz", SymbolKind::Function);
        assert_eq!(found.as_deref(), Some("fizzbuzz.py:1"));
    }

    #[test]
    fn find_symbol_definition_reports_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def other(n):\n    return n\n",
        )
        .unwrap();
        assert_eq!(
            find_symbol_definition(dir.path(), "fizzbuzz", SymbolKind::Function),
            None
        );
    }

    /// The M23 dschat defect this lane fixes, reproduced end to end: a
    /// manifestless single `.py` file (`manifestless_single_py_file_gets_an_
    /// honestly_named_syntax_smoke` above) with acceptance criteria that
    /// actually hold gets a GENUINE, non-vacuous pass — not the smoke-only
    /// vacuous pass the tool used to report.
    #[tokio::test]
    async fn fizzbuzz_shaped_goal_with_matching_acceptance_criteria_is_genuinely_verified() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def fizzbuzz(n):\n    return str(n)\n",
        )
        .unwrap();
        let result = VerifyTool::new()
            .verify_with_cwd(
                params_with_criteria(&["fizzbuzz.py exists", "defines a function named fizzbuzz"]),
                Some(dir.path()),
                None,
            )
            .await;
        assert_eq!(result.is_error, Some(false));
        assert!(
            is_genuine_pass(&result),
            "a real file-exists probe plus a real symbol-defined probe must not be vacuous"
        );
        assert_eq!(vacuous_pass_reason(&result), None);
    }

    /// The mirror case: the criterion is mechanically checkable but FALSE —
    /// verify must report a genuine, actionable FAIL, never a fabricated pass.
    #[tokio::test]
    async fn fizzbuzz_shaped_goal_with_an_unmet_acceptance_criterion_fails_honestly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def other(n):\n    return n\n",
        )
        .unwrap();
        let result = VerifyTool::new()
            .verify_with_cwd(
                params_with_criteria(&["defines a function named fizzbuzz"]),
                Some(dir.path()),
                None,
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result);
        assert!(text.starts_with("FAIL"), "got: {text}");
        assert!(text.contains("fizzbuzz"), "got: {text}");
    }

    /// R35b: no acceptance criteria at all, and the code is syntactically
    /// valid — this used to be `pass_result`'s smoke-only vacuous PASS
    /// (`vacuous_passes` incremented for genuinely correct, uninspected
    /// code). A smoke-only run is never scored as a PASS any more: this now
    /// reports the honest `NO_CHECKS_VERDICT`, exactly like a project with no
    /// detectable checks at all — neither a pass nor a fail, and
    /// `vacuous_passes` is not touched (see
    /// `zero_checks_never_increments_the_vacuous_passes_receipt`).
    #[tokio::test]
    async fn fizzbuzz_shaped_goal_with_no_acceptance_criteria_reports_no_checks_not_a_smoke_pass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fizzbuzz.py"), "def fizzbuzz(n): pass\n").unwrap();
        let result = VerifyTool::new()
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: Some(30),
                    scope: None,
                    risk: None,
                    changed_paths: None,
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                None,
            )
            .await;
        assert!(
            !is_authoritative_pass(&result),
            "smoke-only is never a PASS"
        );
        assert!(!is_genuine_pass(&result));
        assert_eq!(vacuous_pass_reason(&result), None);
        let text = text_of(&result).to_string();
        let observation: VerificationObservation =
            serde_json::from_value(result.structured_content.unwrap()).unwrap();
        assert_eq!(observation.verdict.as_deref(), Some(NO_CHECKS_VERDICT));
        assert!(
            text.contains("no executable checks derived from the supplied criteria (0 unparsed)"),
            "got: {text}"
        );
    }

    /// R35b (A32 root cause 2): the ORIGINAL M23 dschat defect — freeform
    /// criteria the probe grammar cannot parse. This used to fall through to
    /// the smoke-only vacuous PASS (`vacuous_passes` incremented even though
    /// nothing was genuinely checked); it must now report `NO_CHECKS_VERDICT`
    /// naming how many criteria were unparsed, and never increment
    /// `vacuous_passes` — driven through the real global receipts registry
    /// (same pattern as
    /// `zero_checks_never_increments_the_vacuous_passes_receipt`) so a future
    /// refactor that routes this path back through `pass_result` would be
    /// caught here.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn unparseable_acceptance_criteria_reports_no_checks_not_a_smoke_pass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fizzbuzz.py"), "def fizzbuzz(n): pass\n").unwrap();
        crate::receipts::begin_run();
        let before = crate::receipts::snapshot().lines();
        let result = VerifyTool::new()
            .verify_with_cwd(
                params_with_criteria(&["the output feels right", "fizzbuzz(3) is basically Fizz"]),
                Some(dir.path()),
                None,
            )
            .await;
        let after = crate::receipts::snapshot().lines();
        crate::receipts::end_run();
        assert!(
            !is_authoritative_pass(&result),
            "smoke-only is never a PASS"
        );
        assert_eq!(vacuous_pass_reason(&result), None);
        let text = text_of(&result);
        assert!(
            text.contains("no executable checks derived from the supplied criteria (2 unparsed)"),
            "got: {text}"
        );
        assert_eq!(
            before[1], after[1],
            "an unparseable-criteria NO CHECKS result must not move the verification_integrity \
             line"
        );
        assert!(after[1].contains("vacuous_passes=0"), "got: {}", after[1]);
    }

    /// A criterion mechanically checkable but with no repo checks at all
    /// (`NoChecksConfigured`, not just an all-smoke `Checks`): the probe still
    /// runs as a real check rather than falling back to an honest-but-useless
    /// NO CHECKS.
    #[tokio::test]
    async fn no_checks_configured_still_prefers_a_matching_acceptance_probe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "notes\n").unwrap();
        let mut params = params_with_criteria(&["README.md exists"]);
        params.scope = Some(VerificationScope::Auto);
        params.changed_paths = Some(vec!["README.md".to_string()]);
        let result = VerifyTool::new()
            .verify_with_cwd(params, Some(dir.path()), None)
            .await;
        assert_eq!(result.is_error, Some(false));
        assert!(
            is_genuine_pass(&result),
            "a matching file-exists probe must stand in for NO CHECKS"
        );
    }

    // ── R44a/#A39: a probe must never join an absolute or `..`-escaping path
    // straight into the host filesystem ─────────────────────────────────────

    #[test]
    fn safe_probe_path_accepts_a_normal_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = safe_probe_path(dir.path(), "src/lib.rs").unwrap();
        assert_eq!(resolved, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn safe_probe_path_rejects_an_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_probe_path(dir.path(), "/etc/passwd").is_err());
    }

    #[test]
    fn safe_probe_path_rejects_a_parent_dir_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_probe_path(dir.path(), "../outside.txt").is_err());
        assert!(safe_probe_path(dir.path(), "sub/../../outside.txt").is_err());
    }

    #[tokio::test]
    async fn file_exists_probe_with_absolute_path_is_unusable_not_a_host_lookup() {
        // Even a path that genuinely exists on the host must never be looked
        // up directly — an absolute probe path is always a harness bug (the
        // acceptance-criteria compiler should have relativised or dropped
        // it), never a real check to run against the goal worktree.
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_probe_check(
            &ProbeKind::FileExists("/etc/hosts".to_string()),
            dir.path(),
            5,
        )
        .await;
        assert!(
            matches!(outcome, CheckOutcome::SpawnError(_)),
            "an absolute probe path must be reported unusable, not PASS or FAIL"
        );
    }

    #[tokio::test]
    async fn file_exists_probe_with_parent_dir_escape_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_probe_check(
            &ProbeKind::FileExists("../outside.txt".to_string()),
            dir.path(),
            5,
        )
        .await;
        assert!(
            matches!(outcome, CheckOutcome::SpawnError(_)),
            "a `..`-escaping probe path must be reported unusable, not PASS or FAIL"
        );
    }

    #[tokio::test]
    async fn file_exists_probe_with_a_normal_relative_path_still_works() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fizzbuzz.py"), "print(1)\n").unwrap();
        let found = run_probe_check(
            &ProbeKind::FileExists("fizzbuzz.py".to_string()),
            dir.path(),
            5,
        )
        .await;
        assert!(matches!(found, CheckOutcome::Passed { .. }));

        let missing = run_probe_check(
            &ProbeKind::FileExists("missing.py".to_string()),
            dir.path(),
            5,
        )
        .await;
        assert!(matches!(missing, CheckOutcome::Failed { .. }));
    }

    // ── R21c-b: structured `probe:*` marker parsing ─────────────────────────

    #[test]
    fn structured_probe_parses_a_file_exists_marker() {
        let dir = no_language_dir();
        let check = probe_for_criterion(dir.path(), "probe:file_exists path=fizzbuzz.py").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::FileExists("fizzbuzz.py".to_string()))
        );
    }

    #[test]
    fn structured_probe_parses_a_symbol_defined_marker_with_its_named_file() {
        let dir = no_language_dir();
        let check = probe_for_criterion(
            dir.path(),
            "probe:symbol_defined path=fizzbuzz.py symbol=fizzbuzz kind=function",
        )
        .unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::SymbolDefined {
                symbol: "fizzbuzz".to_string(),
                kind: SymbolKind::Function,
                file: Some("fizzbuzz.py".to_string()),
            })
        );
    }

    #[test]
    fn structured_probe_parses_a_command_marker() {
        let dir = no_language_dir();
        let check =
            probe_for_criterion(dir.path(), "probe:command cmd=\"pytest -k smoke\"").unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::CommandExitsZero("pytest -k smoke".to_string()))
        );
    }

    /// R21b's `render()` appends `" — <original criterion>"` after the
    /// marker; that trailing text must never confuse the parser (e.g. a
    /// stray `path=` inside the human-readable original text must not win
    /// over the real one before the dash).
    #[test]
    fn structured_probe_ignores_text_after_the_original_criterion_separator() {
        let dir = no_language_dir();
        let check = probe_for_criterion(
            dir.path(),
            "probe:file_exists path=fizzbuzz.py — fizzbuzz.py exists in the repo",
        )
        .unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::FileExists("fizzbuzz.py".to_string()))
        );
    }

    #[test]
    fn structured_probe_takes_priority_over_the_free_text_heuristics() {
        // Would ALSO match `symbol_defined_probe`'s free-text heuristic if
        // the structured marker were not tried first — pin that the
        // structured, unambiguous form wins.
        let dir = no_language_dir();
        let check = probe_for_criterion(
            dir.path(),
            "probe:file_exists path=fizzbuzz.py — defines a function named x",
        )
        .unwrap();
        assert_eq!(
            check.exec,
            Exec::Probe(ProbeKind::FileExists("fizzbuzz.py".to_string()))
        );
    }

    // ── R21c-b: verify loads the bound goal card's own criteria ─────────────

    /// The coordinator's ask: a cheap model dispatched as a goal worker will
    /// not reliably pass `acceptance_criteria` itself, so when it omits the
    /// param entirely, `verify` must find its OWN bound goal card (via the
    /// same in-process `GOAL_WORKERS` registry `interrupt_agent`/`goal_status`
    /// already use) and probe that card's real acceptance criteria.
    ///
    /// Drives the real global `SessionManager::instance()` singleton (the
    /// same pool `load_bound_goal_acceptance_criteria` reads in production),
    /// matching the pattern `orchestrator.rs`'s own
    /// `goal_status_reports_a_registered_goal_worker_as_alive_and_leaves_it_alone`
    /// test uses for the identical registry. `#[serial(goal_workers)]` for
    /// the same reason that test carries it: `GOAL_WORKERS` is process-global.
    #[tokio::test]
    #[serial_test::serial(goal_workers)]
    async fn goal_worker_session_with_no_param_loads_the_bound_cards_criteria() {
        use crate::agents::platform_extensions::goal_engine;
        use crate::agents::platform_extensions::orchestrator;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("fizzbuzz.py"),
            "def fizzbuzz(n):\n    return str(n)\n",
        )
        .unwrap();

        let pool = crate::session::SessionManager::instance()
            .pool_clone()
            .await
            .unwrap();
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: format!("r21c-b-test-{}", uuid::Uuid::new_v4()),
                slug: None,
                description: None,
                root_path: None,
                site_url: None,
                repo_url: None,
                notes: None,
                tags: None,
            },
        )
        .await
        .unwrap();
        let card = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "fizzbuzz".to_string(),
                description: Some(String::new()),
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "acceptance_criteria": [
                        "fizzbuzz.py exists",
                        "defines a function named fizzbuzz",
                    ],
                })),
            },
        )
        .await
        .unwrap();

        let session_id = format!("r21c-b-goal-session-{}", uuid::Uuid::new_v4());
        orchestrator::register_goal_worker(
            &card.id,
            &session_id,
            goal_engine::GoalKill::None,
            None,
        );

        let result = VerifyTool::new()
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: Some(30),
                    scope: None,
                    risk: None,
                    changed_paths: None,
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                Some(session_id.as_str()),
            )
            .await;

        // Drop the registry entry before any assertion can panic and skip it.
        orchestrator::take_goal_worker(&card.id);

        assert_eq!(result.is_error, Some(false));
        assert!(
            is_genuine_pass(&result),
            "the bound goal card's own acceptance criteria should have been loaded and \
             probed, producing a genuine pass instead of the smoke-only vacuous one"
        );
        assert_eq!(vacuous_pass_reason(&result), None);
    }

    /// The mirror case: a session id that is NOT a registered goal worker
    /// (an interactive session, or simply a session this process never
    /// dispatched) must behave exactly as before this lane for CRITERIA
    /// LOADING — no criteria loaded. As of R35b the resulting smoke-only run
    /// is no longer scored as a vacuous PASS (see
    /// `fizzbuzz_shaped_goal_with_no_acceptance_criteria_reports_no_checks_not_a_smoke_pass`);
    /// it now honestly reports `NO_CHECKS_VERDICT`.
    #[tokio::test]
    #[serial_test::serial(goal_workers)]
    async fn non_goal_worker_session_with_no_param_is_unchanged() {
        use crate::agents::platform_extensions::orchestrator;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fizzbuzz.py"), "def fizzbuzz(n): pass\n").unwrap();

        let session_id = format!("r21c-b-non-goal-session-{}", uuid::Uuid::new_v4());
        // Not registered in GOAL_WORKERS at all — confirm that up front so a
        // stale entry from another test can never make this a false pass.
        assert_eq!(
            orchestrator::live_goal_worker_card_for_session(&session_id),
            None
        );

        let result = VerifyTool::new()
            .verify_with_cwd(
                VerifyParams {
                    command: None,
                    path: None,
                    timeout_secs: Some(30),
                    scope: None,
                    risk: None,
                    changed_paths: None,
                    acceptance_criteria: None,
                },
                Some(dir.path()),
                Some(session_id.as_str()),
            )
            .await;

        assert!(
            !is_authoritative_pass(&result),
            "smoke-only is never a PASS"
        );
        assert!(!is_genuine_pass(&result));
        assert_eq!(
            vacuous_pass_reason(&result),
            None,
            "a non-goal-worker session must never have criteria synthesized for it"
        );
        let text = text_of(&result);
        assert!(
            text.contains("no executable checks derived from the supplied criteria (0 unparsed)"),
            "got: {text}"
        );
    }

    /// R21c's headline receipt fix: zero checks — and no acceptance-criteria
    /// probe available to stand in for one — must never increment
    /// `vacuous_passes`. `pass_result` is the only site in this module that
    /// increments it, and `no_checks_result` never calls it; this test
    /// exercises the real global registry end to end (the same pattern
    /// `receipts.rs`'s own sidecar tests use) so a future refactor that
    /// routes `no_checks_result` through `pass_result` would be caught here.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn zero_checks_never_increments_the_vacuous_passes_receipt() {
        crate::receipts::begin_run();
        let before = crate::receipts::snapshot().lines();
        let _ = no_checks_result("nothing to run yet.");
        let after = crate::receipts::snapshot().lines();
        crate::receipts::end_run();
        assert_eq!(
            before[1], after[1],
            "an honest NO CHECKS result must not move the verification_integrity line"
        );
        assert!(after[1].contains("vacuous_passes=0"), "got: {}", after[1]);
    }
}
