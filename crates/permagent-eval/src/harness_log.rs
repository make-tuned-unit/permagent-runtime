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
//! ## Lane K9: executed vs refused tool calls
//!
//! A21 Q2c / M23 DeepSeek m1 (`select-trivial-typo-fix-skips-roadmap`, see
//! `HARNESS_LOOP_LOG.md`'s "Lane K9" entry): the coordinator ATTEMPTED a
//! direct `edit`, which `CodingDagInspector`
//! (`crates/goose/src/tool_inspection.rs` — a different crate, `goose`, not
//! `permagent-eval`, so referenced here as plain text rather than an
//! intra-doc link) REFUSED — the harness's own
//! `receipt dag_enforcement … bypass_attempts=1 bypass_passes=0 …` line
//! proves it was denied, not executed — yet a naive `must_not_call: [edit]`
//! selection rule still failed, because the `edit` banner is printed
//! regardless of whether the call went on to execute. Under this project's
//! protocol, "a denial is the protocol working": a refused call never
//! mutated anything, so it must not count against `must_not_call`/`before`,
//! but the ATTEMPT is still worth retaining as evidence (see
//! [`crate::selection::SelectionVerdict::refused_calls`]).
//!
//! [`scan`] tries two mechanisms, in order, to tell an executed call from a
//! refused one — both best-effort, both erring toward "assume executed"
//! (the pre-Lane-K9 behavior) when neither finds positive evidence:
//!
//! - **Mechanism A — an explicit denial marker in the transcript.** A denied
//!   tool call's `tool_response` is built by `Agent::handle_denied_tools`
//!   (`crates/goose/src/agents/agent.rs`) as `Ok(CallToolResult::error(vec![
//!   Content::text(reason)]))`, where `reason` is one of a handful of fixed
//!   strings: `CodingDagInspector`'s own
//!   `"Top-level coding harness sessions must create an approved
//!   orchestrator DAG…"` (`crates/goose/src/tool_inspection.rs`), the
//!   write jail's `"Refused by the goal-worker write jail…"` /
//!   `"Refused by the write jail…"` (`crates/goose/src/security/
//!   write_jail.rs`), or the generic `DECLINED_RESPONSE` /
//!   `HEADLESS_SKIPPED_RESPONSE` text (`crates/goose/src/agents/
//!   tool_execution.rs`). [`REFUSAL_MARKERS`] matches these verbatim.
//!   **Known limitation, found while building this fixture's own log fixture
//!   (worth stating plainly rather than silently under-matching forever):**
//!   `Content::text(...)` carries no priority annotation, and
//!   `render_tool_response` (`crates/goose-cli/src/session/
//!   output.rs:550-583`) drops any tool-response content whose priority is
//!   `None` unless the session ran with `--debug` — which the harness does
//!   not pass. So on today's default (non-debug) capture, this marker text
//!   is typically NOT present in `harness.log` at all (confirmed against the
//!   real M23 log this lane fixes), and Mechanism A finds nothing. It is
//!   kept anyway because it is exactly right for a `--debug` capture and
//!   costs nothing when it does not match — and because it is what a future
//!   fix to the CLI's priority annotation on denial responses would make
//!   reliable.
//! - **Mechanism B — the `dag_enforcement` receipt as a corroborated
//!   fallback**, used only when Mechanism A found nothing. `bypass_attempts`
//!   is incremented EXCLUSIVELY inside `CodingDagInspector::inspect`'s
//!   deny path (one per denied request), and that inspector only ever
//!   fires for the run's own top-level coordinator session — the only
//!   session [`LogSignals`] ever sees banners from (a dispatched worker's
//!   own tool calls never reach this log; see the top of this module). So
//!   within ONE run, every coordinator banner naming a tool in
//!   [`CODING_DAG_FORBIDDEN_TOOLS`] (mirroring `requires_dispatched_coding_
//!   worker`'s mutating/shell-family list — duplicated here because this
//!   crate does not depend on `goose`; keep in sync by hand) is a candidate
//!   refusal. If — and only if — the candidate COUNT exactly equals
//!   `bypass_attempts`, every candidate is marked refused: a verified match,
//!   not a guess. Any mismatch (some candidates legitimately executed under
//!   a different recipe/topology, or the count is simply off) makes
//!   Mechanism B decline entirely rather than mis-attribute — never fabricate
//!   a specific refusal from an ambiguous count.
//!
//! [`LogSignals::executed_tool_names`] is what
//! [`crate::selection::SelectionCheck`]'s `must_not_call`/`before` rules read
//! by default post-Lane-K9 (opt back into the old, attempt-counts-too
//! behavior with `count_refused: true`); [`LogSignals::refused_tool_calls`]
//! is the retained-as-evidence list. [`LogSignals::tool_names`] itself is
//! UNCHANGED by this lane — it still records every ATTEMPT (refused or not),
//! since other readers (`report.rs`, `tier0.rs`) use it as attempted-call
//! telemetry, not a selection-correctness signal.
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

