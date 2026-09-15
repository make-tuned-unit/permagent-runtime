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
/// `def`, a JS/TS `function` declaration, and a generic `name(params) {`
/// method shape (Java/Go/C-family, and JS/TS class methods) that requires a
/// trailing `{` so an ordinary call statement is never mistaken for a
/// definition. A signature change this misses is a false NEGATIVE — no HOLD
/// is raised for it — which is why [`enumerate_call_sites`]'s recall-first,
/// name-based matching is a second, independent line of defense once a
/// change IS detected, not a substitute for widening this list further as
/// new languages show up in the loop.
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
/// [`definition_patterns`] in order and returns the first match.
fn definition_in_line(line: &str) -> Option<(String, String)> {
    for re in definition_patterns() {
        if let Some(caps) = re.captures(line) {
            let name = caps.get(1)?.as_str().to_string();
            let params = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            return Some((name, normalize_params(params)));
        }
    }
    None
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
    // rejoining with a single canonical `, ` separator.
    params
        .split(',')
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
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

/// Find every function/method signature change in a unified diff: a symbol
/// defined on BOTH the removed (`-`) and added (`+`) side of the SAME file,
/// whose parameter lists differ. Deliberately not hunk-scoped — a large
/// rewrite can separate the old and new definition lines into different
/// hunks — so old/new definitions are collected per (file, symbol) across the
/// WHOLE diff and compared once at the end.
pub fn detect_signature_changes(diff: &str) -> Vec<SignatureChange> {
    let mut current_file: Option<String> = None;
    // (file, symbol) -> (normalized params, raw trimmed line)
    let mut removed: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut added: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut order: Vec<(String, String)> = Vec::new();

    for raw_line in diff.lines() {
        if let Some(rest) = raw_line.strip_prefix("+++ ") {
            let path = strip_diff_path_prefix(rest.trim());
            if path != "/dev/null" {
                current_file = Some(path);
            }
            continue;
        }
        if raw_line.starts_with("--- ") {
            // The old-side header; the file identity comes from `+++` above.
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("diff --git ") {
            // "a/<path> b/<path>" — tentatively set from the b/ side; a
            // following `+++` line (the common case) overrides this.
            if let Some(b_idx) = rest.find(" b/") {
                current_file = Some(rest[b_idx + 3..].trim().to_string());
            }
            continue;
        }

        let Some(file) = current_file.clone() else {
            continue;
        };

        if let Some(content) = raw_line.strip_prefix('+') {
            if let Some((name, params)) = definition_in_line(content) {
                let key = (file, name);
                if !added.contains_key(&key) {
                    order.push(key.clone());
                }
                added.insert(key, (params, content.trim().to_string()));
            }
        } else if let Some(content) = raw_line.strip_prefix('-') {
            if let Some((name, params)) = definition_in_line(content) {
                let key = (file, name);
                if !removed.contains_key(&key) {
                    order.push(key.clone());
                }
                removed.insert(key, (params, content.trim().to_string()));
            }
        }
    }

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
/// ITSELF a definition of the symbol (the old signature, the new signature,
/// or an unrelated overload) is never counted as a call site of itself.
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
                    let is_definition_of_this_symbol =
                        definition_in_line(line).is_some_and(|(name, _)| name == change.symbol);
                    if is_definition_of_this_symbol {
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
/// `Missed`. Either way it is unaccounted, and the non-empty result is what
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
            let key = format!("{}:{}", cs.file, cs.line);
            let accounted = verdicts.iter().any(|v| {
                v.symbol == e.symbol
                    && v.site == key
                    && matches!(
                        v.status,
                        CallSiteStatus::Updated | CallSiteStatus::Unaffected
                    )
            });
            if !accounted {
                missing.push(cs.clone());
            }
        }
    }
    missing
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

    #[test]
    fn no_enumerations_means_nothing_can_be_missing() {
        assert!(missing_call_sites(&[], &[]).is_empty());
    }
}
