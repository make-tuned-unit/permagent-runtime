//! Static classifier for `command_exit_zero` checks — the "is this command
//! allowed to run unattended?" decision, with no I/O and no side effects.
//!
//! Model-authored completion checks are compiled from acceptance criteria and
//! then executed by [`super::checks::run_command_check`] through `/bin/sh -c`
//! with the user's own privileges and the project root as cwd. Nothing else
//! stands between a sentence the model wrote and a shell. This module is that
//! something: it lexes the command line and sorts it into one of three tiers.
//!
//! - [`Tier::Auto`] — every simple command's first token is allowlisted for this
//!   project and no deny category fires. Runs now.
//! - [`Tier::AgentTrust`] — the ONLY thing wrong is an unrecognised first token.
//!   The agent may self-approve if it has earned enough privilege in this
//!   project (see [`super::decide`]).
//! - [`Tier::User`] — a deny category fired, or the command could not be lexed
//!   with confidence. Always a human decision.
//!
//! Two rules govern every change to this file:
//!
//! 1. **Fail closed.** Anything the lexer cannot account for — a heredoc, an
//!    unbalanced quote, an unexpanded `$VAR` in a path operand — is
//!    [`Tier::User`], never [`Tier::Auto`]. A classifier that guesses is worse
//!    than no classifier, because it launders a guess into a permission.
//! 2. **The deny table is reviewed, and every row is tested.** [`DenyCategory`]
//!    is the table. Each variant has at least one test below naming it.
//!
//! The tiers describe the command's *shape*, not the caller's trust: user-
//! authored checks bypass this module entirely at the call site, they are not
//! a fourth tier here.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

// ── Result types ────────────────────────────────────────────────────────────

/// Where a command lands on the ladder. Ordered: a compound command takes the
/// **highest** tier any of its parts reaches.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Allowlisted and clean — run without asking anyone.
    Auto,
    /// Unknown first token, nothing else wrong — the agent may self-approve.
    AgentTrust,
    /// Denied outright, or unparseable. A person decides.
    User,
}

impl Tier {
    /// Stable name for audit rows and the Inbox card.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Auto => "auto",
            Tier::AgentTrust => "agent_trust",
            Tier::User => "user",
        }
    }
}

/// The reviewed deny table. Each variant is a category the user signed off on;
/// each has a test below. Adding a variant means adding a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyCategory {
    /// Piping into a shell or scripting interpreter — `… | sh`, `… | python`,
    /// `… | xargs`. Turns any upstream output into executable code.
    PipeToInterpreter,
    /// Network transfer or remote-shell tooling — curl, wget, ssh, nc, rsync.
    /// Exfiltration and remote-code-fetch both live here.
    NetworkTool,
    /// Deleting or moving a path that is not inside the project root.
    DestructiveOutsideRoot,
    /// Git subcommands that rewrite history, mutate a remote, or discard the
    /// working tree. A verification check has no business doing any of these.
    GitMutating,
    /// sudo / doas / su / pkexec — running the check as somebody else.
    PrivilegeEscalation,
    /// Writing output to a path outside the project root.
    RedirectOutsideRoot,
    /// A `$(…)` or backtick substitution whose contents hit one of the above.
    /// Reported separately so the Inbox card can say *where* the problem is.
    CommandSubstitution,
    /// The lexer could not account for the command with confidence — an
    /// unbalanced quote, a heredoc, an unexpanded variable in a path operand.
    /// Not a judgement that the command is bad; a refusal to guess.
    Unparseable,
}

impl DenyCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            DenyCategory::PipeToInterpreter => "pipe_to_interpreter",
            DenyCategory::NetworkTool => "network_tool",
            DenyCategory::DestructiveOutsideRoot => "destructive_outside_root",
            DenyCategory::GitMutating => "git_mutating",
            DenyCategory::PrivilegeEscalation => "privilege_escalation",
            DenyCategory::RedirectOutsideRoot => "redirect_outside_root",
            DenyCategory::CommandSubstitution => "command_substitution",
            DenyCategory::Unparseable => "unparseable",
        }
    }
}

/// The verdict on one command line.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Classification {
    pub tier: Tier,
    /// Set iff `tier == User` because of the deny table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<DenyCategory>,
    /// The unrecognised first token that forced `AgentTrust`, if that is why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_token: Option<String>,
    /// One sentence, written for the person reading the Inbox card.
    pub reason: String,
    /// No write-looking flags and no redirects anywhere. Gates the lower
    /// self-approval threshold; only meaningful when `tier == AgentTrust`.
    pub read_only_looking: bool,
}