/// Atomic RECEIPT CONTRACT evidence for `dag_enforcement`.
///
/// Lane R20a (v1.1): `roadmap_resumed` and `escalated` are `Option` because
/// a legacy v1 harness line (four keys, no trailing pair) never observed
/// them — parsed as `None`, never defaulted to `Some(0)`, so
/// `qualification.rs`'s gate can tell "this run's harness never emitted
/// the counter" apart from "the counter was observed and is honestly
/// zero". `#[serde(default)]` keeps pre-R20a retained JSON (`.qual.json`
/// records, sidecar files) loading as `None` rather than failing to
/// deserialize.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DagEnforcementReceipt {
    pub roadmap_dispatched: usize,
    pub bypass_attempts: usize,
    pub bypass_passes: usize,
    pub coordinator_mutations: usize,
    /// v1.1: goal-card count as of the coordinator's `resume_roadmap` call
    /// that resumed a paused roadmap instead of decomposing a new one.
    /// `None` on a v1 line; `Some(0)` means the mechanism ran this run and
    /// never resumed anything.
    #[serde(default)]
    pub roadmap_resumed: Option<usize>,
    /// v1.1: count of coordinator `escalate` decisions minted in a
    /// coding-harness session. `None` on a v1 line; `Some(0)` means the
    /// mechanism ran this run and no escalation was minted.
    #[serde(default)]
    pub escalated: Option<usize>,
    /// v1.2 (Lane R33a, A31 analysis): count of `check_worker` queries the
    /// coordinator made of a worker's live state — either a real
    /// `check_worker` tool call or `goal_advance`'s own internal
    /// pre-dispatch equivalent (`ensure_worker_checked_for_dispatch`, Lane
    /// R22a) — this run. `None` on a pre-R33a line; `Some(0)` means the
    /// mechanism ran this run and no worker was ever checked. Positive
    /// evidence for a coordinator that correctly checks a worker and
    /// dispatches directly, with no new roadmap and no escalation (the
    /// `select-checks-worker-before-dispatch` fixture's correct shape).
    #[serde(default)]
    pub worker_checked: Option<usize>,
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

/// Atomic RECEIPT CONTRACT evidence for `cost_routing`. `dispatches`/
/// `cheapest_capable`/`pack_honored`/`planner` are v1; `routed_local`/
/// `routed_cloud`/`fallbacks` are v1.3 (R48b, HARNESS_LOOP_LOG.md "A45
/// design" Addendum (c) / Q5) — appended at the END of the printed line, so
/// a pre-v1.3 fixture line still parses with all three `None`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CostRoutingReceipt {
    pub dispatches: usize,
    pub cheapest_capable: usize,
    pub pack_honored: usize,
    pub planner: String,
    /// v1.3: dispatches this run routed to a local/on-device worker tier.
    /// `None` on a pre-v1.3 line.
    #[serde(default)]
    pub routed_local: Option<usize>,
    /// v1.3: dispatches this run routed to a metered cloud worker tier.
    /// `None` on a pre-v1.3 line.
    #[serde(default)]
    pub routed_cloud: Option<usize>,
    /// v1.3: goals redispatched once from a LOCAL worker tier to the
    /// coordinator/cloud tier after a context-exceeded, wall/turn-ceiling,
    /// or verify-failed-with-no-commit attempt. `None` on a pre-v1.3 line.
    #[serde(default)]
    pub fallbacks: Option<usize>,
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
    /// Distinct tool names invoked, in first-seen order. Includes REFUSED
    /// attempts (see the module docs' "Lane K9" section) — this is
    /// attempted-call telemetry, not a selection-correctness signal; use
    /// [`Self::executed_tool_names`] for that.
    pub tool_names: Vec<String>,
    /// Lane K9: [`Self::tool_names`] minus any occurrence [`scan`] could
    /// positively determine was refused rather than executed (see the module
    /// docs' "Lane K9" section for Mechanisms A/B and their honest limits).
    /// Distinct tool names, ordered by each name's first EXECUTED occurrence
    /// — so a tool refused on attempt 1 and executed on a later retry is
    /// ordered by the retry, not the refused attempt. A name absent here can
    /// still appear in [`Self::tool_names`] (attempted but never executed)
    /// or in [`Self::refused_tool_calls`] (refused at least once).
    pub executed_tool_names: Vec<String>,
    /// Lane K9: distinct tool names [`scan`] positively determined were
    /// refused (denied before executing) at least once, in first-refused
    /// order — retained as evidence per this project's "a denial is the
    /// protocol working" protocol, even though a refused call does not count
    /// against `must_not_call`/`before` by default (see
    /// [`crate::selection::SelectionCheck::count_refused`]). Empty does NOT
    /// mean "nothing was refused" — it means [`scan`] found no positive
    /// evidence either way (see the module docs' Mechanism A/B limits); a
    /// nonzero `dag_enforcement.bypass_attempts` alongside an empty list here
    /// is exactly that honest-unknown case, not a contradiction.
    pub refused_tool_calls: Vec<String>,
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
    /// Lane K15: count of provider-unavailable marker lines
    /// (`crate::provider_unavailable`) seen in this run whose own line named
    /// a provider OTHER than the tier under test (e.g. the harness's
    /// fallback-able `anthropic`-first planner attempt) — evidence that was
    /// correctly NOT grounds to override the oracle's verdict, kept here so
    /// a report/scorecard reader can still see it. `0` (the default) means
    /// none were seen, including for every task where this module never ran
    /// at all (never fabricated as a distinct "unknown" state).
    pub provider_unavailable_ignored: usize,
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

