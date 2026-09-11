//! Write jail (C3): file writes/edits OUTSIDE the session's working directory
//! require user confirmation — routed through the same Decision-Inbox seam as
//! every other tool approval (#760), so a jailed write is an answerable card,
//! never a silent refusal or a hang.
//!
//! Confirmation, NOT a hard wall: the coding harness legitimately writes
//! absolute paths (its own worktrees, temp scratch), so the jail ALLOWS
//! without asking:
//!   - anything under the session's working directory (relative paths resolve
//!     there — the same `resolve_path` the developer tools use);
//!   - temp directories (`std::env::temp_dir()`, `/tmp`, `/var/folders`);
//!   - the goal-engine worktree namespace (any path containing a
//!     `.permagent-goal-worktrees` component — worker sessions also run WITH
//!     their worktree as working dir, so this mainly covers the supervising
//!     session reaching into a worker's tree);
//!   - paths the user names in config (`SECURITY_WRITE_JAIL_ALLOW`: a YAML
//!     list or a comma-separated string of prefixes).
//!
//! `SECURITY_WRITE_JAIL_ENABLED: false` turns the jail off entirely (explicit
//! config over hidden defaults). If the session's working directory cannot be
//! resolved, the jail FAILS OPEN (allows, with a warning log): a jail that
//! cannot see its own boundary must not take the agent down with it.
//!
//! ## Lane Y9: a dispatched goal worker gets a HARD jail, not a confirmation
//!
//! One session type cannot use the confirmation semantics above: an internal
//! goal worker (a `SubAgent` session whose working dir is its own
//! `.permagent-goal-worktrees/<run_id>` worktree, repointed there by Lane Y3's
//! `goal_engine::point_session_at_worktree`). For it, "outside the working
//! directory" means the project checkout its goal will later be landed into,
//! and `RequireApproval` resolves to neither a clean allow nor a clean deny:
//!   - in-process there is no approver at all — a worker `Agent` is never
//!     `set_headless(true)` (only `scheduler.rs` does that), and
//!     `subagent_handler::get_agent_messages` never answers a confirmation, so
//!     the park in `tool_execution::handle_approval_tool_requests` waits on a
//!     oneshot nobody holds;
//!   - on the CLI's own non-interactive surface the park is auto-ALLOWED
//!     (`goose-cli/src/session/mod.rs`: "Tool confirmation required in
//!     non-interactive mode, auto-allowing" → `Permission::AllowOnce`).
//!
//! And `shell` was never inspected here at all, so `git -C <trunk> commit`,
//! `cd <trunk> && …` and `sed -i <trunk>/f` bypassed the jail outright. Lane
//! A14's M14 forensics measured the consequences on 4 of 8 zai runs: two
//! workers committed straight onto `main`, two left the fix uncommitted in the
//! trunk working tree (which the oracle then graded), and one produced a
//! byte-identical duplicate write whose dirty trunk made auto-land refuse with
//! "the project root has uncommitted changes".
//!
//! So for a goal-worker session this inspector returns [`InspectionAction::Deny`]
//! with a reason that NAMES the worktree (Y4c delivers an inspector's reason
//! verbatim to the model, so it can correct itself and retry inside its own
//! tree), for:
//!   - `write`/`edit`/`text_editor`/`str_replace_editor` targets outside the
//!     worktree — including another goal's worktree, so the namespace
//!     allowlist above does NOT apply to a worker;
//!   - `shell` commands that escape it structurally: `cd`/`pushd` out,
//!     `git -C`/`--git-dir`/`--work-tree` at an outside repo with a mutating
//!     subcommand, an output redirection to an outside path, or a mutating
//!     file command (`rm`, `cp`, `mv`, `tee`, `sed -i`, …) with an outside
//!     target.
//! Reads stay unrestricted everywhere, temp scratch and
//! `SECURITY_WRITE_JAIL_ALLOW` keep working, and every other session — the
//! coordinator included — keeps the confirmation semantics unchanged.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::config::{Config, GooseMode};
use crate::conversation::message::{Message, ToolRequest};
use crate::session::{Session, SessionManager, SessionType};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

pub const WRITE_JAIL_INSPECTOR_NAME: &str = "write_jail";

/// Directory component that marks the goal-engine worktree namespace (see
/// `goal_engine::GOAL_WORKTREES_DIR`).
const GOAL_WORKTREES_COMPONENT: &str = ".permagent-goal-worktrees";

/// Finding id on every Lane Y9 goal-worker refusal — the one string to grep
/// for when asking "did a worker try to leave its worktree, and was it
/// stopped".
pub const WORKER_JAIL_FINDING_ID: &str = "goal_worker_outside_worktree";

pub struct WriteJailInspector {
    session_manager: Option<Arc<SessionManager>>,
}

impl WriteJailInspector {
    pub fn new(session_manager: Option<Arc<SessionManager>>) -> Self {
        Self { session_manager }
    }

    /// The session row this jail's boundary is read from. Y9 needs the whole
    /// row, not just `working_dir`: `session_type` is half of the goal-worker
    /// predicate ([`jail_role`]).
    async fn session_row(&self, session_id: &str) -> Option<Session> {
        let manager = self.session_manager.as_ref()?;
        match manager.get_session(session_id, false).await {
            Ok(session) => Some(session),
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "write jail could not resolve the session working dir; failing open"
                );
                None
            }
        }
    }
}

fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "write" | "edit" | "text_editor" | "str_replace_editor"
    ) || name.ends_with("__write")
        || name.ends_with("__edit")
        || name.ends_with("__text_editor")
}

fn write_path(request: &ToolRequest) -> Option<String> {
    let tool_call = request.tool_call.as_ref().ok()?;
    if !is_write_tool(tool_call.name.as_ref()) {
        return None;
    }
    let args = tool_call.arguments.as_ref()?;
    args.get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Resolve `.`/`..` lexically, without touching the filesystem — so a
/// `../../..` escape is judged by where it LANDS, and a non-existent target
/// (the common case for a new file) can still be normalized.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonical form for containment checks: normalize lexically, then resolve
/// symlinks through the nearest EXISTING ancestor and re-append the rest.
/// Handles the macOS `/var` → `/private/var` symlink (temp dirs, tempdir-rooted
/// sessions) without requiring the target file to exist yet.
fn canonical_for_containment(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    let mut existing = normalized.as_path();
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if existing.exists() {
            let mut out =
                std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
            for part in remainder.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                remainder.push(name.to_os_string());
                existing = parent;
            }
            _ => return normalized,
        }
    }
}

fn has_goal_worktree_component(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == GOAL_WORKTREES_COMPONENT)
}

