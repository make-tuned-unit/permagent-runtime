//! Best-effort signal extraction from a harness run's captured stdout+stderr
//! log.
//!
//! There is no structured event log to read instead — `permagent run
//! --output-format text` is a human transcript — so this is a dumb line
//! scanner over the CLI's *current* rendering, not a parser of a stable
//! machine format. If that rendering changes, these counts silently drift to
//! zero rather than error for legacy counts (v1 receipts stay `None`):
//! treat [`LogSignals`] as a coarse, best-effort
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
//! - Rate limits: matched case-insensitively on the phrases "rate limit" /
//!   "rate_limit" / "too many requests", or a bare `429` **only** when it
//!   appears next to `status`/`http` context or in parentheses (`status: 429`,
//!   `HTTP 429`, `(429)`). Lane M5's run of `01a08578-...-7b7fe0189429`
//!   reported 4 phantom rate-limit events from a bare `.contains("429")`
//!   matching that goal-card UUID's trailing digits (see `### Lane M5
//!   report`, `HARNESS_LOOP_LOG.md`) — a plain word-boundary on `429` is not
//!   enough either, since a token count like "10,429 out" would still match
//!   it. Requiring HTTP-status context (or the unambiguous phrases above) is
//!   what keeps hex UUIDs and token counts out while still catching
//!   `Request failed: … (status: 429)` (`ProviderError`'s `RequestFailed`
//!   Display, built by `provider_error_from_reqwest`,
//!   `crates/goose/src/providers/errors.rs`) and the harness's own
//!   retry-exhausted notice, e.g. "Z.AI rate limit — waited and retried, and
//!   it is still limited…" (the `RateLimitExceeded` arm of the reply loop,
//!   `crates/goose/src/agents/agent.rs`, built from
//!   `wait_status::rate_limit_headline`). The first three matching lines are
//!   kept verbatim in [`LogSignals::rate_limit_examples`] so a future reader
//!   can verify the count without re-reading the whole log.
//!
//! ## X-obs-1: signals from lanes still landing
//!
//! The remaining [`LogSignals`] fields are best-effort parses of lines OTHER
//! in-flight lanes of the coding-harness excellence DAG print (see
//! `docs/orchestrator/CODING_HARNESS_EXCELLENCE_ITERATION_DAG.md`): X-lat's
//! `harness_overhead` timing line, X-land's `landed <id> → <sha>` lines and
//! drain summary, X-dag's `create_roadmap` dispatch count, and X-verify's
//! reviewer/vacuous-pass receipts. None of those lanes had landed in this
//! binary as of this module's writing, so every one of these fields is
//! expected to read as absent (`None`/empty) until they do — that is the
//! correct, honest reading, not a bug in the regexes below.
//!
//! Each is matched with a REGEX rather than an exact string, deliberately
//! tolerant of formatting drift the producing lane might still make (extra
//! whitespace, a `ms`/unit suffix on a number, `->` vs `→`) — the exact
//! patterns are documented above each field. Critically: **absence must read
//! as "unknown", never as a defaulted/fabricated value**. A counter field
//! (`create_roadmap_dispatched`, the reviewer receipts, `vacuous_pass`) is
//! `Option<usize>`, not a bare `usize` that defaults to `0` — `None` means "no
//! matching line found in this log", which the qualification layer
//! ([`crate::qualification`]) must treat as missing evidence. In particular a
//! reviewer-mutation detector that has not been wired into the binary yet
//! must NEVER be read as "checked, and it was clean": that would turn a
//! silent gap in instrumentation into a false safety pass.

/// Wall-clock breakdown parsed from the line
/// `harness_overhead p50=<ms> p95=<ms> model=<ms> tools=<ms> wall=<ms>` that
/// Lane X-lat prints once it lands (separating harness overhead from model
/// and tool time — the parent DAG's Latency area). All fields are
/// milliseconds. The regex tolerates an optional non-digit unit suffix on
/// each number (e.g. `p50=86ms`) and extra whitespace between fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarnessOverhead {
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub model_ms: u64,
    pub tools_ms: u64,
    pub wall_ms: u64,
}