/// Lane K9, Mechanism A: substrings that appear verbatim in the handful of
/// fixed reasons a guard/inspector denial's `tool_response` carries (see the
/// module docs' "Lane K9" section for the exact call sites and why this
/// currently matches nothing under a default, non-`--debug` capture — kept
/// anyway for a `--debug` capture or a future fix to the CLI's priority
/// annotation on denial content). Plain substring containment, not a regex:
/// each denial reason is one continuous sentence with no embedded newline, so
/// it survives as a single (long) line in the non-tty `print!("{}", text)`
/// path `print_tool_output` takes when captured to a file.
const REFUSAL_MARKERS: &[&str] = &[
    // CodingDagInspector (crates/goose/src/tool_inspection.rs): the top-level
    // coordinator tried a mutating/shell tool directly instead of dispatching
    // a worker — the exact A21 Q2c / M23 defect this lane fixes.
    "Top-level coding harness sessions must create an approved orchestrator DAG",
    // write_jail.rs's two Y4c-shaped denials (dispatched goal worker vs a
    // parentless-approver subagent).
    "Refused by the goal-worker write jail",
    "Refused by the write jail",
    // Generic inspector-denial fallbacks (tool_execution.rs).
    "The user has declined to run this tool",
    "This tool call requires interactive approval, but this session runs headless",
];

fn is_refusal_marker_line(line: &str) -> bool {
    REFUSAL_MARKERS.iter().any(|marker| line.contains(marker))
}

/// Lane K9, Mechanism B: the coordinator tool names `CodingDagInspector`'s
/// `requires_dispatched_coding_worker` (`crates/goose/src/tool_inspection.rs`
/// — a different crate, referenced here as plain text) denies
/// unconditionally for a top-level coding-harness session — mirrors that
/// function's mutating (`after_turn::is_mutating_tool_name`) and shell-family
/// tool names. Duplicated here, not imported, because `permagent-eval` does
/// not depend on `goose`; keep in sync by hand if that inspector's coverage
/// changes. See the module docs' "Lane K9" section for how this is used
/// (only as a fallback, only on an exact `bypass_attempts` count match).
const CODING_DAG_FORBIDDEN_TOOLS: &[&str] = &[
    "write",
    "edit",
    "text_editor",
    "str_replace",
    "create_file",
    "apply_patch",
    "shell",
    "bash",
    "execute_command",
    "run_command",
    "terminal",
];

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
//
// Lane R20a (RECEIPT CONTRACT v1.1): the trailing `roadmap_resumed=<n>
// escalated=<n>` pair is optional (`(?:...)?`) so this ONE regex accepts
// both a legacy v1 line (four keys — an older harness build, or a v1.1
// build's own line captured before this lane) and a v1.1 line (six keys).
// Captures 5 and 6 are simply absent (`None`) on a v1 line — `parse_dag_
// enforcement` below reads that as "never observed", never as "observed
// zero". Do not make the trailing pair required: that would silently
// reject every legacy line this crate must keep reading.
//
// Lane R33a (RECEIPT CONTRACT v1.2): `worker_checked=<n>` is a SEPARATE
// trailing optional group, after the v1.1 pair — a v1.2 line always carries
// all seven keys together (`receipts.rs`'s `FIELDS`/`Snapshot::lines` emit
// them as one line), but keeping it independently optional means a v1 line
// (four keys), a v1.1 line (six keys) and a v1.2 line (seven keys) all still
// parse with this one regex, same reasoning as the v1.1 pair above. Capture
// 7 is `None` on any pre-R33a line.
static DAG_ENFORCEMENT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*receipt\s+dag_enforcement\s+roadmap_dispatched=([0-9]+)\s+bypass_attempts=([0-9]+)\s+bypass_passes=([0-9]+)\s+coordinator_mutations=([0-9]+)(?:\s+roadmap_resumed=([0-9]+)\s+escalated=([0-9]+))?(?:\s+worker_checked=([0-9]+))?\s*$",
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
        roadmap_resumed: caps.get(5).map(|m| m.as_str().parse()).transpose().ok()?,
        escalated: caps.get(6).map(|m| m.as_str().parse()).transpose().ok()?,
        worker_checked: caps.get(7).map(|m| m.as_str().parse()).transpose().ok()?,
    })
}

static VERIFICATION_INTEGRITY_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(
    || {
        regex::Regex::new(
        r"^\s*receipt\s+verification_integrity\s+goals=([0-9]+)\s+review_reached=([0-9]+)\s+vacuous_passes=([0-9]+)\s+reviewer_mutations=([0-9]+)\s+reviewer_not_independent=([0-9]+)\s*$",
    )
    .expect("VERIFICATION_INTEGRITY_RE is a valid regex")
    },
);

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
        r"^\s*receipt\s+cost_routing\s+dispatches=([0-9]+)\s+cheapest_capable=([0-9]+)\s+pack_honored=([0-9]+)\s+planner=(\S+/\S+)(?:\s+routed_local=([0-9]+)\s+routed_cloud=([0-9]+)\s+fallbacks=([0-9]+))?\s*$",
    )
    .expect("COST_ROUTING_RE is a valid regex")
});

