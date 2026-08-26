//! Best-effort signal extraction from a harness run's captured stdout+stderr
//! log.
//!
//! There is no structured event log to read instead — `permagent run
//! --output-format text` is a human transcript — so this is a dumb line
//! scanner over the CLI's *current* rendering, not a parser of a stable
//! machine format. If that rendering changes, these counts silently drift to
//! zero rather than error: treat [`LogSignals`] as a coarse, best-effort
//! supplement to the oracle's pass/fail verdict, never a substitute for it.
//!
//! The markers matched here are the literal strings the CLI prints today:
//!
//! - Tool-call banner: every text-editor/shell/delegate/todo/default tool
//!   call is announced by `print_tool_header`
//!   (`crates/goose-cli/src/session/output.rs:962-977`) with a line of the
//!   shape `"  ▸ {tool}"` or `"  ▸ {tool} {extension}"`, preceded by a
//!   40-dash separator. A delegated sub-agent tool call is announced the same
//!   way by `render_subagent_tool_call`
//!   (`crates/goose-cli/src/session/output.rs:887-911`), whose banner reads
//!   `"  ▸ [subagent:{id}] {tool}"` or `"  ▸ [subagent:{id}] {tool} |
//!   {extension}"` (built by `format_subagent_tool_call_message`,
//!   `crates/goose-cli/src/session/output.rs:876-885`). Both forms start the
//!   line with the `▸` glyph (U+25B8), which is what [`scan`] keys on — the
//!   surrounding `console`-crate color styling and the optional dash
//!   separator vary with `NO_COLOR`/tty detection, but the glyph and its
//!   position (first non-whitespace character on the line) do not.
//! - Turn-limit hit: when the agent loop exhausts `max_turns`, it yields the
//!   exact assistant text `"I've reached the maximum number of actions I can
//!   do without user input. Would you like me to continue?"`
//!   (`crates/goose/src/agents/agent.rs:1806-1813`). `render_message`'s
//!   `MessageContent::Text` arm prints that text via `print_markdown`
//!   (`crates/goose-cli/src/session/output.rs:236`), and since the harness's
//!   stdout is redirected to a log file rather than a tty, `print_markdown`
//!   takes its plain `print!("{}", content)` path
//!   (`crates/goose-cli/src/session/output.rs:997`) — so the sentence
//!   survives verbatim in the captured log.
//! - Rate limits: matched case-insensitively on "rate limit" / "rate_limit" /
//!   "429", per the eval's own convention (providers report rate limiting in
//!   varied prose; there is no single canonical CLI string for it).

/// Coarse signals mined from one harness run's combined stdout+stderr log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSignals {
    /// Number of tool-call banner lines seen (see module docs).
    pub tool_calls: usize,
    /// Number of lines that look like a provider rate-limit response.
    pub rate_limit_events: usize,
    /// Whether the harness exhausted its turn budget without finishing.
    pub max_turns_hit: bool,
    /// Distinct tool names invoked, in first-seen order.
    pub tool_names: Vec<String>,
}

/// The glyph `print_tool_header`/`render_subagent_tool_call` prefix every
/// tool-call banner with (see module docs for the exact call sites).
const TOOL_CALL_MARKER: char = '▸';

/// The exact sentence yielded when the agent loop exhausts `max_turns`
/// (`crates/goose/src/agents/agent.rs:1809`).
const MAX_TURNS_MARKER: &str =
    "I've reached the maximum number of actions I can do without user input.";

/// Scan a harness run's captured log text for tool-call, rate-limit and
/// turn-limit signals. Pure and best-effort: see the module docs for what it
/// matches and why it is not authoritative.
pub fn scan(log_text: &str) -> LogSignals {
    let mut signals = LogSignals::default();
    let mut seen_tools: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in log_text.lines() {
        if let Some(tool) = tool_call_banner_tool(line) {
            signals.tool_calls += 1;
            if seen_tools.insert(tool.clone()) {
                signals.tool_names.push(tool);
            }
        }

        let lower = line.to_ascii_lowercase();
        if lower.contains("rate limit") || lower.contains("rate_limit") || lower.contains("429") {
            signals.rate_limit_events += 1;
        }

        if line.contains(MAX_TURNS_MARKER) {
            signals.max_turns_hit = true;
        }
    }

    signals
}

