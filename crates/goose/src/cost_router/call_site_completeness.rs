//! E7 ("call-site completeness") — the harness-side helper behind
//! [`super::review_gate`]'s reviewer mandate item E7. This is aimed squarely
//! at `harness-refactor-shipping-quote-signature`, the single most persistent
//! miss across every provider (M17–M24): a worker changes a function/method
//! signature, updates the "obvious" callers, and misses one whose call shape
//! differs (the canonical repro: `admin/manualQuote.ts` calling
//! `shippingQuote` with a different argument shape than the callers the
//! worker actually looked at). The cross-family reviewer approved anyway,
//! every time, because nothing in its prompt ever told it to go look for the
//! rest of the call sites. Lane R18h's recipe-prose bullet ("grep every call
//! site on a signature change") asked the model to remember to do this on its
//! own; this module makes it a mechanism instead: the harness computes the
//! call-site list itself and hands it to the reviewer, so nothing depends on
//! the model remembering.
//!
//! Two pure passes, both independently testable without a real checkout:
//!
//! 1. [`detect_signature_changes`] reads a unified diff and finds every
//!    `fn`/`function`/`def`/method DEFINITION line whose parameter list
//!    changed between the `-` and `+` side of the same file.
//! 2. [`enumerate_call_sites`] takes those changes plus a snapshot of repo
//!    file contents — the live seam can pass real file reads (or a `rg`/
//!    tree-sitter reference-graph result, e.g. the ranked-tags map
//!    `agents::platform_extensions::analyze::repo_map` already builds); a
//!    unit test passes a small in-memory fixture — and lists every line,
//!    anywhere in the repo, that invokes the changed symbol BY NAME.
//!
//! The name-based match is deliberate: it is NOT shape-based, so a call
//! written in a completely different shape than the "obvious" callers
//! (different argument count, wrapped in another expression, called through a
//! differently-named local) is still found. Precision is not the goal here —
//! recall is. A reviewer handed a false positive can dismiss it in one
//! `CALLSITE ... | UNAFFECTED | ...` line; a reviewer never told about a real
//! call site has no way to catch it at all.
//!
//! [`render_call_site_section`] renders the enumeration as the
//! `BEGIN_CALL_SITES`/`END_CALL_SITES` block
//! [`super::review_gate::build_review_prompt_with_call_sites`] injects, and
//! [`parse_call_site_line`] reads the reviewer's per-site `CALLSITE` lines
//! back out (called from [`super::review_gate::parse_review`]) so
//! [`missing_call_sites`] — consumed by
//! [`super::review_gate::gate_decision_with_call_sites`] — can turn any
//! unaccounted call site into a HOLD that names it, regardless of what
//! verdict the reviewer emitted.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

// ── Signature-change detection (pure, over a unified diff) ───────────────────

/// One function/method signature change detected in a diff: the same symbol
/// name defined on both the `-` and `+` side of the same file, with a
/// DIFFERENT (post-whitespace-normalization) parameter list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureChange {
    pub symbol: String,
    pub file: String,
    pub old_signature: String,
    pub new_signature: String,
}

/// The definition-line patterns this module recognizes: Rust `fn`, Python
/// `def`, a JS/TS `function` declaration, a JS/TS `const name = (params) =>`
/// arrow function, and a generic `name(params) {` method shape (Java/Go/
/// C-family, and JS/TS class methods) that requires a trailing `{` so an
/// ordinary call statement is never mistaken for a definition. A signature
/// change this misses is a false NEGATIVE — no HOLD is raised for it — which
/// is why [`enumerate_call_sites`]'s recall-first, name-based matching is a
/// second, independent line of defense once a change IS detected, not a
/// substitute for widening this list further as new languages show up in the
/// loop.
///
/// Every pattern here is matched against a string that may itself be several
/// diff lines joined with `\n` (see [`collect_definitions_in_run`]) — a
/// reflowed multi-line signature — so `[^)]*` (which, unlike `.`, matches
/// newlines by default) is what lets a parameter list spanning lines close
/// on the line that actually holds its `)`.
fn definition_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // Rust: `fn name(params)` / `pub fn name<T>(params)` / `pub(crate) async fn name(params)`
            Regex::new(
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)",
            )
            .expect("static regex is valid"),
            // Python: `def name(params):` / `async def name(params):`
            Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)")
                .expect("static regex is valid"),
            // JS/TS function declaration: `function name(params)` / `export async function* name(params)`
            Regex::new(
                r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)",
            )
            .expect("static regex is valid"),
            // JS/TS arrow function assigned to a binding: `const name = (params) => ` /
            // `export const name: T = async (params): R => {`. The trailing `=>` is
            // load-bearing — without it `const name = (params)` alone could be almost
            // any expression, not necessarily a function.
            Regex::new(
                r"^\s*(?:export\s+)?(?:default\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?::\s*[^=]+?)?=\s*(?:async\s+)?\(([^)]*)\)\s*(?::\s*[A-Za-z0-9_<>\[\].,\s|]+)?\s*=>",
            )
            .expect("static regex is valid"),
            // Generic method-shaped definition: `name(params) {` / `name(params): Type {`,
            // optionally preceded by visibility/modifier keywords. The trailing
            // `{` is load-bearing — it is what keeps `foo(a, b);` (a CALL) from
            // matching as a definition.
            Regex::new(
                r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|export\s+|async\s+|override\s+|abstract\s+)*([A-Za-z_$][A-Za-z0-9_$]*)\s*\(([^)]*)\)\s*(?::\s*[A-Za-z0-9_<>\[\].,\s|]+)?\s*\{\s*$",
            )
            .expect("static regex is valid"),
        ]
    })
}

/// Does `line` define a function/method, and if so, what is its name and
/// (whitespace-normalized) parameter list? Tries each pattern in
/// [`definition_patterns`] in order and returns the first match. Public so
/// the orchestrator's salvage reviewer (Lane R62e) can show the reviewer a
/// callee's definition line using the same recognizer E7 uses.
pub fn definition_in_line(line: &str) -> Option<(String, String)> {
    for re in definition_patterns() {
        if let Some(caps) = re.captures(line) {
            let name = caps.get(1)?.as_str().to_string();
            let params = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            return Some((name, normalize_params(params)));
        }
    }
    None
}

/// The keyword-led shapes of [`definition_patterns`], cut off at the `(` that
/// OPENS the parameter list: what a definition line looks like once its
/// parameter list has been reflowed onto the following lines
/// (`export function quoteShipping(`). [`definition_in_line`] cannot see these
/// on their own — its `\(([^)]*)\)` needs the closing paren — which is why a
/// reflowed definition used to be listed as a call site of itself.
///
/// Only the keyword-led shapes appear here. The generic `name(params) {`
/// method shape is deliberately left out: `name(` alone is exactly what an
/// ordinary multi-line CALL looks like, and mistaking a call for a definition
/// would DROP a real call site from the list — the one thing E7 must never do.
fn definition_head_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            // Rust: `pub(crate) async fn name<T>(`
            Regex::new(
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s*\(",
            )
            .expect("static regex is valid"),
            // Python: `async def name(`
            Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
                .expect("static regex is valid"),
            // JS/TS: `export default async function* name(`
            Regex::new(
                r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*\(",
            )
            .expect("static regex is valid"),
        ]
    })
}

/// The symbol a definition HEAD line names, when the line's last
/// non-whitespace character is the `(` that opens the parameter list (i.e. the
/// parameter list genuinely continues on the next line). Requiring the open
/// paren to be last is what keeps this from matching a one-line definition
/// [`definition_in_line`] already handles, or anything else.
fn definition_head_symbol(line: &str) -> Option<String> {
    if !line.trim_end().ends_with('(') {
        return None;
    }
    for re in definition_head_patterns() {
        if let Some(caps) = re.captures(line) {
            return Some(caps.get(1)?.as_str().to_string());
        }
    }
    None
}

