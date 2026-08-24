//! Lint for *gameable* completion checks.
//!
//! `checks.rs` runs whatever a goal declares. Nothing, until now, asked whether
//! what it declared could ever have failed. A check that cannot fail is not
//! evidence — it is a checkbox the worker wrote for itself, and a ledger full
//! of them reads as "verified" while proving nothing.
//!
//! This module is pure: no I/O, no clock, no process. It takes the declared
//! checks (and the acceptance criteria they were compiled from) and returns
//! findings. `verification/mod.rs` stamps the per-check findings onto
//! `CheckResult::lint`, which (a) surfaces the reason in the evidence digest
//! and the verifier's prompt, and (b) stops that check counting as support for
//! a Pass.
//!
//! The rules are reimplemented from unlazy's described heuristics
//! (<https://github.com/Leonxlnx/unlazy>, MIT, `scripts/gate-lint.mjs`) against
//! Permagent's typed `CompletionCheck` shape. No upstream source is vendored.
//!
//! Deliberately advisory-by-construction: a finding never *fails* a goal on its
//! own (that would manufacture false-fails out of a heuristic). It withdraws
//! the check's standing as proof, which at worst leaves a verdict Uncertain and
//! in front of a human.

use super::checks::CompletionCheck;

/// Which heuristic fired. Stable strings — they are written into the stored
/// verification record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintRule {
    /// The command cannot fail: every segment is a shell built-in or a printer.
    TautologicalCommand,
    /// The `expect` token is a word that appears in almost any successful
    /// output — asserting it asserts nothing.
    WeakExpect,
    /// A `grep_absent` that cannot prove an absence (no paths, empty pattern).
    UnprovableGrepAbsent,
    /// The whole ledger is `file_exists` — existence is not behaviour.
    ExistenceOnlyLedger,
    /// An acceptance criterion that names an activity ("refactor the parser")
    /// rather than an observable outcome ("the parser accepts `a|b`").
    ActivityNotOutcome,
}

impl LintRule {
    pub fn as_str(self) -> &'static str {
        match self {
            LintRule::TautologicalCommand => "tautological_command",
            LintRule::WeakExpect => "weak_expect",
            LintRule::UnprovableGrepAbsent => "unprovable_grep_absent",
            LintRule::ExistenceOnlyLedger => "existence_only_ledger",
            LintRule::ActivityNotOutcome => "activity_not_outcome",
        }
    }
}

/// One finding against one declared check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFinding {
    pub check_index: usize,
    pub rule: LintRule,
    pub reason: String,
}

impl LintFinding {
    /// The line stored on `CheckResult::lint` and shown in the digest.
    pub fn line(&self) -> String {
        format!("[{}] {}", self.rule.as_str(), self.reason)
    }
}

/// Per-check findings plus ledger-level notes (which belong to no single
/// check — a criterion's wording, or the shape of the ledger as a whole).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    pub per_check: Vec<LintFinding>,
    pub notes: Vec<String>,
}

impl LintReport {
    pub fn is_empty(&self) -> bool {
        self.per_check.is_empty() && self.notes.is_empty()
    }

    /// The finding line for `index`, if that check was flagged.
    pub fn for_check(&self, index: usize) -> Option<String> {
        self.per_check
            .iter()
            .find(|f| f.check_index == index)
            .map(|f| f.line())
    }
}

/// Commands whose exit status is decided by the shell, not by the state of the
/// world. `exit` is here for `exit 0`; a non-zero `exit` is equally useless as
/// evidence of success.
const TRIVIAL_COMMANDS: &[&str] = &[
    "echo", "printf", "true", ":", "exit", "pwd", "date", "hostname", "whoami", "sleep", "id",
    "uname", "return",
];

/// Words that appear in the successful output of almost anything. Asserting one
/// is indistinguishable from asserting nothing.
const WEAK_EXPECT_TOKENS: &[&str] = &[
    "ok",
    "okay",
    "done",
    "pass",
    "passed",
    "passes",
    "passing",
    "success",
    "successful",
    "successfully",
    "succeeded",
    "yes",
    "true",
    "0",
    "1",
    "fine",
    "good",
    "complete",
    "completed",
    "finished",
    "works",
    "working",
];

/// Regexes that match anything at all.
const CATCH_ALL_PATTERNS: &[&str] = &[".*", ".+", ".", "^", "$", "^.*$", "[\\s\\S]*", ""];

