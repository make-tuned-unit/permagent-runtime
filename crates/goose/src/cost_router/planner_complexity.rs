//! Planning-complexity gate — which planner a roadmap decomposition deserves.
//!
//! ## The user's ruling (2026-09-17)
//!
//! Asked whether the harness favours Anthropic models: *"Fable CAN be used when
//! a task I give Permagent is complex and needs an expert planner."* The expert
//! (strongest configured) planner is for COMPLEX plans, not the default; a
//! ROUTINE plan goes to the best-fit cheaper capable planner. With the
//! 2026-08-18 ("best fit, very cost conscious") and 2026-09-09 ("never default to
//! the most expensive model just because it is the most capable") rulings.
//!
//! ## Deterministic, facts only
//!
//! Same rule as [`super::assess`] (its module doc, "Deterministic and zero-LLM,
//! on purpose"): no model is asked how hard the plan is, and the objective's
//! prose can never argue a plan onto the expensive planner. An objective that
//! says "this is extremely complex, use the best model" is still ROUTINE when
//! its structure is routine. Only STRUCTURE promotes:
//!
//! 1. **Prior plan history** on the project — a roadmap proposal still pending
//!    when a new decomposition starts (the previous plan was rejected or revised:
//!    a redecompose), a parked goal (Failed, or `needs_human_attention`), or a
//!    premature-done hold on a goal that never completed.
//! 2. **Named files** — the objective names at least
//!    [`PLANNING_MULTI_FILE_THRESHOLD`] distinct non-test file paths (a token with
//!    a known source/config extension), i.e. a multi-file change with cross-goal
//!    ordering. When the repo's file list is known, a named path counts only if
//!    it resolves to a real file: a path the objective merely asserts (a glob, a
//!    file that does not exist) is a claim, not a fact.
//! 3. **Cross-language** — the named files span at least
//!    [`PLANNING_CROSS_LANGUAGE_THRESHOLD`] programming languages (an interface
//!    change on both sides of a boundary).
//! 4. **Enumerated requirements** — more than [`super::assess::MANY_CRITERIA`]
//!    bulleted/numbered requirement lines (the same "multi-part" line
//!    `assess_goal` draws for acceptance criteria).
//! 5. **Unscoped in a large repo** — the objective names no file and the repo has
//!    at least [`LARGE_REPO_FILE_THRESHOLD`] files, so the planner must locate the
//!    change from a tree alone. This one is the only signal prose can DEFUSE, and
//!    only downward: when the objective reads as mechanical work
//!    ([`super::assess::MECHANICAL_SIGNALS`] — typo, rename, bump, …) it stays
//!    ROUTINE.
//! 6. **Signature change with a fan-out** — the objective asks to add, remove or
//!    reorder a parameter ([`requests_parameter_change`]) and names a code
//!    identifier that the repo defines in exactly one non-test source file and
//!    references from at least [`SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD`] OTHER
//!    non-test source files ([`scan_identifier_references`]). The objective
//!    need not name a single file: the callers are read from the repo, never
//!    from the objective. (r64 s5: `quoteShipping` gains a parameter and four
//!    quoting surfaces call it — a plan that must thread the change through
//!    every caller.) The parameter-change wording only says WHAT kind of change
//!    is asked for; it never promotes on its own, the grounded fan-out does.
//!
//! Anything else is ROUTINE. The verdict carries every reason, routine or
//! complex, so the roadmap proposal and the tracing line are auditable.

use serde::Serialize;

/// Distinct named files at or above which a plan is a multi-file change. Two
/// files (a fix and its test) is still routine; three is where goal ordering and
/// shared interfaces start to matter. Deliberately below
/// [`super::assess`]'s per-goal `MANY_FILES` (8): that line sizes ONE worker's
/// goal, this one sizes the whole plan.
pub const PLANNING_MULTI_FILE_THRESHOLD: usize = 3;

/// Distinct programming languages among the named files at or above which a plan
/// is a cross-language change.
pub const PLANNING_CROSS_LANGUAGE_THRESHOLD: usize = 2;

/// Repo size (files walked by the decomposition's repo-facts probe) at or above
/// which an objective that names no file is unscoped enough to need the expert
/// planner.
pub const LARGE_REPO_FILE_THRESHOLD: usize = 500;

/// Other non-test source files (besides the defining one) that must reference
/// an identifier the objective asks to change the parameters of before the
/// plan is a signature change with a fan-out. Three callers is the same line as
/// [`PLANNING_MULTI_FILE_THRESHOLD`]: the definition plus three call sites is a
/// four-file change whose ordering the plan has to get right.
pub const SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD: usize = 3;

/// Most code identifiers from one objective that are resolved against the repo,
/// so a long objective cannot turn the scan into one pass per word.
pub const PLANNING_MAX_IDENTIFIERS: usize = 16;

/// A source file larger than this is skipped by the identifier scan (generated
/// or vendored code; a hand-written source file is far smaller).
pub const PLANNING_SCAN_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Total bytes the identifier scan reads per objective before it stops, so a
/// huge repo bounds the decomposition's wall time. The scan's counts are then a
/// lower bound, which can only miss a promotion, never invent one.
pub const PLANNING_SCAN_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// Words that, next to a parameter noun in the same sentence, ask for a change
/// to a function's parameter list ("one more argument added", "add a
/// `currency` parameter", "change the signature"). Negated uses ("do not
/// change the signature") are ignored, see [`NEGATION_WORDS`].
const PARAMETER_CHANGE_VERBS: &[&str] = &[
    "add",
    "adds",
    "added",
    "adding",
    "additional",
    "another",
    "more",
    "extra",
    "new",
    "remove",
    "removes",
    "removed",
    "removing",
    "drop",
    "drops",
    "dropped",
    "dropping",
    "reorder",
    "reorders",
    "reordered",
    "reordering",
    "change",
    "changes",
    "changed",
    "changing",
    "gain",
    "gains",
    "gained",
];

