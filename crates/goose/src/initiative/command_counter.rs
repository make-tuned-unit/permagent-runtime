//! W3 — deterministic repeated-command counter (the pattern-threshold signal).
//!
//! Consumes `TerminalCommandCompleted` activity events and detects when the
//! user has run the *same* command enough times within a window to be worth
//! proposing as an automation. This is pure, zero-token logic — it is the only
//! branch of the Tier 0 gate that can produce an origination candidate.
//!
//! NORMALIZATION (v1, CONSERVATIVE — "exact-after-arg-strip"):
//! a false "want me to automate this?" is the creepy-failure to avoid, so v1
//! errs hard toward precision. We collapse whitespace and replace ONLY
//! unambiguously-volatile tokens (paths, pure numbers, long hashes) with a
//! `<arg>` placeholder. Meaningful string arguments (commit messages, branch
//! names, flag values) are KEPT, so commands that differ in intent do NOT
//! collapse together. v1.1 may loosen this (strip quoted strings / flag
//! values) once the precision floor is proven in the field.
//!
//! Only `exit_code == 0` commands count — a command the user keeps re-running
//! and failing is not an automation candidate.
//!
//! Time is taken from the *event* (`event.timestamp`), never the wall clock,
//! so windowing is deterministic and testable.

use crate::events::activity::{ActivityEvent, ActivityEventType};
use chrono::{DateTime, Duration, Utc};
use std::collections::VecDeque;

/// Default occurrences within the window before a pattern is surfaced.
pub const DEFAULT_THRESHOLD: usize = 3;
/// Default rolling window over which repeats are counted.
pub const DEFAULT_WINDOW_SECS: i64 = 7 * 24 * 60 * 60; // 7 days
/// How many distinct raw exemplars to retain for the draft step.
const MAX_EXEMPLARS: usize = 3;

/// A detected repeated-command pattern ripe for an automation proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPattern {
    /// The normalized command string (the observation key — also what the
    /// recognition novelty/pruning lookup keys on).
    pub normalized: String,
    /// Number of in-window occurrences at detection time.
    pub count: usize,
    /// A few of the raw commands that matched (for the Tier 1 draft prompt).
    pub exemplars: Vec<String>,
}

/// Rolling, in-memory counter of normalized command occurrences.
///
/// Bounded by the window: entries older than `window` relative to the latest
/// event are pruned on every `record`. State is reconstructable from Brain@Raw
/// activity memories on restart; a cold start simply re-accumulates.
pub struct CommandCounter {
    threshold: usize,
    window: Duration,
    /// (normalized, raw, event_time), oldest-first.
    entries: VecDeque<(String, String, DateTime<Utc>)>,
}

impl Default for CommandCounter {
    fn default() -> Self {
        Self::new(DEFAULT_THRESHOLD, DEFAULT_WINDOW_SECS)
    }
}

impl CommandCounter {
    pub fn new(threshold: usize, window_secs: i64) -> Self {
        Self {
            threshold: threshold.max(1),
            window: Duration::seconds(window_secs.max(0)),
            entries: VecDeque::new(),
        }
    }

    /// Record an activity event. Returns `Some(CommandPattern)` when a
    /// successful command has reached the threshold within the window.
    ///
    /// Non-terminal events, non-zero exits, and empty commands are ignored.
    pub fn record(&mut self, event: &ActivityEvent) -> Option<CommandPattern> {
        if event.event_type != ActivityEventType::TerminalCommandCompleted {
            return None;
        }
        if event.payload.get("exit_code").and_then(|v| v.as_i64()) != Some(0) {
            return None;
        }
        let raw = event
            .payload
            .get("command")
            .and_then(|v| v.as_str())?
            .trim();
        if raw.is_empty() {
            return None;
        }
        let normalized = normalize_command(raw);
        if normalized.is_empty() {
            return None;
        }

        let now = event.timestamp;
        self.entries
            .push_back((normalized.clone(), raw.to_string(), now));
        self.prune(now);

        let mut count = 0usize;
        let mut exemplars: Vec<String> = Vec::new();
        for (norm, raw_cmd, _) in &self.entries {
            if norm == &normalized {
                count += 1;
                if exemplars.len() < MAX_EXEMPLARS && !exemplars.contains(raw_cmd) {
                    exemplars.push(raw_cmd.clone());
                }
            }
        }

        if count >= self.threshold {
            Some(CommandPattern {
                normalized,
                count,
                exemplars,
            })
        } else {
            None
        }
    }

