//! Completion check runner — daemon-executed, deterministic checks declared on
//! goal cards at `metadata_json.completion_checks`.
//!
//! Checks run sequentially in the goal's working_dir with no short-circuit.
//! Output is captured verbatim with a last-16KiB cap per stream and a
//! `truncated` flag. `error` never counts as pass.

use permagent::verification_approval::{self as approval, ChecksSource, DenyCategory, Tier};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Per-stream output cap (last 16KiB kept).
pub const MAX_TAIL_BYTES: usize = 16 * 1024;
/// Cap on grep_absent matched lines reported as evidence.
const MAX_GREP_MATCHES: usize = 20;
/// Cap on a single reported grep match line.
const MAX_GREP_LINE_BYTES: usize = 500;
/// HTTP assert request timeout.
const HTTP_TIMEOUT_SECS: u64 = 30;

fn default_timeout_secs() -> u64 {
    120
}

/// A single completion check declared on a goal card.
///
/// Tagged enum with `deny_unknown_fields`: unknown check types or stray fields
/// fail to parse rather than silently passing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompletionCheck {
    /// Run a shell command in the goal working_dir; pass iff exit code 0 **and**
    /// — when `expect` is set — the expected token is present in the captured
    /// output.
    ///
    /// Exit 0 alone is not proof. A command can exit 0 having done nothing
    /// (`echo ok`), having skipped the work it was supposed to do (a test
    /// runner that matched zero tests), or having printed the very failure it
    /// was meant to catch. `expect` is the assertion half: a plain substring,
    /// or a regex when the value is wrapped in slashes (`/\d+ passed/`).
    /// Matched against combined stdout+stderr (each capped at
    /// [`MAX_TAIL_BYTES`]).
    ///
    /// Optional so every card written before this field existed still parses
    /// under `deny_unknown_fields`; an `expect`-less check behaves exactly as
    /// it always did.
    CommandExitZero {
        cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<String>,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
    },
    /// Assert an HTTP response from a loopback-only URL (SSRF guard).
    HttpAssert {
        method: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        path: String,
        status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body_contains: Option<String>,
    },
    /// Pass iff the path exists under the working_dir.
    FileExists { path: String },
    /// Pass iff the regex pattern matches zero lines across all listed files.
    GrepAbsent { pattern: String, paths: Vec<String> },
}

impl CompletionCheck {
    pub fn type_name(&self) -> &'static str {
        match self {
            CompletionCheck::CommandExitZero { .. } => "command_exit_zero",
            CompletionCheck::HttpAssert { .. } => "http_assert",
            CompletionCheck::FileExists { .. } => "file_exists",
            CompletionCheck::GrepAbsent { .. } => "grep_absent",
        }
    }

    /// Short human-readable summary of what this check does (for the digest).
    pub fn summary(&self) -> String {
        match self {
            CompletionCheck::CommandExitZero {
                cmd,
                expect: Some(e),
                ..
            } => format!("command exits 0 and output matches {}: `{}`", e, cmd),
            CompletionCheck::CommandExitZero { cmd, .. } => format!("command exits 0: `{}`", cmd),
            CompletionCheck::HttpAssert {
                method,
                path,
                status,
                ..
            } => format!("HTTP {} {} returns {}", method, path, status),
            CompletionCheck::FileExists { path } => format!("file exists: {}", path),
            CompletionCheck::GrepAbsent { pattern, .. } => {
                format!("pattern absent: /{}/", pattern)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<String>>,
    /// For a `command_exit_zero` carrying an `expect`: whether the expected
    /// token was found. `None` when no `expect` was declared. This is what
    /// lets the digest say *which half* failed — the exit code or the
    /// assertion — instead of a bare "the check failed".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_matched: Option<bool>,
    /// Error/explanation message (set on `error`, and on some `fail`s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result row stored at `metadata_json.verification.check_results[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_index: usize,
    #[serde(rename = "type")]
    pub check_type: String,
    pub status: CheckStatus,
    pub started_at: String,
    pub duration_ms: u64,
    pub evidence: CheckEvidence,
    pub truncated: bool,
    /// Human-readable summary for a result with no entry in the declared-check
    /// list (the synthesized placeholder scan). `None` for declared checks —
    /// their summary comes from `CompletionCheck::summary()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Set when [`super::check_lint`] judged this check *gameable*. A linted
    /// check does not count as verification evidence, and this string is the
    /// reason, carried into the digest and the verifier's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lint: Option<String>,
    /// What the approval ladder decided about this command, when it was a
    /// `command_exit_zero`. Carried onto the goal card so the verification
    /// section can show who authorised the command next to the command itself
    /// — a self-approval that nobody can see is a silent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<approval::AuditRow>,
}

impl CheckResult {
    fn error(check_index: usize, check_type: &str, started_at: String, message: String) -> Self {
        CheckResult {
            check_index,
            check_type: check_type.to_string(),
            status: CheckStatus::Error,
            started_at,
            duration_ms: 0,
            evidence: CheckEvidence {
                message: Some(message),
                ..Default::default()
            },
            truncated: false,
            summary: None,
            lint: None,
            approval: None,
        }
    }
}

// ── Path guards ─────────────────────────────────────────────────────────────

/// Resolve `rel` under `working_dir`, rejecting absolute paths, lexical
/// traversal above the working_dir, and (for existing paths) symlink escapes.
pub fn resolve_under(working_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(format!("absolute path not allowed: '{}'", rel));
    }

    // Lexical traversal guard: never allow net movement above working_dir.
    let mut depth: i64 = 0;
    for comp in rel_path.components() {
        match comp {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("path escapes working_dir: '{}'", rel));
                }
            }
            other => {
                return Err(format!(
                    "unsupported path component {:?} in '{}'",
                    other, rel
                ));
            }
        }
    }

    let wd_canon = working_dir
        .canonicalize()
        .map_err(|e| format!("working_dir not resolvable: {}", e))?;
    let joined = wd_canon.join(rel_path);

    // Symlink-escape guard: if the path exists, its canonical form must stay
    // under the canonical working_dir.
    match joined.canonicalize() {
        Ok(canon) => {
            if canon.starts_with(&wd_canon) {
                Ok(canon)
            } else {
                Err(format!(
                    "path resolves outside working_dir (symlink escape?): '{}'",
                    rel
                ))
            }
        }
        // Non-existent path: lexical guard already passed; caller decides
        // what non-existence means (e.g. file_exists → fail).
        Err(_) => Ok(joined),
    }
}