/// Built-in always-allowed roots: the temp directories agents and the coding
/// harness use as scratch space.
///
/// The character-device sinks (`/dev/null`, `/dev/fd/*`, …) are NOT here on
/// purpose: see [`is_device_sink`], which judges them on the RAW word because
/// canonicalizing `/dev/stdout` follows it to whatever this process's stdout
/// currently is — a path in some unrelated tree.
fn builtin_allowed_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        std::env::temp_dir(),
        canonical_for_containment(&std::env::temp_dir()),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/var/folders"),
        PathBuf::from("/private/var/folders"),
    ];
    roots.dedup();
    roots
}

/// User-configured allowlist: `SECURITY_WRITE_JAIL_ALLOW` as a YAML/JSON list
/// of paths, or a single comma-separated string.
fn configured_allowed_roots() -> Vec<PathBuf> {
    let config = Config::global();
    if let Ok(paths) = config.get_param::<Vec<String>>("SECURITY_WRITE_JAIL_ALLOW") {
        return paths.into_iter().map(PathBuf::from).collect();
    }
    if let Ok(joined) = config.get_param::<String>("SECURITY_WRITE_JAIL_ALLOW") {
        return joined
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    Vec::new()
}

/// Is `candidate` (already canonicalized) under a root this jail allows
/// without asking, regardless of session? Temp scratch and the operator's own
/// `SECURITY_WRITE_JAIL_ALLOW` prefixes — the two exceptions a goal worker
/// keeps too (Y9), unlike the goal-worktree namespace.
fn universally_allowlisted(candidate: &Path) -> Option<&'static str> {
    for allowed in builtin_allowed_roots() {
        if candidate.starts_with(&allowed) {
            return Some("temp directory");
        }
    }
    for allowed in configured_allowed_roots() {
        if candidate.starts_with(canonical_for_containment(&allowed)) {
            return Some("configured allowlist");
        }
    }
    None
}

/// The jail decision for one write target. `raw_path` is the tool argument
/// verbatim; relative paths resolve against `working_dir` exactly as the
/// developer tools resolve them at execution time.
///
/// Y9: `role` decides only what an OUTSIDE verdict means downstream — a
/// confirmation for an ordinary session, a refusal for a goal worker — plus
/// one boundary difference the worker needs: the `.permagent-goal-worktrees`
/// namespace allowlist is for a SUPERVISING session reaching into a worker's
/// tree, so it must not let one worker write into another worker's tree.
fn jail_verdict(raw_path: &str, working_dir: &Path, role: &JailRole) -> JailVerdict {
    let resolved = crate::agents::platform_extensions::developer::edit::resolve_path(
        raw_path,
        Some(working_dir),
    );
    let candidate = canonical_for_containment(&resolved);
    let root = canonical_for_containment(role.containment_root(working_dir));

    if candidate.starts_with(&root) {
        return JailVerdict::Inside;
    }
    if matches!(role, JailRole::Confirm) && has_goal_worktree_component(&candidate) {
        return JailVerdict::Allowlisted("goal worktree namespace");
    }
    if let Some(kind) = universally_allowlisted(&candidate) {
        return JailVerdict::Allowlisted(kind);
    }
    JailVerdict::Outside {
        resolved: candidate,
    }
}

