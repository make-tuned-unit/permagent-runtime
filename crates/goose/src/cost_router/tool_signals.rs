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
    /// Wire-level tool outcome when available. Text is only a fallback for
    /// synthetic/direct callers: successful test output commonly contains
    /// `0 failed`, which must never be classified as a failure.
    pub is_error: Option<bool>,
}

/// Extract signals from a short run of goose tool turns. Pure.
pub fn extract(turns: &[ToolTurn<'_>]) -> ToolTranscriptSignals {
    if turns.is_empty() {
        return ToolTranscriptSignals::default();
    }

    let mut severity: f32 = 0.0;
    let mut edits = 0u32;
    let mut reads = 0u32;
    let mut last_failure: Option<String> = None;
    let mut repeat_fail = 0u32;

    for t in turns {
        let kind = tool_kind(t.name);
        // The wire is the truth. Text sniffing survives only for synthetic
        // callers that have no `is_error` to give us — a passing suite prints
        // `0 failed`, and reading that as a failure is what made the harness
        // answer a green run with "verify is still failing the same way".
        let failed = t
            .is_error
            .unwrap_or_else(|| contains_any(t.result, HARD) || contains_any(t.result, CRITICAL));

        // A successful mutation starts a new diagnostic epoch, and a successful
        // verify closes it. Failures before either seam are resolved evidence,
        // not proof that the current tail is spinning.
        if (kind == "edit" || kind == "verify") && t.is_error == Some(false) {
            severity = 0.0;
            last_failure = None;
            repeat_fail = 0;
        }

        if failed && contains_any(t.result, CRITICAL) {
            severity = severity.max(1.0);
        } else if failed && contains_any(t.result, HARD) {
            severity = severity.max(0.7);
        }
        match kind {
            "edit" => edits += 1,
            "read" => reads += 1,
            "shell" | "verify" => {
                if failed {
                    // Normalised, so "timed out after 120 seconds" and "after
                    // 300 seconds" are one spin — but two genuinely different
                    // errors are not.
                    let fingerprint = failure_fingerprint(t.result);
                    if last_failure.as_deref() == Some(fingerprint.as_str()) {
                        repeat_fail += 1;
                    } else {
                        repeat_fail = 0;
                    }
                    last_failure = Some(fingerprint);
                } else {
                    // A passing unrelated shell breaks consecutiveness but does
                    // not erase unresolved severity; only an edit or a
                    // successful verify closes that diagnostic epoch.
                    last_failure = None;
                    repeat_fail = 0;
                }
            }
            _ => {}
        }
    }

    let n = turns.len() as f32;
    let spinning = if repeat_fail >= 1 { 0.7 } else { 0.0 };
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

/// Stable enough to recognize the same command failure across elapsed-time,
/// port, PID, and count changes without collapsing different error text.
fn failure_fingerprint(result: &str) -> String {
    let mut out = String::with_capacity(result.len().min(1024));
    let mut in_digits = false;
    let mut in_space = false;
    for ch in result.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
            in_space = false;
        } else if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_digits = false;
            in_space = true;
        } else {
            out.push(ch);
            in_digits = false;
            in_space = false;
        }
        if out.len() >= 1024 {
            break;
        }
    }
    out.trim().to_string()
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
    let mut owned: Vec<(String, String, bool)> = Vec::new();
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
                    let (body, is_error) = match &resp.tool_result {
                        Ok(r) => (
                            serde_json::to_string(&r.content).unwrap_or_default(),
                            r.is_error == Some(true),
                        ),
                        Err(e) => (e.to_string(), true),
                    };
                    owned.push((name, body, is_error));
                }
                _ => {}
            }
        }
    }
    let turns: Vec<ToolTurn<'_>> = owned
        .iter()
        .map(|(n, r, is_error)| ToolTurn {
            name: n,
            result: r,
            is_error: Some(*is_error),
        })
        .collect();
    extract(&turns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Role};

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
            is_error: Some(true),
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
                is_error: Some(true),
            },
            ToolTurn {
                name: "shell",
                result: fail,
                is_error: Some(true),
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
                is_error: Some(false),
            },
            ToolTurn {
                name: "analyze",
                result: "fn foo",
                is_error: Some(false),
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
                is_error: Some(false),
            },
            ToolTurn {
                name: "text_editor",
                result: "ok",
                is_error: Some(false),
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
            is_error: Some(false),
        }]);
        assert!(s.is_quiet());
    }

    #[test]
    fn different_failures_are_not_the_same_spin() {
        let s = extract(&[
            ToolTurn {
                name: "verify",
                result: "error: TypeScript is not installed",
                is_error: Some(true),
            },
            ToolTurn {
                name: "verify",
                result: "error: production build timed out",
                is_error: Some(true),
            },
        ]);
        assert_eq!(s.spinning, 0.0);
    }

    /// The fingerprint normalises digits, so a timeout that grew from 120s to
    /// 300s is still the same command failing the same way.
    #[test]
    fn the_same_failure_with_different_numbers_is_one_spin() {
        let s = extract(&[
            ToolTurn {
                name: "verify",
                result: "error: build timed out after 120 seconds",
                is_error: Some(true),
            },
            ToolTurn {
                name: "verify",
                result: "error: build timed out after 300 seconds",
                is_error: Some(true),
            },
        ]);
        assert_eq!(s.spinning, 0.7);
    }

    #[test]
    fn successful_verify_clears_old_failure_signals_even_with_zero_failed_text() {
        let s = extract(&[
            ToolTurn {
                name: "verify",
                result: "error: build timed out after 120 seconds",
                is_error: Some(true),
            },
            ToolTurn {
                name: "verify",
                result: "error: build timed out after 300 seconds",
                is_error: Some(true),
            },
            ToolTurn {
                name: "verify",
                result: "test result: ok. 42 passed; 0 failed",
                is_error: Some(false),
            },
        ]);
        assert_eq!(s.spinning, 0.0);
        assert_eq!(s.severity, 0.0);
    }

    #[test]
    fn successful_unrelated_shell_breaks_failure_consecutiveness() {
        let fail = "error: build timed out";
        let s = extract(&[
            ToolTurn {
                name: "verify",
                result: fail,
                is_error: Some(true),
            },
            ToolTurn {
                name: "shell",
                result: "working tree clean",
                is_error: Some(false),
            },
            ToolTurn {
                name: "verify",
                result: fail,
                is_error: Some(true),
            },
        ]);
        assert_eq!(s.spinning, 0.0);
        assert!(
            s.severity >= 0.7,
            "an unrelated pass breaks the run but does not resolve the failure"
        );
    }

    /// Build one assistant message carrying a tool request and its result,
    /// the way a live conversation does — the only input `extract_from_messages`
    /// trusts.
    fn tool_exchange(name: &str, id: &str, text: &str, ok: bool) -> Message {
        let result = if ok {
            CallToolResult::success(vec![Content::text(text)])
        } else {
            CallToolResult::error(vec![Content::text(text)])
        };
        Message::new(
            Role::Assistant,
            0,
            vec![
                MessageContent::tool_request(id, Ok(CallToolRequestParams::new(name.to_string()))),
                MessageContent::tool_response(id, Ok(result)),
            ],
        )
    }

    /// The live bug. A PASSING cargo suite prints `0 failed`; `"failed"` is in
    /// `HARD`, so text sniffing scored two green runs as a repeat failure and
    /// `decide_hold` injected "Verify is still failing the same way" *after a
    /// pass*. The wire already carried `is_error: false` — read it.
    #[test]
    fn a_passing_suite_that_prints_zero_failed_is_not_a_failure() {
        let pass = "test result: ok. 42 passed; 0 failed; 0 ignored";
        let s = extract_from_messages(&[
            tool_exchange("developer__verify", "v1", pass, true),
            tool_exchange("developer__verify", "v2", pass, true),
        ]);

        assert_eq!(s.severity, 0.0, "a successful verify is not severe");
        assert_eq!(s.spinning, 0.0, "two green runs are not a spin");
        assert_eq!(
            crate::cost_router::decide_hold(
                crate::cost_router::WorkflowRole::Mechanical,
                true,
                &s,
                0
            ),
            crate::cost_router::HoldOutcome::Allow,
            "a green verify must never be answered with 'still failing'"
        );
    }

    /// The other half of the same claim: dropping text sniffing must not blind
    /// the detectors that were doing real work. A genuinely failing command,
    /// repeated, still scores as a spin, still corroborates a verify climb, and
    /// still holds — because a *failing* verify leaves `verify_ran` false.
    #[test]
    fn a_genuinely_failing_repeated_run_is_still_caught() {
        let fail = "error: linker exited with code 1";
        let s = extract_from_messages(&[
            tool_exchange("developer__verify", "v1", fail, false),
            tool_exchange("developer__verify", "v2", fail, false),
        ]);

        assert!(s.spinning >= 0.5, "the same failure twice is a spin");
        assert!(corroborates_verify_climb(&s));
        assert!(matches!(
            crate::cost_router::decide_hold(
                crate::cost_router::WorkflowRole::Mechanical,
                false,
                &s,
                0
            ),
            crate::cost_router::HoldOutcome::Hold { .. }
        ));
    }

    /// Two *different* failures deliberately stop scoring as a spin (see the
    /// `failing_shells >= 2` deletion), so severity is the axis that has to
    /// keep carrying them into the escalation path.
    #[test]
    fn two_different_failures_still_reach_the_escalation_path() {
        let s = extract_from_messages(&[
            tool_exchange("developer__verify", "v1", "error: type mismatch", false),
            tool_exchange("developer__verify", "v2", "error: no such file", false),
        ]);

        assert_eq!(s.spinning, 0.0, "different errors are not the same spin");
        assert!(s.severity >= 0.7, "but they are still hard failures");
        assert!(corroborates_verify_climb(&s));
        assert_eq!(corroborating_consecutive(1, &s, 3), 3);
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