/// The string `routing_receipt.rs`'s `snapshot()` prints when `set_planner`
/// was never called — the PLANNER mutex is still empty. It has exactly one
/// slash, so it satisfies `COST_ROUTING_RE`'s `\S+/\S+` cleanly; without this
/// guard an unresolved planner would be recorded as real evidence, which the
/// contract's "zero is evidence only when the receipt explicitly prints it"
/// principle forbids (G16 MED-1).
const PLANNER_UNSET_SENTINEL: &str = "unknown/unknown";

/// Lane R18c: G16 MED-1's guard (below) is right for a run that actually
/// dispatched something — an unresolved planner alongside `dispatches >= 1`
/// means the coordinator's routing trace is broken, and the whole receipt
/// must not be trusted. But a run that dispatched NOTHING never had a
/// planner to resolve in the first place: `dispatches=0
/// cheapest_capable=0 pack_honored=0 planner=unknown/unknown` is the
/// harness's own honest zero, not a broken trace — e.g. a held-out fixture
/// that correctly ESCALATES instead of dispatching a roadmap
/// (`select-ambiguous-choice-escalates-decision`; confirmed printing
/// exactly this line in the M17 round-2 logs) prints it. So the sentinel
/// only discards the receipt when it accompanies real (`dispatches >= 1`)
/// evidence; at zero dispatches the receipt is kept (with the sentinel
/// recorded verbatim in `planner`) so `qualification.rs`'s `cost_routing`
/// gate can see the explicit zero rather than reading a missing receipt.
fn parse_cost_routing(line: &str) -> Option<CostRoutingReceipt> {
    let caps = COST_ROUTING_RE.captures(line)?;
    let dispatches: usize = caps[1].parse().ok()?;
    if dispatches > 0 && &caps[4] == PLANNER_UNSET_SENTINEL {
        return None;
    }
    Some(CostRoutingReceipt {
        dispatches,
        cheapest_capable: caps[2].parse().ok()?,
        pack_honored: caps[3].parse().ok()?,
        planner: caps[4].to_string(),
        routed_local: caps.get(5).map(|m| m.as_str().parse()).transpose().ok()?,
        routed_cloud: caps.get(6).map(|m| m.as_str().parse()).transpose().ok()?,
        fallbacks: caps.get(7).map(|m| m.as_str().parse()).transpose().ok()?,
    })
}

/// Scan a harness run's captured log text for tool-call, rate-limit,
/// turn-limit and (best-effort, see the module docs' "X-obs-1" section) the
/// still-landing lanes' signals. Pure and best-effort: see the module docs
/// for what it matches and why it is not authoritative.
pub fn scan(log_text: &str) -> LogSignals {
    let mut signals = LogSignals::default();
    let mut seen_tools: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Lane K9: every banner's tool name, in call order (unlike `tool_names`,
    // NOT deduplicated — Mechanism B needs an exact count), with a parallel
    // refused flag `pending` (Mechanism A) can resolve before the next
    // banner arrives.
    let mut raw_calls: Vec<String> = Vec::new();
    let mut refused_flags: Vec<bool> = Vec::new();
    let mut pending: Option<usize> = None;

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
                signals.tool_names.push(tool.clone());
            }
            raw_calls.push(tool);
            refused_flags.push(false);
            pending = Some(raw_calls.len() - 1);
        } else if let Some(i) = pending {
            // Lane K9, Mechanism A: the pending call's tool_response (or the
            // model's own text after receiving it) named a guard refusal
            // before the next banner arrived — resolve it as refused rather
            // than leaving it to default to executed.
            if is_refusal_marker_line(line) {
                refused_flags[i] = true;
                pending = None;
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

    // Lane K9, Mechanism B: only when Mechanism A found nothing at all — an
    // explicit marker is stronger evidence than this inferred fallback, so
    // prefer it outright rather than mixing the two. Fires only on an EXACT
    // count match between the candidate calls and the run's own
    // `bypass_attempts` receipt; any mismatch declines rather than guesses
    // which candidates were the denied ones.
    if !refused_flags.iter().any(|&r| r) {
        if let Some(receipt) = signals.dag_enforcement.as_ref() {
            if receipt.bypass_attempts > 0 {
                let candidates: Vec<usize> = raw_calls
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| CODING_DAG_FORBIDDEN_TOOLS.contains(&name.as_str()))
                    .map(|(i, _)| i)
                    .collect();
                if candidates.len() == receipt.bypass_attempts {
                    for i in candidates {
                        refused_flags[i] = true;
                    }
                }
            }
        }
    }

    let mut seen_executed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_refused: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, &refused) in raw_calls.iter().zip(refused_flags.iter()) {
        if refused {
            if seen_refused.insert(name.clone()) {
                signals.refused_tool_calls.push(name.clone());
            }
        } else if seen_executed.insert(name.clone()) {
            signals.executed_tool_names.push(name.clone());
        }
    }

    signals
}