impl Classification {
    fn auto() -> Self {
        Self {
            tier: Tier::Auto,
            deny: None,
            unknown_token: None,
            reason: "every command is an allowlisted runner and nothing in the deny table fired"
                .to_string(),
            read_only_looking: true,
        }
    }

    fn deny(cat: DenyCategory, reason: impl Into<String>) -> Self {
        Self {
            tier: Tier::User,
            deny: Some(cat),
            unknown_token: None,
            reason: reason.into(),
            read_only_looking: false,
        }
    }

    fn unknown(token: &str) -> Self {
        Self {
            tier: Tier::AgentTrust,
            deny: None,
            unknown_token: Some(token.to_string()),
            reason: format!(
                "`{}` is not on this project's allowlist, but nothing in the deny table fired",
                token
            ),
            read_only_looking: true,
        }
    }
}

// ── Configuration ───────────────────────────────────────────────────────────

/// Everything the classifier needs to know about *this* project.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// First tokens that may auto-run. Project allowlist ∪ [`default_allowlist`]
    /// ∪ the first token of the project's configured `build_command`.
    pub allowlist: BTreeSet<String>,
    /// Canonicalised project root. Paths outside it are the deny line.
    pub project_root: PathBuf,
}

impl GateConfig {
    /// Defaults plus a project's own additions and its build command.
    pub fn new(
        project_root: impl Into<PathBuf>,
        extra: impl IntoIterator<Item = String>,
        build_command: Option<&str>,
    ) -> Self {
        let mut allowlist: BTreeSet<String> =
            default_allowlist().iter().map(|s| s.to_string()).collect();
        allowlist.extend(extra);
        if let Some(first) = build_command.and_then(first_token_of) {
            allowlist.insert(first);
        }
        // Resolve the root once so a symlinked project path (/tmp → /private/tmp
        // on macOS) does not read as "outside itself". Falling back to the path
        // as given keeps a not-yet-existing root usable, and a mismatch that
        // survives only ever parks a command — never releases one.
        let project_root: PathBuf = project_root.into();
        let project_root = project_root.canonicalize().unwrap_or(project_root);
        Self {
            allowlist,
            project_root,
        }
    }
}

/// Best-effort first token of a configured build command, for allowlisting.
/// Returns `None` when the command is empty or does not lex cleanly — an
/// unlexable build command grants nothing rather than granting everything.
pub fn first_token_of(cmd: &str) -> Option<String> {
    let lexed = lex(cmd).ok()?;
    let first = lexed.first()?;
    let (name, _) = strip_wrappers(&first.words)?;
    Some(name.to_string())
}

// ── Default allowlist ───────────────────────────────────────────────────────

/// The standard build/test/lint runners and read-only inspection tools.
///
/// Deliberately absent, and why:
/// - `sed`, `awk`, `perl` — programming languages that can write files, not
///   search tools. They reach [`Tier::AgentTrust`] and can be earned.
/// - `node`, `python`, `bun`, `ruby`, `sh` — interpreters. Same reasoning.
/// - `rm`, `mv`, `cp`, `mkdir` — mutation. Earned, not granted.
/// - `git` is here, but only its read-only subcommands survive
///   [`git_subcommand_verdict`]; the mutating ones are a deny row.
pub fn default_allowlist() -> &'static [&'static str] {
    &[
        // Rust
        "cargo",
        "rustc",
        "rustfmt",
        "rustup",
        "clippy-driver",
        // Node / JS / TS
        "npm",
        "npx",
        "pnpm",
        "yarn",
        "tsc",
        "eslint",
        "prettier",
        "vitest",
        "jest",
        "vite",
        // Python
        "pytest",
        "tox",
        "ruff",
        "mypy",
        "black",
        "flake8",
        "pylint",
        // Go
        "go",
        "gofmt",
        "golangci-lint",
        // Swift / Xcode
        "swift",
        "swiftc",
        "xcodebuild",
        "xcrun",
        "swiftlint",
        "swift-format",
        // Generic build drivers
        "make",
        "cmake",
        "ninja",
        "bazel",
        "just",
        // Read-only search and inspection
        "grep",
        "egrep",
        "fgrep",
        "rg",
        "ag",
        "ack",
        "fd",
        "find",
        "ls",
        "cat",
        "head",
        "tail",
        "wc",
        "sort",
        "uniq",
        "cut",
        "tr",
        "jq",
        "yq",
        "tree",
        "stat",
        "file",
        "basename",
        "dirname",
        "realpath",
        "diff",
        "cmp",
        "shasum",
        "sha256sum",
        "md5sum",
        "test",
        "[",
        "which",
        "type",
        // Shell builtins and reporters that cannot touch anything: no file is
        // read or written, no socket opened, no process started. `check_lint`
        // keeps the same list (its `NO_EFFECT_COMMANDS`) for the opposite
        // reason — a check built only from these proves nothing. Both readings
        // are right: harmless to run, worthless as evidence.
        "true",
        "false",
        "echo",
        "printf",
        "pwd",
        "exit",
        ":",
        "date",
        "hostname",
        "whoami",
        "id",
        "sleep",
        // Read-only VCS (subcommand-gated below)
        "git",
    ]
}