/// Verbs that describe *doing work*. A criterion that opens with one and never
/// names something observable states an activity, not an outcome.
const ACTIVITY_VERBS: &[&str] = &[
    "run",
    "rerun",
    "re-run",
    "add",
    "update",
    "refactor",
    "review",
    "investigate",
    "explore",
    "consider",
    "improve",
    "clean",
    "tidy",
    "document",
    "write",
    "discuss",
    "think",
    "look",
    "try",
    "attempt",
    "check",
    "verify",
    "test",
    "handle",
    "address",
    "work",
    "implement",
    "fix",
    "make",
    "ensure",
];

/// Words and shapes that make a criterion *observable* — something a check
/// could be compiled from, or a human could look at and disagree with.
const OUTCOME_SIGNALS: &[&str] = &[
    "returns",
    "return ",
    "exits",
    "exit ",
    "responds",
    "renders",
    "contains",
    "matches",
    "equals",
    "exists",
    "no ",
    "zero",
    "absent",
    "without",
    "status",
    "http",
    "passes with",
    "outputs",
    "prints",
    "logs",
    "shows",
    "displays",
    "is ",
    "are ",
    "must",
    "should",
];

/// Lint a goal's declared checks and its acceptance criteria.
pub fn lint(checks: &[CompletionCheck], acceptance_criteria: &[String]) -> LintReport {
    let mut report = LintReport::default();

    for (i, check) in checks.iter().enumerate() {
        if let Some((rule, reason)) = lint_one(check) {
            report.per_check.push(LintFinding {
                check_index: i,
                rule,
                reason,
            });
        }
    }

    // Ledger shape: "prose with checkboxes". A ledger that only asserts files
    // exist proves the worker created paths, never that anything behaves. Flag
    // every check so none of them stands as proof.
    if !checks.is_empty()
        && checks
            .iter()
            .all(|c| matches!(c, CompletionCheck::FileExists { .. }))
    {
        report.notes.push(
            "every declared check is `file_exists` — the ledger proves paths were \
                   created, not that anything works"
                .to_string(),
        );
        for i in 0..checks.len() {
            if report.per_check.iter().any(|f| f.check_index == i) {
                continue;
            }
            report.per_check.push(LintFinding {
                check_index: i,
                rule: LintRule::ExistenceOnlyLedger,
                reason: "the only kind of check on this goal is `file_exists`; existence \
                         is not behaviour"
                    .to_string(),
            });
        }
        report.per_check.sort_by_key(|f| f.check_index);
    }

    for c in acceptance_criteria {
        if let Some(reason) = lint_criterion(c) {
            report.notes.push(reason);
        }
    }

    report
}

/// Lint one check in isolation. `None` when nothing fired.
pub fn lint_one(check: &CompletionCheck) -> Option<(LintRule, String)> {
    match check {
        CompletionCheck::CommandExitZero { cmd, expect, .. } => {
            if is_tautological_command(cmd) {
                return Some((
                    LintRule::TautologicalCommand,
                    format!(
                        "`{}` cannot fail — its exit status is decided by the shell, not \
                         by the state of the work",
                        cmd.trim()
                    ),
                ));
            }
            let e = expect.as_deref()?;
            let weak = weak_expect_reason(e)?;
            Some((LintRule::WeakExpect, weak))
        }
        CompletionCheck::GrepAbsent { pattern, paths } => {
            if paths.is_empty() {
                return Some((
                    LintRule::UnprovableGrepAbsent,
                    "grep_absent names no paths — there is nothing to prove the pattern \
                     absent from"
                        .to_string(),
                ));
            }
            if pattern.trim().is_empty() {
                return Some((
                    LintRule::UnprovableGrepAbsent,
                    "grep_absent has an empty pattern — an empty pattern matches every \
                     line, so this can only ever fail or be meaningless"
                        .to_string(),
                ));
            }
            None
        }
        _ => None,
    }
}

/// True when every segment of the command line is a shell built-in or a
/// printer, i.e. the command's exit status is a foregone conclusion.
///
/// Segments split on `;`, `&&`, `||`, `|` and newlines: one real command
/// anywhere in the line is enough to make the check substantive, because its
/// failure can still propagate (`cargo test | tail` exits with `tail`'s status,
/// but `cargo test && echo ok` exits non-zero when cargo does).
fn is_tautological_command(cmd: &str) -> bool {
    let segments = split_segments(cmd);
    if segments.is_empty() {
        // An empty command is `/bin/sh -c ""` — exit 0, always.
        return true;
    }
    segments.iter().all(|s| is_trivial_segment(s))
}