/// The parameter-list nouns [`PARAMETER_CHANGE_VERBS`] must share a sentence
/// with. "calling convention" is matched as a two-word phrase.
const PARAMETER_NOUNS: &[&str] = &[
    "parameter",
    "parameters",
    "param",
    "params",
    "argument",
    "arguments",
    "signature",
    "signatures",
    "arity",
];

/// A change verb preceded (within two words) by one of these is negated.
const NEGATION_WORDS: &[&str] = &["not", "never", "no", "without", "dont", "don", "t"];

/// Keywords that, directly before an identifier, make that occurrence its
/// definition (`fn`, `function`, `def`, `func`, `const x =`, `class X`, …).
const DEFINITION_KEYWORDS: &[&str] = &[
    "fn",
    "function",
    "def",
    "func",
    "fun",
    "class",
    "struct",
    "enum",
    "trait",
    "interface",
    "type",
    "const",
    "let",
    "var",
    "val",
    "static",
    "macro_rules",
];

/// The verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningComplexity {
    /// The best-fit cheaper capable planner is enough.
    Routine,
    /// The expert (strongest configured) planner is warranted.
    Complex,
}

impl PlanningComplexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanningComplexity::Routine => "routine",
            PlanningComplexity::Complex => "complex",
        }
    }
}

/// What the harness already knows about earlier plans for this project.
///
/// 2026-09-25 DAG K cost fix: `parked_goals`/`held_goals` alone used to force
/// `Complex` for every future `decompose_roadmap` call on a project after any
/// one goal parked or held, regardless of whether the NEW objective has
/// anything to do with the stuck one (see `assess_planning_complexity`'s
/// `history_overlap` gate below). `parked_titles`/`held_titles` carry the
/// stuck goals' titles so the assessment can require the new objective to
/// actually overlap one of them before history escalates the plan — a
/// project-wide "we've had trouble" signal is not "this new plan is hard."
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PriorPlanHistory {
    /// A roadmap proposal is still pending on the project as a new decomposition
    /// starts: the previous plan was never approved into cards (rejected, or
    /// revised and re-requested).
    pub unapproved_proposal: bool,
    /// Live goal cards on this project that parked (Failed, or flagged
    /// `needs_human_attention`).
    pub parked_goals: usize,
    /// Live goal cards on this project that a premature-done hold fired on and
    /// that have not completed since.
    pub held_goals: usize,
    /// Titles of the parked goals counted in `parked_goals` (bounded; see
    /// `PRIOR_GOAL_TITLES_CAP`). Used only to test objective overlap — never
    /// counted on their own.
    pub parked_titles: Vec<String>,
    /// Titles of the held goals counted in `held_goals` (bounded; see
    /// `PRIOR_GOAL_TITLES_CAP`). Used only to test objective overlap — never
    /// counted on their own.
    pub held_titles: Vec<String>,
}

/// Cap on how many parked/held goal titles a caller need supply — the
/// overlap check is cheap per title but the query result is otherwise
/// unbounded on a long-lived project.
pub const PRIOR_GOAL_TITLES_CAP: usize = 20;

/// Content words shared between the new objective and a stuck goal's title
/// below which the two are not considered "the same problem." Deliberately
/// low: an exact rewording (the 01a0d5ea case, "Instrument event detail
/// pages…" redispatched with different phrasing) has to clear it, but two
/// unrelated one-line objectives on the same project must not.
const HISTORY_OVERLAP_JACCARD_THRESHOLD: f64 = 0.2;

/// Generic words too common to signal that two objectives are "the same
/// problem" — kept deliberately short and non-domain-specific so it never
/// filters out the words that actually distinguish one objective from
/// another.
const HISTORY_OVERLAP_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "have", "has", "are", "was",
    "were", "will", "add", "adds", "update", "updates", "make", "makes", "when", "then", "also",
    "page", "pages", "support", "should", "need", "needs", "please", "ensure",
];

/// Lowercased content-word set of `text`: alphanumeric runs of at least 3
/// characters, generic stopwords dropped. Used only to compare one objective
/// against another for topical overlap (never to score difficulty).
fn content_tokens(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
        .filter(|w| !HISTORY_OVERLAP_STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Whether `new_objective` and `prior_title` name overlapping work: their
/// content-word sets share at least one word and clear
/// [`HISTORY_OVERLAP_JACCARD_THRESHOLD`] Jaccard similarity. Deterministic,
/// no LLM — the same "facts only" rule as the rest of this module.
fn objectives_overlap(new_objective: &str, prior_title: &str) -> bool {
    let a = content_tokens(new_objective);
    let b = content_tokens(prior_title);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let intersection = a.intersection(&b).count();
    if intersection == 0 {
        return false;
    }
    let union = a.union(&b).count();
    (intersection as f64 / union as f64) >= HISTORY_OVERLAP_JACCARD_THRESHOLD
}

/// Where one code identifier the objective names lives in the repo — read from
/// the files, never from the objective ([`scan_identifier_references`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IdentifierReferences {
    pub identifier: String,
    /// Non-test source files with a definition of it (a definition keyword
    /// directly before the name).
    pub defined_in: Vec<String>,
    /// Non-test source files that mention it as a whole word, definitions
    /// included.
    pub referenced_in: Vec<String>,
}

impl IdentifierReferences {
    /// Non-test source files other than its single definition that reference
    /// it, or `None` when it is not defined in exactly one file.
    pub fn caller_files(&self) -> Option<usize> {
        match self.defined_in.as_slice() {
            [definition] => Some(
                self.referenced_in
                    .iter()
                    .filter(|f| *f != definition)
                    .count(),
            ),
            _ => None,
        }
    }
}

/// The recorded summary of one resolved identifier (file lists are summarized
/// to counts so a common name in a large repo cannot bloat the record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdentifierSignal {
    pub identifier: String,
    pub defined_in: Vec<String>,
    pub referencing_files: usize,
}