/// Git subcommands a check may run unattended. Everything here only reads.
const READ_ONLY_GIT: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "rev-parse",
    "rev-list",
    "ls-files",
    "ls-tree",
    "ls-remote",
    "describe",
    "blame",
    "cat-file",
    "grep",
    "shortlog",
    "name-rev",
    "merge-base",
    "symbolic-ref",
    "check-ignore",
    "count-objects",
    "whatchanged",
    "version",
];

/// Git subcommands that rewrite history, mutate a remote, or discard work.
const MUTATING_GIT: &[&str] = &[
    "push",
    "reset",
    "checkout",
    "restore",
    "switch",
    "clean",
    "rebase",
    "filter-branch",
    "filter-repo",
    "cherry-pick",
    "revert",
    "merge",
    "am",
    "apply",
    "commit",
    "gc",
    "prune",
    "reflog",
    "update-ref",
    "update-index",
    "remote",
    "submodule",
    "worktree",
    "stash",
    "notes",
    "replace",
    "fetch",
    "pull",
    "clone",
    "init",
    "mv",
    "rm",
    "tag",
    "branch",
];

/// Shell and scripting interpreters. Piping into one of these turns arbitrary
/// upstream bytes into code, which is the whole hazard.
const INTERPRETERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "fish",
    "ash",
    "busybox",
    "python",
    "python2",
    "python3",
    "perl",
    "ruby",
    "node",
    "deno",
    "bun",
    "php",
    "lua",
    "osascript",
    "pwsh",
    "powershell",
    "xargs",
    "eval",
    "source",
    ".",
];

/// Network transfer and remote-shell tools.
const NETWORK_TOOLS: &[&str] = &[
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "nc",
    "ncat",
    "netcat",
    "telnet",
    "ftp",
    "socat",
    "http",
    "https",
    "httpie",
    "aria2c",
    "axel",
    "lynx",
    "w3m",
    "dig",
    "host",
    "nslookup",
    "ping",
    "traceroute",
    "openssl",
];

/// Privilege escalation.
const PRIVILEGE_TOOLS: &[&str] = &["sudo", "doas", "su", "pkexec", "runas", "gosu"];

/// Commands whose path operands must stay inside the project root.
const DESTRUCTIVE_TOOLS: &[&str] = &["rm", "rmdir", "unlink", "shred", "trash", "mv"];

/// Commands that change the working directory.
///
/// These are not dangerous in themselves — they are *unanalysable*. Every
/// containment decision this module makes about a relative path is measured
/// from the check's own cwd, and a `cd` silently moves that ground for
/// everything after it: `cd /etc && rm hosts` would otherwise be judged as
/// deleting `<project>/hosts`. Rather than model directory state, refuse.
const DIRECTORY_CHANGERS: &[&str] = &["cd", "pushd", "popd", "chdir"];

/// Transparent prefixes — they run whatever follows, so classify that instead.
const WRAPPERS: &[&str] = &[
    "env", "command", "nice", "ionice", "timeout", "stdbuf", "nohup", "time", "setsid", "exec",
];

/// Flags that suggest a command writes something. Used only for the *lower*
/// self-approval threshold; a false positive costs an extra approval, never
/// safety.
const WRITE_FLAGS: &[&str] = &[
    "-i",
    "--in-place",
    "-o",
    "--output",
    "--output-dir",
    "--out",
    "--out-dir",
    "-w",
    "--write",
    "--fix",
    "--force",
    "-f",
    "--delete",
    "-d",
    "--prune",
    "--save",
    "--save-dev",
    "--overwrite",
    "--emit",
    "-u",
    "--update",
];

/// Redirect targets that are always fine regardless of the root check.
const SAFE_REDIRECT_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"];

// ── Public entry point ──────────────────────────────────────────────────────