fn split_segments(cmd: &str) -> Vec<String> {
    cmd.replace("&&", "\n")
        .replace("||", "\n")
        .replace([';', '|'], "\n")
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_trivial_segment(segment: &str) -> bool {
    // Skip leading `FOO=bar` environment assignments and shell noise.
    let head = segment
        .split_whitespace()
        .find(|w| !w.contains('=') || w.starts_with('='))
        .unwrap_or("");
    if head.is_empty() {
        return true;
    }
    // `/bin/echo` and `echo` are the same command for this purpose.
    let base = head.rsplit('/').next().unwrap_or(head);
    let base = base.trim_start_matches('\\');
    TRIVIAL_COMMANDS.contains(&base)
}

/// Why this `expect` is too weak to be an assertion, if it is.
fn weak_expect_reason(expect: &str) -> Option<String> {
    let raw = expect.trim();
    // Unwrap the `/regex/` form before judging the token inside it.
    let inner = raw
        .strip_prefix('/')
        .and_then(|s| s.strip_suffix('/'))
        .unwrap_or(raw);
    let normalized: String = inner
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == ':' || c == '"' || c == '\'')
        .to_ascii_lowercase();

    if CATCH_ALL_PATTERNS.iter().any(|p| *p == inner.trim()) {
        return Some(format!(
            "expect `{}` matches any output at all — it asserts nothing",
            raw
        ));
    }
    if normalized.chars().count() < 2 {
        return Some(format!(
            "expect `{}` is a single character — far too broad to be an assertion",
            raw
        ));
    }
    if WEAK_EXPECT_TOKENS.iter().any(|t| *t == normalized) {
        return Some(format!(
            "expect `{}` is a word that appears in almost any successful output — a \
             check asserting it is indistinguishable from one asserting nothing",
            raw
        ));
    }
    None
}