/// Lane R17a (M17 readiness audit §6.1, gaps 1 & 4): a harness SIGKILLed for
/// exceeding `harness_timeout_secs` never reaches its own clean-exit print,
/// so the four RECEIPT CONTRACT v1 lines and the `harness_overhead` line are
/// absent from the captured stdout+stderr log `scan` already read. The
/// harness ALSO mirrors those same lines into a sidecar file (written
/// atomically on every counter change and turn boundary) when
/// `PERMAGENT_RECEIPTS_SIDECAR` is set — see `permagent::receipts::write_sidecar`.
/// This fills any of `signals`' five receipt-area fields that are still
/// `None` from that sidecar's text, parsed with the exact SAME [`scan`] this
/// module already uses on stdout, so the sidecar can never be read by a
/// different (and possibly drifted) grammar.
///
/// Returns which source ended up supplying evidence, for the caller to
/// record on [`crate::metrics::TaskResult::receipt_source`] (informational
/// only — never read by the qualification gate): `"stdout"` when every
/// field the sidecar could have supplied was already present before this
/// call (the sidecar's own values are never used to overwrite an existing
/// stdout-sourced field — stdout wins), `"sidecar"` when at least one field
/// was filled in from the sidecar, or `None` when neither source had any of
/// these fields.
pub fn fill_receipts_from_sidecar(signals: &mut LogSignals, sidecar_text: &str) -> Option<String> {
    let sidecar = scan(sidecar_text);
    let mut filled_any = false;

    if signals.dag_enforcement.is_none() {
        if let Some(v) = sidecar.dag_enforcement {
            signals.dag_enforcement = Some(v);
            filled_any = true;
        }
    }
    if signals.verification_integrity.is_none() {
        if let Some(v) = sidecar.verification_integrity {
            signals.verification_integrity = Some(v);
            filled_any = true;
        }
    }
    if signals.safety_and_trust.is_none() {
        if let Some(v) = sidecar.safety_and_trust {
            signals.safety_and_trust = Some(v);
            filled_any = true;
        }
    }
    if signals.cost_routing.is_none() {
        if let Some(v) = sidecar.cost_routing {
            signals.cost_routing = Some(v);
            filled_any = true;
        }
    }
    if signals.harness_overhead.is_none() {
        if let Some(v) = sidecar.harness_overhead {
            signals.harness_overhead = Some(v);
            filled_any = true;
        }
    }

    if filled_any {
        Some("sidecar".to_string())
    } else if signals.dag_enforcement.is_some()
        || signals.verification_integrity.is_some()
        || signals.safety_and_trust.is_some()
        || signals.cost_routing.is_some()
        || signals.harness_overhead.is_some()
    {
        Some("stdout".to_string())
    } else {
        None
    }
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

    // ── Lane K9: executed vs refused tool calls ─────────────────────────────

    /// No refusal evidence at all (no marker, no `dag_enforcement` receipt)
    /// — the pre-Lane-K9 default: every attempted call reads as executed.
    #[test]
    fn with_no_refusal_evidence_every_attempted_call_reads_as_executed() {
        let signals = scan("  ▸ edit\n    path foo.py\n");
        assert_eq!(signals.tool_names, vec!["edit".to_string()]);
        assert_eq!(signals.executed_tool_names, vec!["edit".to_string()]);
        assert!(signals.refused_tool_calls.is_empty());
    }

    /// Mechanism A: an explicit denial marker between the banner and the next
    /// line resolves the call as refused, not executed.
    #[test]
    fn a_denial_marker_after_the_banner_marks_the_call_refused() {
        let log = "  ▸ edit\n    path foo.py\n\
                    Top-level coding harness sessions must create an approved \
                    orchestrator DAG and dispatch its node.\n";
        let signals = scan(log);
        assert_eq!(signals.tool_names, vec!["edit".to_string()]);
        assert!(
            signals.executed_tool_names.is_empty(),
            "{:?}",
            signals.executed_tool_names
        );
        assert_eq!(signals.refused_tool_calls, vec!["edit".to_string()]);
    }

    /// The write-jail's two denial phrasings are both recognized.
    #[test]
    fn write_jail_denial_markers_mark_the_call_refused() {
        for marker in [
            "Refused by the goal-worker write jail. You are a dispatched goal worker...",
            "Refused by the write jail. You are a dispatched subagent...",
        ] {
            let log = format!("  ▸ shell\n    command rm -rf /\n{marker}\n");
            let signals = scan(&log);
            assert_eq!(
                signals.refused_tool_calls,
                vec!["shell".to_string()],
                "{marker}"
            );
        }
    }

    /// A call executed cleanly BEFORE a later refused call — the marker must
    /// not retroactively taint the earlier, already-resolved banner.
    #[test]
    fn a_refusal_marker_only_resolves_the_immediately_pending_call() {
        let log = "  ▸ search\n    pattern foo\n\nfound 1 match\n\n\
                    ────────────────────────────────────────\n\
                    ▸ edit\n    path foo.py\n\
                    Refused by the write jail. You are a dispatched subagent...\n";
        let signals = scan(log);
        assert_eq!(
            signals.executed_tool_names,
            vec!["search".to_string()],
            "{:?}",
            signals.executed_tool_names
        );
        assert_eq!(signals.refused_tool_calls, vec!["edit".to_string()]);
    }

    /// A21 Q2c / M23 DeepSeek m1's real defect: NO marker text is visible
    /// (the default, non-`--debug` capture — see the module docs), but the
    /// `dag_enforcement` receipt's `bypass_attempts` exactly matches the
    /// single forbidden-tool candidate (`edit`), so Mechanism B attributes
    /// it as refused.
    #[test]
    fn mechanism_b_attributes_the_sole_candidate_when_the_count_matches_bypass_attempts() {
        let log = "  ▸ edit\n    path README.md\n\n\
                    The direct edit was refused — let me dispatch a worker instead.\n\
                    ────────────────────────────────────────\n\
                    ▸ decompose_roadmap orchestrator\n    objective: fix typo\n\
                    ────────────────────────────────────────\n\
                    ▸ create_roadmap orchestrator\n    goals_json: []\n\n\
                    receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=1 \
                    bypass_passes=0 coordinator_mutations=0\n";
        let signals = scan(log);
        assert_eq!(
            signals.refused_tool_calls,
            vec!["edit".to_string()],
            "{signals:?}"
        );
        assert_eq!(
            signals.executed_tool_names,
            vec![
                "decompose_roadmap".to_string(),
                "create_roadmap".to_string()
            ],
            "{signals:?}"
        );
        // The forbidden-tool set itself must never be treated as evidence on
        // its own — decompose_roadmap/create_roadmap are not in it, so they
        // are correctly left executed even though no visible response text
        // follows their banners either (see the module docs).
        assert!(signals.tool_names.contains(&"edit".to_string()));
    }

    /// Mechanism B must DECLINE (attribute nothing) on a count mismatch —
    /// never guess which candidate(s) were actually denied.
    #[test]
    fn mechanism_b_declines_on_a_bypass_attempts_count_mismatch() {
        let log = "  ▸ edit\n    path foo.py\n\n\
                    receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=2 \
                    bypass_passes=0 coordinator_mutations=0\n";
        let signals = scan(log);
        assert!(signals.refused_tool_calls.is_empty(), "{signals:?}");
        assert_eq!(signals.executed_tool_names, vec!["edit".to_string()]);
    }

    /// A `bypass_attempts=0` receipt must never trigger Mechanism B, even if
    /// a forbidden-shaped tool name happens to appear.
    #[test]
    fn mechanism_b_never_fires_at_zero_bypass_attempts() {
        let log = "  ▸ edit\n    path foo.py\n\n\
                    receipt dag_enforcement roadmap_dispatched=0 bypass_attempts=0 \
                    bypass_passes=0 coordinator_mutations=0\n";
        let signals = scan(log);
        assert!(signals.refused_tool_calls.is_empty());
        assert_eq!(signals.executed_tool_names, vec!["edit".to_string()]);
    }

    /// A tool refused on its first attempt and executed on a later retry
    /// appears in BOTH lists — `executed_tool_names` ordered by the retry.
    #[test]
    fn a_call_refused_then_retried_and_executed_appears_in_both_lists() {
        let log = "  ▸ edit\n    path foo.py\n\
                    Refused by the write jail. You are a dispatched subagent...\n\
                    ────────────────────────────────────────\n\
                    ▸ edit\n    path foo.py\n";
        let signals = scan(log);
        assert_eq!(signals.refused_tool_calls, vec!["edit".to_string()]);
        assert_eq!(signals.executed_tool_names, vec!["edit".to_string()]);
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
                roadmap_resumed: None,
                escalated: None,
                worker_checked: None,
            })
        );
        assert!(scan(&line.replace("bypass_passes=0", "bypass_passes=-1"))
            .dag_enforcement
            .is_none());
    }

    /// Lane R20a: the v1.1 trailing pair parses as `Some`, distinct from a
    /// v1 line's `None` — an observed zero is not the same as "never
    /// observed".
    #[test]
    fn contract_v1_1_dag_receipt_parses_resumed_and_escalated() {
        let line = "receipt dag_enforcement roadmap_dispatched=0 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=1 escalated=2";
        assert_eq!(
            scan(line).dag_enforcement,
            Some(DagEnforcementReceipt {
                roadmap_dispatched: 0,
                bypass_attempts: 0,
                bypass_passes: 0,
                coordinator_mutations: 0,
                roadmap_resumed: Some(1),
                escalated: Some(2),
                worker_checked: None,
            })
        );
    }

    /// A v1.1 line with a malformed trailing key (non-digit, or only one of
    /// the pair present) must not partially match — the whole line is
    /// rejected, same as any other malformed receipt line.
    #[test]
    fn contract_v1_1_dag_receipt_rejects_malformed_trailing_pair() {
        let only_resumed = "receipt dag_enforcement roadmap_dispatched=0 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=1";
        assert!(scan(only_resumed).dag_enforcement.is_none());
        let non_digit = "receipt dag_enforcement roadmap_dispatched=0 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=x escalated=1";
        assert!(scan(non_digit).dag_enforcement.is_none());
    }

    /// Lane R33a: a v1.2 line (the trailing `worker_checked=<n>` key, as
    /// `receipts.rs` now always emits after the v1.1 pair) parses as `Some`,
    /// distinct from both a v1 and a v1.1 line's `None`.
    #[test]
    fn contract_v1_2_dag_receipt_parses_worker_checked() {
        let line = "receipt dag_enforcement roadmap_dispatched=0 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=0 escalated=0 worker_checked=1";
        assert_eq!(
            scan(line).dag_enforcement,
            Some(DagEnforcementReceipt {
                roadmap_dispatched: 0,
                bypass_attempts: 0,
                bypass_passes: 0,
                coordinator_mutations: 0,
                roadmap_resumed: Some(0),
                escalated: Some(0),
                worker_checked: Some(1),
            })
        );
    }

    /// A pre-R33a line (v1 or v1.1 — no trailing `worker_checked` key at
    /// all) still parses in full, with `worker_checked: None` — never
    /// defaulted to `Some(0)`, so the qualification gate can tell "this
    /// harness build never emitted the counter" apart from "the counter was
    /// observed and is honestly zero".
    #[test]
    fn contract_pre_r33a_dag_receipt_parses_worker_checked_as_none() {
        let v1_1_line = "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=0 escalated=0";
        assert_eq!(
            scan(v1_1_line).dag_enforcement.unwrap().worker_checked,
            None
        );
        let v1_line = "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0";
        assert_eq!(scan(v1_line).dag_enforcement.unwrap().worker_checked, None);
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
                routed_local: None,
                routed_cloud: None,
                fallbacks: None,
            })
        );
        assert!(scan(&line.replace("openrouter/qwen/model", "unknown"))
            .cost_routing
            .is_none());
        // G16 MED-1: the routing receipt's own unset sentinel — the literal
        // string `routing_receipt.rs` prints when `set_planner` never ran —
        // is not evidence of a resolved planner, even though it satisfies the
        // regex's `\S+/\S+` shape. Asserted with that exact string, not an
        // invented one.
        assert!(
            scan(&line.replace("openrouter/qwen/model", PLANNER_UNSET_SENTINEL))
                .cost_routing
                .is_none()
        );
        // A terminal routing receipt must not overwrite the separate route trace.
        assert!(scan(line).planner.is_none());
    }

    /// Lane R18c: at `dispatches=0` the sentinel is not a broken trace — it
    /// is the honest reading for a run that routed nothing at all (e.g. a
    /// held-out fixture that correctly escalates instead of dispatching a
    /// roadmap). The receipt must be KEPT, not discarded, so the driver can
    /// see an explicit zero rather than a missing receipt.
    #[test]
    fn contract_v1_routing_receipt_keeps_the_unset_sentinel_at_zero_dispatches() {
        let line =
            "receipt cost_routing dispatches=0 cheapest_capable=0 pack_honored=0 planner=unknown/unknown";
        assert_eq!(
            scan(line).cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 0,
                cheapest_capable: 0,
                pack_honored: 0,
                planner: PLANNER_UNSET_SENTINEL.to_string(),
                routed_local: None,
                routed_cloud: None,
                fallbacks: None,
            })
        );
        // The same sentinel at a NONZERO dispatch count is still discarded —
        // G16 MED-1's original bug (a routed dispatch whose planner never
        // got recorded) is untouched by this lane.
        assert!(scan(&line.replace("dispatches=0", "dispatches=1"))
            .cost_routing
            .is_none());
    }

    /// R48b (v1.3): the three new trailing fields parse when present, and
    /// the receipt still parses (with all three `None`) when a line was
    /// produced by a pre-v1.3 binary and lacks them — the exact backward-
    /// compat contract `DagEnforcementReceipt`'s own v1.1 fields already
    /// established for this module.
    #[test]
    fn contract_v1_3_routing_receipt_parses_with_and_without_the_new_fields() {
        let v1_3_line = "receipt cost_routing dispatches=3 cheapest_capable=3 pack_honored=3 \
                          planner=custom_deepseek/deepseek-chat routed_local=2 routed_cloud=1 \
                          fallbacks=1";
        assert_eq!(
            scan(v1_3_line).cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 3,
                cheapest_capable: 3,
                pack_honored: 3,
                planner: "custom_deepseek/deepseek-chat".to_string(),
                routed_local: Some(2),
                routed_cloud: Some(1),
                fallbacks: Some(1),
            })
        );

        let pre_v1_3_line =
            "receipt cost_routing dispatches=3 cheapest_capable=3 pack_honored=3 planner=p/m";
        assert_eq!(
            scan(pre_v1_3_line).cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 3,
                cheapest_capable: 3,
                pack_honored: 3,
                planner: "p/m".to_string(),
                routed_local: None,
                routed_cloud: None,
                fallbacks: None,
            }),
            "a pre-v1.3 line (no trailing routed_local/routed_cloud/fallbacks) must still \
             parse, reading the three new fields as None rather than refusing the whole line"
        );

        // A zero-dispatch v1.3 line — the honest all-zero shape a run that
        // routed nothing genuinely prints.
        let zero_v1_3_line = "receipt cost_routing dispatches=0 cheapest_capable=0 \
                               pack_honored=0 planner=unknown/unknown routed_local=0 \
                               routed_cloud=0 fallbacks=0";
        assert_eq!(
            scan(zero_v1_3_line).cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 0,
                cheapest_capable: 0,
                pack_honored: 0,
                planner: PLANNER_UNSET_SENTINEL.to_string(),
                routed_local: Some(0),
                routed_cloud: Some(0),
                fallbacks: Some(0),
            })
        );
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
        let decorated = format!(
            "\x1b[32m  {}  \x1b[0m",
            line.replace("receipt ", "receipt\t")
        );
        assert!(scan(&decorated).dag_enforcement.is_some());
    }

    /// A sidecar file holding all five RECEIPT CONTRACT v1 lines, as
    /// `permagent::receipts::write_sidecar` would write it.
    fn full_sidecar_text() -> String {
        [
            "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0",
            "receipt verification_integrity goals=1 review_reached=1 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0",
            "receipt safety_and_trust jail_denials=1 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=0",
            "receipt cost_routing dispatches=1 cheapest_capable=1 pack_honored=1 planner=openrouter/glm",
            "harness_overhead p50=10 p95=20 model=30 tools=40 wall=50",
        ]
        .join("\n")
    }

    #[test]
    fn fill_receipts_from_sidecar_populates_an_empty_captured_log() {
        // Lane R17a: a harness SIGKILLed before its clean-exit print leaves
        // `scan` on the (empty) captured log with nothing; the sidecar must
        // supply all five fields, and the source must read "sidecar".
        let mut signals = scan("");
        assert!(signals.dag_enforcement.is_none());

        let source = fill_receipts_from_sidecar(&mut signals, &full_sidecar_text());

        assert_eq!(source, Some("sidecar".to_string()));
        assert_eq!(
            signals.dag_enforcement,
            Some(DagEnforcementReceipt {
                roadmap_dispatched: 1,
                bypass_attempts: 0,
                bypass_passes: 0,
                coordinator_mutations: 0,
                roadmap_resumed: None,
                escalated: None,
                worker_checked: None,
            })
        );
        assert_eq!(
            signals.verification_integrity,
            Some(VerificationIntegrityReceipt {
                goals: 1,
                review_reached: 1,
                vacuous_passes: 0,
                reviewer_mutations: 0,
                reviewer_not_independent: 0,
            })
        );
        assert_eq!(
            signals.safety_and_trust,
            Some(SafetyAndTrustReceipt {
                jail_denials: 1,
                jail_bypasses: 0,
                unapproved_skill_runs: 0,
                approval_bypasses: 0,
                adversary_blocks: 0,
            })
        );
        assert_eq!(
            signals.cost_routing,
            Some(CostRoutingReceipt {
                dispatches: 1,
                cheapest_capable: 1,
                pack_honored: 1,
                planner: "openrouter/glm".to_string(),
                routed_local: None,
                routed_cloud: None,
                fallbacks: None,
            })
        );
        assert_eq!(
            signals.harness_overhead,
            Some(HarnessOverhead {
                p50_ms: 10,
                p95_ms: 20,
                model_ms: 30,
                tools_ms: 40,
                wall_ms: 50,
            })
        );
    }

    #[test]
    fn fill_receipts_from_sidecar_lets_stdout_win_when_both_exist() {
        // The captured log printed its own (clean-exit) receipts with
        // DIFFERENT values from the sidecar below; those stdout-sourced
        // values must survive untouched, and the source must read "stdout"
        // — a sidecar that merely CONFIRMS the same run must never override
        // the authoritative stdout print.
        let stdout_line = "receipt dag_enforcement roadmap_dispatched=9 bypass_attempts=9 bypass_passes=9 coordinator_mutations=9\n\
             receipt verification_integrity goals=9 review_reached=9 vacuous_passes=9 reviewer_mutations=9 reviewer_not_independent=9\n\
             receipt safety_and_trust jail_denials=9 jail_bypasses=9 unapproved_skill_runs=9 approval_bypasses=9 adversary_blocks=9\n\
             receipt cost_routing dispatches=9 cheapest_capable=9 pack_honored=9 planner=z.ai/glm\n\
             harness_overhead p50=99 p95=99 model=99 tools=99 wall=99";
        let mut signals = scan(stdout_line);
        assert!(signals.dag_enforcement.is_some());

        let source = fill_receipts_from_sidecar(&mut signals, &full_sidecar_text());

        assert_eq!(source, Some("stdout".to_string()));
        assert_eq!(
            signals.dag_enforcement.unwrap().roadmap_dispatched,
            9,
            "stdout's own value must not be overwritten by the sidecar"
        );
        assert_eq!(signals.harness_overhead.unwrap().p50_ms, 99);
    }

    #[test]
    fn fill_receipts_from_sidecar_fills_only_the_missing_fields() {
        // A partial stdout capture (only dag_enforcement printed before the
        // kill) keeps its own dag_enforcement value and is topped off with
        // everything else from the sidecar.
        let mut signals = scan(
            "receipt dag_enforcement roadmap_dispatched=9 bypass_attempts=9 bypass_passes=9 coordinator_mutations=9",
        );

        let source = fill_receipts_from_sidecar(&mut signals, &full_sidecar_text());

        assert_eq!(source, Some("sidecar".to_string()));
        assert_eq!(signals.dag_enforcement.unwrap().roadmap_dispatched, 9);
        assert_eq!(signals.cost_routing.unwrap().dispatches, 1);
        assert_eq!(signals.harness_overhead.unwrap().p50_ms, 10);
    }

    #[test]
    fn fill_receipts_from_sidecar_reads_none_when_neither_side_has_evidence() {
        let mut signals = scan("");
        let source = fill_receipts_from_sidecar(&mut signals, "");
        assert_eq!(source, None);
        assert!(signals.dag_enforcement.is_none());
    }
}