/// Classify one `command_exit_zero` command line.
///
/// `cwd` is the directory the check will actually run in (already resolved
/// under the project root by the check runner); relative path operands are
/// judged against it.
pub fn classify(cmd: &str, cwd: &Path, cfg: &GateConfig) -> Classification {
    classify_inner(cmd, cwd, cfg, 0)
}

/// Depth cap: substitutions nested deeper than this are refused rather than
/// walked, so a hostile command cannot exhaust the stack.
const MAX_DEPTH: usize = 8;

fn classify_inner(cmd: &str, cwd: &Path, cfg: &GateConfig, depth: usize) -> Classification {
    if depth > MAX_DEPTH {
        return Classification::deny(
            DenyCategory::Unparseable,
            "command substitutions are nested too deeply to check",
        );
    }
    if cmd.trim().is_empty() {
        return Classification::deny(DenyCategory::Unparseable, "the command is empty");
    }

    let segments = match lex(cmd) {
        Ok(s) => s,
        Err(why) => return Classification::deny(DenyCategory::Unparseable, why),
    };
    if segments.is_empty() {
        return Classification::deny(DenyCategory::Unparseable, "the command is empty");
    }

    let mut worst = Classification::auto();
    for seg in &segments {
        let c = classify_segment(seg, cwd, cfg, depth);
        worst = worse_of(worst, c);
        if worst.tier == Tier::User {
            return worst;
        }
    }
    worst
}

/// Keep whichever classification is worse. A tie keeps the one already in hand,
/// whose reason was recorded first.
///
/// `read_only_looking` is the exception: it is an AND across everything seen so
/// far, because one writing command in a chain makes the whole chain write.
fn worse_of(a: Classification, b: Classification) -> Classification {
    let read_only_looking = a.read_only_looking && b.read_only_looking;
    let mut kept = if b.tier > a.tier { b } else { a };
    kept.read_only_looking = read_only_looking;
    kept
}

fn classify_segment(seg: &Segment, cwd: &Path, cfg: &GateConfig, depth: usize) -> Classification {
    let mut worst = Classification::auto();

    // Redirects first: a redirect out of the root is a deny no matter what the
    // command is.
    for r in &seg.redirects {
        if !r.writes {
            continue;
        }
        if r.target.has_unexpanded_var {
            return Classification::deny(
                DenyCategory::Unparseable,
                format!(
                    "output is redirected to `{}`, which contains a variable this check cannot expand",
                    r.target.text
                ),
            );
        }
        if SAFE_REDIRECT_TARGETS.contains(&r.target.text.as_str()) {
            continue;
        }
        if !is_inside_root(&r.target.text, cwd, &cfg.project_root) {
            return Classification::deny(
                DenyCategory::RedirectOutsideRoot,
                format!(
                    "output is redirected to `{}`, which is outside the project root",
                    r.target.text
                ),
            );
        }
        worst.read_only_looking = false;
    }

    // Anything inside `$(…)` or backticks, wherever it appeared.
    for sub in &seg.substitutions {
        let inner = classify_inner(sub, cwd, cfg, depth + 1);
        if inner.tier == Tier::User {
            let cat = inner.deny.unwrap_or(DenyCategory::CommandSubstitution);
            let cat = if cat == DenyCategory::Unparseable {
                cat
            } else {
                DenyCategory::CommandSubstitution
            };
            return Classification::deny(
                cat,
                format!(
                    "a command substitution `$({})` is not allowed to run: {}",
                    sub.trim(),
                    inner.reason
                ),
            );
        }
        worst = worse_of(worst, inner);
    }

    let Some((name, rest)) = strip_wrappers(&seg.words) else {
        // Only assignments / only flags — nothing executes.
        return worst;
    };

    // Being piped INTO is what makes an interpreter dangerous here.
    if seg.piped_into && INTERPRETERS.contains(&name) {
        return Classification::deny(
            DenyCategory::PipeToInterpreter,
            format!(
                "output is piped into `{}`, which would execute whatever the upstream command printed",
                name
            ),
        );
    }

    if DIRECTORY_CHANGERS.contains(&name) {
        return Classification::deny(
            DenyCategory::Unparseable,
            format!(
                "`{}` moves the working directory, so where the paths after it point \
                 cannot be known without running the command",
                name
            ),
        );
    }

    if PRIVILEGE_TOOLS.contains(&name) {
        return Classification::deny(
            DenyCategory::PrivilegeEscalation,
            format!("`{}` runs the check with escalated privileges", name),
        );
    }

    if NETWORK_TOOLS.contains(&name) {
        return Classification::deny(
            DenyCategory::NetworkTool,
            format!(
                "`{}` transfers data over the network or opens a remote shell",
                name
            ),
        );
    }

    if DESTRUCTIVE_TOOLS.contains(&name) {
        if let Some(d) = destructive_path_verdict(name, rest, cwd, &cfg.project_root) {
            return d;
        }
        worst.read_only_looking = false;
    }

    // An interpreter run directly: check what it was asked to run.
    if INTERPRETERS.contains(&name) {
        if let Some(script) = inline_script_of(name, rest) {
            let inner = classify_inner(script, cwd, cfg, depth + 1);
            if inner.tier == Tier::User {
                return Classification::deny(
                    inner.deny.unwrap_or(DenyCategory::Unparseable),
                    format!("`{} -c` would run: {}", name, inner.reason),
                );
            }
            worst = worse_of(worst, inner);
        }
    }

    // `find … -exec <cmd>` and `find … -delete` are two other commands wearing
    // find's name.
    if name == "find" {
        if let Some(d) = find_verdict(rest, cwd, cfg, depth) {
            return d;
        }
    }

    if name == "git" {
        if let Some(d) = git_subcommand_verdict(rest) {
            return d;
        }
    }

    if rest.iter().any(|w| WRITE_FLAGS.contains(&w.text.as_str())) {
        worst.read_only_looking = false;
    }

    // Finally: is the first token something this project trusts?
    let allowed = cfg.allowlist.contains(name) && (name != "git" || git_is_read_only(rest));
    if !allowed {
        worst = worse_of(worst, Classification::unknown(name));
    }

    worst
}