/// The facts the assessment reads — nothing a model wrote about difficulty.
/// Not `Copy`: `history` now carries `Vec<String>` title lists.
#[derive(Debug, Clone)]
pub struct PlanningFacts<'a> {
    /// The operator's objective. Read for STRUCTURE only (file tokens, list
    /// lines, code identifiers, whether a parameter change is asked for) plus
    /// the down-only mechanical vocabulary.
    pub objective: &'a str,
    /// Files the repo-facts probe walked (capped by that probe).
    pub repo_file_count: usize,
    /// The repo's files (relative paths) when known. Non-empty turns on
    /// grounding: named paths count only when they resolve to one of these.
    /// Empty (no repo, or a greenfield directory) keeps the lexical count.
    pub repo_files: &'a [String],
    /// Where the objective's code identifiers live in the repo.
    pub identifier_references: &'a [IdentifierReferences],
    pub history: PriorPlanHistory,
}

/// The auditable verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanningComplexityVerdict {
    pub complexity: PlanningComplexity,
    /// Every signal that decided the verdict, in plain words.
    pub reasons: Vec<String>,
    /// Distinct file paths the objective names.
    pub named_files: Vec<String>,
    /// Distinct programming languages among `named_files`.
    pub named_languages: Vec<String>,
    /// Bulleted / numbered requirement lines in the objective.
    pub enumerated_requirements: usize,
    /// Named paths that did not resolve to a real repo file (not counted).
    pub unresolved_named_files: Vec<String>,
    /// Named test files (resolved, not counted toward the multi-file line).
    pub named_test_files: Vec<String>,
    /// Whether the objective asks to add, remove or reorder a parameter.
    pub parameter_change_requested: bool,
    /// The objective's code identifiers as resolved in the repo.
    pub identifiers: Vec<IdentifierSignal>,
    /// Distinct non-test source files that reference any named identifier.
    pub identifier_reference_files: usize,
    /// Identifiers whose parameter change fans out to
    /// [`SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD`] or more caller files.
    pub signature_changes: Vec<String>,
}

impl PlanningComplexityVerdict {
    /// The JSON recorded on the roadmap proposal.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Extensions that make a token a file path. Programming languages map to a
/// language; docs/config files count as named files but not as a language.
/// `None` for an extension that is neither. Shared with the review-complexity
/// assessment ([`super::recommend::ReviewFacts::from_diff`]).
pub(crate) fn language_of_extension(ext: &str) -> Option<Option<&'static str>> {
    let lang = match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => {
            Some("javascript/typescript")
        }
        "py" => Some("python"),
        "go" => Some("go"),
        "swift" | "m" | "mm" => Some("swift/objective-c"),
        "kt" | "kts" | "java" | "scala" => Some("jvm"),
        "rb" => Some("ruby"),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" => Some("c/c++"),
        "cs" => Some("dotnet"),
        "php" => Some("php"),
        "ex" | "exs" => Some("elixir"),
        "dart" => Some("dart"),
        "zig" => Some("zig"),
        "md" | "toml" | "json" | "yaml" | "yml" | "sh" | "bash" | "zsh" | "sql" | "html"
        | "css" | "scss" | "txt" | "lock" | "xml" | "plist" | "proto" | "graphql" => None,
        _ => return None,
    };
    Some(lang)
}

/// Library/framework names spelled like a file (`Node.js`) — never a path.
const FRAMEWORK_NAMES: &[&str] = &[
    "node.js",
    "next.js",
    "nuxt.js",
    "vue.js",
    "react.js",
    "express.js",
    "three.js",
    "d3.js",
    "chart.js",
    "ember.js",
    "angular.js",
    "deno.js",
];

/// The file path a whitespace-separated token names, lowercased, or `None`.
fn file_token(raw: &str) -> Option<(String, Option<&'static str>)> {
    let token = raw
        .trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')))
        .trim_end_matches('.')
        .to_lowercase();
    if token.is_empty() || token.contains("://") || FRAMEWORK_NAMES.contains(&token.as_str()) {
        return None;
    }
    let (stem, ext) = token.rsplit_once('.')?;
    if stem.is_empty() || stem.ends_with('/') || stem.ends_with('.') {
        return None;
    }
    let language = language_of_extension(ext)?;
    Some((token, language))
}

/// Whether a line is a bulleted or numbered list item (`- x`, `* x`, `• x`,
/// `1. x`, `2) x`).
fn is_enumerated_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("• ") {
        return true;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return false;
    }
    let mut rest = t.chars().skip(digits);
    matches!(
        (rest.next(), rest.next()),
        (Some('.') | Some(')'), Some(' '))
    )
}

/// Whether a lowercased named path resolves to a real repo file: the same
/// relative path, or a repo file whose path ends with `/<named path>` (the
/// objective said `quote.ts` for `src/shipping/quote.ts`).
fn path_resolves(named: &str, repo_files: &[String]) -> bool {
    let named = named.strip_prefix("./").unwrap_or(named);
    let suffix = format!("/{named}");
    repo_files.iter().any(|file| {
        let file = file.replace('\\', "/").to_lowercase();
        file == named || file.ends_with(&suffix)
    })
}

/// The objective's sentences: lines are joined into paragraphs (a blank line or
/// a bulleted/numbered line starts a new one, so a wrapped sentence stays
/// whole), then split after `.`, `!` or `?` followed by whitespace, and at `;`.
fn sentences(text: &str) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_enumerated_line(line) {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            if trimmed.is_empty() {
                continue;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(trimmed);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }

    let mut out = Vec::new();
    for paragraph in paragraphs {
        let mut sentence = String::new();
        let mut chars = paragraph.chars().peekable();
        while let Some(c) = chars.next() {
            sentence.push(c);
            let ends = match c {
                ';' => true,
                '.' | '!' | '?' => chars.peek().is_none_or(|n| n.is_whitespace()),
                _ => false,
            };
            if ends {
                out.push(std::mem::take(&mut sentence));
            }
        }
        if !sentence.trim().is_empty() {
            out.push(sentence);
        }
    }
    out
}