/// Loopback-only SSRF guard for http_assert base URLs.
pub fn assert_loopback_url(base: &str) -> Result<url::Url, String> {
    let parsed =
        url::Url::parse(base).map_err(|e| format!("invalid base_url '{}': {}", base, e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("scheme '{}' not allowed (http/https only)", s)),
    }
    let host_ok = match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !host_ok {
        return Err(format!(
            "base_url '{}' is not loopback — only localhost/127.0.0.1/::1 are allowed",
            base
        ));
    }
    Ok(parsed)
}

// ── Output capture helpers ──────────────────────────────────────────────────

/// Keep the last `max_bytes` of a string (UTF-8 boundary safe).
/// Returns (tail, truncated).
pub fn tail_str(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    (s.get(start..).unwrap_or_default().to_string(), true)
}

fn tail_bytes(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    let s = String::from_utf8_lossy(bytes);
    tail_str(&s, max_bytes)
}

// ── The approval gate ───────────────────────────────────────────────────────

/// One command the gate refused to run, ready to become a Decision Inbox card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedCheck {
    pub check_index: usize,
    pub cmd: String,
    pub cwd: Option<String>,
    /// The first token, when there is one to allowlist. `None` for a command
    /// the lexer could not read — there is nothing safe to add in that case,
    /// so the Inbox card offers approve-once only.
    pub first_token: Option<String>,
    pub tier: Tier,
    pub deny: Option<DenyCategory>,
    pub reason: String,
}

/// The approval ladder, carried through one verification run.
///
/// Holds the project's state on the way in and collects what happened on the
/// way out: an [`approval::AuditRow`] per gated command, and a [`ParkedCheck`]
/// for each one that was refused.
#[derive(Debug)]
pub struct CheckGate {
    config: approval::GateConfig,
    settings: approval::ApprovalSettings,
    source: ChecksSource,
    goal_id: Option<String>,
    rows: Vec<approval::AuditRow>,
    parked: Vec<ParkedCheck>,
    /// Gated commands that actually ran — the privilege this run may earn if
    /// the goal later verifies clean.
    clean_candidates: u32,
    /// Approve-once grants this run spent. Must be persisted, or the grant
    /// outlives its one use.
    spent_grants: Vec<String>,
}

impl CheckGate {
    /// The production gate: this project's allowlist and privilege, and the
    /// provenance stamped on this goal's checks.
    pub fn new(
        project_root: impl Into<PathBuf>,
        settings: approval::ApprovalSettings,
        build_command: Option<&str>,
        source: ChecksSource,
        goal_id: Option<String>,
    ) -> Self {
        let config = settings.gate_config(project_root, build_command);
        Self {
            config,
            settings,
            source,
            goal_id,
            rows: Vec::new(),
            parked: Vec::new(),
            clean_candidates: 0,
            spent_grants: Vec::new(),
        }
    }

    /// A gate for checks the user wrote themselves, which the ladder does not
    /// govern. Also what the runner's own tests use: they assert on how a
    /// command is *executed*, not on whether it is allowed to be.
    pub fn user_authored(project_root: impl Into<PathBuf>) -> Self {
        Self::new(
            project_root,
            approval::ApprovalSettings::default(),
            None,
            ChecksSource::User,
            None,
        )
    }

    /// Audit rows produced this run, for the project's visible history.
    pub fn audit_rows(&self) -> &[approval::AuditRow] {
        &self.rows
    }

    /// Commands the gate refused. Non-empty means the goal must be parked.
    pub fn parked(&self) -> &[ParkedCheck] {
        &self.parked
    }

    /// How much privilege this run earns *if* the goal later verifies clean.
    /// The caller decides whether it did; the gate only counts.
    pub fn clean_candidates(&self) -> u32 {
        self.clean_candidates
    }