    /// Drop entries older than the window relative to `now` (event time).
    fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - self.window;
        while let Some((_, _, ts)) = self.entries.front() {
            if *ts < cutoff {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Conservative v1 normalization: whitespace-collapse + volatile-token strip.
/// Keeps the program, subcommands, flags, and meaningful string args; replaces
/// only paths / pure numbers / long hashes with `<arg>`.
pub fn normalize_command(raw: &str) -> String {
    raw.split_whitespace()
        .map(|tok| if is_volatile(tok) { "<arg>" } else { tok })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A token is volatile iff it is unambiguously instance-specific: a path, a
/// pure number, or a long hex string (sha/hash/id). Flags (`-x`, `--long`) and
/// ordinary words are NOT volatile — keeping them preserves precision.
fn is_volatile(tok: &str) -> bool {
    if tok.starts_with('/')
        || tok.starts_with("~/")
        || tok.starts_with("./")
        || tok.starts_with("../")
    {
        return true;
    }
    if tok.parse::<f64>().is_ok() {
        return true;
    }
    if tok.len() >= 7 && tok.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::activity::SourceSurface;
    use serde_json::json;

    fn cmd_event(command: &str, exit: i64, at: DateTime<Utc>) -> ActivityEvent {
        let mut e = ActivityEvent::new(
            ActivityEventType::TerminalCommandCompleted,
            SourceSurface::Terminal,
            json!({ "command": command, "exit_code": exit, "working_directory": "/x" }),
        );
        e.timestamp = at;
        e
    }

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn repeats_cross_threshold() {
        let mut c = CommandCounter::new(3, DEFAULT_WINDOW_SECS);
        assert!(c
            .record(&cmd_event("git status && git pull", 0, t(0)))
            .is_none());
        assert!(c
            .record(&cmd_event("git status && git pull", 0, t(10)))
            .is_none());
        let p = c
            .record(&cmd_event("git status && git pull", 0, t(20)))
            .expect("third repeat fires");
        assert_eq!(p.count, 3);
        assert_eq!(p.normalized, "git status && git pull");
        assert_eq!(
            p.exemplars.len(),
            1,
            "identical raw collapses to one exemplar"
        );
    }

    #[test]
    fn distinct_commands_do_not_collapse() {
        let mut c = CommandCounter::new(2, DEFAULT_WINDOW_SECS);
        // Different commit messages must NOT be treated as the same workflow —
        // this is the creepy false-positive we refuse to make in v1.
        c.record(&cmd_event(r#"git commit -m "fix a""#, 0, t(0)));
        assert!(
            c.record(&cmd_event(r#"git commit -m "fix b""#, 0, t(1)))
                .is_none(),
            "meaningful string args keep distinct commands apart"
        );
    }

    #[test]
    fn volatile_paths_and_hashes_collapse() {
        let mut c = CommandCounter::new(2, DEFAULT_WINDOW_SECS);
        c.record(&cmd_event("cat /tmp/abc1234.log", 0, t(0)));
        let p = c
            .record(&cmd_event("cat /tmp/def5678.log", 0, t(1)))
            .expect("path-varying repeats collapse");
        assert_eq!(p.normalized, "cat <arg>");
        assert_eq!(p.count, 2);
        assert_eq!(p.exemplars.len(), 2, "distinct raw exemplars retained");
    }

    #[test]
    fn failed_commands_are_ignored() {
        let mut c = CommandCounter::new(2, DEFAULT_WINDOW_SECS);
        c.record(&cmd_event("make build", 1, t(0)));
        assert!(
            c.record(&cmd_event("make build", 1, t(1))).is_none(),
            "non-zero exits never count toward automation"
        );
    }

    #[test]
    fn non_terminal_events_ignored() {
        let mut c = CommandCounter::new(1, DEFAULT_WINDOW_SECS);
        let e = ActivityEvent::new(
            ActivityEventType::FileEdited,
            SourceSurface::FileViewer,
            json!({ "path": "/x" }),
        );
        assert!(c.record(&e).is_none());
    }

    #[test]
    fn out_of_window_repeats_are_pruned() {
        let mut c = CommandCounter::new(2, 60); // 60s window
        c.record(&cmd_event("npm run build", 0, t(0)));
        // Second occurrence 120s later — first has aged out, so no threshold.
        assert!(
            c.record(&cmd_event("npm run build", 0, t(120))).is_none(),
            "occurrences outside the window do not accumulate"
        );
    }

    #[test]
    fn normalize_is_whitespace_stable() {
        assert_eq!(
            normalize_command("  git   status  &&  git pull  "),
            "git status && git pull"
        );
    }
}