/// One `landed <id> → <sha>` line (Lane X-land: a dispatched goal's edits
/// landing into the oracle-visible tree under `PERMAGENT_HEADLESS_AUTOLAND` —
/// see [`crate::runner::run_task`]'s Lane K3 wiring). `id` is the goal id
/// exactly as printed; `sha` is the commit it landed as, which may be
/// abbreviated. The regex accepts either `→` or an ASCII `->` arrow.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LandedGoal {
    pub id: String,
    pub sha: String,
}

/// Atomic RECEIPT CONTRACT v1 evidence for `dag_enforcement`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DagEnforcementReceipt {
    pub roadmap_dispatched: usize,
    pub bypass_attempts: usize,
    pub bypass_passes: usize,
    pub coordinator_mutations: usize,
}

/// Atomic RECEIPT CONTRACT v1 evidence for `verification_integrity`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerificationIntegrityReceipt {
    pub goals: usize,
    pub review_reached: usize,
    pub vacuous_passes: usize,
    pub reviewer_mutations: usize,
    pub reviewer_not_independent: usize,
}

/// Atomic RECEIPT CONTRACT v1 evidence for `safety_and_trust`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SafetyAndTrustReceipt {
    pub jail_denials: usize,
    pub jail_bypasses: usize,
    pub unapproved_skill_runs: usize,
    pub approval_bypasses: usize,
    pub adversary_blocks: usize,
}

/// Atomic RECEIPT CONTRACT v1 evidence for `cost_routing`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CostRoutingReceipt {
    pub dispatches: usize,
    pub cheapest_capable: usize,
    pub pack_honored: usize,
    pub planner: String,
}

/// Coarse signals mined from one harness run's combined stdout+stderr log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSignals {
    /// Number of tool-call banner lines seen (see module docs).
    pub tool_calls: usize,
    /// Number of lines that look like a genuine provider rate-limit signal
    /// (see the module docs for exactly what counts and why a bare `429`
    /// substring does not).
    pub rate_limit_events: usize,
    /// The first (up to [`RATE_LIMIT_EXAMPLES_CAP`]) lines that matched
    /// [`RATE_LIMIT_RE`], verbatim, in file order — so a report reader can
    /// verify [`Self::rate_limit_events`] without re-reading the whole log.
    /// Empty when `rate_limit_events` is 0.
    pub rate_limit_examples: Vec<String>,
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
    /// The first `harness_overhead p50=… …` line's parse, if the log has one.
    /// See the module docs' "X-obs-1" section: `None` means no such line was
    /// found, NOT that overhead was zero.
    pub harness_overhead: Option<HarnessOverhead>,
    /// Every `landed <id> → <sha>` line, in file order. Empty means none were
    /// found — which, under `PERMAGENT_HEADLESS_AUTOLAND=1`, is itself a
    /// meaningful (zero-landed) reading rather than missing evidence, since a
    /// `Vec` naturally represents "found zero or more".
    pub landed_goals: Vec<LandedGoal>,
    /// The first line matching `drain summary` (case-insensitive), with the
    /// marker stripped — the raw remainder is kept verbatim rather than
    /// parsed into fields, since the drain-summary line shape is still owned
    /// by Lane X-land and not yet fixed. `None` means no such line was found.
    pub drain_summary: Option<String>,
    /// The dispatched-goal count from a `create_roadmap … dispatched <n>`
    /// line (Lane X-dag). `None` means no such line was found.
    pub create_roadmap_dispatched: Option<usize>,
    /// Count of lines matching the literal reviewer receipt "review not
    /// independent" (Lane X-verify), case-insensitive. `None` means no such
    /// line was found — this must NOT be read as "reviewer was independent
    /// every time", since the detector emitting this receipt may simply not
    /// be wired into the binary under test yet.
    pub reviewer_not_independent: Option<usize>,
    /// Count of lines matching the literal reviewer receipt "reviewer
    /// mutated" (Lane X-verify), case-insensitive. `None` means no such line
    /// was found — see [`Self::reviewer_not_independent`] for why this must
    /// not be read as a verified-clean zero.
    pub reviewer_mutated: Option<usize>,
    /// Count of lines mentioning `vacuous_pass`/`vacuous pass` (Lane
    /// X-verify's vacuous-pass counter), case-insensitive. `None` means no
    /// such line was found — see [`Self::reviewer_not_independent`].
    pub vacuous_pass: Option<usize>,
    /// Complete `dag_enforcement` receipt; absent or malformed lines remain unknown.
    pub dag_enforcement: Option<DagEnforcementReceipt>,
    /// Complete `verification_integrity` receipt; absent or malformed lines remain unknown.
    pub verification_integrity: Option<VerificationIntegrityReceipt>,
    /// Complete `safety_and_trust` receipt; absent or malformed lines remain unknown.
    pub safety_and_trust: Option<SafetyAndTrustReceipt>,
    /// Complete `cost_routing` receipt; absent or malformed lines remain unknown.
    pub cost_routing: Option<CostRoutingReceipt>,
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