// ── Per-tool verdicts ───────────────────────────────────────────────────────

/// `git <sub>`: deny the mutating ones outright. An unrecognised subcommand is
/// not denied — it simply is not read-only, so `git` stops counting as
/// allowlisted and the whole command falls to [`Tier::AgentTrust`].
fn git_subcommand_verdict(rest: &[Word]) -> Option<Classification> {
    let sub = git_subcommand(rest)?;
    if MUTATING_GIT.contains(&sub) {
        return Some(Classification::deny(
            DenyCategory::GitMutating,
            format!(
                "`git {}` rewrites history, mutates a remote, or discards the working tree",
                sub
            ),
        ));
    }
    None
}

fn git_is_read_only(rest: &[Word]) -> bool {
    match git_subcommand(rest) {
        Some(sub) => READ_ONLY_GIT.contains(&sub),
        None => false,
    }
}

/// First non-flag word after `git`, skipping git's own global options.
/// Global options that take a value (`-C <dir>`, `-c <k=v>`) consume it.
fn git_subcommand(rest: &[Word]) -> Option<&str> {
    let mut i = 0;
    while i < rest.len() {
        let w = rest[i].text.as_str();
        if w == "-C" || w == "-c" || w == "--git-dir" || w == "--work-tree" || w == "--namespace" {
            i += 2;
            continue;
        }
        if w.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(w);
    }
    None
}

/// `rm` / `mv` / friends: every path operand must resolve inside the root.
fn destructive_path_verdict(
    name: &str,
    rest: &[Word],
    cwd: &Path,
    root: &Path,
) -> Option<Classification> {
    let mut saw_operand = false;
    let mut end_of_flags = false;
    for w in rest {
        if !end_of_flags && w.text == "--" {
            end_of_flags = true;
            continue;
        }
        if !end_of_flags && w.text.starts_with('-') && w.text.len() > 1 {
            continue;
        }
        saw_operand = true;
        if w.has_unexpanded_var {
            return Some(Classification::deny(
                DenyCategory::Unparseable,
                format!(
                    "`{}` targets `{}`, which contains a variable this check cannot expand — \
                     where it points cannot be known without running it",
                    name, w.text
                ),
            ));
        }
        if !is_inside_root(&w.text, cwd, root) {
            return Some(Classification::deny(
                DenyCategory::DestructiveOutsideRoot,
                format!(
                    "`{}` targets `{}`, which is outside the project root",
                    name, w.text
                ),
            ));
        }
    }
    if !saw_operand {
        return Some(Classification::deny(
            DenyCategory::Unparseable,
            format!("`{}` has no path operand this check can inspect", name),
        ));
    }
    None
}