    /// Approve-once grants this run spent, for the caller to persist.
    pub fn spent_grants(&self) -> &[String] {
        &self.spent_grants
    }
}

/// **This is the point at which a model-authored sentence becomes a process.**
///
/// Every `command_exit_zero` check passes through here before
/// [`run_command_check`] is allowed to hand it to `/bin/sh`. There is no other
/// path to the shell from a completion check, and there must not be one: if you
/// are adding a new way to execute check-declared text, it belongs behind this
/// function or behind a gate of its own.
///
/// Returns `Ok(())` when the command may run. `Err(reason)` means it may not,
/// and the caller must record the check as an error and leave the goal for a
/// person — never fall through to running it anyway.
pub fn gate_command_check(
    index: usize,
    cmd: &str,
    cwd: Option<&str>,
    gate: &mut CheckGate,
) -> Result<(), String> {
    let clean_runs = gate.settings.clean_runs;
    let outcome = approval::decide(cmd, cwd, gate.source, &mut gate.settings, &gate.config);

    // A user-authored check is not governed by the ladder, so it produces no
    // audit row and earns no privilege — there is nothing to account for.
    if outcome.decision == approval::GateDecision::UserAuthored {
        return Ok(());
    }

    gate.rows
        .push(outcome.audit_row(cmd, cwd, clean_runs, gate.goal_id.as_deref()));

    if outcome.decision == approval::GateDecision::ApprovedOnce {
        gate.spent_grants.push(cmd.to_string());
    }

    if outcome.allowed() {
        if outcome.decision.counts_toward_privilege() {
            gate.clean_candidates = gate.clean_candidates.saturating_add(1);
        }
        return Ok(());
    }

    gate.parked.push(ParkedCheck {
        check_index: index,
        cmd: cmd.to_string(),
        cwd: cwd.map(|c| c.to_string()),
        first_token: approval::first_token_of(cmd),
        tier: outcome.classification.tier,
        deny: outcome.classification.deny,
        reason: outcome.reason.clone(),
    });
    Err(outcome.reason)
}

// ── Runner ──────────────────────────────────────────────────────────────────

/// Run all checks sequentially in `working_dir`. No short-circuit: every
/// declared check produces a result row.
///
/// `gate` decides which `command_exit_zero` checks are allowed to run at all,
/// and accumulates what it decided.
pub async fn run_checks(
    checks: &[CompletionCheck],
    working_dir: &Path,
    gate: &mut CheckGate,
) -> Vec<CheckResult> {
    let mut results = Vec::with_capacity(checks.len());
    for (i, check) in checks.iter().enumerate() {
        results.push(run_one(i, check, working_dir, gate).await);
    }
    results
}

async fn run_one(
    index: usize,
    check: &CompletionCheck,
    working_dir: &Path,
    gate: &mut CheckGate,
) -> CheckResult {
    let started_at = chrono::Utc::now().to_rfc3339();
    let start = std::time::Instant::now();
    let check_type = check.type_name();
    // Where this check's audit row will land, if the gate writes one.
    let row_slot = gate.rows.len();

    let outcome = match check {
        CompletionCheck::CommandExitZero {
            cmd,
            cwd,
            expect,
            timeout_secs,
        } => match gate_command_check(index, cmd, cwd.as_deref(), gate) {
            Ok(()) => {
                run_command_check(
                    cmd,
                    cwd.as_deref(),
                    expect.as_deref(),
                    *timeout_secs,
                    working_dir,
                )
                .await
            }
            // Not a verdict on the work — an Error, which parks the goal for a
            // person rather than condemning a diff that may be finished and
            // correct. The command did not run, so nothing was learned about it.
            Err(why) => Err(format!(
                "this check was not run: {}. Approve it in the Decision Inbox to let it run.",
                why
            )),
        },
        CompletionCheck::HttpAssert {
            method,
            base_url,
            path,
            status,
            body_contains,
        } => {
            run_http_check(
                method,
                base_url.as_deref(),
                path,
                *status,
                body_contains.as_deref(),
            )
            .await
        }
        CompletionCheck::FileExists { path } => run_file_exists(path, working_dir),
        CompletionCheck::GrepAbsent { pattern, paths } => {
            run_grep_absent(pattern, paths, working_dir)
        }
    };

    // The row this check's own gating produced, if any — never a neighbour's.
    let approval = gate.rows.get(row_slot).cloned();

    match outcome {
        Ok((status, evidence, truncated)) => CheckResult {
            check_index: index,
            check_type: check_type.to_string(),
            status,
            started_at,
            duration_ms: start.elapsed().as_millis() as u64,
            evidence,
            truncated,
            summary: None,
            lint: None,
            approval,
        },
        Err(message) => {
            let mut r = CheckResult::error(index, check_type, started_at, message);
            r.duration_ms = start.elapsed().as_millis() as u64;
            r.approval = approval;
            r
        }
    }
}

type CheckOutcome = Result<(CheckStatus, CheckEvidence, bool), String>;