/// If `line` is a tool-call banner, return the tool name. Requires the
/// (ANSI-stripped) `▸` glyph to be the FIRST non-whitespace character on the
/// line — this is what stops a line that merely mentions a tool name, or the
/// glyph, in prose from being counted.
fn tool_call_banner_tool(line: &str) -> Option<String> {
    let stripped = strip_ansi(line);
    let trimmed = stripped.trim_start();
    let rest = trimmed.strip_prefix(TOOL_CALL_MARKER)?;
    let rest = rest.trim_start();
    let first = rest.split_whitespace().next()?;

    if first.starts_with('[') {
        // The subagent form: "▸ [subagent:id] tool ..." — skip the bracketed
        // subagent marker to land on the real tool token.
        let after_bracket = rest.split_once(' ')?.1;
        // `split_whitespace` already skips leading whitespace, so no trim here.
        let tool = after_bracket.split_whitespace().next()?;
        return Some(tool.to_string());
    }
    Some(first.to_string())
}

/// Strip ANSI SGR escape sequences (`\x1b[...m`), the only escapes the CLI's
/// `console`-based styling emits. In practice the harness's log is captured
/// to a file (not a tty), so `console` auto-detects no color support and
/// these sequences are normally already absent — this is a defensive no-op
/// in that common case.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic log mirroring the real CLI's rendering: two ordinary tool
    /// banners, one subagent-delegated tool banner (reusing one tool name to
    /// prove de-duplication), a rate-limit line, prose that merely mentions a
    /// tool name and the glyph (must NOT be counted), and the turn-limit
    /// sentence.
    fn sample_log() -> String {
        [
            "",
            "  ────────────────────────────────────────",
            "  ▸ shell",
            "    ls -la",
            "",
            "  ────────────────────────────────────────",
            "  ▸ text_editor developer",
            "    path /tmp/foo.py",
            "",
            "I'll now use the shell tool to look around, see the ▸ marker style.",
            "",
            "  ▸ [subagent:worker_42] delegate | developer",
            "",
            "Error: rate limit exceeded (HTTP 429), please retry",
            "",
            "I've reached the maximum number of actions I can do without user input. \
             Would you like me to continue?",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn scans_a_realistic_log_for_all_signals() {
        let signals = scan(&sample_log());
        assert_eq!(signals.tool_calls, 3, "{signals:?}");
        assert_eq!(signals.rate_limit_events, 1, "{signals:?}");
        assert!(signals.max_turns_hit);
        assert_eq!(signals.tool_names, vec!["shell", "text_editor", "delegate"]);
    }

    #[test]
    fn empty_log_yields_zeroes() {
        assert_eq!(scan(""), LogSignals::default());
    }

    #[test]
    fn a_tool_name_that_only_appears_in_prose_is_not_counted() {
        let log = "I'll now use the shell tool to look around, see the ▸ marker style.\n\
                   The developer extension exposes text_editor too.\n";
        let signals = scan(log);
        assert_eq!(signals.tool_calls, 0);
        assert!(signals.tool_names.is_empty());
        assert_eq!(signals.rate_limit_events, 0);
        assert!(!signals.max_turns_hit);
    }

    #[test]
    fn rate_limit_matches_are_case_insensitive_and_match_429() {
        let log = "RATE LIMIT hit\nsome 429 status\nRate_Limit backoff\n";
        assert_eq!(scan(log).rate_limit_events, 3);
    }

    #[test]
    fn tool_banner_without_extension_still_counts() {
        let signals = scan("  ▸ todo__write\n");
        assert_eq!(signals.tool_calls, 1);
        assert_eq!(signals.tool_names, vec!["todo__write"]);
    }

    #[test]
    fn strips_ansi_styling_around_the_glyph_and_tool_name() {
        let log = "  \u{1b}[2m▸\u{1b}[0m \u{1b}[2mshell\u{1b}[0m\n";
        let signals = scan(log);
        assert_eq!(signals.tool_calls, 1);
        assert_eq!(signals.tool_names, vec!["shell"]);
    }
}