/// Whether the objective asks to add, remove or reorder a function parameter:
/// one sentence holds both a non-negated [`PARAMETER_CHANGE_VERBS`] word and a
/// [`PARAMETER_NOUNS`] word (or "calling convention"). Describes the KIND of
/// change only; it never promotes a plan without the repo's fan-out.
pub fn requests_parameter_change(objective: &str) -> bool {
    sentences(objective).iter().any(|sentence| {
        let words: Vec<String> = sentence
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let has_noun = words.iter().any(|w| PARAMETER_NOUNS.contains(&w.as_str()))
            || words
                .windows(2)
                .any(|pair| pair[0] == "calling" && pair[1] == "convention");
        let has_verb = words.iter().enumerate().any(|(i, w)| {
            PARAMETER_CHANGE_VERBS.contains(&w.as_str())
                && !words[i.saturating_sub(2)..i]
                    .iter()
                    .any(|prev| NEGATION_WORDS.contains(&prev.as_str()))
        });
        has_noun && has_verb
    })
}

fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether the occurrence of an identifier starting at byte `start` of
/// `bytes` is its definition: the word directly before it (whitespace
/// skipped) is a [`DEFINITION_KEYWORDS`] entry.
fn is_definition_at(bytes: &[u8], start: usize) -> bool {
    let mut end = start;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == start {
        return false;
    }
    let mut begin = end;
    while begin > 0 && is_identifier_byte(bytes[begin - 1]) {
        begin -= 1;
    }
    std::str::from_utf8(&bytes[begin..end])
        .map(|keyword| DEFINITION_KEYWORDS.contains(&keyword))
        .unwrap_or(false)
}

/// Resolve `identifiers` against the repo at `root`: for each, the non-test
/// source files (a programming-language extension, not a test path) that
/// define it and that mention it as a whole word. Reads only files in
/// `repo_files` (the repo-facts walk, which already skips vendored and build
/// directories), bounded by [`PLANNING_SCAN_MAX_FILE_BYTES`] per file and
/// [`PLANNING_SCAN_MAX_TOTAL_BYTES`] overall; unreadable files are skipped.
/// These are grounded facts about the repo, not claims in the objective.
pub fn scan_identifier_references(
    root: &std::path::Path,
    repo_files: &[String],
    identifiers: &[String],
) -> Vec<IdentifierReferences> {
    let identifiers: Vec<&String> = identifiers
        .iter()
        .filter(|id| !id.is_empty() && id.bytes().all(is_identifier_byte))
        .take(PLANNING_MAX_IDENTIFIERS)
        .collect();
    let mut out: Vec<IdentifierReferences> = identifiers
        .iter()
        .map(|id| IdentifierReferences {
            identifier: (*id).clone(),
            ..Default::default()
        })
        .collect();
    if out.is_empty() {
        return out;
    }
    let mut total_bytes: u64 = 0;
    for file in repo_files {
        let is_source = file
            .rsplit_once('.')
            .and_then(|(_, ext)| language_of_extension(&ext.to_lowercase()))
            .is_some_and(|language| language.is_some());
        if !is_source || crate::tool_monitor::is_test_path(file) {
            continue;
        }
        let path = root.join(file);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > PLANNING_SCAN_MAX_FILE_BYTES {
            continue;
        }
        if total_bytes.saturating_add(meta.len()) > PLANNING_SCAN_MAX_TOTAL_BYTES {
            break;
        }
        total_bytes += meta.len();
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let bytes = content.as_bytes();
        for (slot, id) in out.iter_mut().zip(identifiers.iter()) {
            let len = id.len();
            let mut referenced = false;
            let mut defined = false;
            for (start, _) in content.match_indices(id.as_str()) {
                let before_ok = start == 0 || !is_identifier_byte(bytes[start - 1]);
                let after_ok =
                    start + len >= bytes.len() || !is_identifier_byte(bytes[start + len]);
                if !(before_ok && after_ok) {
                    continue;
                }
                referenced = true;
                if is_definition_at(bytes, start) {
                    defined = true;
                    break;
                }
            }
            if referenced {
                slot.referenced_in.push(file.clone());
            }
            if defined {
                slot.defined_in.push(file.clone());
            }
        }
    }
    out
}