/// Does `line` bind `symbol` with `const`/`let`/`var` (the shape an arrow
/// function or a function-valued constant is declared in)? A binding of the
/// symbol is a DEFINITION of it, whatever the right-hand side looks like, so
/// it is never a call site of itself.
fn binding_declares_symbol(line: &str, symbol: &str) -> bool {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    ["const ", "let ", "var "].iter().any(|kw| {
        trimmed
            .strip_prefix(kw)
            .and_then(|rest| rest.trim_start().strip_prefix(symbol))
            .is_some_and(|rest| {
                let rest = rest.trim_start();
                rest.starts_with('=') || rest.starts_with(':')
            })
    })
}

/// Does `line` DEFINE `symbol`, rather than call it? The union of the three
/// recognizers: a complete one-line definition ([`definition_in_line`]), a
/// reflowed definition's head line ([`definition_head_symbol`]), and a
/// `const`/`let`/`var` binding ([`binding_declares_symbol`]).
///
/// Lane R67a: a symbol's own definition site is NOT a call site of it, and a
/// file that only defines the symbol is not a caller — listing one asks the
/// reviewer to account for something that cannot be "updated for the new
/// signature", which is exactly what sent r67 s1 back with no real finding.
/// One shared recognizer so the pure enumeration and the live `git grep` seam
/// (the orchestrator's salvage/review context) agree on what a definition is.
pub fn line_defines_symbol(line: &str, symbol: &str) -> bool {
    if definition_in_line(line).is_some_and(|(name, _)| name == symbol) {
        return true;
    }
    if definition_head_symbol(line).is_some_and(|name| name == symbol) {
        return true;
    }
    binding_declares_symbol(line, symbol)
}

/// Collapse whitespace runs in a parameter list so `(a, b)` and `(a,   b)`
/// compare equal — only a REAL textual change to the parameter list (added,
/// removed, renamed, or reordered parameters) should count as a signature
/// change, not reformatting.
fn normalize_params(params: &str) -> String {
    // Normalize BOTH inter-token whitespace and comma spacing (`a,b` and
    // `a, b` must compare equal — only a change to the parameter LIST, not
    // its formatting, should ever count as a signature change) by splitting
    // on commas first, collapsing whitespace within each part, then
    // rejoining with a single canonical `, ` separator. Empty parts are
    // dropped so a trailing comma — the norm once a parameter list is
    // reflowed one-per-line — never counts as a phantom parameter, and a
    // parameter-less `()` still normalizes to `""`.
    params
        .split(',')
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Strip a leading `a/` or `b/` diff path prefix and any trailing
/// whitespace-separated junk (a timestamp some diff generators append after
/// the path), the way `git diff --git a/x b/y` / `+++ b/x` paths are
/// conventionally written.
fn strip_diff_path_prefix(path: &str) -> String {
    let first = path.split_whitespace().next().unwrap_or(path);
    first
        .strip_prefix("b/")
        .or_else(|| first.strip_prefix("a/"))
        .unwrap_or(first)
        .to_string()
}

/// Bound on how many consecutive same-sign diff lines [`collect_definitions_in_run`]
/// will join when a definition pattern needs more than the line it starts on
/// to reach its closing `)` — generous enough for any real signature (even a
/// heavily-annotated one), cheap enough to never matter for harness latency.
const MAX_SIGNATURE_JOIN_LINES: usize = 40;

/// Scan one contiguous run of same-sign (all `+` or all `-`) diff content
/// lines from a single file for function/method definitions. A definition
/// that fits on one line is found at window length 1, exactly as before; a
/// definition whose parameter list was reflowed across several lines is
/// found by growing the window one line at a time and joining with `\n`
/// until [`definition_in_line`] matches. Stopping at the SMALLEST matching
/// window (rather than always taking the whole run) is what keeps two
/// back-to-back one-line definitions in the same run — no blank/context line
/// between them — from being fused into one candidate signature.
fn collect_definitions_in_run(
    run: &[String],
    file: &str,
    map: &mut HashMap<(String, String), (String, String)>,
    order: &mut Vec<(String, String)>,
) {
    let mut i = 0;
    while i < run.len() {
        let max_len = (run.len() - i).min(MAX_SIGNATURE_JOIN_LINES);
        let mut found: Option<(usize, String, String)> = None;
        for len in 1..=max_len {
            let joined = run[i..i + len].join("\n");
            if let Some((name, params)) = definition_in_line(&joined) {
                found = Some((len, name, params));
                break;
            }
        }
        match found {
            Some((len, name, params)) => {
                let raw = run[i..i + len].join("\n").trim().to_string();
                let key = (file.to_string(), name);
                if !map.contains_key(&key) {
                    order.push(key.clone());
                }
                map.insert(key, (params, raw));
                i += len;
            }
            None => i += 1,
        }
    }
}

/// Process a completed run (if non-empty) into `map`/`order` and clear it.
/// Called whenever a run of same-sign diff lines ends — the opposite sign
/// appears, a context/hunk-header line appears, or the file changes — since
/// a signature never spans across any of those boundaries.
fn flush_run(
    run: &mut Vec<String>,
    file: Option<&str>,
    map: &mut HashMap<(String, String), (String, String)>,
    order: &mut Vec<(String, String)>,
) {
    if !run.is_empty() {
        if let Some(file) = file {
            collect_definitions_in_run(run, file, map, order);
        }
        run.clear();
    }
}

/// Find every function/method signature change in a unified diff: a symbol
/// defined on BOTH the removed (`-`) and added (`+`) side of the SAME file,
/// whose parameter lists differ. Deliberately not hunk-scoped — a large
/// rewrite can separate the old and new definition lines into different
/// hunks — so old/new definitions are collected per (file, symbol) across the
/// WHOLE diff and compared once at the end.
///
/// A definition line is not required to fit on a single diff line: consecutive
/// added lines (and, separately, consecutive removed lines) are joined and
/// tried together so a signature reflowed across several lines — the common
/// shape a formatter produces once a parameter list gets long — is still
/// recognized. See [`collect_definitions_in_run`].
// Boundary-safe by construction: `b_idx` is `str::find`'s index for the ASCII
// literal " b/" and the `+ 3` is that literal's own byte length, i.e. the
// match's end boundary, so the slice cannot split a UTF-8 character.
#[allow(clippy::string_slice)]
pub fn detect_signature_changes(diff: &str) -> Vec<SignatureChange> {
    let mut current_file: Option<String> = None;
    // (file, symbol) -> (normalized params, raw trimmed line(s))
    let mut removed: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut added: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut order: Vec<(String, String)> = Vec::new();

    let mut added_run: Vec<String> = Vec::new();
    let mut removed_run: Vec<String> = Vec::new();

    for raw_line in diff.lines() {
        if let Some(rest) = raw_line.strip_prefix("+++ ") {
            flush_run(
                &mut added_run,
                current_file.as_deref(),
                &mut added,
                &mut order,
            );
            flush_run(
                &mut removed_run,
                current_file.as_deref(),
                &mut removed,
                &mut order,
            );
            let path = strip_diff_path_prefix(rest.trim());
            if path != "/dev/null" {
                current_file = Some(path);
            }
            continue;
        }
        if raw_line.starts_with("--- ") {
            // The old-side header; the file identity comes from `+++` above.
            flush_run(
                &mut added_run,
                current_file.as_deref(),
                &mut added,
                &mut order,
            );
            flush_run(
                &mut removed_run,
                current_file.as_deref(),
                &mut removed,
                &mut order,
            );
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("diff --git ") {
            // "a/<path> b/<path>" — tentatively set from the b/ side; a
            // following `+++` line (the common case) overrides this.
            flush_run(
                &mut added_run,
                current_file.as_deref(),
                &mut added,
                &mut order,
            );
            flush_run(
                &mut removed_run,
                current_file.as_deref(),
                &mut removed,
                &mut order,
            );
            if let Some(b_idx) = rest.find(" b/") {
                current_file = Some(rest[b_idx + 3..].trim().to_string());
            }
            continue;
        }

        if current_file.is_none() {
            continue;
        }

        if let Some(content) = raw_line.strip_prefix('+') {
            // An added line ends any removed-run in progress (a diff never
            // interleaves +/- within what should be read as one signature).
            flush_run(
                &mut removed_run,
                current_file.as_deref(),
                &mut removed,
                &mut order,
            );
            added_run.push(content.to_string());
            continue;
        }
        if let Some(content) = raw_line.strip_prefix('-') {
            flush_run(
                &mut added_run,
                current_file.as_deref(),
                &mut added,
                &mut order,
            );
            removed_run.push(content.to_string());
            continue;
        }

        // A context line or hunk header ends any run in progress: a
        // signature never spans across unchanged/context lines.
        flush_run(
            &mut added_run,
            current_file.as_deref(),
            &mut added,
            &mut order,
        );
        flush_run(
            &mut removed_run,
            current_file.as_deref(),
            &mut removed,
            &mut order,
        );
    }
    flush_run(
        &mut added_run,
        current_file.as_deref(),
        &mut added,
        &mut order,
    );
    flush_run(
        &mut removed_run,
        current_file.as_deref(),
        &mut removed,
        &mut order,
    );

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for key in order {
        if !seen.insert(key.clone()) {
            continue;
        }
        if let (Some((old_params, old_line)), Some((new_params, new_line))) =
            (removed.get(&key), added.get(&key))
        {
            if old_params != new_params {
                out.push(SignatureChange {
                    file: key.0,
                    symbol: key.1,
                    old_signature: old_line.clone(),
                    new_signature: new_line.clone(),
                });
            }
        }
    }
    out
}

// ── Call-site enumeration (pure, over a repo file snapshot) ──────────────────

/// One call site: a line, anywhere in the repo snapshot, that invokes a
/// changed symbol by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    pub text: String,
}

/// Every call site found for one [`SignatureChange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteEnumeration {
    pub symbol: String,
    /// The file the signature change itself lives in (for the prompt's
    /// "changed in <file>" framing) — not necessarily excluded from
    /// `call_sites`: another function in the SAME file that calls the
    /// changed symbol is still a real call site.
    pub file: String,
    pub call_sites: Vec<CallSite>,
}