/// How many verbatim [`RATE_LIMIT_RE`] matches [`scan`] keeps in
/// [`LogSignals::rate_limit_examples`].
const RATE_LIMIT_EXAMPLES_CAP: usize = 3;

/// A genuine rate-limit signal — deliberately NOT a bare `.contains("429")`.
///
/// Lane M5 found that substring check false-positiving on a goal-card UUID
/// ending in `429` (see the module docs). This regex instead matches only:
///
/// - the unambiguous phrases "rate limit" / "rate_limit" / "too many
///   requests" (case-insensitive, no boundary needed — these phrases don't
///   occur inside hex ids or plain numbers), or
/// - a `429` that sits next to explicit HTTP-status context: `status`/`http`
///   within 10 non-digit characters before it, or wrapped in parentheses
///   (`status: 429`, `HTTP 429`, `(429)`).
///
/// A bare `\b429\b` alone is NOT enough: a token count like "10,429 out" has
/// a word boundary on both sides of `429` too, so the status/parens context
/// requirement is what actually keeps hex UUIDs (digits run together, no
/// boundary at all — `\b` never fires inside "...0189429") *and* token counts
/// (boundary, but no HTTP context) from matching. (The `regex` crate has no
/// look-around, so "no trailing digit" is expressed the same way: `\b` already
/// refuses to match right before another digit, since digits are word
/// characters too — no explicit negative lookahead is needed or available.)
static RATE_LIMIT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)rate[ _]limit|too many requests|\b(?:status|http)\b[^0-9\n]{0,10}\b429\b|\(\s*429\s*\)",
    )
    .expect("RATE_LIMIT_RE is a valid regex")
});

/// `harness_overhead p50=<ms> p95=<ms> model=<ms> tools=<ms> wall=<ms>` — the
/// exact key names Lane X-lat's spec gives, each value captured as digits
/// only so an optional unit suffix (`86ms`) is tolerated without being
/// captured.
static HARNESS_OVERHEAD_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"harness_overhead\s+p50=(\d+)\w*\s+p95=(\d+)\w*\s+model=(\d+)\w*\s+tools=(\d+)\w*\s+wall=(\d+)\w*",
    )
    .expect("HARNESS_OVERHEAD_RE is a valid regex")
});

/// `landed <id> → <sha>` — `id` is any run of non-space characters, `sha` is
/// 6-40 hex characters (full or abbreviated). Accepts either the unicode `→`
/// or an ASCII `->` arrow.
static LANDED_GOAL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"landed\s+(\S+)\s*(?:→|->)\s*([0-9a-fA-F]{6,40})\b")
        .expect("LANDED_GOAL_RE is a valid regex")
});