/// THE assessment. Pure and deterministic; see the module doc for the signals.
pub fn assess_planning_complexity(facts: &PlanningFacts<'_>) -> PlanningComplexityVerdict {
    let grounded = !facts.repo_files.is_empty();
    let mut named_files: Vec<String> = Vec::new();
    let mut named_languages: Vec<String> = Vec::new();
    let mut unresolved_named_files: Vec<String> = Vec::new();
    let mut named_test_files: Vec<String> = Vec::new();
    for raw in facts.objective.split_whitespace() {
        if let Some((path, language)) = file_token(raw) {
            if grounded && !path_resolves(&path, facts.repo_files) {
                if !unresolved_named_files.contains(&path) {
                    unresolved_named_files.push(path);
                }
                continue;
            }
            if crate::tool_monitor::is_test_path(&path) {
                if !named_test_files.contains(&path) {
                    named_test_files.push(path);
                }
                continue;
            }
            if !named_files.contains(&path) {
                named_files.push(path);
            }
            if let Some(lang) = language {
                if !named_languages.iter().any(|l| l == lang) {
                    named_languages.push(lang.to_string());
                }
            }
        }
    }
    let enumerated_requirements = facts
        .objective
        .lines()
        .filter(|l| is_enumerated_line(l))
        .count();

    let mut complex: Vec<String> = Vec::new();
    let h = &facts.history;
    if h.unapproved_proposal {
        complex.push(
            "a previous roadmap proposal for this project was never approved into cards \
             (rejected or revised) — this is a redecompose"
                .to_string(),
        );
    }
    // 2026-09-25 DAG K cost fix: a parked/held goal only escalates THIS plan
    // when its title overlaps the new objective — otherwise a project with
    // one unrelated stuck goal would force every future decompose_roadmap
    // call onto the expert planner forever, independent of the new
    // objective's own size (the actual complexity signal below).
    let overlapping_parked = h
        .parked_titles
        .iter()
        .filter(|title| objectives_overlap(facts.objective, title))
        .count();
    if overlapping_parked > 0 {
        complex.push(format!(
            "{overlapping_parked} of {} goal(s) on this project parked (failed or needs human \
             attention) share this objective's wording — likely a redispatch of the same stuck \
             work",
            h.parked_goals
        ));
    }
    let overlapping_held = h
        .held_titles
        .iter()
        .filter(|title| objectives_overlap(facts.objective, title))
        .count();
    if overlapping_held > 0 {
        complex.push(format!(
            "{overlapping_held} of {} goal(s) on this project were held for a premature done \
             and share this objective's wording — likely a redispatch of the same stuck work",
            h.held_goals
        ));
    }
    if named_files.len() >= PLANNING_MULTI_FILE_THRESHOLD {
        complex.push(format!(
            "names {} distinct files (>= {PLANNING_MULTI_FILE_THRESHOLD}): a multi-file change",
            named_files.len()
        ));
    }
    if named_languages.len() >= PLANNING_CROSS_LANGUAGE_THRESHOLD {
        complex.push(format!(
            "names files in {} languages ({}): a cross-language change",
            named_languages.len(),
            named_languages.join(", ")
        ));
    }
    if enumerated_requirements > super::assess::MANY_CRITERIA {
        complex.push(format!(
            "enumerates {enumerated_requirements} requirements (> {}): a multi-part plan",
            super::assess::MANY_CRITERIA
        ));
    }
    let parameter_change_requested = requests_parameter_change(facts.objective);
    let mut signature_changes: Vec<String> = Vec::new();
    let mut reference_files: Vec<&String> = Vec::new();
    let mut identifiers: Vec<IdentifierSignal> = Vec::new();
    for refs in facts.identifier_references {
        for file in &refs.referenced_in {
            if !reference_files.contains(&file) {
                reference_files.push(file);
            }
        }
        identifiers.push(IdentifierSignal {
            identifier: refs.identifier.clone(),
            defined_in: refs.defined_in.clone(),
            referencing_files: refs.referenced_in.len(),
        });
        if !parameter_change_requested {
            continue;
        }
        if let Some(callers) = refs.caller_files() {
            if callers >= SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD {
                complex.push(format!(
                    "asks to change the parameters of `{}` (defined in {}), which {callers} other \
                     source file(s) call (>= {SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD}): a \
                     signature change that fans out to its callers",
                    refs.identifier, refs.defined_in[0]
                ));
                signature_changes.push(refs.identifier.clone());
            }
        }
    }
    let identifier_reference_files = reference_files.len();

    let lowered = facts.objective.to_lowercase();
    let reads_mechanical = super::assess::MECHANICAL_SIGNALS
        .iter()
        .any(|s| lowered.contains(s));
    let unscoped_large =
        named_files.is_empty() && facts.repo_file_count >= LARGE_REPO_FILE_THRESHOLD;
    if unscoped_large && !reads_mechanical {
        complex.push(format!(
            "names no file in a large repo ({} files >= {LARGE_REPO_FILE_THRESHOLD}): the planner \
             must locate the change from the tree",
            facts.repo_file_count
        ));
    }

    let complexity = if complex.is_empty() {
        PlanningComplexity::Routine
    } else {
        PlanningComplexity::Complex
    };
    let reasons = if complexity == PlanningComplexity::Complex {
        complex
    } else {
        let mut reasons = vec![
            format!(
                "names {} non-test file(s) in {} language(s) (multi-file at \
                 {PLANNING_MULTI_FILE_THRESHOLD}, cross-language at \
                 {PLANNING_CROSS_LANGUAGE_THRESHOLD})",
                named_files.len(),
                named_languages.len()
            ),
            format!(
                "{enumerated_requirements} enumerated requirement(s) (multi-part above {})",
                super::assess::MANY_CRITERIA
            ),
            format!("repo has {} file(s)", facts.repo_file_count),
            format!(
                "named identifiers are referenced from {identifier_reference_files} source \
                 file(s); no parameter change fans out to {SIGNATURE_CHANGE_CALLER_FILES_THRESHOLD} \
                 or more caller files{}",
                if parameter_change_requested {
                    ""
                } else {
                    " (no parameter change asked for)"
                }
            ),
            "no pending, parked or held plan history on this project".to_string(),
        ];
        if !unresolved_named_files.is_empty() {
            reasons.push(format!(
                "{} named path(s) do not exist in the repo and were not counted: {}",
                unresolved_named_files.len(),
                unresolved_named_files.join(", ")
            ));
        }
        if unscoped_large {
            reasons.push(
                "names no file in a large repo, but reads as mechanical work — prose may only \
                 route a plan down"
                    .to_string(),
            );
        }
        reasons
    };
    PlanningComplexityVerdict {
        complexity,
        reasons,
        named_files,
        named_languages,
        enumerated_requirements,
        unresolved_named_files,
        named_test_files,
        parameter_change_requested,
        identifiers,
        identifier_reference_files,
        signature_changes,
    }
}

