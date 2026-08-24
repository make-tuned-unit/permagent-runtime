//! Transcript signals from goose tool results — evidence, not a dispatcher.
//!
//! Switchyard's stage-router mined Claude Code / Codex tool names. This table
//! is keyed to *our* tools (`shell`, `text_editor`, `developer__*`, `verify`).
//! Severity / spinning / exploring / production never swap the main-loop model;
//! they corroborate verify-driven escalation and hold-done.

use serde::{Deserialize, Serialize};

/// Windowed transcript evidence for one worker turn (or a short run of turns).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ToolTranscriptSignals {
    /// 0.0–1.0. Critical errors (OOM, connection refused) sit at 1.0.
    pub severity: f32,
    /// Repeated unproductive tool use (same failing command).
    pub spinning: f32,
    /// Read/search-heavy, few writes.
    pub exploring: f32,
    /// Writes / edits after reads — producing code.
    pub production: f32,
}

impl ToolTranscriptSignals {
    pub fn is_quiet(&self) -> bool {
        self.severity < 0.05
            && self.spinning < 0.05
            && self.exploring < 0.05
            && self.production < 0.05
    }

    /// Two-axis corroboration (error vs production). One maxed axis is ~0.46.
    pub fn confidence(&self) -> f32 {
        let error = (self.severity + self.spinning).min(1.0);
        let stuck = (self.exploring - self.production).clamp(-1.0, 1.0);
        (error * 0.6 + stuck.abs() * 0.4).min(1.0)
    }

    /// Human line for the Build meter / goal card — not raw scores.
    pub fn prose(&self) -> Option<String> {
        if self.severity >= 1.0 {
            return Some("hit a hard failure (memory or connection)".into());
        }
        if self.spinning >= 0.5 {
            return Some("stuck on the same failing command".into());
        }
        if self.exploring >= 0.5 && self.production < 0.2 {
            return Some("still reading, not yet writing".into());
        }
        if self.production >= 0.5 && self.severity < 0.3 {
            return Some("writing code after the reads".into());
        }
        None
    }
}

const CRITICAL: &[&str] = &[
    "out of memory",
    "cannot allocate memory",
    "connection refused",
    "econnrefused",
];
const HARD: &[&str] = &[
    "modulenotfounderror",
    "syntaxerror",
    "assertionerror",
    "no such file or directory",
    "command not found",
    "timed out",
    "failed",
    "error:",
];

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    let h = hay.to_ascii_lowercase();
    needles.iter().any(|n| h.contains(n))
}

fn tool_kind(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("verify") {
        "verify"
    } else if n.contains("text_editor") || n.contains("edit") || n.contains("str_replace") {
        "edit"
    } else if n.contains("shell") || n.contains("bash") || n.ends_with("__shell") {
        "shell"
    } else if n.contains("search")
        || n.contains("read")
        || n.contains("analyze")
        || n.contains("grep")
    {
        "read"
    } else {
        "other"
    }
}

/// One tool call + its result text, already on the goose wire.
#[derive(Debug, Clone)]
pub struct ToolTurn<'a> {
    pub name: &'a str,
    pub result: &'a str,
}