/// The PATH completion checks run with. The daemon is launched by launchd,
/// whose PATH omits every developer toolchain — `cargo check` exited 127 here
/// and condemned finished goal work. Prepend the standard tool homes to
/// whatever PATH the daemon inherited.
fn check_path() -> String {
    let inherited = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    let home = std::env::var("HOME").unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for candidate in [
        format!("{home}/.cargo/bin"),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
    ] {
        if !inherited.split(':').any(|p| p == candidate) && Path::new(&candidate).is_dir() {
            parts.push(candidate);
        }
    }
    parts.push(inherited);
    parts.join(":")
}

/// **The single seam where a model-authored completion check reaches the OS.**
///
/// Every `command_exit_zero` check — hand-authored on a card, or compiled from
/// a goal's acceptance criteria by `orchestrator::checks_from_acceptance` —
/// executes here and nowhere else. Everything an approval decision would need
/// is already resolved and in scope at this one point: the literal command,
/// the canonical CWD, the shell, the PATH, and the timeout.
///
/// A user-consent gate (unlazy hash-binds approval to exactly this tuple)
/// belongs at the top of this function as a single early `Err(...)` return; no
/// other code path has to change for one to exist. Deliberately left un-gated
/// here — whether model-authored shell needs consent before running is an open
/// product decision, not something this seam should presume.
async fn execute_check_shell(
    cmd: &str,
    dir: &Path,
    timeout_secs: u64,
) -> Result<std::process::Output, String> {
    let timeout = timeout_secs.clamp(1, 600);

    #[cfg(windows)]
    let (shell, flag) = ("cmd", "/C");
    #[cfg(not(windows))]
    let (shell, flag) = ("/bin/sh", "-c");

    let fut = tokio::process::Command::new(shell)
        .arg(flag)
        .arg(cmd)
        .current_dir(dir)
        .env("PATH", check_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();

    match tokio::time::timeout(Duration::from_secs(timeout), fut).await {
        Err(_) => Err(format!("command timed out after {}s", timeout)),
        Ok(Err(e)) => Err(format!("failed to spawn command: {}", e)),
        Ok(Ok(o)) => Ok(o),
    }
}

/// Evaluate an `expect` assertion against captured output.
///
/// `/pattern/` (slash-wrapped, at least one character between) is a regex;
/// anything else is a plain substring. An unparseable regex is an `Err` — the
/// check errors rather than silently passing on a broken assertion.
pub fn expect_matches(expect: &str, haystack: &str) -> Result<bool, String> {
    if let Some(inner) = expect
        .strip_prefix('/')
        .and_then(|s| s.strip_suffix('/'))
        .filter(|s| !s.is_empty())
    {
        let re = regex::Regex::new(inner)
            .map_err(|e| format!("invalid expect regex '/{}/': {}", inner, e))?;
        return Ok(re.is_match(haystack));
    }
    Ok(haystack.contains(expect))
}

async fn run_command_check(
    cmd: &str,
    cwd: Option<&str>,
    expect: Option<&str>,
    timeout_secs: u64,
    working_dir: &Path,
) -> CheckOutcome {
    let dir = match cwd {
        Some(rel) => {
            let resolved = resolve_under(working_dir, rel)?;
            if !resolved.is_dir() {
                return Err(format!("cwd '{}' is not a directory", rel));
            }
            resolved
        }
        None => working_dir
            .canonicalize()
            .map_err(|e| format!("working_dir not resolvable: {}", e))?,
    };

    let output = execute_check_shell(cmd, &dir, timeout_secs).await?;

    let (stdout_tail, out_trunc) = tail_bytes(&output.stdout, MAX_TAIL_BYTES);
    let (stderr_tail, err_trunc) = tail_bytes(&output.stderr, MAX_TAIL_BYTES);
    let exit_code = output.status.code();
    // `code()` is None exactly when the process we spawned died by a SIGNAL.
    // Note what that process is: `/bin/sh -c "<cmd>"` on macOS execs a single
    // simple command in place, so for the ordinary check (`cargo test`) the
    // signalled process IS the check itself. A signalled check is therefore
    // ambiguous — SIGKILL from the OOM killer (the machine) and SIGSEGV from a
    // crashing test binary (the work) are indistinguishable from here.
    let signal_killed = exit_code.is_none();

    // The assertion half. Matched over combined stdout+stderr because tools
    // disagree about which stream carries a summary line (cargo prints its
    // test totals to stdout, npm and rustc to stderr) — an `expect` that only
    // watched stdout would false-fail correct work for half the ecosystem.
    // Evaluated over the CAPTURED TAIL, so a token pushed past the 16 KiB cap
    // by later output is (honestly) not found.
    let expect_matched = match expect {
        None => None,
        Some(e) => Some(expect_matches(e, &format!("{stdout_tail}\n{stderr_tail}"))?),
    };

    let status = if output.status.success() {
        match expect_matched {
            // Exit 0 alone is not proof: the declared token was not in the
            // output, so the command exiting 0 says nothing about the outcome.
            Some(false) => CheckStatus::Fail,
            _ => CheckStatus::Pass,
        }
    } else if exit_code == Some(127) || exit_code == Some(126) || signal_killed {
        // Not a verdict on the diff — an Error parks the goal for human review
        // instead of condemning work that may be finished and correct:
        //
        // - 127: "command not found" — the CHECK ENVIRONMENT failing. This exact
        //   code (cargo absent from the daemon's launchd PATH) gave real,
        //   finished goal work a fail verdict and auto-cancelled it.
        // - 126: POSIX "found but not executable" (permission denied, or not an
        //   executable format) — same category as 127.
        // - signalled: unprovable either way (see above), so it is reported as
        //   unprovable rather than asserted as a failure of the work. This is
        //   not a softening: `clamp_with_check_results` clamps Error and Fail
        //   alike to a Fail verdict, so nothing is auto-approved by landing
        //   here. What changes is only what the verifier is TOLD — "killed by a
        //   signal, cause unknown" instead of a bare failure with no exit code.
        //
        // 128+N stays Fail on purpose: that is a normal exit, reported by a
        // surviving shell whose CHILD was signalled, and the shell surviving is
        // evidence the machine was not the thing that died.
        CheckStatus::Error
    } else {
        CheckStatus::Fail
    };

    Ok((
        status,
        CheckEvidence {
            exit_code,
            stdout_tail: Some(stdout_tail),
            stderr_tail: Some(stderr_tail),
            expect_matched,
            // With no exit code there is nothing else in the row to explain an
            // `error`, and an unexplained error is not evidence.
            message: signal_killed
                .then(|| {
                    format!(
                        "the check was terminated by {} before it could report an exit code — \
                         this may be the machine (OOM/kill) or the check crashing, and the two \
                         cannot be told apart here",
                        describe_termination(&output.status)
                    )
                })
                // Say WHICH HALF failed. A bare "the check failed" next to
                // `exit 0` reads as a contradiction in the digest.
                .or_else(|| match (expect_matched, expect) {
                    (Some(false), Some(e)) if output.status.success() => Some(format!(
                        "command exited 0 but its output does not match the required \
                         expect assertion {} — exit 0 alone is not proof",
                        e
                    )),
                    (Some(false), Some(e)) => Some(format!(
                        "command failed AND its output does not match the required \
                         expect assertion {}",
                        e
                    )),
                    _ => None,
                }),
            ..Default::default()
        },
        out_trunc || err_trunc,
    ))
}

/// Name the signal that killed a check, when the platform can say. The number
/// is the actionable part: 9/15 read as the machine, 6/11 as a crash.
fn describe_termination(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    let _ = status;
    "a signal".to_string()
}

async fn run_http_check(
    method: &str,
    base_url: Option<&str>,
    path: &str,
    expected_status: u16,
    body_contains: Option<&str>,
) -> CheckOutcome {
    let base = base_url.unwrap_or("http://127.0.0.1");
    let parsed = assert_loopback_url(base)?;

    // Guard against authority-smuggling via the path ("//evil.com/...").
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(format!(
            "path must start with a single '/' (got '{}')",
            path
        ));
    }

    let method = match method.to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "HEAD" => reqwest::Method::HEAD,
        "POST" => reqwest::Method::POST,
        m => return Err(format!("HTTP method '{}' not allowed (GET/HEAD/POST)", m)),
    };

    let url = format!("{}{}", parsed.as_str().trim_end_matches('/'), path);
    // Re-verify the final URL is still loopback after joining.
    assert_loopback_url(&url)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .request(method, &url)
        .send()
        .await
        .map_err(|e| format!("request to '{}' failed: {}", url, e))?;

    let http_status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let (body_excerpt, truncated) = tail_str(&body, MAX_TAIL_BYTES);

    let status_ok = http_status == expected_status;
    let body_ok = body_contains.is_none_or(|needle| body.contains(needle));

    let mut message = None;
    if !status_ok {
        message = Some(format!(
            "expected status {}, got {}",
            expected_status, http_status
        ));
    } else if !body_ok {
        message = Some(format!(
            "body does not contain '{}'",
            body_contains.unwrap_or_default()
        ));
    }

    Ok((
        if status_ok && body_ok {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        CheckEvidence {
            http_status: Some(http_status),
            body_excerpt: Some(body_excerpt),
            message,
            ..Default::default()
        },
        truncated,
    ))
}

