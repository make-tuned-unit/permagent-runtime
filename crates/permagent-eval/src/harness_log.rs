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
//! [`LogSignals::tool_names`] IS the tool-call sequence G3's
//! [`crate::selection::SelectionCheck`] evaluates a `category: "selection"`
//! fixture against (see that module) — first-seen order, de-duplicated. A
//! `before` ordering rule only ever needs a tool's FIRST occurrence, which
//! de-duplication preserves exactly (the first-seen index never changes when
//! a repeated call is dropped), so `selection_check` reads this field
//! directly rather than needing a second, non-deduplicated sequence.
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
    /// The FIRST `planner=<provider/model> reason=<...> workers=<...>`
    /// route-tracing line seen, if any (see [`parse_planner_line`] for the
    /// exact grammar this is best-effort matched against). `None` for a run
    /// whose harness build/version does not emit this line yet — this
    /// module never fabricates a route from anything else.
    pub planner: Option<PlannerRoute>,
}

/// One coordinator's routing decision, as best-effort parsed from a
/// `planner=<provider>/<model> reason=<...> workers=<provider>/<model>,...`
/// tracing line (D2, Lane Z4 brief — extends the harness's own
/// `planner`-line convention so `permagent-eval graduate` can attribute a
/// retained run's evidence to the coordinator AND any worker roles it
/// actually delegated to, not just the tier's pinned model). `workers` is
/// empty (never absent) when the run delegated to nobody.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlannerRoute {
    pub provider: String,
    pub model: String,
    /// Free text, matched as a single whitespace-delimited token — the
    /// grammar has no quoting, so a reason containing a space is truncated
    /// at the first space (best-effort, like the rest of this module).
    pub reason: String,
    /// Each entry is `provider/model`, split verbatim on `/` (not
    /// re-validated against the knowledge base here).
    pub workers: Vec<String>,
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

        if signals.planner.is_none() {
            signals.planner = parse_planner_line(line);
        }
    }

    signals
}

/// Best-effort parse of a `planner=<provider>/<model> reason=<...>
/// workers=<provider>/<model>,...` line (see [`PlannerRoute`]'s doc for the
/// grammar and why it exists). Matches the `planner=` token ANYWHERE on the
/// line (not only as the first token) since a tracing line is normally
/// prefixed by a timestamp/level the way the `console` crate's own styling
/// prefixes the tool-call banner — see the module doc for that precedent.
/// `reason=`/`workers=` are optional; `workers=` with nothing after the `=`
/// parses as an empty (not absent) list. Returns `None` when no `planner=`
/// token is found, or when it does not carry a `provider/model` pair.
fn parse_planner_line(line: &str) -> Option<PlannerRoute> {
    let stripped = strip_ansi(line);
    let mut provider_model: Option<(String, String)> = None;
    let mut reason: Option<String> = None;
    let mut workers: Option<Vec<String>> = None;
    for token in stripped.split_whitespace() {
        if let Some(rest) = token.strip_prefix("planner=") {
            if let Some((p, m)) = rest.split_once('/') {
                if !p.is_empty() && !m.is_empty() {
                    provider_model = Some((p.to_string(), m.to_string()));
                }
            }
        } else if let Some(rest) = token.strip_prefix("reason=") {
            reason = Some(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("workers=") {
            workers = Some(if rest.is_empty() {
                Vec::new()
            } else {
                rest.split(',').map(|s| s.to_string()).collect()
            });
        }
    }
    let (provider, model) = provider_model?;
    Some(PlannerRoute {
        provider,
        model,
        reason: reason.unwrap_or_default(),
        workers: workers.unwrap_or_default(),
    })
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

    // ── D2/D4: the `planner=` route-tracing line ──────────────────────────

    #[test]
    fn parses_a_planner_line_with_reason_and_workers() {
        let log = "2026-09-09T10:00:00Z INFO planner=anthropic/claude-sonnet-5 reason=strongest \
                    workers=zai/glm-4.7,ollama/qwen3\n";
        let signals = scan(log);
        let route = signals.planner.expect("a planner= line must be captured");
        assert_eq!(route.provider, "anthropic");
        assert_eq!(route.model, "claude-sonnet-5");
        assert_eq!(route.reason, "strongest");
        assert_eq!(
            route.workers,
            vec!["zai/glm-4.7".to_string(), "ollama/qwen3".to_string()]
        );
    }

    #[test]
    fn a_planner_line_with_empty_workers_parses_as_an_empty_list_not_none() {
        let log = "planner=ollama/qwen2.5:7b reason=only_candidate workers=\n";
        let signals = scan(log);
        let route = signals.planner.expect("must still capture the route");
        assert_eq!(route.provider, "ollama");
        assert_eq!(route.model, "qwen2.5:7b");
        assert!(route.workers.is_empty());
    }

    #[test]
    fn a_planner_line_missing_reason_and_workers_still_captures_provider_model() {
        let log = "planner=zai/glm-4.7\n";
        let signals = scan(log);
        let route = signals.planner.expect("must capture provider/model alone");
        assert_eq!(route.provider, "zai");
        assert_eq!(route.model, "glm-4.7");
        assert_eq!(route.reason, "");
        assert!(route.workers.is_empty());
    }

    #[test]
    fn only_the_first_planner_line_is_captured() {
        let log = "planner=anthropic/claude-sonnet-5 reason=first workers=\n\
                    planner=openai/gpt-5.6 reason=second workers=\n";
        let signals = scan(log);
        assert_eq!(signals.planner.unwrap().reason, "first");
    }

    #[test]
    fn no_planner_line_leaves_it_none() {
        let signals = scan("nothing planner-shaped here\n");
        assert!(signals.planner.is_none());
    }

    #[test]
    fn a_malformed_planner_token_without_a_slash_is_ignored() {
        let signals = scan("planner=not-a-pair reason=x workers=\n");
        assert!(signals.planner.is_none());
    }
}