enum JailVerdict {
    Inside,
    Allowlisted(&'static str),
    Outside { resolved: PathBuf },
}

// ── Lane Y9: the dispatched goal worker's hard jail ───────────────────────

/// What this session's jail boundary MEANS (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
enum JailRole {
    /// Every ordinary session, the coding-harness coordinator included: a
    /// write outside the working dir is a CONFIRMATION, exactly as C3 shipped.
    Confirm,
    /// A dispatched goal worker, jailed to the worktree it was given: an
    /// outside write — or a shell command that structurally escapes — is
    /// REFUSED, with a reason naming `worktree`.
    GoalWorker { worktree: PathBuf },
}

impl JailRole {
    /// The directory writes must stay under. For a worker this is its worktree
    /// root even if its session sits in a subdirectory of it; for everyone else
    /// it is the session's working dir, unchanged.
    fn containment_root<'a>(&'a self, working_dir: &'a Path) -> &'a Path {
        match self {
            JailRole::Confirm => working_dir,
            JailRole::GoalWorker { worktree } => worktree.as_path(),
        }
    }
}

/// `<…>/.permagent-goal-worktrees/<run_id>` — the goal worktree ROOT that
/// `path` sits in, or `None` when `path` is not in the worktree namespace.
///
/// Truncating at the run-id component (rather than trusting `path` itself to
/// be the root) means a worker whose cwd is a subdirectory of its worktree is
/// still jailed to the whole worktree, and a path that names only the
/// namespace directory itself (`…/.permagent-goal-worktrees`) yields `None`
/// rather than a root that would contain every worker's tree.
fn goal_worktree_root(path: &Path) -> Option<PathBuf> {
    let mut root = PathBuf::new();
    let mut components = path.components();
    let mut found_namespace = false;
    for component in components.by_ref() {
        root.push(component.as_os_str());
        if component.as_os_str() == GOAL_WORKTREES_COMPONENT {
            found_namespace = true;
            break;
        }
    }
    if !found_namespace {
        return None;
    }
    let run_id = components.next()?;
    root.push(run_id.as_os_str());
    Some(root)
}

/// A session is a dispatched goal worker when it is a `SubAgent` AND it is
/// running inside a goal worktree. Both halves are required: `SubAgent` alone
/// is every summoned child (they legitimately work in the project root), and a
/// worktree path alone is any session a user opened there by hand.
///
/// The predicate reads the same `sessions.working_dir` column Lane Y3 repoints
/// at dispatch, so there is no second source of truth to drift from.
fn jail_role(session: &Session) -> JailRole {
    if !matches!(session.session_type, SessionType::SubAgent) {
        return JailRole::Confirm;
    }
    match goal_worktree_root(&session.working_dir) {
        Some(worktree) => JailRole::GoalWorker { worktree },
        None => JailRole::Confirm,
    }
}

/// The refusal a goal worker gets, in the Y4c shape: say who is refusing, name
/// the ONE writable root, say what to do instead, and say why it matters to
/// the worker's own goal (a dirty trunk blocks its own land).
fn worker_denial_reason(what: &str, target: &Path, worktree: &Path) -> String {
    format!(
        "Refused by the goal-worker write jail. You are a dispatched goal worker and {worktree} \
         — the worktree your goal/<id> branch is checked out in — is the ONLY directory you may \
         write in. {what} resolves to {target}, which is outside it. Reading anywhere is fine; \
         creating, editing, deleting or committing files is not. The project checkout is \
         especially off limits: a stray write there leaves it dirty and BLOCKS your own goal from \
         landing (auto-land refuses with \"the project root has uncommitted changes\"), and a \
         commit made there bypasses review entirely. Redo this inside {worktree} — relative paths \
         and the shell's working directory already resolve there — and commit on your goal branch \
         in that worktree.",
        worktree = worktree.display(),
        target = target.display(),
        what = what,
    )
}

// ── Lane Y9: shell confinement for a goal worker ──────────────────────────
//
// SCOPE, stated up front: this is a STRUCTURAL escape detector over a
// shallowly tokenized command, not a shell parser and not a sandbox. It
// refuses the four shapes that actually produced M12/M14's bypasses — a cwd
// change out of the worktree, a `git` pointed at another repo with a mutating
// subcommand, an output redirection to an outside path, and a mutating file
// command with an outside target. It deliberately does NOT try to judge
// arbitrary absolute paths appearing anywhere in a command line (that would
// refuse ordinary READS, e.g. `cat <trunk>/README.md`), and a command that
// hides its target behind a variable, a heredoc body, an `eval`, or an
// interpreter one-liner still gets through. The real containment for a
// hostile process is an OS sandbox; this is the correctness guard that keeps a
// COOPERATIVE model — which is what a dispatched worker is — inside its tree,
// and tells it why when it strays.

fn is_shell_tool(name: &str) -> bool {
    let base = name.rsplit("__").next().unwrap_or(name);
    matches!(
        base,
        "shell" | "bash" | "execute_command" | "run_command" | "terminal"
    )
}

fn shell_command(request: &ToolRequest) -> Option<String> {
    let tool_call = request.tool_call.as_ref().ok()?;
    if !is_shell_tool(tool_call.name.as_ref()) {
        return None;
    }
    let args = tool_call.arguments.as_ref()?;
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// One shallow shell token.
#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    /// Anything that ends the current command: `;`, `&&`, `||`, `|`, `&`,
    /// newline, `(`/`)`, and (for our purposes) an input redirection.
    Sep,
    /// An OUTPUT redirection (`>`, `>>`, `2>`, `&>`): the next word is a file
    /// the command writes to.
    Redirect,
}

fn push_word(word: &mut String, out: &mut Vec<Tok>) {
    if !word.is_empty() {
        out.push(Tok::Word(std::mem::take(word)));
    }
}

/// Split a command line into words, separators and output redirections.
/// Quotes and backslash escapes are honored so a quoted path stays one token;
/// everything else (expansion, heredoc bodies, subshell semantics) is left
/// alone — see the SCOPE note above.
fn tokenize(command: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut word = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            // A backslash escapes the next character (a line continuation
            // escapes the newline away entirely).
            '\\' => match chars.next() {
                None | Some('\n') => {}
                Some(next) => word.push(next),
            },
            ';' | '\n' | '(' | ')' => {
                push_word(&mut word, &mut out);
                out.push(Tok::Sep);
            }
            '&' => {
                push_word(&mut word, &mut out);
                if chars.peek() == Some(&'&') {
                    chars.next();
                    out.push(Tok::Sep);
                } else if chars.peek() == Some(&'>') {
                    chars.next();
                    if chars.peek() == Some(&'>') {
                        chars.next();
                    }
                    out.push(Tok::Redirect);
                } else {
                    out.push(Tok::Sep);
                }
            }
            '|' => {
                push_word(&mut word, &mut out);
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                out.push(Tok::Sep);
            }
            '<' => {
                // Input redirection and heredocs are reads: end the command,
                // and let whatever follows be ordinary words nothing matches.
                push_word(&mut word, &mut out);
                while chars.peek() == Some(&'<') {
                    chars.next();
                }
                out.push(Tok::Sep);
            }
            '>' => {
                // A leading file descriptor (`2>`) belongs to the operator.
                if word == "1" || word == "2" {
                    word.clear();
                }
                push_word(&mut word, &mut out);
                if chars.peek() == Some(&'>') {
                    chars.next();
                }
                if chars.peek() == Some(&'&') {
                    // `>&2` duplicates a descriptor; there is no file target.
                    chars.next();
                    out.push(Tok::Sep);
                } else {
                    out.push(Tok::Redirect);
                }
            }
            c if c.is_whitespace() => push_word(&mut word, &mut out),
            _ => word.push(c),
        }
    }
    push_word(&mut word, &mut out);
    out
}

/// One command in a pipeline/list: its words, plus the files it redirects
/// output into.
#[derive(Debug, Default)]
struct Segment {
    words: Vec<String>,
    redirects: Vec<String>,
}