/// A criterion that names an activity instead of an observable outcome.
/// Conservative: it must *open* with an activity verb AND carry no outcome
/// signal, no number, no backticked token and no path.
fn lint_criterion(criterion: &str) -> Option<String> {
    let text = criterion.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();

    let first = lower
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
    if !ACTIVITY_VERBS.contains(&first) {
        return None;
    }
    if OUTCOME_SIGNALS.iter().any(|s| lower.contains(s)) {
        return None;
    }
    if text.contains('`') || text.contains('/') || text.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }

    Some(format!(
        "acceptance criterion \"{}\" names an activity, not an outcome — nothing here \
         could be observed to be true or false, so no check can be compiled from it and \
         the verifier has nothing to grade against",
        text
    ))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(c: &str) -> CompletionCheck {
        CompletionCheck::CommandExitZero {
            cmd: c.to_string(),
            cwd: None,
            expect: None,
            timeout_secs: 120,
        }
    }

    fn cmd_expect(c: &str, e: &str) -> CompletionCheck {
        CompletionCheck::CommandExitZero {
            cmd: c.to_string(),
            cwd: None,
            expect: Some(e.to_string()),
            timeout_secs: 120,
        }
    }

    // ── tautological_command ────────────────────────────────────────────────

    #[test]
    fn echo_ok_lints_as_tautological() {
        let (rule, reason) = lint_one(&cmd("echo ok")).expect("echo ok must lint");
        assert_eq!(rule, LintRule::TautologicalCommand);
        assert!(reason.contains("cannot fail"), "reason: {reason}");
    }

    #[test]
    fn chained_printers_and_exit_zero_lint_as_tautological() {
        for c in [
            "true",
            ":",
            "exit 0",
            "echo done && printf built",
            "/bin/echo hi; true",
            "  ",
            "RUST_LOG=debug echo ok",
        ] {
            assert_eq!(
                lint_one(&cmd(c)).map(|(r, _)| r),
                Some(LintRule::TautologicalCommand),
                "expected {c:?} to lint as tautological"
            );
        }
    }

    /// Negative test for the tautological rule: real build/test commands must
    /// never be flagged, including ones that merely *pipe into* or *chain with*
    /// a printer.
    #[test]
    fn real_commands_do_not_lint_as_tautological() {
        for c in [
            "cargo check",
            "cargo test -p permagent",
            "npm run build",
            "cargo test && echo ok",
            "cargo test 2>&1 | tail -5",
            "test -f README.md",
            "grep -q FIXME src/lib.rs",
        ] {
            assert_eq!(lint_one(&cmd(c)), None, "{c:?} must not lint");
        }
    }

    // ── weak_expect ─────────────────────────────────────────────────────────

    #[test]
    fn weak_expect_tokens_lint() {
        for e in [
            "ok", "OK", "done.", "success", "passed", "0", "yes", "/.*/", "x",
        ] {
            assert_eq!(
                lint_one(&cmd_expect("cargo test", e)).map(|(r, _)| r),
                Some(LintRule::WeakExpect),
                "expected expect {e:?} to lint as weak"
            );
        }
    }

    /// Negative test for the weak-expect rule: a real assertion survives.
    #[test]
    fn substantive_expect_does_not_lint() {
        for e in [
            "test result: ok. 412 passed",
            "/\\d+ passed/",
            "0 vulnerabilities",
            "Compiling permagent",
            "wrote dist/index.html",
        ] {
            assert_eq!(
                lint_one(&cmd_expect("cargo test", e)),
                None,
                "expect {e:?} must not lint"
            );
        }
    }

    // ── unprovable_grep_absent ──────────────────────────────────────────────

    #[test]
    fn grep_absent_without_paths_or_pattern_lints() {
        assert_eq!(
            lint_one(&CompletionCheck::GrepAbsent {
                pattern: "TODO".into(),
                paths: vec![]
            })
            .map(|(r, _)| r),
            Some(LintRule::UnprovableGrepAbsent)
        );
        assert_eq!(
            lint_one(&CompletionCheck::GrepAbsent {
                pattern: "  ".into(),
                paths: vec!["src/lib.rs".into()]
            })
            .map(|(r, _)| r),
            Some(LintRule::UnprovableGrepAbsent)
        );
    }

    /// Negative test: a well-formed grep_absent is untouched.
    #[test]
    fn well_formed_grep_absent_does_not_lint() {
        assert_eq!(
            lint_one(&CompletionCheck::GrepAbsent {
                pattern: "todo!\\(".into(),
                paths: vec!["src/lib.rs".into()]
            }),
            None
        );
    }

    // ── existence_only_ledger ───────────────────────────────────────────────

    #[test]
    fn ledger_of_only_file_exists_flags_every_check() {
        let checks = vec![
            CompletionCheck::FileExists {
                path: "a.rs".into(),
            },
            CompletionCheck::FileExists {
                path: "b.rs".into(),
            },
        ];
        let report = lint(&checks, &[]);
        assert_eq!(report.per_check.len(), 2);
        assert!(report
            .per_check
            .iter()
            .all(|f| f.rule == LintRule::ExistenceOnlyLedger));
        assert_eq!(report.notes.len(), 1);
        assert!(report.for_check(0).is_some());
        assert!(report.for_check(1).is_some());
    }

    /// Negative test: one substantive check redeems the ledger.
    #[test]
    fn file_exists_alongside_a_real_check_does_not_lint() {
        let checks = vec![
            CompletionCheck::FileExists {
                path: "a.rs".into(),
            },
            cmd("cargo check"),
        ];
        let report = lint(&checks, &[]);
        assert!(report.is_empty(), "unexpected findings: {report:?}");
    }

    #[test]
    fn empty_ledger_produces_nothing() {
        assert!(lint(&[], &[]).is_empty());
    }

    // ── activity_not_outcome ────────────────────────────────────────────────

    #[test]
    fn activity_titled_criteria_are_noted() {
        for c in [
            "Refactor the session builder",
            "Investigate why voice drops out",
            "Clean up the orchestrator",
            "Review the prompt manager",
        ] {
            assert!(
                lint_criterion(c).is_some(),
                "expected {c:?} to be noted as an activity"
            );
        }
    }

    /// Negative test for the activity rule: outcomes, and activity-shaped
    /// sentences that still name something observable, must survive.
    #[test]
    fn outcome_criteria_are_not_noted() {
        for c in [
            "The daemon returns 200 on /health",
            "`cargo test -p permagent` exits 0",
            "No TODO markers remain in src/lib.rs",
            "Run the suite until it passes with 412 tests",
            "Update crates/goose/src/lib.rs so the module compiles",
            "the roster shows every agent",
        ] {
            assert_eq!(lint_criterion(c), None, "{c:?} must not be noted");
        }
    }

    #[test]
    fn criteria_notes_land_in_the_report() {
        let report = lint(&[cmd("cargo check")], &["Refactor the parser".to_string()]);
        assert!(report.per_check.is_empty());
        assert_eq!(report.notes.len(), 1);
        assert!(report.notes[0].contains("names an activity"));
    }

    #[test]
    fn finding_line_carries_rule_and_reason() {
        let f = LintFinding {
            check_index: 3,
            rule: LintRule::WeakExpect,
            reason: "because".to_string(),
        };
        assert_eq!(f.line(), "[weak_expect] because");
    }
}