/// r64 s5 (`harness-refactor-shipping-quote-signature`): the fixture repo's
/// source files, verbatim. Shared by this module's tests and the
/// orchestrator's end-to-end decomposition test.
#[cfg(test)]
pub(crate) const R64_S5_FIXTURE_FILES: &[(&str, &str)] = &[
    (
        "src/types.ts",
        "export type Zone = \"domestic\" | \"international\";\nexport type PackageType = \"standard\" | \"fragile\";\n\nexport interface Order {\n  id: string;\n  weightKg: number;\n  zone: Zone;\n  packageType: PackageType;\n}\n",
    ),
    (
        "src/shipping/quote.ts",
        "import type { Zone } from \"../types.ts\";\n\nconst BASE_RATE_CENTS_PER_KG: Record<Zone, number> = {\n  domestic: 200,\n  international: 550,\n};\n\n/**\n * Quotes shipping cost in cents for a package of the given weight and zone.\n */\nexport function quoteShipping(weightKg: number, zone: Zone): number {\n  return Math.round(weightKg * BASE_RATE_CENTS_PER_KG[zone]);\n}\n",
    ),
    (
        "src/admin/manualQuote.ts",
        "import { quoteShipping } from \"../shipping/quote.ts\";\nimport type { Zone } from \"../types.ts\";\n\n/** Support-desk tool: quote shipping for a one-off manual order. */\nexport function computeManualQuote(weightKg: number, zone: Zone): number {\n  return quoteShipping(weightKg, zone);\n}\n",
    ),
    (
        "src/api/quoteHandler.ts",
        "import { quoteShipping } from \"../shipping/quote.ts\";\nimport type { Order } from \"../types.ts\";\n\nexport interface QuoteResponse {\n  shippingCents: number;\n}\n\n/** The public API handler used by the storefront's quote endpoint. */\nexport function handleQuoteRequest(order: Order): QuoteResponse {\n  return { shippingCents: quoteShipping(order.weightKg, order.zone) };\n}\n",
    ),
    (
        "src/cart/cartSummary.ts",
        "import { quoteShipping } from \"../shipping/quote.ts\";\nimport type { Order } from \"../types.ts\";\n\n/** Estimated total shipping cost (cents) for every order in a cart preview. */\nexport function estimateCartShipping(orders: Order[]): number {\n  return orders.reduce((sum, o) => sum + quoteShipping(o.weightKg, o.zone), 0);\n}\n",
    ),
    (
        "src/checkout/checkout.ts",
        "import { quoteShipping } from \"../shipping/quote.ts\";\nimport type { Order } from \"../types.ts\";\n\nexport interface CheckoutSummary {\n  itemsTotalCents: number;\n  shippingCents: number;\n  grandTotalCents: number;\n}\n\nexport function buildCheckoutSummary(order: Order): CheckoutSummary {\n  const itemsTotalCents = 5000; // placeholder demo total, not under test\n  const shippingCents = quoteShipping(order.weightKg, order.zone);\n  return {\n    itemsTotalCents,\n    shippingCents,\n    grandTotalCents: itemsTotalCents + shippingCents,\n  };\n}\n",
    ),
    ("README.md", "# Shipping quotes\n"),
    ("package.json", "{ \"name\": \"shipping\", \"type\": \"module\" }\n"),
];

/// r64 s5: the fixture task's own prompt — it names surfaces and
/// `quoteShipping`, but not one file path.
#[cfg(test)]
pub(crate) const R64_S5_TASK_PROMPT: &str =
    "We need to charge 25% more to ship fragile packages, and it has to apply
everywhere a shipping cost gets quoted -- checkout, the cart preview, the
admin manual-quote tool, and the storefront's quote API. Right now none of
them account for whether a package is fragile at all. Add fragile-package
support end to end so every place that quotes shipping reflects it.