/// A word-boundary `SYMBOL(` matcher — deliberately name-based rather than
/// shape-based (see the module doc): recall over precision, so a call
/// written in a different shape than the "obvious" callers is still caught.
fn call_regex(symbol: &str) -> Regex {
    Regex::new(&format!(r"\b{}\s*\(", regex::escape(symbol))).expect("escaped symbol is valid")
}

/// Enumerate every call site of every changed symbol across `repo_files` (an
/// iterator of `(path, contents)` — the live seam can pass real file reads or
/// a code-map's file set; tests pass an in-memory fixture). A line that is
/// ITSELF a definition of the symbol (the old signature, the new signature, a
/// reflowed definition's head line, a `const` binding, or an unrelated
/// overload) is never counted as a call site of itself — see
/// [`line_defines_symbol`].
pub fn enumerate_call_sites<'a, I>(
    changes: &[SignatureChange],
    repo_files: I,
) -> Vec<CallSiteEnumeration>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let files: Vec<(&str, &str)> = repo_files.into_iter().collect();
    changes
        .iter()
        .map(|change| {
            let re = call_regex(&change.symbol);
            let mut call_sites = Vec::new();
            for (path, contents) in &files {
                for (idx, line) in contents.lines().enumerate() {
                    if line_defines_symbol(line, &change.symbol) {
                        continue;
                    }
                    if re.is_match(line) {
                        call_sites.push(CallSite {
                            file: (*path).to_string(),
                            line: idx + 1,
                            text: line.trim().to_string(),
                        });
                    }
                }
            }
            CallSiteEnumeration {
                symbol: change.symbol.clone(),
                file: change.file.clone(),
                call_sites,
            }
        })
        .collect()
}

/// Render the enumeration as the `BEGIN_CALL_SITES`/`END_CALL_SITES` body
/// [`super::review_gate::build_review_prompt_with_call_sites`] injects.
pub fn render_call_site_section(enumerations: &[CallSiteEnumeration]) -> String {
    if enumerations.is_empty() {
        return "(no signature changes were detected in this diff — E7 does not apply)".to_string();
    }
    let mut out = String::new();
    for (i, e) in enumerations.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "SYMBOL {} (signature changed in {}) — {} call site(s) found in the repo:\n",
            e.symbol,
            e.file,
            e.call_sites.len()
        ));
        if e.call_sites.is_empty() {
            out.push_str(
                "  (no call sites found elsewhere in the repo snapshot handed to this review)\n",
            );
        }
        for cs in &e.call_sites {
            out.push_str(&format!("  {}:{}: {}\n", cs.file, cs.line, cs.text));
        }
    }
    out
}

// ── The reviewer's per-site verdict (parsed back out of its response) ────────

/// What the reviewer said about ONE call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallSiteStatus {
    /// The diff already updated this call site for the new signature.
    Updated,
    /// The new signature does not actually require a change here.
    Unaffected,
    /// It needs updating for the new signature and the diff never touched it.
    Missed,
}

impl CallSiteStatus {
    fn parse(tok: &str) -> Option<CallSiteStatus> {
        match tok.trim().to_ascii_uppercase().as_str() {
            "UPDATED" => Some(CallSiteStatus::Updated),
            "UNAFFECTED" => Some(CallSiteStatus::Unaffected),
            "MISSED" => Some(CallSiteStatus::Missed),
            _ => None,
        }
    }
}

/// One parsed `CALLSITE <symbol> | <file:line> | STATUS | <note>` line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSiteVerdict {
    pub symbol: String,
    /// `file:line`, matching [`CallSite`]'s own `"{file}:{line}"` key.
    pub site: String,
    pub status: CallSiteStatus,
    pub note: String,
}

/// Parse the tail of a `CALLSITE <symbol> | <file:line> | STATUS | <note>`
/// line — the grammar [`super::review_gate::REVIEW_SYSTEM_PROMPT`] asks the
/// reviewer to emit once per call site named in `BEGIN_CALL_SITES`. Returns
/// `None` (dropping the line, same "drop what doesn't parse" discipline as
/// `review_gate::parse_finding`) on a missing symbol/site or an unrecognized
/// status.
pub fn parse_call_site_line(rest: &str) -> Option<CallSiteVerdict> {
    let parts: Vec<&str> = rest.splitn(4, '|').collect();
    if parts.len() < 3 {
        return None;
    }
    let symbol = parts[0].trim().to_string();
    let site = parts[1].trim().to_string();
    if symbol.is_empty() || site.is_empty() {
        return None;
    }
    let status = CallSiteStatus::parse(parts[2])?;
    let note = parts
        .get(3)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Some(CallSiteVerdict {
        symbol,
        site,
        status,
        note,
    })
}

/// The call sites [`enumerate_call_sites`] found that the reviewer's own
/// per-site table never accounted for as `Updated` or `Unaffected` — either
/// it is entirely ABSENT from the table, or the reviewer itself marked it
/// `Missed`. Matching is LENIENT (Lane R67a, see [`account_site`]): symbol and
/// FILE, with the line number optional, so one line per file accounts for
/// every site listed in that file. Either way it is unaccounted, and the
/// non-empty result is what
/// [`super::review_gate::gate_decision_with_call_sites`] turns into a HOLD
/// naming exactly these sites — regardless of the verdict token the reviewer
/// emitted, the same "the reviewer never decides the gate on its own
/// recognizance" discipline the rubber-stamp check already applies to a bare
/// APPROVE.
pub fn missing_call_sites(
    enumerations: &[CallSiteEnumeration],
    verdicts: &[CallSiteVerdict],
) -> Vec<CallSite> {
    let mut missing = Vec::new();
    for e in enumerations {
        for cs in &e.call_sites {
            if account_site(&e.symbol, cs, verdicts) != SiteAccounting::Accounted {
                missing.push(cs.clone());
            }
        }
    }
    missing
}