/// `find` can delete and can execute. Both are other tools in disguise.
fn find_verdict(
    rest: &[Word],
    cwd: &Path,
    cfg: &GateConfig,
    depth: usize,
) -> Option<Classification> {
    let mut i = 0;
    while i < rest.len() {
        match rest[i].text.as_str() {
            "-delete" => {
                // Judge the search roots the way `rm` judges its operands.
                for w in rest.iter().take_while(|w| !w.text.starts_with('-')) {
                    if w.has_unexpanded_var || !is_inside_root(&w.text, cwd, &cfg.project_root) {
                        return Some(Classification::deny(
                            DenyCategory::DestructiveOutsideRoot,
                            format!(
                                "`find {} -delete` would delete outside the project root",
                                w.text
                            ),
                        ));
                    }
                }
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                // Everything up to `;` or `+` is the command find will run.
                let inner: Vec<String> = rest[i + 1..]
                    .iter()
                    .take_while(|w| w.text != ";" && w.text != "+")
                    .map(|w| w.text.clone())
                    .collect();
                if inner.is_empty() {
                    return Some(Classification::deny(
                        DenyCategory::Unparseable,
                        "`find -exec` has no command this check can inspect",
                    ));
                }
                let joined = inner.join(" ");
                let c = classify_inner(&joined, cwd, cfg, depth + 1);
                if c.tier == Tier::User {
                    return Some(Classification::deny(
                        c.deny.unwrap_or(DenyCategory::Unparseable),
                        format!("`find -exec` would run `{}`: {}", joined, c.reason),
                    ));
                }
                if c.tier == Tier::AgentTrust {
                    return Some(Classification {
                        reason: format!(
                            "`find -exec` would run `{}`, which is not allowlisted",
                            joined
                        ),
                        ..c
                    });
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The script body of `sh -c '…'` / `python -c '…'`, if present.
fn inline_script_of<'a>(name: &str, rest: &'a [Word]) -> Option<&'a str> {
    let flag = if name == "perl" || name == "ruby" {
        "-e"
    } else {
        "-c"
    };
    let idx = rest.iter().position(|w| w.text == flag)?;
    rest.get(idx + 1).map(|w| w.text.as_str())
}

// ── Path containment ────────────────────────────────────────────────────────

/// Is `raw` a path inside `root`, judged lexically from `cwd`?
///
/// Lexical on purpose: the check has not run yet, so the path may not exist,
/// and `canonicalize` would follow a symlink the command has not created. A
/// leading `~` is treated as the home directory, which is outside any project
/// root the daemon manages.
pub fn is_inside_root(raw: &str, cwd: &Path, root: &Path) -> bool {
    if raw.is_empty() {
        return false;
    }
    let expanded: PathBuf = if raw == "~" || raw.starts_with("~/") {
        match std::env::var_os("HOME") {
            Some(h) => {
                let mut p = PathBuf::from(h);
                if let Some(tail) = raw.strip_prefix("~/") {
                    p.push(tail);
                }
                p
            }
            None => return false,
        }
    } else if raw.starts_with('~') {
        // ~otheruser — not ours.
        return false;
    } else {
        PathBuf::from(raw)
    };

    let joined = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let normalized = normalize_lexically(&joined);
    let root_norm = normalize_lexically(root);
    normalized.starts_with(&root_norm)
}

/// Resolve `.` and `..` textually. `..` above the top yields the top, which
/// keeps the result from silently escaping.
fn normalize_lexically(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
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

// ── Lexer ───────────────────────────────────────────────────────────────────

/// One word of a simple command, with what the lexer noticed about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// Contained `$VAR` / `${VAR}` outside single quotes — value unknowable here.
    pub has_unexpanded_var: bool,
}

#[derive(Debug, Clone)]
struct Redirect {
    target: Word,
    /// `>`/`>>`/`&>` write; `<` does not.
    writes: bool,
}

/// One simple command: the words, its redirects, whether a pipe feeds it, and
/// every command substitution that appeared anywhere inside it.
#[derive(Debug, Clone)]
struct Segment {
    words: Vec<Word>,
    redirects: Vec<Redirect>,
    piped_into: bool,
    substitutions: Vec<String>,
}

/// Split a command line into simple commands.
///
/// Returns `Err` — which the caller turns into [`DenyCategory::Unparseable`] —
/// for anything this lexer does not model: unbalanced quotes or parens, and
/// heredocs.
fn lex(input: &str) -> Result<Vec<Segment>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    let mut segments: Vec<Segment> = Vec::new();
    let mut cur = Segment {
        words: Vec::new(),
        redirects: Vec::new(),
        piped_into: false,
        substitutions: Vec::new(),
    };
    let mut word = String::new();
    let mut word_has_var = false;
    let mut word_started = false;
    // Some(writes) while the next word is a redirect target.
    let mut pending_redirect: Option<bool> = None;

    macro_rules! flush_word {
        () => {
            if word_started {
                let w = Word {
                    text: std::mem::take(&mut word),
                    has_unexpanded_var: word_has_var,
                };
                word_has_var = false;
                word_started = false;
                match pending_redirect.take() {
                    Some(writes) => cur.redirects.push(Redirect { target: w, writes }),
                    None => cur.words.push(w),
                }
            }
        };
    }

    macro_rules! flush_segment {
        ($piped:expr) => {{
            flush_word!();
            if pending_redirect.is_some() {
                return Err("a redirection has no target".to_string());
            }
            let piped_into = cur.piped_into;
            let subs = std::mem::take(&mut cur.substitutions);
            let words = std::mem::take(&mut cur.words);
            let redirects = std::mem::take(&mut cur.redirects);
            if !words.is_empty() || !redirects.is_empty() || !subs.is_empty() {
                segments.push(Segment {
                    words,
                    redirects,
                    piped_into,
                    substitutions: subs,
                });
            }
            cur = Segment {
                words: Vec::new(),
                redirects: Vec::new(),
                piped_into: $piped,
                substitutions: Vec::new(),
            };
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                if i + 1 >= chars.len() {
                    return Err("the command ends with a dangling backslash".to_string());
                }
                word.push(chars[i + 1]);
                word_started = true;
                i += 2;
            }
            '\'' => {
                let (lit, next) = read_single_quoted(&chars, i)?;
                word.push_str(&lit);
                word_started = true;
                i = next;
            }
            '"' => {
                let (lit, subs, has_var, next) = read_double_quoted(&chars, i)?;
                word.push_str(&lit);
                cur.substitutions.extend(subs);
                word_has_var |= has_var;
                word_started = true;
                i = next;
            }
            '`' => {
                let (inner, next) = read_backtick(&chars, i)?;
                cur.substitutions.push(inner);
                word.push_str("<substitution>");
                word_started = true;
                i = next;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '(' => {
                let (inner, next) = read_paren_group(&chars, i + 1)?;
                cur.substitutions.push(inner);
                word.push_str("<substitution>");
                word_started = true;
                i = next;
            }
            '<' | '>' if i + 1 < chars.len() && chars[i + 1] == '(' => {
                // Process substitution — the inner command really does run.
                let (inner, next) = read_paren_group(&chars, i + 1)?;
                cur.substitutions.push(inner);
                word.push_str("<substitution>");
                word_started = true;
                i = next;
            }
            '$' => {
                word_has_var = true;
                word.push('$');
                word_started = true;
                i += 1;
            }
            '|' => {
                let double = chars.get(i + 1) == Some(&'|');
                flush_segment!(!double);
                i += if double { 2 } else { 1 };
            }
            '&' => {
                if chars.get(i + 1) == Some(&'>') {
                    flush_word!();
                    pending_redirect = Some(true);
                    i += 2;
                    continue;
                }
                flush_segment!(false);
                i += if chars.get(i + 1) == Some(&'&') { 2 } else { 1 };
            }
            ';' | '\n' => {
                flush_segment!(false);
                i += 1;
            }
            '>' => {
                // `>>` (append) and `>|` (clobber) are two characters; a plain
                // `>` is one. All three write, which is all the gate cares about.
                let two_char = matches!(chars.get(i + 1), Some('>') | Some('|'));
                i += if two_char { 2 } else { 1 };
                flush_word!();
                pending_redirect = Some(true);
            }
            '<' => {
                if chars.get(i + 1) == Some(&'<') {
                    return Err(
                        "the command uses a heredoc, which this check cannot inspect".to_string(),
                    );
                }
                flush_word!();
                pending_redirect = Some(false);
                i += 1;
            }
            '(' => {
                // A subshell group: lex its contents as their own segments.
                let (inner, next) = read_paren_group(&chars, i)?;
                flush_word!();
                let inner_segments = lex(&inner)?;
                for mut s in inner_segments {
                    s.piped_into |= cur.piped_into;
                    segments.push(s);
                }
                i = next;
            }
            ')' => return Err("the command has an unbalanced `)`".to_string()),
            '{' | '}' if !word_started => {
                // Brace grouping — the contents are ordinary segments already.
                i += 1;
            }
            c if c.is_whitespace() => {
                flush_word!();
                i += 1;
            }
            c => {
                // A leading digit immediately before `>` is an fd number.
                if c.is_ascii_digit() && !word_started && chars.get(i + 1) == Some(&'>') {
                    i += 1;
                    continue;
                }
                word.push(c);
                word_started = true;
                i += 1;
            }
        }
    }

    flush_segment!(false);
    // The final flush writes the accumulators one last time and nobody reads
    // them again. Naming them here is cheaper than an allow attribute, and it
    // documents that the end of input really is the end.
    let _ = (&cur, word_has_var, word_started);
    Ok(segments)
}

fn read_single_quoted(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\'' {
            return Ok((out, i + 1));
        }
        out.push(chars[i]);
        i += 1;
    }
    Err("the command has an unterminated single quote".to_string())
}

/// Inside double quotes: `$(…)` and backticks still run, `\` still escapes,
/// `$VAR` still expands.
fn read_double_quoted(
    chars: &[char],
    start: usize,
) -> Result<(String, Vec<String>, bool, usize), String> {
    let mut out = String::new();
    let mut subs = Vec::new();
    let mut has_var = false;
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '"' => return Ok((out, subs, has_var, i + 1)),
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '`' => {
                let (inner, next) = read_backtick(chars, i)?;
                subs.push(inner);
                out.push_str("<substitution>");
                i = next;
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let (inner, next) = read_paren_group(chars, i + 1)?;
                subs.push(inner);
                out.push_str("<substitution>");
                i = next;
            }
            '$' => {
                has_var = true;
                out.push('$');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Err("the command has an unterminated double quote".to_string())
}

fn read_backtick(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            return Ok((out, i + 1));
        }
        out.push(chars[i]);
        i += 1;
    }
    Err("the command has an unterminated backtick substitution".to_string())
}

/// Read a `(…)` group starting at the `(`, honouring nesting and quotes.
/// Returns the inner text and the index just past the `)`.
fn read_paren_group(chars: &[char], start: usize) -> Result<(String, usize), String> {
    debug_assert_eq!(chars[start], '(');
    let mut depth = 0usize;
    let mut out = String::new();
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                if depth > 0 {
                    out.push(chars[i]);
                    out.push(chars[i + 1]);
                }
                i += 2;
            }
            '\'' => {
                let (lit, next) = read_single_quoted(chars, i)?;
                out.push('\'');
                out.push_str(&lit);
                out.push('\'');
                i = next;
            }
            '"' => {
                // Copy verbatim; the recursive lex will re-read it.
                let mut j = i + 1;
                out.push('"');
                while j < chars.len() {
                    if chars[j] == '\\' && j + 1 < chars.len() {
                        out.push(chars[j]);
                        out.push(chars[j + 1]);
                        j += 2;
                        continue;
                    }
                    if chars[j] == '"' {
                        break;
                    }
                    out.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    return Err("the command has an unterminated double quote".to_string());
                }
                out.push('"');
                i = j + 1;
            }
            '(' => {
                depth += 1;
                if depth > 1 {
                    out.push('(');
                }
                i += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((out, i + 1));
                }
                out.push(')');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Err("the command has an unbalanced `(`".to_string())
}