Keep `quoteShipping`'s existing calling convention: it stays a plain
function taking the same scalar weight and zone arguments it already
takes today, just with one more scalar argument added for the package
type (e.g. a \"standard\" / \"fragile\" string) -- do not turn it into a
function that takes a single `Order`-shaped object instead. The call
sites that already work with full order objects (checkout, the cart
preview, the API handler) should keep doing that and just pass the
package type through the same way they already pass weight and zone.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(objective: &str, repo_file_count: usize) -> PlanningFacts<'_> {
        PlanningFacts {
            objective,
            repo_file_count,
            repo_files: &[],
            identifier_references: &[],
            history: PriorPlanHistory::default(),
        }
    }

    #[test]
    fn one_file_fix_in_a_small_repo_is_routine() {
        let v = assess_planning_complexity(&facts(
            "Fix the off-by-one in src/pagination.rs so the last page is included.",
            12,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        assert_eq!(v.named_files, vec!["src/pagination.rs".to_string()]);
        assert!(!v.reasons.is_empty(), "a routine verdict still says why");
    }

    #[test]
    fn multi_file_refactor_is_complex() {
        let v = assess_planning_complexity(&facts(
            "Refactor the quote pipeline: split src/quote.rs into src/quote/parse.rs and \
             src/quote/price.rs, and update src/api.rs callers.",
            40,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        assert!(v.named_files.len() >= PLANNING_MULTI_FILE_THRESHOLD);
        assert!(v.reasons.iter().any(|r| r.contains("multi-file")));
    }

    #[test]
    fn cross_language_change_is_complex() {
        let v = assess_planning_complexity(&facts(
            "Add a `currency` field to crates/server/src/quote.rs and show it in ui/src/Quote.tsx",
            60,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        assert_eq!(v.named_languages.len(), 2);
        assert!(v.reasons.iter().any(|r| r.contains("cross-language")));
    }

    #[test]
    fn an_unapproved_proposal_is_complex_without_needing_overlap() {
        // A pending, never-approved roadmap proposal is about THIS decompose
        // call directly (it's a redecompose of the very objective in flight),
        // so unlike parked/held goals it does not need a title-overlap check.
        let v = assess_planning_complexity(&PlanningFacts {
            objective: "Fix the off-by-one in src/pagination.rs",
            repo_file_count: 12,
            repo_files: &[],
            identifier_references: &[],
            history: PriorPlanHistory {
                unapproved_proposal: true,
                ..Default::default()
            },
        });
        assert_eq!(v.complexity, PlanningComplexity::Complex);
    }

    /// 2026-09-25 DAG K cost fix (root cause behind goal 01a0d5ea's
    /// escalations): a parked or held goal ELSEWHERE on the project — with no
    /// wording in common with the new objective — must not force every future
    /// decompose_roadmap call on that project onto the expert planner.
    #[test]
    fn unrelated_parked_or_held_goals_do_not_alone_make_a_plan_complex() {
        for history in [
            PriorPlanHistory {
                parked_goals: 1,
                parked_titles: vec!["Fix the login page CSS on mobile Safari".to_string()],
                ..Default::default()
            },
            PriorPlanHistory {
                held_goals: 2,
                held_titles: vec![
                    "Rotate the staging API key".to_string(),
                    "Clean up dead feature flags".to_string(),
                ],
                ..Default::default()
            },
        ] {
            let v = assess_planning_complexity(&PlanningFacts {
                objective: "Fix the off-by-one in src/pagination.rs",
                repo_file_count: 12,
                repo_files: &[],
                identifier_references: &[],
                history,
            });
            assert_eq!(v.complexity, PlanningComplexity::Routine, "{v:?}");
        }
    }

    /// The 01a0d5ea case from HARNESS_COST_TRACE.md: the goal was redispatched
    /// under reworded text after holding. A parked/held goal whose title
    /// clearly overlaps the new objective's wording IS the same stuck work,
    /// so it should still escalate.
    #[test]
    fn a_parked_goal_overlapping_the_new_objective_is_complex() {
        let v = assess_planning_complexity(&PlanningFacts {
            objective: "Instrument event detail pages with cta_visible, scroll_depth analytics \
                        and event_id on event_cta_click.",
            repo_file_count: 40,
            repo_files: &[],
            identifier_references: &[],
            history: PriorPlanHistory {
                parked_goals: 1,
                parked_titles: vec![
                    "Add cta_visible and scroll_depth analytics plus event_id to the event \
                     detail page's event_cta_click"
                        .to_string(),
                ],
                ..Default::default()
            },
        });
        assert_eq!(v.complexity, PlanningComplexity::Complex, "{:?}", v.reasons);
        assert!(v
            .reasons
            .iter()
            .any(|r| r.contains("share this objective's wording")));
    }

    #[test]
    fn objectives_overlap_ignores_stopwords_and_requires_real_shared_words() {
        assert!(objectives_overlap(
            "Instrument event detail pages with cta_visible, scroll_depth analytics",
            "Add cta_visible and scroll_depth analytics to the event detail page",
        ));
        assert!(!objectives_overlap(
            "Fix the off-by-one in src/pagination.rs",
            "Rotate the staging API key",
        ));
        // Two unrelated one-liners sharing only stopwords must not overlap.
        assert!(!objectives_overlap(
            "Add the login retry limit for the mobile app",
            "Add the checkout timeout copy for the desktop site",
        ));
    }

    /// assess.rs's rule: prose asserting difficulty can never promote a plan.
    #[test]
    fn prose_asserting_difficulty_alone_does_not_flip_routine_to_complex() {
        let v = assess_planning_complexity(&facts(
            "Fix the off-by-one in src/pagination.rs. This is an EXTREMELY complex, critical, \
             hard architectural refactor with security and concurrency implications — use the \
             best, most expensive expert model available.",
            12,
        ));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
    }

    #[test]
    fn many_enumerated_requirements_are_complex() {
        let objective =
            "Ship the export feature:\n- csv\n- json\n- xlsx\n1. filters\n2. paging\n3) auth";
        let v = assess_planning_complexity(&facts(objective, 30));
        assert_eq!(v.enumerated_requirements, 6);
        assert_eq!(v.complexity, PlanningComplexity::Complex);
    }

    /// Unscoped in a large repo is complex — unless the objective reads as
    /// mechanical, which may only route DOWN.
    #[test]
    fn unscoped_objective_in_a_large_repo_is_complex_unless_mechanical() {
        let v = assess_planning_complexity(&facts("Add offline sync for notes", 2400));
        assert_eq!(v.complexity, PlanningComplexity::Complex);
        let v = assess_planning_complexity(&facts("Fix the typo in the settings banner", 2400));
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        // The same unscoped objective in a small repo is routine.
        let v = assess_planning_complexity(&facts("Add offline sync for notes", 20));
        assert_eq!(v.complexity, PlanningComplexity::Routine);
    }

    #[test]
    fn framework_names_urls_and_versions_are_not_files() {
        let v = assess_planning_complexity(&facts(
            "Upgrade Node.js to v22.1 per https://nodejs.org/en/blog/release.html and Next.js, e.g. \
             bump package.json",
            20,
        ));
        assert_eq!(v.named_files, vec!["package.json".to_string()]);
        assert!(v.named_languages.is_empty());
    }

    fn write_fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let mut listed = Vec::new();
        for (path, content) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, content).unwrap();
            listed.push(path.to_string());
        }
        (dir, listed)
    }

    fn assess_in_repo(
        objective: &str,
        root: &std::path::Path,
        files: &[String],
    ) -> PlanningComplexityVerdict {
        let mut identifiers: Vec<String> =
            crate::agents::platform_extensions::orchestrator::text_code_identifiers(objective)
                .into_iter()
                .collect();
        identifiers.sort();
        let references = scan_identifier_references(root, files, &identifiers);
        assess_planning_complexity(&PlanningFacts {
            objective,
            repo_file_count: files.len(),
            repo_files: files,
            identifier_references: &references,
            history: PriorPlanHistory::default(),
        })
    }

    /// r64 s5: the fixture prompt names no file path and the repo is tiny, so
    /// the lexical signals read ROUTINE. The grounded signature signal reads
    /// the repo: `quoteShipping` is defined in one file and called from four.
    #[test]
    fn r64_s5_signature_refactor_prompt_is_complex_against_its_fixture_repo() {
        let (dir, files) = write_fixture(R64_S5_FIXTURE_FILES);
        let v = assess_in_repo(R64_S5_TASK_PROMPT, dir.path(), &files);
        assert_eq!(v.complexity, PlanningComplexity::Complex, "{:?}", v.reasons);
        assert!(v.named_files.is_empty(), "{:?}", v.named_files);
        assert!(v.parameter_change_requested);
        assert_eq!(v.signature_changes, vec!["quoteShipping".to_string()]);
        let quote = v
            .identifiers
            .iter()
            .find(|i| i.identifier == "quoteShipping")
            .expect("quoteShipping is resolved");
        assert_eq!(quote.defined_in, vec!["src/shipping/quote.ts".to_string()]);
        assert_eq!(quote.referencing_files, 5);
        assert!(
            v.reasons.iter().any(|r| r.contains("fans out")),
            "{:?}",
            v.reasons
        );
        // Without the repo facts the same prompt is ROUTINE: the promotion is
        // the repo's, not the prose's.
        let ungrounded = assess_planning_complexity(&facts(R64_S5_TASK_PROMPT, files.len()));
        assert_eq!(ungrounded.complexity, PlanningComplexity::Routine);
    }

    #[test]
    fn a_one_file_typo_fix_stays_routine_in_a_grounded_repo() {
        let (dir, files) = write_fixture(R64_S5_FIXTURE_FILES);
        let v = assess_in_repo(
            "There is a typo in `README.md`: it says \"recieve\" where it should say \
             \"receive\". Fix it directly.",
            dir.path(),
            &files,
        );
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        assert_eq!(v.named_files, vec!["readme.md".to_string()]);
        assert!(v.signature_changes.is_empty());
    }

    /// Named paths are claims until the repo shows them: a glob or a file that
    /// does not exist never counts toward the multi-file line, and named test
    /// files do not count either (a fix plus its test is still routine).
    #[test]
    fn named_paths_count_only_when_they_resolve_and_are_not_tests() {
        let (dir, files) = write_fixture(&[
            ("src/pricing.py", "def price():\n    return 1\n"),
            ("src/inventory.py", "def stock():\n    return 1\n"),
            ("tests/test_checkout.py", "def test_it():\n    pass\n"),
        ]);
        let v = assess_in_repo(
            "Fix src/pricing.py and src/inventory.py so tests/test_checkout.py passes; also \
             touch src/ghost.py and src/**/*.py",
            dir.path(),
            &files,
        );
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
        assert_eq!(v.named_files.len(), 2, "{:?}", v.named_files);
        assert_eq!(
            v.named_test_files,
            vec!["tests/test_checkout.py".to_string()]
        );
        assert!(v
            .unresolved_named_files
            .contains(&"src/ghost.py".to_string()));
    }

    /// A widely-called identifier is not a signature change unless the
    /// objective asks for a parameter change, and "do not change the
    /// signature" is not asking for one.
    #[test]
    fn fan_out_without_a_parameter_change_stays_routine() {
        let (dir, files) = write_fixture(R64_S5_FIXTURE_FILES);
        for objective in [
            "Fix the rounding bug in quoteShipping so it rounds half up.",
            "Fix the rounding bug in quoteShipping; do not change its signature.",
        ] {
            let v = assess_in_repo(objective, dir.path(), &files);
            assert_eq!(
                v.complexity,
                PlanningComplexity::Routine,
                "{objective}: {:?}",
                v.reasons
            );
            assert!(!v.parameter_change_requested, "{objective}");
            assert_eq!(v.identifier_reference_files, 5, "{objective}");
        }
    }

    /// A parameter change to an identifier with fewer than three caller files
    /// is routine; difficulty prose next to it changes nothing.
    #[test]
    fn a_parameter_change_with_a_small_fan_out_is_routine_even_with_difficulty_prose() {
        let (dir, files) = write_fixture(&[
            ("a.py", "def old_name(x):\n    return x\n"),
            ("b.py", "from a import old_name\n\nprint(old_name(1))\n"),
        ]);
        let v = assess_in_repo(
            "Add a `scale` parameter to old_name and update its caller. This is an EXTREMELY \
             complex architectural change — use the best model.",
            dir.path(),
            &files,
        );
        assert!(v.parameter_change_requested);
        assert_eq!(v.complexity, PlanningComplexity::Routine, "{:?}", v.reasons);
    }

    #[test]
    fn parameter_change_wording_is_read_per_sentence_and_respects_negation() {
        assert!(requests_parameter_change(
            "Add a `currency` parameter to apply_discount."
        ));
        assert!(requests_parameter_change(
            "Keep the calling convention, with one more\nargument added for the type."
        ));
        assert!(requests_parameter_change(
            "Change the signature of parse_amount"
        ));
        assert!(!requests_parameter_change("Do not change the signature."));
        assert!(!requests_parameter_change(
            "Add logging. The argument parser is fine."
        ));
        assert!(!requests_parameter_change("Fix the typo in the README"));
    }

    #[test]
    fn identifier_scan_finds_definitions_and_whole_word_references_outside_tests() {
        let (dir, files) = write_fixture(&[
            ("src/lib.rs", "pub fn apply_discount(x: u32) -> u32 { x }\n"),
            (
                "src/a.rs",
                "let y = apply_discount(1);\nlet z = apply_discount_v2(1);\n",
            ),
            ("src/b.rs", "let apply_discounts = 3;\n"),
            ("src/lib_test.rs", "apply_discount(2);\n"),
            ("docs/notes.md", "apply_discount is documented here\n"),
        ]);
        let refs = scan_identifier_references(dir.path(), &files, &["apply_discount".to_string()]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].defined_in, vec!["src/lib.rs".to_string()]);
        assert_eq!(
            refs[0].referenced_in,
            vec!["src/lib.rs".to_string(), "src/a.rs".to_string()]
        );
        assert_eq!(refs[0].caller_files(), Some(1));
    }
}