/// Lane R67a: a reviewer `CALLSITE` line's site field split into its file and
/// its OPTIONAL line number — `src/cart/cartSummary.ts:7` and
/// `src/cart/cartSummary.ts` both name the same file, the first with a line.
/// Anything after the last `:` that is not a number is part of the path.
fn parse_site_ref(site: &str) -> (&str, Option<usize>) {
    match site.rsplit_once(':') {
        Some((file, tail)) if !file.trim().is_empty() => match tail.trim().parse::<usize>() {
            Ok(line) => (file.trim(), Some(line)),
            Err(_) => (site.trim(), None),
        },
        _ => (site.trim(), None),
    }
}

/// Do a reviewer-written path and an enumerated path name the same file? Equal
/// after dropping a `./` prefix, or one is a whole-component suffix of the
/// other (a reviewer commonly shortens `src/admin/manualQuote.ts` to
/// `admin/manualQuote.ts`). Comparing whole components, never a bare basename,
/// is what keeps `a/quote.ts` and `b/quote.ts` distinct.
fn same_file(reviewer: &str, listed: &str) -> bool {
    let components = |path: &str| -> Vec<String> {
        path.trim()
            .trim_start_matches("./")
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .map(|part| part.to_string())
            .collect()
    };
    let (a, b) = (components(reviewer), components(listed));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    long.iter()
        .rev()
        .zip(short.iter().rev())
        .all(|(l, s)| l == s)
}

/// Lane R67a: how the reviewer's table accounts for ONE listed call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteAccounting {
    /// A `CALLSITE` line for this symbol in this file said `UPDATED` or
    /// `UNAFFECTED`, and nothing marked this site `MISSED`.
    Accounted,
    /// The reviewer marked this site — or the whole file — `MISSED`.
    ExplicitlyMissed,
    /// No `CALLSITE` line names this symbol in this file at all.
    Absent,
}

/// Lane R67a: the LENIENT accounting rule. A reviewer line accounts for a
/// listed site when its symbol matches and its FILE matches, whatever line
/// number it carries (or none at all): one `UPDATED`/`UNAFFECTED` line for a
/// file therefore accounts for every site listed in that file. The strict
/// `file:line` rule it replaces rejected correct reviews that wrote one line
/// per file (r67 s1) instead of one per line.
///
/// The real guarantee is untouched: an explicit `MISSED` — for this exact
/// line, or for the file with no line number — still marks the site missed
/// even when another line in the same file was accounted for, and a symbol or
/// file the reviewer wrote NO line for is still unaccounted.
fn account_site(symbol: &str, cs: &CallSite, verdicts: &[CallSiteVerdict]) -> SiteAccounting {
    let relevant: Vec<&CallSiteVerdict> = verdicts
        .iter()
        .filter(|v| v.symbol.trim() == symbol.trim())
        .filter(|v| same_file(parse_site_ref(&v.site).0, &cs.file))
        .collect();
    if relevant.is_empty() {
        return SiteAccounting::Absent;
    }
    let explicitly_missed = relevant.iter().any(|v| {
        v.status == CallSiteStatus::Missed
            && parse_site_ref(&v.site).1.is_none_or(|line| line == cs.line)
    });
    let positive = relevant.iter().any(|v| {
        matches!(
            v.status,
            CallSiteStatus::Updated | CallSiteStatus::Unaffected
        )
    });
    if positive && !explicitly_missed {
        SiteAccounting::Accounted
    } else {
        SiteAccounting::ExplicitlyMissed
    }
}

/// Lane R67a: the unaccounted call sites grouped the way the reviewer is asked
/// to answer them — one entry per (symbol, file) — so the complaint can name a
/// FILE and its symbol and tell the reviewer that one line per file is enough,
/// instead of repeating a bare `file:line` per site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnaccountedCallSiteFile {
    pub symbol: String,
    pub file: String,
    /// The listed lines in this file that went unaccounted, in listed order.
    pub lines: Vec<usize>,
    /// The reviewer marked this file (or a line in it) `MISSED`, as opposed to
    /// writing no `CALLSITE` line for it at all.
    pub explicitly_missed: bool,
}

impl UnaccountedCallSiteFile {
    /// The finding's evidence: every unaccounted `file:line` in this group,
    /// so a single-site group reads exactly as it always did.
    pub fn evidence(&self) -> String {
        self.lines
            .iter()
            .map(|line| format!("{}:{}", self.file, line))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The finding's summary: names the symbol and the FILE, and — when the
    /// reviewer simply wrote no line for it — says that one `CALLSITE` line
    /// per changed symbol per file, with no line number, is enough.
    pub fn complaint(&self) -> String {
        if self.explicitly_missed {
            format!(
                "E7: the reviewer marked the call site(s) of {} in {} MISSED — the changed \
                 signature's callers there still need updating",
                self.symbol, self.file
            )
        } else {
            format!(
                "E7: no CALLSITE line accounted for {symbol} in {file} — emit one CALLSITE line \
                 per changed symbol per file (`CALLSITE {symbol} | {file} | UPDATED` or \
                 `UNAFFECTED`); the line number is optional and one line covers every site \
                 listed in that file",
                symbol = self.symbol,
                file = self.file
            )
        }
    }
}

/// Lane R67a: [`missing_call_sites`] grouped by (symbol, file), in listed
/// order — what [`super::review_gate::gate_decision_with_call_sites`] turns
/// into one finding per file.
pub fn unaccounted_call_site_files(
    enumerations: &[CallSiteEnumeration],
    verdicts: &[CallSiteVerdict],
) -> Vec<UnaccountedCallSiteFile> {
    let mut out: Vec<UnaccountedCallSiteFile> = Vec::new();
    for e in enumerations {
        for cs in &e.call_sites {
            let accounting = account_site(&e.symbol, cs, verdicts);
            if accounting == SiteAccounting::Accounted {
                continue;
            }
            let missed = accounting == SiteAccounting::ExplicitlyMissed;
            match out
                .iter_mut()
                .find(|g| g.symbol == e.symbol && g.file == cs.file)
            {
                Some(group) => {
                    group.lines.push(cs.line);
                    group.explicitly_missed |= missed;
                }
                None => out.push(UnaccountedCallSiteFile {
                    symbol: e.symbol.clone(),
                    file: cs.file.clone(),
                    lines: vec![cs.line],
                    explicitly_missed: missed,
                }),
            }
        }
    }
    out
}

/// Lane R66a: the call sites [`enumerate_call_sites`] found that the
/// reviewer's table has NO line for at all (any status, and — Lane R67a — no
/// line naming that symbol in that FILE either) — format non-compliance, as
/// opposed to an explicit `Missed` judgement. A subset of
/// [`missing_call_sites`]: when the two are equal, every unaccounted site is
/// one the reviewer simply never wrote a line for.
pub fn absent_call_sites(
    enumerations: &[CallSiteEnumeration],
    verdicts: &[CallSiteVerdict],
) -> Vec<CallSite> {
    let mut absent = Vec::new();
    for e in enumerations {
        for cs in &e.call_sites {
            if account_site(&e.symbol, cs, verdicts) == SiteAccounting::Absent {
                absent.push(cs.clone());
            }
        }
    }
    absent
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_signature_changes ──────────────────────────────────────────

    #[test]
    fn rust_param_list_change_is_detected() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n\
                     --- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,3 +1,3 @@\n\
                     -pub fn shipping_quote(order: &Order) -> Quote {\n\
                     +pub fn shipping_quote(order: &Order, opts: &QuoteOpts) -> Quote {\n\
                     unchanged";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "shipping_quote");
        assert_eq!(changes[0].file, "src/lib.rs");
        assert!(changes[0].old_signature.contains("order: &Order) -> Quote"));
        assert!(changes[0]
            .new_signature
            .contains("order: &Order, opts: &QuoteOpts) -> Quote"));
    }

    #[test]
    fn typescript_function_signature_change_is_detected() {
        let diff = "--- a/src/shippingQuote.ts\n\
                     +++ b/src/shippingQuote.ts\n\
                     @@ -1,3 +1,3 @@\n\
                     -export function shippingQuote(order, region) {\n\
                     +export function shippingQuote(order, region, opts) {\n\
                     unchanged";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "shippingQuote");
        assert_eq!(changes[0].file, "src/shippingQuote.ts");
    }

    #[test]
    fn python_def_signature_change_is_detected() {
        let diff = "--- a/quote.py\n\
                     +++ b/quote.py\n\
                     @@ -1,3 +1,3 @@\n\
                     -def shipping_quote(order):\n\
                     +def shipping_quote(order, opts):\n\
                     unchanged";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "shipping_quote");
        assert_eq!(changes[0].file, "quote.py");
    }