/// Skip `FOO=bar` assignments and transparent wrappers (`env`, `timeout 60`, …)
/// to find the command that actually runs. Returns its name and its arguments.
fn strip_wrappers(words: &[Word]) -> Option<(&str, &[Word])> {
    let mut i = 0;
    loop {
        // Leading VAR=value assignments.
        while i < words.len() && is_assignment(&words[i].text) {
            i += 1;
        }
        let first = words.get(i)?;
        if !WRAPPERS.contains(&first.text.as_str()) {
            return Some((first.text.as_str(), &words[i + 1..]));
        }
        // Step over the wrapper's own options. `timeout 60 cmd` and
        // `nice -n 5 cmd` both put a bare value where a command would go, so
        // only a value that is not itself a flag is consumed.
        i += 1;
        while i < words.len() {
            let w = words[i].text.as_str();
            if w == "--" {
                i += 1;
                break;
            }
            if w.starts_with('-') {
                i += 1;
                continue;
            }
            // A duration for `timeout`, a niceness for `nice`.
            if first.text == "timeout" && looks_like_duration(w) {
                i += 1;
                continue;
            }
            break;
        }
        if i >= words.len() {
            return None;
        }
    }
}

fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        None | Some(("", _)) => false,
        Some((name, _)) => {
            name.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        }
    }
}

fn looks_like_duration(w: &str) -> bool {
    let core = w.strip_suffix(['s', 'm', 'h', 'd']).unwrap_or(w);
    !core.is_empty() && core.chars().all(|c| c.is_ascii_digit() || c == '.')
}