/// A line mentioning "drain summary" (case-insensitive, optional colon),
/// capturing whatever text follows verbatim — the summary's own field shape
/// is still owned by Lane X-land and not fixed yet.
static DRAIN_SUMMARY_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)drain\s*summary\s*:?\s*(.+)")
        .expect("DRAIN_SUMMARY_RE is a valid regex")
});

/// A `create_roadmap` line (Lane X-dag) reporting how many goals it
/// dispatched, tolerant of intervening words between the two keywords (e.g.
/// `create_roadmap: dispatched 3 goal(s)`).
static CREATE_ROADMAP_DISPATCHED_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)create_roadmap\b.*?dispatched\s+(\d+)")
            .expect("CREATE_ROADMAP_DISPATCHED_RE is a valid regex")
    });

/// The literal reviewer receipt "review not independent" (Lane X-verify),
/// case-insensitive.
static REVIEW_NOT_INDEPENDENT_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)review not independent")
            .expect("REVIEW_NOT_INDEPENDENT_RE is a valid regex")
    });

/// The literal reviewer receipt "reviewer mutated" (Lane X-verify),
/// case-insensitive.
static REVIEWER_MUTATED_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)reviewer mutated").expect("REVIEWER_MUTATED_RE is a valid regex")
});

/// The vacuous-pass flag (Lane X-verify), accepting either `vacuous_pass` or
/// `vacuous pass`, case-insensitive.
static VACUOUS_PASS_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)vacuous[_ ]pass").expect("VACUOUS_PASS_RE is a valid regex")
});

// Receipt regexes are anchored: prose, partial fields, signed counts and
// trailing junk cannot masquerade as a complete exit receipt.
static DAG_ENFORCEMENT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*receipt\s+dag_enforcement\s+roadmap_dispatched=([0-9]+)\s+bypass_attempts=([0-9]+)\s+bypass_passes=([0-9]+)\s+coordinator_mutations=([0-9]+)\s*$",
    )
    .expect("DAG_ENFORCEMENT_RE is a valid regex")
});

fn parse_dag_enforcement(line: &str) -> Option<DagEnforcementReceipt> {
    let caps = DAG_ENFORCEMENT_RE.captures(line)?;
    Some(DagEnforcementReceipt {
        roadmap_dispatched: caps[1].parse().ok()?,
        bypass_attempts: caps[2].parse().ok()?,
        bypass_passes: caps[3].parse().ok()?,
        coordinator_mutations: caps[4].parse().ok()?,
    })
}

static VERIFICATION_INTEGRITY_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*receipt\s+verification_integrity\s+goals=([0-9]+)\s+review_reached=([0-9]+)\s+vacuous_passes=([0-9]+)\s+reviewer_mutations=([0-9]+)\s+reviewer_not_independent=([0-9]+)\s*$",
    )
    .expect("VERIFICATION_INTEGRITY_RE is a valid regex")
});

fn parse_verification_integrity(line: &str) -> Option<VerificationIntegrityReceipt> {
    let caps = VERIFICATION_INTEGRITY_RE.captures(line)?;
    Some(VerificationIntegrityReceipt {
        goals: caps[1].parse().ok()?,
        review_reached: caps[2].parse().ok()?,
        vacuous_passes: caps[3].parse().ok()?,
        reviewer_mutations: caps[4].parse().ok()?,
        reviewer_not_independent: caps[5].parse().ok()?,
    })
}

static SAFETY_AND_TRUST_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*receipt\s+safety_and_trust\s+jail_denials=([0-9]+)\s+jail_bypasses=([0-9]+)\s+unapproved_skill_runs=([0-9]+)\s+approval_bypasses=([0-9]+)\s+adversary_blocks=([0-9]+)\s*$",
    )
    .expect("SAFETY_AND_TRUST_RE is a valid regex")
});