    #[test]
    fn reformatting_only_is_not_a_signature_change() {
        // Whitespace-only difference in the parameter list must NOT trigger.
        let diff = "--- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,2 +1,2 @@\n\
                     -pub fn foo(a: i32,b: i32) {\n\
                     +pub fn foo(a: i32, b: i32) {\n";
        assert!(detect_signature_changes(diff).is_empty());
    }

    #[test]
    fn a_non_signature_diff_detects_nothing() {
        let diff = "--- a/README.md\n\
                     +++ b/README.md\n\
                     @@ -1 +1 @@\n\
                     -old text\n\
                     +new text\n";
        assert!(detect_signature_changes(diff).is_empty());
    }

    #[test]
    fn body_only_change_with_identical_signature_is_not_flagged() {
        let diff = "--- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,3 +1,3 @@\n\
                     pub fn foo(a: i32) {\n\
                     -    a + 1\n\
                     +    a + 2\n\
                     }\n";
        assert!(detect_signature_changes(diff).is_empty());
    }

    // ── multi-line signatures (Lane R65c / M49 r65 s1) ──────────────────────

    #[test]
    fn r65_s1_multiline_typescript_signature_is_detected() {
        // Exact repro from M49 record r65 s1
        // (harness-refactor-shipping-quote-signature): the parameter list is
        // reflowed across multiple lines with a new trailing parameter, and
        // the old single-line-only detector missed it entirely.
        let diff = "--- a/src/shipping/quote.ts\n\
                     +++ b/src/shipping/quote.ts\n\
                     @@ -10,3 +10,7 @@\n\
                     -export function computeManualQuote(weightKg: number, zone: Zone): number {\n\
                     +export function computeManualQuote(\n\
                     +  weightKg: number,\n\
                     +  zone: Zone,\n\
                     +  packageType: PackageType,\n\
                     +): number {\n";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "computeManualQuote");
        assert_eq!(changes[0].file, "src/shipping/quote.ts");
        assert!(changes[0]
            .new_signature
            .contains("packageType: PackageType"));
    }

    #[test]
    fn quote_shipping_multiline_signature_is_also_detected() {
        // Same reflow shape applied to `quoteShipping` — the other function
        // named in r65 s1 — to confirm multi-line joining is general and not
        // special-cased to a single function name.
        let diff = "--- a/src/shipping/quote.ts\n\
                     +++ b/src/shipping/quote.ts\n\
                     @@ -20,3 +20,7 @@\n\
                     -export function quoteShipping(weightKg: number, zone: Zone): number {\n\
                     +export function quoteShipping(\n\
                     +  weightKg: number,\n\
                     +  zone: Zone,\n\
                     +  packageType: PackageType,\n\
                     +): number {\n";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "quoteShipping");
    }

    #[test]
    fn reflowed_multiline_signature_with_no_param_change_is_not_flagged() {
        // The signature only moved onto multiple lines; the parameter list
        // itself (names and arity) is identical, so this must NOT fire.
        let diff = "--- a/src/quote.ts\n\
                     +++ b/src/quote.ts\n\
                     @@ -1,2 +1,5 @@\n\
                     -export function foo(a: number, b: number) {\n\
                     +export function foo(\n\
                     +  a: number,\n\
                     +  b: number,\n\
                     +) {\n";
        assert!(detect_signature_changes(diff).is_empty());
    }

    #[test]
    fn typescript_arrow_function_multiline_signature_is_detected() {
        let diff = "--- a/src/quote.ts\n\
                     +++ b/src/quote.ts\n\
                     @@ -1,2 +1,6 @@\n\
                     -const computeQuote = (weightKg: number, zone: Zone) => {\n\
                     +const computeQuote = (\n\
                     +  weightKg: number,\n\
                     +  zone: Zone,\n\
                     +  packageType: PackageType,\n\
                     +) => {\n";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "computeQuote");
        assert!(changes[0].new_signature.contains("packageType"));
    }

    #[test]
    fn rust_multiline_pub_fn_signature_is_detected() {
        let diff = "--- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,3 +1,5 @@\n\
                     -pub fn shipping_quote(order: &Order) -> Quote {\n\
                     +pub fn shipping_quote(\n\
                     +    order: &Order,\n\
                     +    opts: &QuoteOpts,\n\
                     +) -> Quote {\n";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "shipping_quote");
        assert!(changes[0].new_signature.contains("opts: &QuoteOpts"));
    }

    #[test]
    fn python_multiline_def_with_new_parameter_is_detected() {
        let diff = "--- a/quote.py\n\
                     +++ b/quote.py\n\
                     @@ -1,3 +1,6 @@\n\
                     -def compute_quote(weight_kg, zone):\n\
                     +def compute_quote(\n\
                     +    weight_kg,\n\
                     +    zone,\n\
                     +    package_type=None,\n\
                     +):\n";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].symbol, "compute_quote");
        assert!(changes[0].new_signature.contains("package_type=None"));
    }

    #[test]
    fn multiline_body_only_change_is_not_flagged() {
        // The (multi-line) signature itself is unchanged context; only the
        // body changed.
        let diff = "--- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,4 +1,4 @@\n\
                     pub fn foo(\n\
                         a: i32,\n\
                         b: i32,\n\
                     ) {\n\
                     -    a + b\n\
                     +    a + b + 1\n\
                     }\n";
        assert!(detect_signature_changes(diff).is_empty());
    }

    // ── enumerate_call_sites ───────────────────────────────────────────────

    #[test]
    fn enumeration_finds_callers_including_a_differently_shaped_call() {
        // TS + Python fixture repo. The signature of `shippingQuote` changed
        // (gained an `opts` parameter). Three files call it: one in the
        // "obvious" shape a worker would grep for and update, one in Python
        // with the ordinary shape, and one — `admin/manualQuote.ts` — that
        // calls it wrapped inside another expression with a totally
        // different argument shape. All three must be found.
        let diff = "--- a/src/shippingQuote.ts\n\
                     +++ b/src/shippingQuote.ts\n\
                     @@ -1,3 +1,3 @@\n\
                     -export function shippingQuote(order, region) {\n\
                     +export function shippingQuote(order, region, opts) {\n\
                     unchanged";
        let changes = detect_signature_changes(diff);
        assert_eq!(changes.len(), 1);

        let obvious_caller = "import { shippingQuote } from './shippingQuote';\n\
                               export function checkout(order, region) {\n\
                               \treturn shippingQuote(order, region);\n\
                               }\n";
        let python_caller = "def render_quote(order):\n\
                              \treturn shipping_display(shippingQuote(order, 'US'))\n";
        // The differently-shaped miss: wrapped inside another call, with a
        // completely different argument list than the "obvious" callers.
        let manual_quote = "export function manualQuote(input) {\n\
                             \treturn finalize(shippingQuote(input.order, input.region, undefined), input.override);\n\
                             }\n";
        // The definition file's own POST-change contents — its definition
        // line must never count as a call site of itself.
        let shipping_quote_file =
            "export function shippingQuote(order, region, opts) {\n\treturn { order, region, opts };\n}\n";

        let repo_files = [
            ("src/shippingQuote.ts", shipping_quote_file),
            ("src/checkout.ts", obvious_caller),
            ("src/render.py", python_caller),
            ("admin/manualQuote.ts", manual_quote),
        ];

        let enumerations = enumerate_call_sites(&changes, repo_files);
        assert_eq!(enumerations.len(), 1);
        let sites = &enumerations[0].call_sites;

        let found_in = |file: &str| sites.iter().any(|cs| cs.file == file);
        assert!(found_in("src/checkout.ts"), "obvious caller must be found");
        assert!(found_in("src/render.py"), "python caller must be found");
        assert!(
            found_in("admin/manualQuote.ts"),
            "the differently-shaped call site must be found: {sites:?}"
        );
        assert!(
            !found_in("src/shippingQuote.ts"),
            "the definition line itself must never count as a call site: {sites:?}"
        );
    }

    #[test]
    fn a_definition_line_is_never_counted_as_its_own_call_site() {
        let diff = "--- a/src/lib.rs\n\
                     +++ b/src/lib.rs\n\
                     @@ -1,2 +1,2 @@\n\
                     -pub fn foo(a: i32) {\n\
                     +pub fn foo(a: i32, b: i32) {\n";
        let changes = detect_signature_changes(diff);
        let repo_files = [(
            "src/lib.rs",
            "pub fn foo(a: i32, b: i32) {\n    foo_helper();\n}\n",
        )];
        let enumerations = enumerate_call_sites(&changes, repo_files);
        assert!(
            enumerations[0].call_sites.is_empty(),
            "the definition line itself must not be counted as a call site: {:?}",
            enumerations[0].call_sites
        );
    }

    #[test]
    fn no_signature_changes_means_no_enumerations() {
        let diff = "--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-a\n+b\n";
        let changes = detect_signature_changes(diff);
        assert!(changes.is_empty());
        let enumerations = enumerate_call_sites(&changes, std::iter::empty::<(&str, &str)>());
        assert!(enumerations.is_empty());
    }

    // ── render_call_site_section ───────────────────────────────────────────

    #[test]
    fn render_lists_every_call_site_with_file_and_line() {
        let enumerations = vec![CallSiteEnumeration {
            symbol: "shippingQuote".to_string(),
            file: "src/shippingQuote.ts".to_string(),
            call_sites: vec![CallSite {
                file: "admin/manualQuote.ts".to_string(),
                line: 2,
                text: "return finalize(shippingQuote(input.order, input.region), x);".to_string(),
            }],
        }];
        let section = render_call_site_section(&enumerations);
        assert!(section.contains("shippingQuote"));
        assert!(section.contains("admin/manualQuote.ts:2"));
    }

    #[test]
    fn render_of_no_changes_says_e7_does_not_apply() {
        assert!(render_call_site_section(&[]).contains("does not apply"));
    }

    // ── parse_call_site_line / missing_call_sites ──────────────────────────

    fn cs(file: &str, line: usize) -> CallSite {
        CallSite {
            file: file.to_string(),
            line,
            text: String::new(),
        }
    }

    fn enumeration(symbol: &str, sites: Vec<CallSite>) -> CallSiteEnumeration {
        CallSiteEnumeration {
            symbol: symbol.to_string(),
            file: "src/shippingQuote.ts".to_string(),
            call_sites: sites,
        }
    }

    #[test]
    fn parse_call_site_line_reads_the_grammar() {
        let v = parse_call_site_line(
            " shippingQuote | admin/manualQuote.ts:2 | MISSED | never updated for the new opts param",
        )
        .expect("parses");
        assert_eq!(v.symbol, "shippingQuote");
        assert_eq!(v.site, "admin/manualQuote.ts:2");
        assert_eq!(v.status, CallSiteStatus::Missed);
        assert!(v.note.contains("never updated"));
    }

    #[test]
    fn parse_call_site_line_drops_bad_status_and_missing_fields() {
        assert!(parse_call_site_line("shippingQuote | x.ts:1 | LGTM | ").is_none());
        assert!(parse_call_site_line("shippingQuote | | UPDATED |").is_none());
        assert!(parse_call_site_line("| x.ts:1 | UPDATED |").is_none());
    }

    #[test]
    fn one_unmodified_caller_is_reported_missing() {
        let enumerations = vec![enumeration(
            "shippingQuote",
            vec![cs("src/checkout.ts", 3), cs("admin/manualQuote.ts", 2)],
        )];
        // The reviewer accounted for the obvious caller but never mentioned
        // the differently-shaped one at all.
        let verdicts = vec![CallSiteVerdict {
            symbol: "shippingQuote".to_string(),
            site: "src/checkout.ts:3".to_string(),
            status: CallSiteStatus::Updated,
            note: String::new(),
        }];
        let missing = missing_call_sites(&enumerations, &verdicts);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].file, "admin/manualQuote.ts");
        assert_eq!(missing[0].line, 2);
    }

    #[test]
    fn a_call_site_explicitly_marked_missed_is_still_missing() {
        let enumerations = vec![enumeration(
            "shippingQuote",
            vec![cs("admin/manualQuote.ts", 2)],
        )];
        let verdicts = vec![CallSiteVerdict {
            symbol: "shippingQuote".to_string(),
            site: "admin/manualQuote.ts:2".to_string(),
            status: CallSiteStatus::Missed,
            note: "needs the new opts arg".to_string(),
        }];
        let missing = missing_call_sites(&enumerations, &verdicts);
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn all_callers_accounted_for_leaves_nothing_missing() {
        let enumerations = vec![enumeration(
            "shippingQuote",
            vec![cs("src/checkout.ts", 3), cs("admin/manualQuote.ts", 2)],
        )];
        let verdicts = vec![
            CallSiteVerdict {
                symbol: "shippingQuote".to_string(),
                site: "src/checkout.ts:3".to_string(),
                status: CallSiteStatus::Updated,
                note: String::new(),
            },
            CallSiteVerdict {
                symbol: "shippingQuote".to_string(),
                site: "admin/manualQuote.ts:2".to_string(),
                status: CallSiteStatus::Unaffected,
                note: "call omits opts positionally; default applies".to_string(),
            },
        ];
        assert!(missing_call_sites(&enumerations, &verdicts).is_empty());
    }

    /// Lane R66a: absent means no line at all; an explicit MISSED is not
    /// absent (it is a judgement), though both are missing.
    #[test]
    fn r66a_absent_call_sites_are_the_ones_with_no_line_at_all() {
        let enumerations = vec![enumeration(
            "shippingQuote",
            vec![
                cs("src/checkout.ts", 3),
                cs("admin/manualQuote.ts", 2),
                cs("api/handler.ts", 9),
            ],
        )];
        let verdicts = vec![
            CallSiteVerdict {
                symbol: "shippingQuote".to_string(),
                site: "src/checkout.ts:3".to_string(),
                status: CallSiteStatus::Updated,
                note: String::new(),
            },
            CallSiteVerdict {
                symbol: "shippingQuote".to_string(),
                site: "admin/manualQuote.ts:2".to_string(),
                status: CallSiteStatus::Missed,
                note: "still two arguments".to_string(),
            },
        ];
        let absent = absent_call_sites(&enumerations, &verdicts);
        assert_eq!(absent.len(), 1);
        assert_eq!(absent[0].file, "api/handler.ts");
        assert_eq!(missing_call_sites(&enumerations, &verdicts).len(), 2);
        assert!(absent_call_sites(&[], &[]).is_empty());
    }

    #[test]
    fn no_enumerations_means_nothing_can_be_missing() {
        assert!(missing_call_sites(&[], &[]).is_empty());
    }

    // ── Lane R67a: definition sites are not call sites ────────────────────

    /// The r67 s1 repo snapshot (M52 record r67 s1,
    /// `harness-refactor-shipping-quote-signature`), with both changed
    /// signatures reflowed across several lines exactly as the worker wrote
    /// them. The old single-line-only definition check could not see a
    /// definition HEAD (`export function quoteShipping(`), so both definition
    /// sites — `src/admin/manualQuote.ts:5` and `src/shipping/quote.ts:16` —
    /// were listed as call sites of themselves, and the reviewer had nothing
    /// truthful to say about them.
    fn r67_s1_repo() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "src/shipping/quote.ts",
                "import type { Zone, PackageType } from '../types.ts';\n\
                 \n\
                 const BASE_CENTS_PER_KG = 200;\n\
                 const FRAGILE_SURCHARGE_MULTIPLIER = 1.25;\n\
                 \n\
                 export function quoteShipping(\n\
                 \x20 weightKg: number,\n\
                 \x20 zone: Zone,\n\
                 \x20 packageType: PackageType,\n\
                 ): number {\n\
                 \x20 return baseCents(weightKg, zone);\n\
                 }\n",
            ),
            (
                "src/admin/manualQuote.ts",
                "import { quoteShipping } from '../shipping/quote.ts';\n\
                 \n\
                 export function computeManualQuote(\n\
                 \x20 weightKg: number,\n\
                 \x20 zone: Zone,\n\
                 \x20 packageType: PackageType,\n\
                 ): number {\n\
                 \x20 return quoteShipping(weightKg, zone, packageType);\n\
                 }\n",
            ),
            (
                "src/api/quoteHandler.ts",
                "export function handleQuoteRequest(order) {\n\
                 \x20 return { shippingCents: quoteShipping(order.weightKg, order.zone, order.packageType) };\n\
                 }\n",
            ),
            (
                "src/cart/cartSummary.ts",
                "export function estimateCartShipping(orders) {\n\
                 \x20 return orders.reduce(\n\
                 \x20   (sum, o) => sum + quoteShipping(o.weightKg, o.zone, o.packageType),\n\
                 \x20   0,\n\
                 \x20 );\n\
                 }\n",
            ),
            (
                "src/checkout/checkout.ts",
                "export function buildCheckoutSummary(order) {\n\
                 \x20 const shippingCents = quoteShipping(\n\
                 \x20   order.weightKg,\n\
                 \x20   order.zone,\n\
                 \x20   order.packageType,\n\
                 \x20 );\n\
                 \x20 return { shippingCents };\n\
                 }\n",
            ),
        ]
    }

    fn r67_s1_changes() -> Vec<SignatureChange> {
        let diff = "--- a/src/shipping/quote.ts\n\
                     +++ b/src/shipping/quote.ts\n\
                     @@ -6,3 +6,7 @@\n\
                     -export function quoteShipping(weightKg: number, zone: Zone): number {\n\
                     +export function quoteShipping(\n\
                     +  weightKg: number,\n\
                     +  zone: Zone,\n\
                     +  packageType: PackageType,\n\
                     +): number {\n\
                     --- a/src/admin/manualQuote.ts\n\
                     +++ b/src/admin/manualQuote.ts\n\
                     @@ -3,3 +3,7 @@\n\
                     -export function computeManualQuote(weightKg: number, zone: Zone): number {\n\
                     +export function computeManualQuote(\n\
                     +  weightKg: number,\n\
                     +  zone: Zone,\n\
                     +  packageType: PackageType,\n\
                     +): number {\n";
        detect_signature_changes(diff)
    }

    /// (a) A file that only DEFINES the changed symbol is never listed as a
    /// call site of it — including when the definition's parameter list is
    /// reflowed, so the definition line itself ends at the open paren.
    #[test]
    fn r67a_a_reflowed_definition_site_is_never_listed_as_a_call_site() {
        let changes = r67_s1_changes();
        assert_eq!(changes.len(), 2, "{changes:?}");
        let enumerations = enumerate_call_sites(&changes, r67_s1_repo());

        let sites = |symbol: &str| -> Vec<String> {
            enumerations
                .iter()
                .find(|e| e.symbol == symbol)
                .expect("enumerated")
                .call_sites
                .iter()
                .map(|cs| format!("{}:{}", cs.file, cs.line))
                .collect()
        };
        // `computeManualQuote` is only DEFINED (r67 s1 listed its definition
        // head, src/admin/manualQuote.ts:5, as its own call site).
        assert!(
            sites("computeManualQuote").is_empty(),
            "{:?}",
            sites("computeManualQuote")
        );
        // `quoteShipping`'s four real callers, and NOT its definition head at
        // src/shipping/quote.ts:6.
        assert_eq!(
            sites("quoteShipping"),
            vec![
                "src/admin/manualQuote.ts:8",
                "src/api/quoteHandler.ts:2",
                "src/cart/cartSummary.ts:3",
                "src/checkout/checkout.ts:2",
            ]
        );
    }

    /// (e) The relaxation must not swallow a real call site: a multi-line CALL
    /// (`const x = quoteShipping(` / `return quoteShipping(`) also ends at an
    /// open paren, and is still listed.
    #[test]
    fn r67a_a_multiline_call_is_still_a_call_site() {
        assert!(!line_defines_symbol(
            "  const shippingCents = quoteShipping(",
            "quoteShipping"
        ));
        assert!(!line_defines_symbol(
            "  return quoteShipping(",
            "quoteShipping"
        ));
        assert!(!line_defines_symbol(
            "  computeManualQuote(",
            "computeManualQuote"
        ));
        assert!(line_defines_symbol(
            "export function quoteShipping(",
            "quoteShipping"
        ));
        assert!(line_defines_symbol(
            "pub fn shipping_quote(",
            "shipping_quote"
        ));
        assert!(line_defines_symbol(
            "async def compute_quote(",
            "compute_quote"
        ));
        assert!(line_defines_symbol(
            "export const quoteShipping = (w: number) => w;",
            "quoteShipping"
        ));
        // A definition head for a DIFFERENT symbol never excludes this one.
        assert!(!line_defines_symbol(
            "export function computeManualQuote(",
            "quoteShipping"
        ));
    }

    // ── Lane R67a: lenient accounting (symbol + file, line optional) ───────

    /// The exact r67 s1 list, definition sites included, as the run recorded
    /// it: six sites across five files.
    fn r67_s1_listed_sites() -> Vec<CallSiteEnumeration> {
        vec![
            CallSiteEnumeration {
                symbol: "computeManualQuote".to_string(),
                file: "src/admin/manualQuote.ts".to_string(),
                call_sites: vec![cs("src/admin/manualQuote.ts", 5)],
            },
            CallSiteEnumeration {
                symbol: "quoteShipping".to_string(),
                file: "src/shipping/quote.ts".to_string(),
                call_sites: vec![
                    cs("src/admin/manualQuote.ts", 10),
                    cs("src/api/quoteHandler.ts", 10),
                    cs("src/cart/cartSummary.ts", 7),
                    cs("src/checkout/checkout.ts", 12),
                    cs("src/shipping/quote.ts", 16),
                ],
            },
        ]
    }

    fn verdict(symbol: &str, site: &str, status: CallSiteStatus) -> CallSiteVerdict {
        CallSiteVerdict {
            symbol: symbol.to_string(),
            site: site.to_string(),
            status,
            note: "reviewed".to_string(),
        }
    }

    /// (b)+(c) The r67 s1 send-back: the escalated reviewer answered four of
    /// the six listed sites — one line per file it considered meaningful — and
    /// the strict `file:line` rule turned the other two into the round's ONLY
    /// findings. Lenient accounting credits the four files it answered; the
    /// two files it never named (a different symbol, and the definition file)
    /// are still unaccounted, and the complaint names those FILES and symbols
    /// and says one line per file suffices.
    #[test]
    fn r67a_one_line_per_file_accounts_for_that_files_sites() {
        let enumerations = r67_s1_listed_sites();
        let reviewed = vec![
            verdict(
                "quoteShipping",
                "src/admin/manualQuote.ts:10",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/api/quoteHandler.ts:10",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/cart/cartSummary.ts:7",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/checkout/checkout.ts:12",
                CallSiteStatus::Updated,
            ),
        ];
        let missing: Vec<String> = missing_call_sites(&enumerations, &reviewed)
            .iter()
            .map(|site| format!("{}:{}", site.file, site.line))
            .collect();
        assert_eq!(
            missing,
            vec!["src/admin/manualQuote.ts:5", "src/shipping/quote.ts:16"],
            "the four answered files are accounted for"
        );

        let groups = unaccounted_call_site_files(&enumerations, &reviewed);
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].symbol, "computeManualQuote");
        assert_eq!(groups[0].file, "src/admin/manualQuote.ts");
        assert_eq!(groups[1].file, "src/shipping/quote.ts");
        for group in &groups {
            assert!(!group.explicitly_missed);
            let complaint = group.complaint();
            assert!(complaint.contains(&group.file), "{complaint}");
            assert!(complaint.contains(&group.symbol), "{complaint}");
            assert!(
                complaint.contains("one CALLSITE line per changed symbol per file"),
                "{complaint}"
            );
            assert!(complaint.contains("line number is optional"), "{complaint}");
        }
        assert_eq!(groups[1].evidence(), "src/shipping/quote.ts:16");
    }

    /// (b) One `UPDATED` line for a file accounts for EVERY site listed in it,
    /// and a line number pointing at a different line in the same file still
    /// accounts for that file's sites.
    #[test]
    fn r67a_one_line_covers_several_sites_in_one_file_whatever_the_line_number() {
        let enumerations = vec![enumeration(
            "quoteShipping",
            vec![
                cs("src/admin/manualQuote.ts", 5),
                cs("src/admin/manualQuote.ts", 10),
            ],
        )];
        // No line number at all.
        let by_file = vec![verdict(
            "quoteShipping",
            "src/admin/manualQuote.ts",
            CallSiteStatus::Updated,
        )];
        assert!(missing_call_sites(&enumerations, &by_file).is_empty());
        // One line number, naming only one of the two sites.
        let by_one_line = vec![verdict(
            "quoteShipping",
            "src/admin/manualQuote.ts:10",
            CallSiteStatus::Unaffected,
        )];
        assert!(missing_call_sites(&enumerations, &by_one_line).is_empty());
        // A line number that matches no listed site in that file still counts.
        let by_other_line = vec![verdict(
            "quoteShipping",
            "src/admin/manualQuote.ts:42",
            CallSiteStatus::Updated,
        )];
        assert!(missing_call_sites(&enumerations, &by_other_line).is_empty());
        // A path the reviewer shortened to its trailing components matches.
        let by_short_path = vec![verdict(
            "quoteShipping",
            "admin/manualQuote.ts",
            CallSiteStatus::Updated,
        )];
        assert!(missing_call_sites(&enumerations, &by_short_path).is_empty());
    }

    /// (e) The guarantee survives leniency: an explicit MISSED for a file
    /// marks that file's sites missed (not absent — so the gate requests
    /// changes rather than reading the table as incomplete), and a
    /// line-numbered MISSED marks exactly that line even when another line in
    /// the same file was accounted for.
    #[test]
    fn r67a_an_explicit_missed_still_marks_a_file_or_a_line() {
        let enumerations = vec![enumeration(
            "quoteShipping",
            vec![
                cs("src/admin/manualQuote.ts", 5),
                cs("src/admin/manualQuote.ts", 10),
            ],
        )];
        let file_missed = vec![verdict(
            "quoteShipping",
            "src/admin/manualQuote.ts",
            CallSiteStatus::Missed,
        )];
        assert_eq!(missing_call_sites(&enumerations, &file_missed).len(), 2);
        assert!(absent_call_sites(&enumerations, &file_missed).is_empty());
        let groups = unaccounted_call_site_files(&enumerations, &file_missed);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].explicitly_missed);
        assert_eq!(
            groups[0].evidence(),
            "src/admin/manualQuote.ts:5, src/admin/manualQuote.ts:10"
        );
        assert!(groups[0].complaint().contains("MISSED"));

        let one_line_missed = vec![
            verdict(
                "quoteShipping",
                "src/admin/manualQuote.ts:5",
                CallSiteStatus::Missed,
            ),
            verdict(
                "quoteShipping",
                "src/admin/manualQuote.ts:10",
                CallSiteStatus::Updated,
            ),
        ];
        let missing = missing_call_sites(&enumerations, &one_line_missed);
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert_eq!(missing[0].line, 5);
    }

    /// (b) The per-symbol requirement is kept: a listed symbol with no
    /// CALLSITE line at all is unaccounted (and ABSENT, so the R66a
    /// incomplete-table reading still applies), even when another symbol in
    /// the SAME file was answered.
    #[test]
    fn r67a_a_symbol_with_no_callsite_line_at_all_is_unaccounted() {
        let enumerations = r67_s1_listed_sites();
        let other_symbol_only = vec![verdict(
            "quoteShipping",
            "src/admin/manualQuote.ts",
            CallSiteStatus::Updated,
        )];
        let absent: Vec<String> = absent_call_sites(&enumerations, &other_symbol_only)
            .iter()
            .map(|site| format!("{}:{}", site.file, site.line))
            .collect();
        assert!(
            absent.contains(&"src/admin/manualQuote.ts:5".to_string()),
            "computeManualQuote was never named: {absent:?}"
        );
    }

    /// Two files with the same basename in different directories are never
    /// confused for each other by the path-suffix match.
    #[test]
    fn r67a_a_same_named_file_in_another_directory_does_not_account() {
        let enumerations = vec![enumeration("quoteShipping", vec![cs("src/a/quote.ts", 3)])];
        let elsewhere = vec![verdict(
            "quoteShipping",
            "src/b/quote.ts",
            CallSiteStatus::Updated,
        )];
        assert_eq!(missing_call_sites(&enumerations, &elsewhere).len(), 1);
    }

    /// (a)+(b) together, on the real record: with the definition sites gone
    /// from the list, the SAME reviewer response that was sent back as
    /// `REQUEST_CHANGES` accounts for every listed site and E7 raises nothing.
    #[test]
    fn r67a_the_r67_s1_review_accounts_for_every_site_once_definitions_are_excluded() {
        let enumerations = enumerate_call_sites(&r67_s1_changes(), r67_s1_repo());
        let reviewed = vec![
            verdict(
                "quoteShipping",
                "src/admin/manualQuote.ts",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/api/quoteHandler.ts",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/cart/cartSummary.ts",
                CallSiteStatus::Updated,
            ),
            verdict(
                "quoteShipping",
                "src/checkout/checkout.ts",
                CallSiteStatus::Updated,
            ),
        ];
        assert!(
            missing_call_sites(&enumerations, &reviewed).is_empty(),
            "{:?}",
            missing_call_sites(&enumerations, &reviewed)
        );
        assert!(unaccounted_call_site_files(&enumerations, &reviewed).is_empty());
    }

    #[test]
    fn r67a_a_site_ref_with_no_line_number_is_read_as_a_whole_file() {
        assert_eq!(
            parse_site_ref("src/shipping/quote.ts:16"),
            ("src/shipping/quote.ts", Some(16))
        );
        assert_eq!(
            parse_site_ref(" src/shipping/quote.ts "),
            ("src/shipping/quote.ts", None)
        );
        // A trailing non-numeric segment is part of the path, not a line.
        assert_eq!(
            parse_site_ref("src/shipping/quote.ts:head"),
            ("src/shipping/quote.ts:head", None)
        );
    }
}