fn segments(tokens: Vec<Tok>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut current = Segment::default();
    let mut redirect_next = false;
    for token in tokens {
        match token {
            Tok::Sep => {
                redirect_next = false;
                if !current.words.is_empty() || !current.redirects.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            Tok::Redirect => redirect_next = true,
            Tok::Word(w) => {
                if redirect_next {
                    redirect_next = false;
                    current.redirects.push(w);
                } else {
                    current.words.push(w);
                }
            }
        }
    }
    if !current.words.is_empty() || !current.redirects.is_empty() {
        out.push(current);
    }
    out
}

/// `git` subcommands that CHANGE a repository. `git -C <other-repo> log` stays
/// allowed — reads are never the harm, and refusing them would cost a turn for
/// nothing.
const MUTATING_GIT_SUBCOMMANDS: &[&str] = &[
    "add",
    "am",
    "apply",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "commit-tree",
    "config",
    "fetch",
    "filter-branch",
    "gc",
    "init",
    "merge",
    "mv",
    "notes",
    "prune",
    "pull",
    "push",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "submodule",
    "switch",
    "tag",
    "update-ref",
    "worktree",
    "write-tree",
];

/// Commands where EVERY non-flag argument is a file they may modify.
const MUTATES_EVERY_ARG: &[&str] = &[
    "rm", "rmdir", "mkdir", "touch", "truncate", "tee", "chmod", "chown", "shred", "unlink",
    "patch",
];

/// Commands where the LAST non-flag argument is the destination they write
/// (the earlier ones are sources they only read).
const MUTATES_LAST_ARG: &[&str] = &["cp", "mv", "ln", "install", "rsync"];

/// A structural escape out of the worker's worktree.
struct ShellEscape {
    /// Y4c-style description of the offending construct, e.g. "The `cd` target".
    what: String,
    target: PathBuf,
}

/// Character-device sinks a shell redirects to. These are not files in any
/// tree — `cmd > /dev/null`, `cmd 2>/dev/null`, `… >/dev/null 2>&1` are in
/// practically every build/test/lint invocation a worker runs, and refusing
/// them would be a guaranteed false denial (G13 H1).
///
/// Matched on the RAW word, before canonicalization, deliberately:
/// `/dev/stdout` is a symlink to `/dev/fd/1` and `/dev/fd/1` to whatever this
/// process's stdout currently is, so canonicalizing the word yields a path in
/// an unrelated tree and the result would depend on how the daemon was
/// launched. An allowlist ROOT cannot express that; a raw-word test can.
fn is_device_sink(raw: &str) -> bool {
    matches!(raw, "/dev/null" | "/dev/zero" | "/dev/stdout" | "/dev/stderr")
        || raw.starts_with("/dev/fd/")
        // `/dev/tty`, and the `/dev/ttys003` a terminal session is attached to.
        || raw.starts_with("/dev/tty")
}

/// Resolve a shell word as a path against the session's cwd, expanding a
/// leading `~`. Returns `None` for words that cannot be judged as paths:
/// flags, device sinks, and anything carrying an unexpanded `$`.
fn shell_path_candidate(raw: &str, cwd: &Path) -> Option<PathBuf> {
    if raw.is_empty() || raw.starts_with('-') || raw.contains('$') || raw.contains('`') {
        return None;
    }
    if is_device_sink(raw) {
        return None;
    }
    let expanded = match raw.strip_prefix('~') {
        Some(rest) => {
            let home = std::env::var("HOME").ok()?;
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            Path::new(&home).join(rest)
        }
        None => crate::agents::platform_extensions::developer::edit::resolve_path(raw, Some(cwd)),
    };
    Some(canonical_for_containment(&expanded))
}

/// `Some(target)` when `raw` — resolved against the session's cwd — lands
/// outside the jail `root` and outside every allowlisted root.
fn outside_worktree(raw: &str, cwd: &Path, root: &Path) -> Option<PathBuf> {
    let candidate = shell_path_candidate(raw, cwd)?;
    if candidate.starts_with(root) || universally_allowlisted(&candidate).is_some() {
        return None;
    }
    Some(candidate)
}

/// Strip a leading path from a command word so `/bin/rm` is recognized as
/// `rm`.
fn command_base_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The first structural escape in `command`, if any. See the SCOPE note.
///
/// `cwd` is the session's ACTUAL working directory — the directory the shell
/// will run in, so the directory a relative word resolves against — and
/// `worktree` is the jail root that word is then tested for containment in.
/// They differ whenever a worker's session sits in a SUBDIRECTORY of its
/// worktree, which `goal_worktree_root` explicitly supports: resolving
/// `rm ../build/stale` against the ROOT would compute `<root>/..` and refuse a
/// path that is in fact inside the tree (G13 M4).
fn shell_escape(command: &str, cwd: &Path, worktree: &Path) -> Option<ShellEscape> {
    let root = canonical_for_containment(worktree);
    for segment in segments(tokenize(command)) {
        for redirect in &segment.redirects {
            if let Some(target) = outside_worktree(redirect, cwd, &root) {
                return Some(ShellEscape {
                    what: format!("The output redirection `> {redirect}`"),
                    target,
                });
            }
        }
        let Some(command_word) = segment.words.first() else {
            continue;
        };
        let args = &segment.words[1..];
        match command_base_name(command_word) {
            "cd" | "pushd" => {
                // A bare `cd` goes to $HOME, which is outside by construction.
                let raw = args
                    .iter()
                    .find(|a| !a.starts_with('-'))
                    .cloned()
                    .unwrap_or_else(|| "~".to_string());
                if let Some(target) = outside_worktree(&raw, cwd, &root) {
                    return Some(ShellEscape {
                        what: format!("The `cd {raw}`"),
                        target,
                    });
                }
            }
            "git" => {
                if let Some(escape) = git_escape(args, cwd, &root) {
                    return Some(escape);
                }
            }
            // In-place edit only: `sed 's/a/b/' <outside>` merely READS it.
            "sed" if args.iter().any(|a| a.starts_with("-i")) => {
                if let Some(escape) = first_outside_arg(args, "`sed -i`", cwd, &root, false) {
                    return Some(escape);
                }
            }
            base if MUTATES_EVERY_ARG.contains(&base) => {
                if let Some(escape) =
                    first_outside_arg(args, &format!("`{base}`"), cwd, &root, false)
                {
                    return Some(escape);
                }
            }
            base if MUTATES_LAST_ARG.contains(&base) => {
                if let Some(escape) =
                    first_outside_arg(args, &format!("`{base}`"), cwd, &root, true)
                {
                    return Some(escape);
                }
            }
            _ => {}
        }
    }
    None
}

/// `git -C <dir>` / `--git-dir=` / `--work-tree=` pointed outside the worktree,
/// with a subcommand that would CHANGE that repository.
fn git_escape(args: &[String], cwd: &Path, root: &Path) -> Option<ShellEscape> {
    let mut elsewhere: Option<(String, PathBuf)> = None;
    let mut subcommand: Option<&str> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        let flagged: Option<(&str, String)> = if arg == "-C" {
            args.get(index + 1).map(|v| ("-C", v.clone()))
        } else if let Some(value) = arg.strip_prefix("--git-dir=") {
            Some(("--git-dir", value.to_string()))
        } else if let Some(value) = arg.strip_prefix("--work-tree=") {
            Some(("--work-tree", value.to_string()))
        } else if arg == "--git-dir" || arg == "--work-tree" {
            args.get(index + 1).map(|v| (arg, v.clone()))
        } else {
            None
        };
        if let Some((flag, raw)) = flagged {
            if elsewhere.is_none() {
                elsewhere = outside_worktree(&raw, cwd, root)
                    .map(|target| (format!("`git {flag} {raw}`"), target));
            }
            index += if arg.contains('=') { 1 } else { 2 };
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        subcommand = Some(arg);
        break;
    }
    let (what, target) = elsewhere?;
    let subcommand = subcommand?;
    if !MUTATING_GIT_SUBCOMMANDS.contains(&subcommand) {
        return None;
    }
    Some(ShellEscape {
        what: format!("{what} (a `{subcommand}` changes that repository)"),
        target,
    })
}

/// First argument that resolves outside the worktree. `last_only` checks just
/// the destination argument (`cp src dst` reads `src` — only `dst` is written).
fn first_outside_arg(
    args: &[String],
    what: &str,
    cwd: &Path,
    root: &Path,
    last_only: bool,
) -> Option<ShellEscape> {
    let operands: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let considered: Vec<&String> = if last_only {
        operands.last().into_iter().copied().collect()
    } else {
        operands
    };
    for raw in considered {
        if let Some(target) = outside_worktree(raw, cwd, root) {
            return Some(ShellEscape {
                what: format!("{what}'s target `{raw}`"),
                target,
            });
        }
    }
    None
}

#[async_trait]
impl ToolInspector for WriteJailInspector {
    fn name(&self) -> &'static str {
        WRITE_JAIL_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_enabled(&self) -> bool {
        Config::global()
            .get_param::<bool>("SECURITY_WRITE_JAIL_ENABLED")
            .unwrap_or(true)
    }

    async fn inspect(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>> {
        // Resolve the session once per batch — and only when the batch
        // actually contains something this jail judges, so read-only turns
        // cost nothing. Y9 adds `shell` to that set, because a goal worker's
        // shell is the one tool that can both write into and commit onto the
        // project checkout.
        let has_write = tool_requests.iter().any(|r| write_path(r).is_some());
        let has_shell = tool_requests.iter().any(|r| shell_command(r).is_some());
        if !has_write && !has_shell {
            return Ok(Vec::new());
        }
        let Some(session) = self.session_row(session_id).await else {
            // Fail open: no boundary to enforce (see module docs).
            return Ok(Vec::new());
        };
        let working_dir = session.working_dir.clone();
        let role = jail_role(&session);
        // Nothing to do for an ordinary session with no write in the batch:
        // shell confinement is the goal worker's rule only.
        if !has_write && matches!(role, JailRole::Confirm) {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for request in tool_requests {
            if let Some(raw_path) = write_path(request) {
                match jail_verdict(&raw_path, &working_dir, &role) {
                    JailVerdict::Inside => {}
                    JailVerdict::Allowlisted(kind) => {
                        tracing::debug!(
                            tool_request_id = %request.id,
                            path = %raw_path,
                            allowlist = kind,
                            "write jail: allowlisted write outside the working dir"
                        );
                    }
                    JailVerdict::Outside { resolved } => match &role {
                        JailRole::GoalWorker { worktree } => {
                            tracing::info!(
                                monotonic_counter.goose.write_jail_worker_denials = 1,
                                tool_request_id = %request.id,
                                path = %resolved.display(),
                                worktree = %worktree.display(),
                                "write jail: goal worker refused a write outside its worktree"
                            );
                            results.push(InspectionResult {
                                tool_request_id: request.id.clone(),
                                action: InspectionAction::Deny,
                                reason: worker_denial_reason(
                                    "The write target",
                                    &resolved,
                                    worktree,
                                ),
                                confidence: 1.0,
                                inspector_name: self.name().to_string(),
                                finding_id: Some(WORKER_JAIL_FINDING_ID.to_string()),
                            });
                        }
                        JailRole::Confirm => {
                            tracing::info!(
                                monotonic_counter.goose.write_jail_confirmations = 1,
                                tool_request_id = %request.id,
                                path = %resolved.display(),
                                working_dir = %working_dir.display(),
                                "write jail: write outside the session working dir requires approval"
                            );
                            results.push(InspectionResult {
                                tool_request_id: request.id.clone(),
                                action: InspectionAction::RequireApproval(Some(format!(
                                    "🔒 Write outside the working directory\n\n\
                                     This session works in {} but the agent wants to write to {}. \
                                     Approve if you asked for this; deny otherwise. \
                                     (Standing exceptions can be added via SECURITY_WRITE_JAIL_ALLOW.)",
                                    working_dir.display(),
                                    resolved.display()
                                ))),
                                reason: format!(
                                    "Write target {} is outside the session working directory {}",
                                    resolved.display(),
                                    working_dir.display()
                                ),
                                confidence: 1.0,
                                inspector_name: self.name().to_string(),
                                finding_id: None,
                            });
                        }
                    },
                }
                continue;
            }

            let JailRole::GoalWorker { worktree } = &role else {
                continue;
            };
            let Some(command) = shell_command(request) else {
                continue;
            };
            if let Some(escape) = shell_escape(&command, &working_dir, worktree) {
                tracing::info!(
                    monotonic_counter.goose.write_jail_worker_denials = 1,
                    tool_request_id = %request.id,
                    path = %escape.target.display(),
                    worktree = %worktree.display(),
                    "write jail: goal worker refused a shell command that leaves its worktree"
                );
                results.push(InspectionResult {
                    tool_request_id: request.id.clone(),
                    action: InspectionAction::Deny,
                    reason: worker_denial_reason(&escape.what, &escape.target, worktree),
                    confidence: 1.0,
                    inspector_name: self.name().to_string(),
                    finding_id: Some(WORKER_JAIL_FINDING_ID.to_string()),
                });
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;

    fn write_request(id: &str, tool: &str, path: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams::new(tool.to_string())
                .with_arguments(object!({"path": path, "content": "x"}))),
            metadata: None,
            tool_meta: None,
        }
    }

    async fn jail_with_session(tmp: &tempfile::TempDir) -> (WriteJailInspector, String, PathBuf) {
        let work = tmp.path().join("workspace");
        std::fs::create_dir_all(&work).unwrap();
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let session = session_manager
            .create_session(
                work.clone(),
                "write-jail test".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .expect("create session");
        (
            WriteJailInspector::new(Some(session_manager)),
            session.id,
            work,
        )
    }

    async fn verdicts(
        jail: &WriteJailInspector,
        session_id: &str,
        requests: &[ToolRequest],
    ) -> Vec<InspectionResult> {
        jail.inspect(session_id, requests, &[], GooseMode::Auto)
            .await
            .expect("inspect")
    }

    #[tokio::test]
    async fn writes_inside_the_working_dir_pass_silently() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, work) = jail_with_session(&tmp).await;

        let requests = vec![
            write_request("rel", "write", "src/main.rs"),
            write_request("rel-dot", "edit", "./notes/a.md"),
            write_request(
                "abs",
                "write",
                work.join("deep/nested/file.rs").to_str().unwrap(),
            ),
        ];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert!(
            results.is_empty(),
            "in-jail writes must not require approval: {:?}",
            results
        );
    }

    #[tokio::test]
    async fn absolute_write_outside_requires_approval_with_answerable_message() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, work) = jail_with_session(&tmp).await;

        let requests = vec![write_request(
            "outside",
            "write",
            "/definitely/not/allowed/c3.txt",
        )];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert_eq!(results.len(), 1);
        let InspectionAction::RequireApproval(Some(message)) = &results[0].action else {
            panic!(
                "outside write must require approval: {:?}",
                results[0].action
            );
        };
        assert!(
            message.contains("/definitely/not/allowed/c3.txt"),
            "{message}"
        );
        assert!(
            message.contains(&work.display().to_string())
                || message.contains(&canonical_for_containment(&work).display().to_string()),
            "message must name the working dir: {message}"
        );
        assert_eq!(results[0].inspector_name, WRITE_JAIL_INSPECTOR_NAME);
    }

    #[tokio::test]
    async fn dot_dot_escape_is_judged_by_where_it_lands() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        // Climb far above the working dir and back down into a foreign root.
        let requests = vec![write_request(
            "escape",
            "edit",
            "../../../../../../../../definitely/not/allowed/esc.txt",
        )];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert_eq!(results.len(), 1, "a ..-escape must require approval");
        assert!(matches!(
            results[0].action,
            InspectionAction::RequireApproval(Some(_))
        ));
    }

    #[tokio::test]
    async fn temp_dirs_and_goal_worktrees_are_allowlisted() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        let temp_target = std::env::temp_dir().join("c3-jail-scratch.txt");
        let requests = vec![
            write_request("temp", "write", temp_target.to_str().unwrap()),
            write_request(
                "worktree",
                "write",
                "/repos/.permagent-goal-worktrees/run-42/src/lib.rs",
            ),
        ];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert!(
            results.is_empty(),
            "temp + worktree writes are the coding harness's legitimate paths: {:?}",
            results
        );
    }

    #[tokio::test]
    async fn non_write_tools_and_missing_paths_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        let shell = ToolRequest {
            id: "shell".to_string(),
            tool_call: Ok(CallToolRequestParams::new("shell")
                .with_arguments(object!({"command": "echo hi > /definitely/not/allowed/x"}))),
            metadata: None,
            tool_meta: None,
        };
        let write_without_path = ToolRequest {
            id: "no-path".to_string(),
            tool_call: Ok(
                CallToolRequestParams::new("write").with_arguments(object!({"content": "orphan"}))
            ),
            metadata: None,
            tool_meta: None,
        };
        let results = verdicts(&jail, &session_id, &[shell, write_without_path]).await;
        assert!(results.is_empty(), "{:?}", results);
    }

    /// Lane Y3: an internal goal worker's session is created at the PRIMARY
    /// checkout (its id has to exist before a goal worktree branch name —
    /// `goal/<session_id>` — can be minted) and only afterward repointed at
    /// its resolved goal worktree
    /// (`agents::platform_extensions::goal_engine::point_session_at_worktree`).
    /// Once that repoint has happened, the jail's boundary must have moved
    /// with it: a write back into the primary checkout now requires
    /// approval, while a write inside the worker's own worktree passes
    /// silently. Before Y3's fix the session never got repointed, so this
    /// was inverted — the jail treated the primary checkout as "inside" and
    /// silently allowed the worker to write there.
    #[tokio::test]
    async fn worktree_repointed_session_treats_primary_checkout_as_outside() {
        let tmp = tempfile::tempdir().unwrap();
        // Deliberately NOT under the tempdir the harness itself runs in — the
        // jail's builtin allowlist treats anything under a system temp
        // directory as scratch space (see `temp_dirs_and_goal_worktrees_are_allowlisted`
        // above), which would mask the very containment boundary this test
        // exists to pin. Neither path needs to exist on disk:
        // `canonical_for_containment` walks up to the nearest existing
        // ancestor when the target doesn't exist yet.
        let primary = PathBuf::from("/repos/y3-primary-checkout");
        let worktree = PathBuf::from("/repos/.permagent-goal-worktrees/run-1");
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let session = session_manager
            .create_session(
                primary.clone(),
                "goal worker".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .expect("create session");

        // The Y3 repoint: same mechanism as
        // `goal_engine::point_session_at_worktree` — an `update(...).working_dir(...)`
        // call after the worktree is resolved.
        session_manager
            .update(&session.id)
            .working_dir(worktree.clone())
            .apply()
            .await
            .expect("repoint session at worktree");

        let jail = WriteJailInspector::new(Some(session_manager));

        let inside_worktree = write_request(
            "inside",
            "write",
            worktree.join("src/lib.rs").to_str().unwrap(),
        );
        let results = verdicts(&jail, &session.id, &[inside_worktree]).await;
        assert!(
            results.is_empty(),
            "a write inside the worker's own (repointed) worktree must pass silently: {:?}",
            results
        );

        let outside_to_primary = write_request(
            "outside",
            "write",
            primary.join("src/lib.rs").to_str().unwrap(),
        );
        let results = verdicts(&jail, &session.id, &[outside_to_primary]).await;
        assert_eq!(
            results.len(),
            1,
            "a write back into the PRIMARY checkout must be refused now \
             that the session is repointed at its worktree: {:?}",
            results
        );
        // Lane Y9 strengthened the ACTION for this exact session shape (a
        // `SubAgent` running in a goal worktree) from a confirmation to a
        // refusal — the boundary Y3 fixed is unchanged, but a confirmation had
        // no answerer in a worker (see the module docs), so it resolved to a
        // hang in-process and to an auto-ALLOW on the CLI's non-interactive
        // surface. Ordinary sessions still get `RequireApproval`
        // (`absolute_write_outside_requires_approval_with_answerable_message`).
        assert_eq!(results[0].action, InspectionAction::Deny);
    }

    #[tokio::test]
    async fn unknown_session_fails_open() {
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let jail = WriteJailInspector::new(Some(session_manager));

        let requests = vec![write_request("x", "write", "/definitely/not/allowed/y.txt")];
        let results = verdicts(&jail, "no-such-session", &requests).await;
        assert!(
            results.is_empty(),
            "a jail without a boundary must fail open, not strand the agent"
        );
    }

    /// `env_lock` guards the SECURITY_WRITE_JAIL_ALLOW env config (restored on
    /// drop, panic-safe, serialized against other env-locked tests).
    #[tokio::test]
    async fn config_allowlist_extends_the_jail_in_both_formats() {
        let _env =
            env_lock::lock_env([("SECURITY_WRITE_JAIL_ALLOW", Some(r#"["/opt/c3-allowed"]"#))]);
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        // JSON/YAML list form.
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("a", "write", "/opt/c3-allowed/deploy/x.txt")],
        )
        .await;
        assert!(results.is_empty(), "list-form allowlist: {:?}", results);

        // Comma-separated string form (mid-test flip of the guard-listed var).
        std::env::set_var("SECURITY_WRITE_JAIL_ALLOW", "/opt/c3-a, /opt/c3-b");
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("b", "edit", "/opt/c3-b/y.txt")],
        )
        .await;
        assert!(results.is_empty(), "string-form allowlist: {:?}", results);

        // A path outside the allowlist still parks.
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("c", "write", "/opt/c3-elsewhere/z.txt")],
        )
        .await;
        assert_eq!(results.len(), 1, "non-allowlisted must still confirm");
    }

    /// `env_lock` guards the SECURITY_WRITE_JAIL_ENABLED env config.
    #[tokio::test]
    async fn jail_defaults_on_and_config_disables() {
        let _env = env_lock::lock_env([("SECURITY_WRITE_JAIL_ENABLED", None::<&str>)]);
        let jail = WriteJailInspector::new(None);
        assert!(jail.is_enabled(), "the jail must default ON");

        std::env::set_var("SECURITY_WRITE_JAIL_ENABLED", "false");
        assert!(!jail.is_enabled(), "explicit config must disable the jail");
    }

    #[test]
    fn lexical_normalize_resolves_dots_without_fs() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            lexical_normalize(Path::new("/a/../../etc/passwd")),
            PathBuf::from("/etc/passwd"),
            ".. above root clamps at root"
        );
    }

    #[test]
    fn write_tool_names_match_flat_and_prefixed() {
        for name in [
            "write",
            "edit",
            "developer__write",
            "x__edit",
            "text_editor",
        ] {
            assert!(is_write_tool(name), "{name}");
        }
        for name in ["shell", "search", "read_webpage", "rewrite", "credit"] {
            assert!(!is_write_tool(name), "{name}");
        }
    }

    // ── Lane Y9: the dispatched goal worker's hard jail ───────────────────
    //
    // Paths are deliberately fake absolutes outside any temp root (as in the
    // Y3 test above): a tempdir-rooted fixture would sit under
    // `std::env::temp_dir()`, which the builtin allowlist passes, masking the
    // very boundary these tests pin.

    const TRUNK: &str = "/repos/y9-trunk";
    const WORKTREE: &str = "/repos/.permagent-goal-worktrees/run-1";

    fn shell_request(id: &str, command: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(
                CallToolRequestParams::new("shell").with_arguments(object!({"command": command}))
            ),
            metadata: None,
            tool_meta: None,
        }
    }

    /// A session shaped exactly like a dispatched internal goal worker after
    /// Lane Y3's repoint: `SessionType::SubAgent`, working dir = its own
    /// `.permagent-goal-worktrees/<run_id>` worktree.
    async fn worker_jail(tmp: &tempfile::TempDir, worktree: &str) -> (WriteJailInspector, String) {
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let session = session_manager
            .create_session(
                PathBuf::from(worktree),
                "goal worker".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .expect("create worker session");
        (WriteJailInspector::new(Some(session_manager)), session.id)
    }

    fn assert_worker_denial(results: &[InspectionResult], target: &str) {
        assert_eq!(results.len(), 1, "expected exactly one denial: {results:?}");
        assert_eq!(
            results[0].action,
            InspectionAction::Deny,
            "a goal worker's escape must be REFUSED, not parked for approval"
        );
        assert!(
            results[0].reason.contains(WORKTREE),
            "the refusal must name the worktree so the model can correct itself: {}",
            results[0].reason
        );
        assert!(
            results[0].reason.contains(target),
            "the refusal must name what it refused ({target}): {}",
            results[0].reason
        );
        assert_eq!(
            results[0].finding_id.as_deref(),
            Some(WORKER_JAIL_FINDING_ID)
        );
    }

    /// A12's fx07 and A14's fx05/fx07, in one assertion: the worker edits the
    /// TRUNK copy of the file by absolute path, leaves the project root dirty,
    /// and its own goal can then never land.
    #[tokio::test]
    async fn goal_worker_absolute_write_into_the_trunk_is_refused_naming_its_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        let target = format!("{TRUNK}/inventory.py");
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("trunk-edit", "edit", &target)],
        )
        .await;
        assert_worker_denial(&results, &target);
    }

    #[tokio::test]
    async fn goal_worker_writes_inside_its_own_worktree_pass_silently() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[
                write_request("abs", "edit", &format!("{WORKTREE}/inventory.py")),
                write_request("rel", "write", "src/new_module.py"),
                write_request("temp", "write", "/tmp/y9-scratch.txt"),
            ],
        )
        .await;
        assert!(
            results.is_empty(),
            "the worker's own worktree (and temp scratch) must stay silent: {results:?}"
        );
    }

    /// Lane I1's macOS finding: `/var` is a symlink to `/private/var`, so the
    /// SAME directory has two spellings. Containment is judged on the
    /// symlink-resolved form, so a worker whose session was recorded under one
    /// spelling may still write to its own tree under the other.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn goal_worker_worktree_alias_var_and_private_var_is_the_same_tree() {
        let tmp = tempfile::tempdir().unwrap();
        // `/var/tmp` exists on macOS and is NOT one of the builtin temp roots
        // (`/tmp`, `/private/tmp`, `/var/folders`, `std::env::temp_dir()`), so
        // the allowlist cannot mask this.
        let worktree = "/var/tmp/y9-alias/.permagent-goal-worktrees/run-1";
        let (jail, session_id) = worker_jail(&tmp, worktree).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[write_request(
                "aliased",
                "edit",
                "/private/var/tmp/y9-alias/.permagent-goal-worktrees/run-1/src/lib.rs",
            )],
        )
        .await;
        assert!(
            results.is_empty(),
            "/private/var and /var name the same worktree: {results:?}"
        );

        let results = verdicts(
            &jail,
            &session_id,
            &[write_request(
                "aliased-trunk",
                "edit",
                "/private/var/tmp/y9-alias/workspace/inventory.py",
            )],
        )
        .await;
        assert_eq!(
            results.len(),
            1,
            "the alias must not smuggle a trunk write through: {results:?}"
        );
    }

    /// A14's fx03/fx04: "the worker committed directly on `main`". The write
    /// jail never saw those calls at all — they were `shell`.
    #[tokio::test]
    async fn goal_worker_shell_git_c_at_the_trunk_is_refused_for_mutations_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[shell_request(
                "commit",
                &format!("git -C {TRUNK} commit -am \"fix inventory\""),
            )],
        )
        .await;
        assert_worker_denial(&results, TRUNK);

        for command in [
            format!("git -C {TRUNK} log --oneline -5"),
            format!("git --git-dir={TRUNK}/.git status"),
            "git commit -am \"fix inventory\"".to_string(),
            "git -C . add -A".to_string(),
        ] {
            let results = verdicts(&jail, &session_id, &[shell_request("read", &command)]).await;
            assert!(
                results.is_empty(),
                "reads elsewhere and mutations in its OWN tree must stay allowed: {command} → {results:?}"
            );
        }
    }

    #[tokio::test]
    async fn goal_worker_shell_cd_out_of_the_worktree_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        for command in [
            format!("cd {TRUNK} && python -m pytest"),
            // `..`-relative, landing on the trunk: judged by where it LANDS.
            "cd ../../y9-trunk && git add -A".to_string(),
            format!("pushd {TRUNK}"),
        ] {
            let results = verdicts(&jail, &session_id, &[shell_request("cd", &command)]).await;
            assert_eq!(
                results.len(),
                1,
                "a cwd change out of the worktree must be refused: {command} → {results:?}"
            );
            assert_eq!(results[0].action, InspectionAction::Deny);
        }

        for command in [
            "cd src && python -m pytest".to_string(),
            format!("cd {WORKTREE} && git status"),
            "python -m pytest tests/".to_string(),
        ] {
            let results = verdicts(&jail, &session_id, &[shell_request("ok", &command)]).await;
            assert!(
                results.is_empty(),
                "movement inside the worktree must stay allowed: {command} → {results:?}"
            );
        }
    }

    /// G13 M4: a worker whose session sits in a SUBDIRECTORY of its worktree
    /// (which `goal_worktree_root` exists to support) resolves relative shell
    /// words against that subdirectory, not against the jail root — otherwise
    /// `../build` reads as `<root>/..` and an ordinary in-tree path is refused.
    #[tokio::test]
    async fn goal_worker_relative_shell_paths_resolve_against_the_session_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, &format!("{WORKTREE}/sub")).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[shell_request("sibling", "rm ../build/stale")],
        )
        .await;
        assert!(
            results.is_empty(),
            "`../build` from <worktree>/sub is INSIDE the worktree: {results:?}"
        );

        let results = verdicts(
            &jail,
            &session_id,
            &[shell_request("escape", "rm ../../outside")],
        )
        .await;
        assert_eq!(
            results.len(),
            1,
            "`../../outside` from <worktree>/sub leaves the worktree: {results:?}"
        );
        assert_eq!(results[0].action, InspectionAction::Deny);
        assert!(
            results[0].reason.contains(WORKTREE),
            "containment is still tested against the worktree ROOT: {}",
            results[0].reason
        );
    }

    #[tokio::test]
    async fn goal_worker_shell_writes_outside_are_refused_and_reads_are_not() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        for command in [
            format!("echo 'patched' > {TRUNK}/inventory.py"),
            format!("cat inventory.py >> {TRUNK}/inventory.py"),
            format!("sed -i '' 's/a/b/' {TRUNK}/inventory.py"),
            format!("rm -f {TRUNK}/inventory.py"),
            format!("cp inventory.py {TRUNK}/inventory.py"),
            format!("mkdir -p {TRUNK}/newdir"),
        ] {
            let results = verdicts(&jail, &session_id, &[shell_request("w", &command)]).await;
            assert_eq!(
                results.len(),
                1,
                "a write outside the worktree must be refused: {command} → {results:?}"
            );
            assert_eq!(results[0].action, InspectionAction::Deny);
        }

        for command in [
            format!("cat {TRUNK}/inventory.py"),
            format!("diff inventory.py {TRUNK}/inventory.py"),
            format!("cp {TRUNK}/inventory.py ./reference.py"),
            "echo hi > out.txt".to_string(),
            "python -m pytest 2>&1 | tail -20".to_string(),
            "rm -rf .pytest_cache".to_string(),
            // G13 H1: the device sinks every build/test invocation redirects
            // to. They are outside the worktree by spelling and on no
            // allowlist root, so before the fix these were hard denials.
            "pytest -q > /dev/null 2>&1".to_string(),
            "grep -rn TODO . 2>/dev/null".to_string(),
            "cargo build >/dev/null 2>/dev/null".to_string(),
            "echo note > /dev/stderr".to_string(),
        ] {
            let results = verdicts(&jail, &session_id, &[shell_request("r", &command)]).await;
            assert!(
                results.is_empty(),
                "reads outside and writes inside must stay allowed: {command} → {results:?}"
            );
        }
    }

    /// The `.permagent-goal-worktrees` namespace allowlist exists for a
    /// SUPERVISING session reaching into a worker's tree. It must not let one
    /// worker write into another worker's tree.
    #[tokio::test]
    async fn one_workers_jail_does_not_include_another_workers_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id) = worker_jail(&tmp, WORKTREE).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[write_request(
                "sibling",
                "write",
                "/repos/.permagent-goal-worktrees/run-2/src/lib.rs",
            )],
        )
        .await;
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].action, InspectionAction::Deny);
    }

    /// The coordinator is a top-level session working in the project root. Y9
    /// must not change anything about it: its shell is not inspected here at
    /// all (`CodingDagInspector` owns that policy), and an outside write is
    /// still the C3 confirmation.
    #[tokio::test]
    async fn coordinator_session_is_unaffected() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, work) = jail_with_session(&tmp).await;

        let results = verdicts(
            &jail,
            &session_id,
            &[
                shell_request("git", &format!("git -C {TRUNK} commit -am x")),
                shell_request("cd", "cd / && rm -rf build"),
            ],
        )
        .await;
        assert!(
            results.is_empty(),
            "the write jail must not start policing an ordinary session's shell: {results:?}"
        );

        let results = verdicts(
            &jail,
            &session_id,
            &[
                write_request("inside", "write", work.join("a.rs").to_str().unwrap()),
                write_request("outside", "write", "/definitely/not/allowed/b.rs"),
            ],
        )
        .await;
        assert_eq!(results.len(), 1, "{results:?}");
        assert!(
            matches!(
                results[0].action,
                InspectionAction::RequireApproval(Some(_))
            ),
            "an ordinary session keeps the confirmation semantics: {:?}",
            results[0].action
        );
    }

    #[test]
    fn goal_worktree_root_truncates_at_the_run_id() {
        assert_eq!(
            goal_worktree_root(Path::new(
                "/repos/.permagent-goal-worktrees/run-1/src/deep/dir"
            )),
            Some(PathBuf::from("/repos/.permagent-goal-worktrees/run-1"))
        );
        assert_eq!(
            goal_worktree_root(Path::new("/repos/.permagent-goal-worktrees/run-1")),
            Some(PathBuf::from("/repos/.permagent-goal-worktrees/run-1"))
        );
        // The namespace directory alone is NOT a worktree root — treating it
        // as one would jail a worker to every worker's tree at once.
        assert_eq!(
            goal_worktree_root(Path::new("/repos/.permagent-goal-worktrees")),
            None
        );
        assert_eq!(goal_worktree_root(Path::new("/repos/y9-trunk")), None);
    }

    #[test]
    fn tokenizer_keeps_quoted_paths_whole_and_finds_redirect_targets() {
        let segs = segments(tokenize("sed -i '' 's/a b/c/' \"/repos/y9 trunk/f.py\""));
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].words[0], "sed");
        assert_eq!(
            segs[0].words.last().map(String::as_str),
            Some("/repos/y9 trunk/f.py"),
            "a quoted path with a space must stay one token: {segs:?}"
        );

        let segs = segments(tokenize("python -m pytest | tee /repos/y9-trunk/log.txt"));
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!(segs[1].words, vec!["tee", "/repos/y9-trunk/log.txt"]);

        let segs = segments(tokenize("echo hi >> out/log.txt"));
        assert_eq!(segs[0].redirects, vec!["out/log.txt"]);
        assert_eq!(segs[0].words, vec!["echo", "hi"]);
    }
}