fn parse_safety_and_trust(line: &str) -> Option<SafetyAndTrustReceipt> {
    let caps = SAFETY_AND_TRUST_RE.captures(line)?;
    Some(SafetyAndTrustReceipt {
        jail_denials: caps[1].parse().ok()?,
        jail_bypasses: caps[2].parse().ok()?,
        unapproved_skill_runs: caps[3].parse().ok()?,
        approval_bypasses: caps[4].parse().ok()?,
        adversary_blocks: caps[5].parse().ok()?,
    })
}

static COST_ROUTING_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*receipt\s+cost_routing\s+dispatches=([0-9]+)\s+cheapest_capable=([0-9]+)\s+pack_honored=([0-9]+)\s+planner=(\S+/\S+)\s*$",
    )
    .expect("COST_ROUTING_RE is a valid regex")
});

fn parse_cost_routing(line: &str) -> Option<CostRoutingReceipt> {
    let caps = COST_ROUTING_RE.captures(line)?;
    Some(CostRoutingReceipt {
        dispatches: caps[1].parse().ok()?,
        cheapest_capable: caps[2].parse().ok()?,
        pack_honored: caps[3].parse().ok()?,
        planner: caps[4].to_string(),
    })
}

/// Scan a harness run's captured log text for tool-call, rate-limit,
/// turn-limit and (best-effort, see the module docs' "X-obs-1" section) the
/// still-landing lanes' signals. Pure and best-effort: see the module docs
/// for what it matches and why it is not authoritative.
pub fn scan(log_text: &str) -> LogSignals {
    let mut signals = LogSignals::default();
    let mut seen_tools: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in log_text.lines() {
        // Keep receipt fields separate from legacy diagnostic counters. In
        // particular, `vacuous_passes=0` is not a vacuous-pass event.
        let receipt_line = strip_ansi(line);
        if receipt_line.split_whitespace().next() == Some("receipt") {
            if signals.dag_enforcement.is_none() {
                signals.dag_enforcement = parse_dag_enforcement(&receipt_line);
            }
            if signals.verification_integrity.is_none() {
                signals.verification_integrity = parse_verification_integrity(&receipt_line);
            }
            if signals.safety_and_trust.is_none() {
                signals.safety_and_trust = parse_safety_and_trust(&receipt_line);
            }
            if signals.cost_routing.is_none() {
                signals.cost_routing = parse_cost_routing(&receipt_line);
            }
            continue;
        }

        if let Some(tool) = tool_call_banner_tool(line) {
            signals.tool_calls += 1;
            if seen_tools.insert(tool.clone()) {
                signals.tool_names.push(tool);
            }
        }

        if RATE_LIMIT_RE.is_match(line) {
            signals.rate_limit_events += 1;
            if signals.rate_limit_examples.len() < RATE_LIMIT_EXAMPLES_CAP {
                signals.rate_limit_examples.push(line.to_string());
            }
        }

        if line.contains(MAX_TURNS_MARKER) {
            signals.max_turns_hit = true;
        }

        if signals.planner.is_none() {
            signals.planner = parse_planner_line(line);
        }

        if signals.harness_overhead.is_none() {
            if let Some(caps) = HARNESS_OVERHEAD_RE.captures(line) {
                signals.harness_overhead = parse_harness_overhead(&caps);
            }
        }

        if let Some(caps) = LANDED_GOAL_RE.captures(line) {
            signals.landed_goals.push(LandedGoal {
                id: caps[1].to_string(),
                sha: caps[2].to_string(),
            });
        }

        if signals.drain_summary.is_none() {
            if let Some(caps) = DRAIN_SUMMARY_RE.captures(line) {
                let text = caps[1].trim();
                if !text.is_empty() {
                    signals.drain_summary = Some(text.to_string());
                }
            }
        }

        if signals.create_roadmap_dispatched.is_none() {
            if let Some(caps) = CREATE_ROADMAP_DISPATCHED_RE.captures(line) {
                if let Ok(n) = caps[1].parse::<usize>() {
                    signals.create_roadmap_dispatched = Some(n);
                }
            }
        }

        if REVIEW_NOT_INDEPENDENT_RE.is_match(line) {
            *signals.reviewer_not_independent.get_or_insert(0) += 1;
        }
        if REVIEWER_MUTATED_RE.is_match(line) {
            *signals.reviewer_mutated.get_or_insert(0) += 1;
        }
        if VACUOUS_PASS_RE.is_match(line) {
            *signals.vacuous_pass.get_or_insert(0) += 1;
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

/// Parse a [`HARNESS_OVERHEAD_RE`] match into a [`HarnessOverhead`]. Returns
/// `None` on the (practically unreachable, since the regex only captures
/// digit runs) case a capture fails to parse as `u64` — never on a genuinely
/// missing line, which is filtered out before this is called.
fn parse_harness_overhead(caps: &regex::Captures<'_>) -> Option<HarnessOverhead> {
    Some(HarnessOverhead {
        p50_ms: caps[1].parse().ok()?,
        p95_ms: caps[2].parse().ok()?,
        model_ms: caps[3].parse().ok()?,
        tools_ms: caps[4].parse().ok()?,
        wall_ms: caps[5].parse().ok()?,
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
    fn rate_limit_matches_are_case_insensitive_across_the_real_phrasings() {
        let log = "RATE LIMIT hit\nstatus: 429 too many requests\nRate_Limit backoff\n";
        assert_eq!(scan(log).rate_limit_events, 3);
    }

    /// Lane M5's defect (`### Lane M5 report`, `HARNESS_LOOP_LOG.md`): a bare
    /// `.contains("429")` false-positived on this exact goal-card id from a
    /// real eval run, reporting 4 phantom rate-limit events across the
    /// transcript when there were none. The fix must report 0 for it,
    /// however many times the id recurs.
    #[test]
    fn a_uuid_ending_in_429_is_not_a_rate_limit_event() {
        let log = "dispatched goal 01a08578-5932-71c2-9c53-7b7fe0189429 to worker\n\
                    landed 01a08578-5932-71c2-9c53-7b7fe0189429 -> abc1234def\n\
                    child session for 01a08578-5932-71c2-9c53-7b7fe0189429 exited 0\n\
                    goal 01a08578-5932-71c2-9c53-7b7fe0189429 verified\n";
        let signals = scan(log);
        assert_eq!(signals.rate_limit_events, 0, "{signals:?}");
        assert!(signals.rate_limit_examples.is_empty(), "{signals:?}");
    }

    /// The exact shape a real HTTP 429 takes once it has fallen through to
    /// `provider_error_from_reqwest` and been wrapped by `ProviderError`'s
    /// `RequestFailed` Display (`crates/goose/src/providers/errors.rs`) and
    /// printed by the reply loop's generic error arm
    /// (`crates/goose/src/agents/agent.rs`).
    #[test]
    fn a_real_429_provider_error_line_counts_as_one() {
        let log = "Ran into this error: Request failed: upstream rejected the request \
                    (status: 429).\n";
        let signals = scan(log);
        assert_eq!(signals.rate_limit_events, 1, "{signals:?}");
        assert_eq!(
            signals.rate_limit_examples,
            vec![log.trim_end().to_string()]
        );
    }

    /// The harness's own retry-exhausted notice: the `RateLimitExceeded` arm
    /// of the reply loop's error handling
    /// (`crates/goose/src/agents/agent.rs`), built from
    /// `wait_status::rate_limit_headline` and printed to the transcript via
    /// `render_message`'s `SystemNotificationType::InlineMessage` arm
    /// (`crates/goose-cli/src/session/output.rs`).
    #[test]
    fn a_retrying_after_rate_limit_line_counts_as_one() {
        let log = "Z.AI rate limit — waited and retried, and it is still limited. \
                    Your message was not sent; send it again when the limit clears.\n";
        let signals = scan(log);
        assert_eq!(signals.rate_limit_events, 1, "{signals:?}");
    }

    /// The other half of the M5 fix: a plain word-boundary on `429` is not
    /// enough on its own, since a token count sits at a word boundary too.
    #[test]
    fn a_bare_token_count_that_happens_to_read_429_does_not_match() {
        let log = "699,528 in / 429 out tokens\n";
        assert_eq!(scan(log).rate_limit_events, 0);
    }

    #[test]
    fn rate_limit_examples_are_capped_at_three_and_kept_verbatim() {
        let log = "rate limit hit 1\nrate limit hit 2\nrate limit hit 3\nrate limit hit 4\n";
        let signals = scan(log);
        assert_eq!(signals.rate_limit_events, 4);
        assert_eq!(
            signals.rate_limit_examples,
            vec!["rate limit hit 1", "rate limit hit 2", "rate limit hit 3"]
        );
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

    // --- X-obs-1: the still-landing lanes' signals -------------------------

    #[test]
    fn harness_overhead_line_is_parsed_tolerating_a_unit_suffix() {
        let log = "harness_overhead p50=86ms p95=140ms model=120ms tools=15ms wall=142ms\n";
        let signals = scan(log);
        assert_eq!(
            signals.harness_overhead,
            Some(HarnessOverhead {
                p50_ms: 86,
                p95_ms: 140,
                model_ms: 120,
                tools_ms: 15,
                wall_ms: 142,
            })
        );
    }

    #[test]
    fn harness_overhead_line_is_parsed_without_a_unit_suffix_too() {
        let log = "  harness_overhead   p50=1 p95=2 model=3 tools=4 wall=5  \n";
        let signals = scan(log);
        assert_eq!(
            signals.harness_overhead,
            Some(HarnessOverhead {
                p50_ms: 1,
                p95_ms: 2,
                model_ms: 3,
                tools_ms: 4,
                wall_ms: 5,
            })
        );
    }

    #[test]
    fn landed_goal_lines_are_parsed_with_either_arrow_style() {
        let log = "landed goal-42 → abc1234\nlanded goal-43 -> def5678deadbeef\n";
        let signals = scan(log);
        assert_eq!(
            signals.landed_goals,
            vec![
                LandedGoal {
                    id: "goal-42".to_string(),
                    sha: "abc1234".to_string(),
                },
                LandedGoal {
                    id: "goal-43".to_string(),
                    sha: "def5678deadbeef".to_string(),
                },
            ]
        );
    }

    #[test]
    fn drain_summary_line_is_captured_verbatim_after_the_marker() {
        let log = "some preamble\ndrain summary: landed=2 pending=1 timed_out=0\nmore output\n";
        let signals = scan(log);
        assert_eq!(
            signals.drain_summary.as_deref(),
            Some("landed=2 pending=1 timed_out=0")
        );
    }

    #[test]
    fn create_roadmap_dispatched_count_is_parsed_with_words_between() {
        let log = "create_roadmap: dispatched 3 goal(s)\n";
        let signals = scan(log);
        assert_eq!(signals.create_roadmap_dispatched, Some(3));
    }

    #[test]
    fn reviewer_receipts_are_counted_case_insensitively() {
        let log = "Review not independent: shared the worker's session\n\
                   some other line\n\
                   REVIEWER MUTATED the tree at commit abc123\n\
                   review not independent again\n";
        let signals = scan(log);
        assert_eq!(signals.reviewer_not_independent, Some(2));
        assert_eq!(signals.reviewer_mutated, Some(1));
    }

    #[test]
    fn vacuous_pass_flags_are_counted_with_either_spelling() {
        let log = "vacuous_pass detected on task foo\nanother vacuous pass here\n";
        let signals = scan(log);
        assert_eq!(signals.vacuous_pass, Some(2));
    }

    /// The whole point of X-obs-1: a log with none of these lines (i.e. the
    /// producing lane has not landed in the binary under test) must report
    /// every one of them as `None`/empty — NEVER a fabricated zero/clean
    /// value that could be misread as "checked, and it passed".
    #[test]
    fn all_new_fields_are_unknown_not_defaulted_when_absent() {
        let signals = scan("plain ordinary harness output\nno special markers here\n");
        assert_eq!(signals.harness_overhead, None);
        assert!(signals.landed_goals.is_empty());
        assert_eq!(signals.drain_summary, None);
        assert_eq!(signals.create_roadmap_dispatched, None);
        assert_eq!(signals.reviewer_not_independent, None);
        assert_eq!(signals.reviewer_mutated, None);
        assert_eq!(signals.vacuous_pass, None);
    }

    #[test]
    fn contract_v1_dag_receipt_is_atomic() {
        let line = "receipt dag_enforcement roadmap_dispatched=3 bypass_attempts=2 bypass_passes=0 coordinator_mutations=0";
        assert_eq!(
            scan(line).dag_enforcement,
            Some(DagEnforcementReceipt {
                roadmap_dispatched: 3,
                bypass_attempts: 2,
                bypass_passes: 0,
                coordinator_mutations: 0,
            })
        );
        assert!(scan(&line.replace("bypass_passes=0", "bypass_passes=-1"))
            .dag_enforcement
            .is_none());
    }

    #[test]
    fn contract_v1_verification_receipt_is_not_a_legacy_violation() {
        let line = "receipt verification_integrity goals=3 review_reached=2 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0";
        let signals = scan(line);
        assert_eq!(
            signals.verification_integrity,
            Some(VerificationIntegrityReceipt {
                goals: 3,
                review_reached: 2,
                vacuous_passes: 0,
                reviewer_mutations: 0,
                reviewer_not_independent: 0,
            })
        );
        assert_eq!(signals.vacuous_pass, None);
        assert_eq!(signals.reviewer_mutated, None);
        assert!(scan(&line.replace(" review_reached=2", ""))
            .verification_integrity
            .is_none());
    }

    #[test]
    fn contract_v1_safety_receipt_retains_informational_counts() {
        let line = "receipt safety_and_trust jail_denials=7 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=4";
        assert_eq!(
            scan(line).safety_and_trust,
            Some(SafetyAndTrustReceipt {
                jail_denials: 7,
                jail_bypasses: 0,
                unapproved_skill_runs: 0,
                approval_bypasses: 0,
                adversary_blocks: 4,
            })
        );
        assert!(scan(&format!("{line}junk")).safety_and_trust.is_none());
    }

    #[test]
    fn contract_v1_routing_receipt_retains_planner() {
        let line = "receipt cost_routing dispatches=4 cheapest_capable=4 pack_honored=4 planner=openrouter/qwen/model";
        assert_eq!(
            scan(line).cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 4,
                cheapest_capable: 4,
                pack_honored: 4,
                planner: "openrouter/qwen/model".to_string(),
            })
        );
        assert!(scan(&line.replace("openrouter/qwen/model", "unknown"))
            .cost_routing
            .is_none());
        // A terminal routing receipt must not overwrite the separate route trace.
        assert!(scan(line).planner.is_none());
    }

    #[test]
    fn contract_v1_absence_overflow_and_prose_never_fabricate_zero() {
        let line = "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0";
        for invalid in [
            String::new(),
            format!("the model says {line}"),
            line.replace(
                "roadmap_dispatched=1",
                &format!("roadmap_dispatched={}", "9".repeat(100)),
            ),
        ] {
            let signals = scan(&invalid);
            assert!(signals.dag_enforcement.is_none());
            assert!(signals.verification_integrity.is_none());
            assert!(signals.safety_and_trust.is_none());
            assert!(signals.cost_routing.is_none());
        }
        let decorated = format!("\x1b[32m  {}  \x1b[0m", line.replace("receipt ", "receipt\t"));
        assert!(scan(&decorated).dag_enforcement.is_some());
    }
}