fn run_file_exists(path: &str, working_dir: &Path) -> CheckOutcome {
    let resolved = resolve_under(working_dir, path)?;
    let exists = resolved.exists();
    Ok((
        if exists {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        CheckEvidence {
            message: Some(if exists {
                format!("'{}' exists", path)
            } else {
                format!("'{}' does not exist", path)
            }),
            ..Default::default()
        },
        false,
    ))
}

fn run_grep_absent(pattern: &str, paths: &[String], working_dir: &Path) -> CheckOutcome {
    if paths.is_empty() {
        return Err("grep_absent requires at least one path".to_string());
    }
    let re = regex::Regex::new(pattern).map_err(|e| format!("invalid pattern: {}", e))?;

    let mut matches: Vec<String> = Vec::new();
    let mut total_matches = 0usize;

    for rel in paths {
        let resolved = resolve_under(working_dir, rel)?;
        // A missing file means we cannot prove absence in it — error, not pass.
        let contents = std::fs::read_to_string(&resolved)
            .map_err(|e| format!("cannot read '{}': {}", rel, e))?;
        for (lineno, line) in contents.lines().enumerate() {
            if re.is_match(line) {
                total_matches += 1;
                if matches.len() < MAX_GREP_MATCHES {
                    let (shown, _) = tail_str(line, MAX_GREP_LINE_BYTES);
                    matches.push(format!("{}:{}: {}", rel, lineno + 1, shown));
                }
            }
        }
    }

    let truncated = total_matches > matches.len();
    Ok((
        if total_matches == 0 {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        CheckEvidence {
            matches: Some(matches),
            message: Some(format!("{} match(es) found", total_matches)),
            ..Default::default()
        },
        truncated,
    ))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_checks(json: &str) -> Result<Vec<CompletionCheck>, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn schema_parses_all_four_types() {
        let json = r#"[
            {"type": "command_exit_zero", "cmd": "true"},
            {"type": "command_exit_zero", "cmd": "ls", "cwd": "src", "timeout_secs": 30},
            {"type": "http_assert", "method": "GET", "path": "/health", "status": 200},
            {"type": "file_exists", "path": "README.md"},
            {"type": "grep_absent", "pattern": "TODO", "paths": ["src/main.rs"]}
        ]"#;
        let checks = parse_checks(json).unwrap();
        assert_eq!(checks.len(), 5);
        assert!(matches!(
            checks[0],
            CompletionCheck::CommandExitZero { ref cmd, timeout_secs: 120, .. } if cmd == "true"
        ));
    }

    #[test]
    fn schema_rejects_unknown_type() {
        let json = r#"[{"type": "rm_rf_root", "cmd": "boom"}]"#;
        assert!(parse_checks(json).is_err());
    }

    #[test]
    fn schema_rejects_unknown_fields() {
        let json = r#"[{"type": "file_exists", "path": "a", "sneaky": true}]"#;
        assert!(parse_checks(json).is_err());
    }

    #[test]
    fn tail_str_caps_and_flags() {
        let s = "a".repeat(100);
        let (tail, truncated) = tail_str(&s, 10);
        assert_eq!(tail.len(), 10);
        assert!(truncated);
        let (full, truncated) = tail_str("short", 10);
        assert_eq!(full, "short");
        assert!(!truncated);
    }

    #[test]
    fn tail_str_respects_char_boundaries() {
        let s = "é".repeat(50); // 2 bytes each
        let (tail, truncated) = tail_str(&s, 5);
        assert!(truncated);
        assert!(tail.len() <= 5);
        assert!(tail.chars().all(|c| c == 'é'));
    }

    #[test]
    fn loopback_guard_allows_localhost_and_rejects_remote() {
        assert!(assert_loopback_url("http://localhost:11434").is_ok());
        assert!(assert_loopback_url("http://127.0.0.1:3000").is_ok());
        assert!(assert_loopback_url("http://[::1]:8080").is_ok());
        assert!(assert_loopback_url("http://example.com").is_err());
        assert!(assert_loopback_url("http://169.254.169.254/latest").is_err());
        assert!(assert_loopback_url("http://10.0.0.1:11434").is_err());
        assert!(assert_loopback_url("ftp://127.0.0.1").is_err());
    }

    #[tokio::test]
    async fn http_assert_rejects_non_loopback_without_network() {
        let check = CompletionCheck::HttpAssert {
            method: "GET".to_string(),
            base_url: Some("http://example.com".to_string()),
            path: "/".to_string(),
            status: 200,
            body_contains: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let results = run_checks(
            std::slice::from_ref(&check),
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert!(results[0]
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("not loopback"));
    }

    #[tokio::test]
    async fn http_assert_rejects_authority_smuggling_path() {
        let check = CompletionCheck::HttpAssert {
            method: "GET".to_string(),
            base_url: Some("http://localhost:1".to_string()),
            path: "//evil.com/steal".to_string(),
            status: 200,
            body_contains: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let results = run_checks(
            std::slice::from_ref(&check),
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
    }

    #[tokio::test]
    async fn command_pass_fail_and_output_capture() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![
            CompletionCheck::CommandExitZero {
                cmd: "echo hello-stdout && echo hello-stderr 1>&2".to_string(),
                cwd: None,
                expect: None,
                timeout_secs: 30,
            },
            CompletionCheck::CommandExitZero {
                cmd: "exit 3".to_string(),
                cwd: None,
                expect: None,
                timeout_secs: 30,
            },
        ];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[0].evidence.exit_code, Some(0));
        assert!(results[0]
            .evidence
            .stdout_tail
            .as_deref()
            .unwrap()
            .contains("hello-stdout"));
        assert!(results[0]
            .evidence
            .stderr_tail
            .as_deref()
            .unwrap()
            .contains("hello-stderr"));
        // No short-circuit: second check still ran and failed.
        assert_eq!(results[1].status, CheckStatus::Fail);
        assert_eq!(results[1].evidence.exit_code, Some(3));
    }

    // ── `expect`: exit 0 alone is not proof ─────────────────────────────────

    /// The whole point of the field: a command that exits 0 while printing the
    /// wrong thing must FAIL, and the row must say which half failed.
    #[tokio::test]
    async fn exit_zero_with_a_missed_expect_fails_and_names_the_half() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo 0 tests were run".to_string(),
            cwd: None,
            expect: Some("412 passed".to_string()),
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Fail);
        assert_eq!(results[0].evidence.exit_code, Some(0));
        assert_eq!(results[0].evidence.expect_matched, Some(false));
        let m = results[0].evidence.message.as_deref().unwrap_or_default();
        assert!(m.contains("exited 0"), "message was {m:?}");
        assert!(m.contains("exit 0 alone is not proof"), "message was {m:?}");
    }

    #[tokio::test]
    async fn exit_zero_with_a_met_expect_passes() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo 'test result: ok. 412 passed'".to_string(),
            cwd: None,
            expect: Some("412 passed".to_string()),
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[0].evidence.expect_matched, Some(true));
        assert!(results[0].evidence.message.is_none());
    }

    /// stderr counts: cargo prints test totals to stdout, npm and rustc to
    /// stderr. An `expect` that only watched one stream would false-fail half
    /// the ecosystem.
    #[tokio::test]
    async fn expect_matches_against_stderr_too() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo 'Compiling permagent' 1>&2".to_string(),
            cwd: None,
            expect: Some("Compiling permagent".to_string()),
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
    }

    #[tokio::test]
    async fn expect_supports_slash_wrapped_regex() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![
            CompletionCheck::CommandExitZero {
                cmd: "echo '412 passed; 0 failed'".to_string(),
                cwd: None,
                expect: Some(r"/\d+ passed; 0 failed/".to_string()),
                timeout_secs: 30,
            },
            CompletionCheck::CommandExitZero {
                cmd: "echo '412 passed; 3 failed'".to_string(),
                cwd: None,
                expect: Some(r"/\d+ passed; 0 failed/".to_string()),
                timeout_secs: 30,
            },
        ];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[1].status, CheckStatus::Fail);
    }

    /// A broken assertion is an `error`, never a silent pass.
    #[tokio::test]
    async fn an_unparseable_expect_regex_errors() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo hi".to_string(),
            cwd: None,
            expect: Some("/[unclosed/".to_string()),
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert!(results[0]
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("invalid expect regex"));
    }

    /// Negative control: an `expect`-less check behaves EXACTLY as before —
    /// this field must be inert for every card written before it existed.
    #[tokio::test]
    async fn an_expectless_check_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo anything at all".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[0].evidence.expect_matched, None);
    }

    /// A failing command with an unmet expect is still a fail, and says both.
    #[tokio::test]
    async fn nonzero_exit_with_missed_expect_reports_both_halves() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "echo nope; exit 1".to_string(),
            cwd: None,
            expect: Some("all good".to_string()),
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Fail);
        let m = results[0].evidence.message.as_deref().unwrap_or_default();
        assert!(m.contains("command failed AND"), "message was {m:?}");
    }

    #[test]
    fn expect_field_is_optional_on_the_wire_and_round_trips() {
        // Pre-existing cards (no `expect`) still parse under
        // `deny_unknown_fields`.
        let old = parse_checks(r#"[{"type":"command_exit_zero","cmd":"cargo check"}]"#).unwrap();
        assert!(matches!(
            old[0],
            CompletionCheck::CommandExitZero { expect: None, .. }
        ));
        let with = parse_checks(
            r#"[{"type":"command_exit_zero","cmd":"cargo test","expect":"412 passed"}]"#,
        )
        .unwrap();
        assert!(matches!(
            with[0],
            CompletionCheck::CommandExitZero { expect: Some(ref e), .. } if e == "412 passed"
        ));
        // An absent `expect` is not serialized back out.
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("expect"), "{json}");
    }

    #[test]
    fn expect_matches_handles_substring_and_regex_forms() {
        assert!(expect_matches("passed", "412 passed").unwrap());
        assert!(!expect_matches("passed", "412 failed").unwrap());
        assert!(expect_matches(r"/\d+ passed/", "412 passed").unwrap());
        // A lone slash is not a regex delimiter pair.
        assert!(expect_matches("/", "a/b").unwrap());
        assert!(expect_matches(r"/^ok$/", "ok").unwrap());
        assert!(expect_matches("/[bad/", "x").is_err());
    }

    #[tokio::test]
    async fn exit_126_is_environment_error_not_a_fail_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "exit 126".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert_eq!(results[0].evidence.exit_code, Some(126));
    }

    #[tokio::test]
    async fn exit_1_still_fails_after_environment_exit_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "exit 1".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Fail);
        assert_eq!(results[0].evidence.exit_code, Some(1));
    }

    /// A signalled check has NO exit code, and cannot be graded as a verdict on
    /// the diff. It must also SAY so: an `error` row whose only evidence is a
    /// missing exit code explains nothing to the verifier reading it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_signalled_check_is_unprovable_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            // Two things matter here and both were learned from CI.
            //
            // SIGKILL, not SIGTERM: a shell may CATCH a term and exit 143
            // rather than die by it, which is what Linux `sh` does. SIGKILL
            // cannot be caught or converted to an exit status by any shell.
            //
            // And no nested `sh -c`: run_checks already runs this through
            // `/bin/sh -c`, so wrapping it again had the inner shell kill
            // itself while the OUTER one survived and exited 137 — an ordinary
            // exit code, which is precisely the case the next test covers.
            // macOS `sh` execs the final simple command, so the two were one
            // process and it passed there; dash on ubuntu forked, and it
            // failed. Killing the shell run_checks itself spawned is
            // unambiguous on every unix, and is the shape an OOM-killed check
            // has anyway.
            cmd: "kill -KILL $$".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert_eq!(results[0].evidence.exit_code, None);
        let message = results[0].evidence.message.as_deref().unwrap_or_default();
        assert!(message.contains("signal 9"), "message was {message:?}");
        assert!(message.contains("cannot be told apart"));
    }

    /// The narrowness guard for the case above: when the shell SURVIVES its
    /// signalled child it exits 128+N normally, and that is an ordinary failing
    /// exit code. Without this, "signalled means unprovable" would spread to
    /// every check whose child dies — laundering real failures into `error`.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_surviving_shell_reporting_128_plus_n_still_fails() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            // The trailing commands stop `sh` from exec'ing, so it lives to
            // report its child's 128+15.
            cmd: "sh -c 'kill -TERM $$'; rc=$?; exit $rc".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Fail);
        assert_eq!(results[0].evidence.exit_code, Some(143));
    }

    #[tokio::test]
    async fn command_timeout_is_error_not_pass() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "sleep 30".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 1,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert!(results[0]
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("timed out"));
    }

    #[tokio::test]
    async fn command_cwd_escape_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let checks = vec![
            CompletionCheck::CommandExitZero {
                cmd: "true".to_string(),
                cwd: Some("../..".to_string()),
                expect: None,
                timeout_secs: 10,
            },
            CompletionCheck::CommandExitZero {
                cmd: "true".to_string(),
                cwd: Some("/etc".to_string()),
                expect: None,
                timeout_secs: 10,
            },
        ];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Error);
        assert!(results[0]
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("escapes working_dir"));
        assert_eq!(results[1].status, CheckStatus::Error);
        assert!(results[1]
            .evidence
            .message
            .as_deref()
            .unwrap()
            .contains("absolute path"));
    }

    #[tokio::test]
    async fn command_symlink_escape_rejected() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("sneaky");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            let checks = vec![CompletionCheck::CommandExitZero {
                cmd: "true".to_string(),
                cwd: Some("sneaky".to_string()),
                expect: None,
                timeout_secs: 10,
            }];
            let results = run_checks(
                &checks,
                dir.path(),
                &mut CheckGate::user_authored(dir.path()),
            )
            .await;
            assert_eq!(results[0].status, CheckStatus::Error);
            assert!(results[0]
                .evidence
                .message
                .as_deref()
                .unwrap()
                .contains("outside working_dir"));
        }
    }

    #[tokio::test]
    async fn file_exists_pass_and_fail() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("present.txt"), "x").unwrap();
        let checks = vec![
            CompletionCheck::FileExists {
                path: "present.txt".to_string(),
            },
            CompletionCheck::FileExists {
                path: "missing.txt".to_string(),
            },
            CompletionCheck::FileExists {
                path: "../escape.txt".to_string(),
            },
        ];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[1].status, CheckStatus::Fail);
        assert_eq!(results[2].status, CheckStatus::Error);
    }

    #[tokio::test]
    async fn grep_absent_pass_fail_and_missing_file_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("clean.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join("dirty.rs"),
            "// TODO: fix this\nfn x() {}\n",
        )
        .unwrap();
        let checks = vec![
            CompletionCheck::GrepAbsent {
                pattern: "TODO".to_string(),
                paths: vec!["clean.rs".to_string()],
            },
            CompletionCheck::GrepAbsent {
                pattern: "TODO".to_string(),
                paths: vec!["clean.rs".to_string(), "dirty.rs".to_string()],
            },
            CompletionCheck::GrepAbsent {
                pattern: "TODO".to_string(),
                paths: vec!["nonexistent.rs".to_string()],
            },
        ];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[1].status, CheckStatus::Fail);
        let m = results[1].evidence.matches.as_ref().unwrap();
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("dirty.rs:1:"));
        assert_eq!(results[2].status, CheckStatus::Error);
    }

    #[tokio::test]
    async fn output_tail_cap_sets_truncated_flag() {
        let dir = tempfile::tempdir().unwrap();
        // Emit > 16KiB to stdout.
        let checks = vec![CompletionCheck::CommandExitZero {
            cmd: "yes x | head -c 40000".to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 30,
        }];
        let results = run_checks(
            &checks,
            dir.path(),
            &mut CheckGate::user_authored(dir.path()),
        )
        .await;
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert!(results[0].truncated);
        assert!(results[0].evidence.stdout_tail.as_ref().unwrap().len() <= MAX_TAIL_BYTES);
    }

    #[test]
    fn timeout_clamp_bounds() {
        // Clamp is applied at execution; verify the clamp math used there.
        assert_eq!(0u64.clamp(1, 600), 1);
        assert_eq!(120u64.clamp(1, 600), 120);
        assert_eq!(10_000u64.clamp(1, 600), 600);
    }
}