/// Extract signals from a short run of goose tool turns. Pure.
pub fn extract(turns: &[ToolTurn<'_>]) -> ToolTranscriptSignals {
    if turns.is_empty() {
        return ToolTranscriptSignals::default();
    }

    let mut severity: f32 = 0.0;
    let mut edits = 0u32;
    let mut reads = 0u32;
    let mut failing_shells = 0u32;
    let mut last_shell: Option<&str> = None;
    let mut repeat_fail = 0u32;

    for t in turns {
        let kind = tool_kind(t.name);
        if contains_any(t.result, CRITICAL) {
            severity = severity.max(1.0);
        } else if contains_any(t.result, HARD) {
            severity = severity.max(0.7);
        }
        match kind {
            "edit" => edits += 1,
            "read" => reads += 1,
            "shell" | "verify" => {
                let failed = contains_any(t.result, HARD) || contains_any(t.result, CRITICAL);
                if failed {
                    failing_shells += 1;
                    if last_shell == Some(t.result) {
                        repeat_fail += 1;
                    }
                    last_shell = Some(t.result);
                } else {
                    last_shell = None;
                }
            }
            _ => {}
        }
    }

    let n = turns.len() as f32;
    let spinning = if repeat_fail >= 1 {
        0.7
    } else if failing_shells >= 2 {
        0.5
    } else {
        0.0
    };
    let exploring = if reads > edits && edits == 0 {
        (reads as f32 / n).min(1.0)
    } else {
        0.0
    };
    let production = if edits > 0 {
        (edits as f32 / n).min(1.0)
    } else {
        0.0
    };

    ToolTranscriptSignals {
        severity,
        spinning,
        exploring,
        production,
    }
}

/// Signals may corroborate a verify climb. They never authorize one alone.
pub fn corroborates_verify_climb(signals: &ToolTranscriptSignals) -> bool {
    signals.severity >= 0.7 || signals.spinning >= 0.5
}

/// Fold transcript evidence into the consecutive-failure count that
/// [`super::decide_escalation`] already understands.
///
/// - `consecutive == 0` (no verify fail) stays 0 — signals never authorize a climb.
/// - Verify is already failing AND signals corroborate → treat as the escalate
///   threshold so the existing path may fire sooner, still spend-capped.
pub fn corroborating_consecutive(
    consecutive: u32,
    signals: &ToolTranscriptSignals,
    escalate_at: u32,
) -> u32 {
    if consecutive == 0 {
        return 0;
    }
    if corroborates_verify_climb(signals) {
        consecutive.max(escalate_at)
    } else {
        consecutive
    }
}

/// Pair goose tool requests with their responses from a live conversation.
pub fn extract_from_messages(
    messages: &[crate::conversation::message::Message],
) -> ToolTranscriptSignals {
    use crate::conversation::message::MessageContent;
    use std::collections::HashMap;

    let mut pending: HashMap<String, String> = HashMap::new();
    let mut owned: Vec<(String, String)> = Vec::new();
    for msg in messages {
        for content in &msg.content {
            match content {
                MessageContent::ToolRequest(req) => {
                    if let Ok(call) = &req.tool_call {
                        pending.insert(req.id.clone(), call.name.to_string());
                    }
                }
                MessageContent::ToolResponse(resp) => {
                    let Some(name) = pending.remove(&resp.id) else {
                        continue;
                    };
                    let body = match &resp.tool_result {
                        Ok(r) => serde_json::to_string(&r.content).unwrap_or_default(),
                        Err(e) => e.to_string(),
                    };
                    owned.push((name, body));
                }
                _ => {}
            }
        }
    }
    let turns: Vec<ToolTurn<'_>> = owned
        .iter()
        .map(|(n, r)| ToolTurn { name: n, result: r })
        .collect();
    extract(&turns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_quiet() {
        assert!(extract(&[]).is_quiet());
        assert!(!corroborates_verify_climb(&ToolTranscriptSignals::default()));
    }

    #[test]
    fn oom_is_critical() {
        let s = extract(&[ToolTurn {
            name: "shell",
            result: "out of memory while linking",
        }]);
        assert_eq!(s.severity, 1.0);
        assert!(corroborates_verify_climb(&s));
        assert!(s.prose().unwrap().contains("hard failure"));
    }

    #[test]
    fn repeated_failing_shell_is_spinning() {
        let fail = "error: test result: FAILED\nassertionerror: expected 1";
        let s = extract(&[
            ToolTurn {
                name: "developer__shell",
                result: fail,
            },
            ToolTurn {
                name: "shell",
                result: fail,
            },
        ]);
        assert!(s.spinning >= 0.5);
        assert!(s.prose().unwrap().contains("stuck"));
    }

    #[test]
    fn reads_without_writes_are_exploring() {
        let s = extract(&[
            ToolTurn {
                name: "search",
                result: "3 matches",
            },
            ToolTurn {
                name: "analyze",
                result: "fn foo",
            },
        ]);
        assert!(s.exploring > 0.0);
        assert_eq!(s.production, 0.0);
    }

    #[test]
    fn text_editor_is_production() {
        let s = extract(&[
            ToolTurn {
                name: "search",
                result: "found",
            },
            ToolTurn {
                name: "text_editor",
                result: "ok",
            },
        ]);
        assert!(s.production > 0.0);
    }

    #[test]
    fn claude_multiedit_name_does_not_count_as_goose_edit() {
        // Switchyard-style names must not be our only signal. A foreign MCP
        // search that is not goose-named stays quiet.
        let s = extract(&[ToolTurn {
            name: "mcp__docs__lookup",
            result: "ok",
        }]);
        assert!(s.is_quiet());
    }

    #[test]
    fn confidence_needs_two_axes() {
        let one = ToolTranscriptSignals {
            severity: 0.7,
            ..Default::default()
        };
        assert!(one.confidence() < 0.55);
        let two = ToolTranscriptSignals {
            severity: 0.7,
            spinning: 0.7,
            ..Default::default()
        };
        assert!(two.confidence() > one.confidence());
    }

    #[test]
    fn signals_alone_never_raise_consecutive() {
        let spinning = ToolTranscriptSignals {
            spinning: 0.8,
            ..Default::default()
        };
        assert_eq!(corroborating_consecutive(0, &spinning, 3), 0);
    }

    #[test]
    fn verify_fail_plus_spinning_reaches_threshold() {
        let spinning = ToolTranscriptSignals {
            spinning: 0.8,
            ..Default::default()
        };
        assert_eq!(corroborating_consecutive(1, &spinning, 3), 3);
        assert_eq!(
            corroborating_consecutive(1, &ToolTranscriptSignals::default(), 3),
            1
        );
    }
}
