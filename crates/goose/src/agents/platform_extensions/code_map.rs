//! Shared code-map slicing: the term-matching + directory-ancestry logic for a
//! project's stored code map (`code:{project_id}:map`, written by
//! `POST /api/projects/{id}/index-code`).
//!
//! Two call sites consume the same map and MUST slice it the same way, so the
//! logic lives here instead of being duplicated:
//!
//! - the orchestrator's dispatch-time injection ([`format_code_map_block`]) —
//!   a goal-aware slice prepended to worker instructions; and
//! - the analyze extension's worker-callable `map_query` tool
//!   ([`render_map_query`]) — the on-demand form, added after the goals A/B
//!   measurement (2026-08-10) showed injected maps do not change navigation
//!   behaviour: workers grep anyway unless they can ASK for the slice they
//!   need at the moment they need it.
//!
//! The map is a 2-space-indented tree; directory lines end with `/`. A matched
//! line is only navigable together with its ancestor directories, so every
//! slice here carries ancestry ([`matching_lines_with_ancestors`]).

/// Character budget for the dispatch-time injected code map. Roughly a
/// thousand tokens — enough for the tree and top-level symbols of a mid-sized
/// repo, and far less than the multi-turn ls/grep exploration it replaces. A
/// map that costs more than the exploration would be the feature defeating
/// itself.
pub(crate) const CODE_MAP_INJECT_MAX_CHARS: usize = 4_000;

/// Character budget for a `map_query` answer. Half the injection budget: a
/// query names ONE term, so a result bigger than this means the term is too
/// broad — the honest response is a cut plus "narrow the term", not more map.
pub(crate) const MAP_QUERY_MAX_CHARS: usize = 2_000;

/// Words in a goal that say nothing about WHERE its work lives. Kept small:
/// over-filtering costs matches, and a stray weak term only adds a few lines.
const GOAL_TERM_STOPWORDS: &[&str] = &[
    "should", "every", "their", "there", "which", "record", "update", "create", "write", "make",
    "field", "using", "them", "this", "that", "with", "into", "from", "have", "when", "also",
];

/// Terms worth matching against map lines: lowercase alphanumeric-ish words of
/// 4+ chars, minus stopwords. Snake/kebab identifiers split into their parts
/// ("execution_receipt" matches via "execution" AND "receipt"), and each term
/// also matches its own singular ("receipts" finds `execution_receipt.rs`).
pub(crate) fn goal_terms(goal_text: &str) -> Vec<String> {
    let mut terms: Vec<String> = goal_text
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric()))
        .filter(|t| t.len() >= 4)
        .filter(|t| !GOAL_TERM_STOPWORDS.contains(t))
        .map(str::to_string)
        .collect();
    terms.sort_unstable();
    terms.dedup();
    terms
}

/// Does this map line mention any goal term (allowing for a trailing plural)?
pub(crate) fn line_matches(line_lower: &str, terms: &[String]) -> bool {
    terms.iter().any(|t| {
        line_lower.contains(t.as_str())
            || (t.len() > 4
                && t.strip_suffix('s')
                    .is_some_and(|stem| line_lower.contains(stem)))
    })
}

/// The ancestry walk both slices share: mark every map line matching `terms`,
/// PLUS each match's not-yet-marked ancestor directory lines — a path fragment
/// without its ancestry is not navigable. Returns the inclusion bitmap and the
/// characters the marked lines would occupy (newlines included).
///
/// `char_cap` bounds the walk: once the marked lines alone exceed it, scoring
/// stops — a caller with a budget learns "there is more than fits" without
/// paying to score a 100k-line map to the end.
pub(crate) fn matching_lines_with_ancestors(
    lines: &[&str],
    terms: &[String],
    char_cap: usize,
) -> (Vec<bool>, usize) {
    let mut included = vec![false; lines.len()];
    let mut relevant_chars = 0usize;
    if terms.is_empty() {
        return (included, 0);
    }
    // Stack of (indent_level, line_index) for the current directory chain.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let indent = line.len() - line.trim_start().len();
        while stack.last().is_some_and(|&(d, _)| d >= indent) {
            stack.pop();
        }
        let is_dir = line.trim_end().ends_with('/');
        if line_matches(&line.to_lowercase(), terms) {
            for &(_, ai) in &stack {
                if !included[ai] {
                    included[ai] = true;
                    relevant_chars += lines[ai].len() + 1;
                }
            }
            if !included[i] {
                included[i] = true;
                relevant_chars += line.len() + 1;
            }
        }
        if is_dir {
            stack.push((indent, i));
        }
        if relevant_chars > char_cap {
            break;
        }
    }
    (included, relevant_chars)
}

/// A snapshot disclaimer every slice carries: the index is not a live view —
/// a worker that trusts a stale entry over the filesystem edits files that
/// moved.
const SNAPSHOT_DISCLAIMER: &str = "\nThe map reflects the last index, not necessarily HEAD — \
                                   confirm a path exists before editing it.";

/// Pure: render a stored map as the dispatch-time instructions block, sliced
/// around the goal.
///
/// The first measurement (2026-08-10, goals A/B) showed why a flat top-slice is
/// inert: 4k chars of a 1,660-file tree never reaches the deep path the goal
/// actually concerns, so the worker greps exactly as if the map were absent.
/// The slice is therefore goal-aware: lines mentioning the goal's own terms are
/// included WITH their ancestor directories, and whatever budget remains
/// carries the plain top of the tree for orientation. A goal whose terms match
/// nothing falls back to the flat slice — a worse map than none is not on the
/// menu.
pub(crate) fn format_code_map_block(content: &str, goal_text: &str) -> Option<String> {
    let map = content.trim();
    if map.is_empty() {
        return None;
    }

    let terms = goal_terms(goal_text);
    let lines: Vec<&str> = map.lines().collect();
    let budget = CODE_MAP_INJECT_MAX_CHARS;

    // ── Relevant subtree: matching lines plus their unemitted ancestors ──
    let (included, relevant_chars) = matching_lines_with_ancestors(&lines, &terms, budget);

    // ── Compose: relevant subtree first (it is the goal's map), then fill the
    //    remaining budget with the top of the tree for orientation. ──
    let mut shown = String::new();
    if relevant_chars > 0 {
        shown.push_str("## Paths matching this goal\n");
        for (i, line) in lines.iter().enumerate() {
            if included[i] {
                if shown.len() + line.len() + 1 > budget {
                    break;
                }
                shown.push_str(line);
                shown.push('\n');
            }
        }
        shown.push_str("## Repository overview\n");
    }
    let mut truncated = false;
    for (i, line) in lines.iter().enumerate() {
        if included[i] {
            continue; // already shown in the subtree section
        }
        if shown.len() + line.len() + 1 > budget {
            truncated = true;
            break;
        }
        shown.push_str(line);
        shown.push('\n');
    }

    let mut block = format!(
        "Codebase map (pre-indexed — use this to navigate instead of exploring \
         with ls/grep; it lists the tree with per-file line counts and symbols):\n{}",
        shown
    );
    if truncated {
        block.push_str(
            "\n[map truncated — deeper paths exist; explore only below the directories shown]",
        );
    }
    block.push_str(SNAPSHOT_DISCLAIMER);
    Some(block)
}

/// Pure: the `map_query` tool's answer for a fetched (or absent) map.
///
/// `map` is the stored map content when the project has one, `None` when it
/// was never indexed (or the Brain is absent). Split from the Brain fetch and
/// project resolution so every branch — ancestry, plural, no-map, budget cut —
/// is unit-testable without a daemon.
pub(crate) fn render_map_query(
    map: Option<&str>,
    term: &str,
    project_label: &str,
    project_id: &str,
) -> String {
    let Some(map) = map.map(str::trim).filter(|m| !m.is_empty()) else {
        return format!(
            "No code map is indexed for project {project_label}. Index it first with \
             POST /api/projects/{project_id}/index-code, then query again."
        );
    };

    // A query term is already the caller's chosen needle: split and normalize
    // it like a goal, but never let normalization eat it entirely — a short
    // term ("db", "ios") still deserves a literal match.
    let mut terms = goal_terms(term);
    if terms.is_empty() {
        let raw = term.trim().to_lowercase();
        if raw.is_empty() {
            return "map_query needs a non-empty term to search for.".to_string();
        }
        terms = vec![raw];
    }

    let lines: Vec<&str> = map.lines().collect();
    let (included, relevant_chars) =
        matching_lines_with_ancestors(&lines, &terms, MAP_QUERY_MAX_CHARS);

    if relevant_chars == 0 {
        return format!(
            "No lines in project {project_label}'s code map match \"{term}\" \
             (case-insensitive, trailing-plural tolerant). Try a different term, or run \
             analyze on a directory for a live view — the map is a snapshot of the last index."
        );
    }

    let mut shown = String::new();
    let mut cut = relevant_chars > MAP_QUERY_MAX_CHARS;
    for (i, line) in lines.iter().enumerate() {
        if !included[i] {
            continue;
        }
        // Cut at a line boundary so the slice never ends mid-entry, which
        // reads as a file that exists with a mangled name.
        if shown.len() + line.len() + 1 > MAP_QUERY_MAX_CHARS {
            cut = true;
            break;
        }
        shown.push_str(line);
        shown.push('\n');
    }

    let mut answer = format!(
        "Code map lines in project {project_label} matching \"{term}\" \
         (each with its ancestor directories):\n{shown}"
    );
    if cut {
        answer.push_str(
            "[output cut at the character budget on a line boundary — more matches exist; \
             narrow the term]",
        );
    }
    answer.push_str(SNAPSHOT_DISCLAIMER);
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A matched deep path must arrive WITH its ancestor directory lines — an
    /// orphan basename is not navigable — and unrelated siblings must not.
    #[test]
    fn map_query_match_carries_its_ancestry() {
        let map =
            "crates/\n  goose/\n    src/\n      agents/\n        execution_receipt.rs [200L, 8F]\n\
                   other/\n  unrelated.rs [5L, 1F]\n";
        let out = render_map_query(Some(map), "execution receipt", "\"P\" (slug: p)", "pid-1");
        assert!(out.contains("execution_receipt.rs"));
        for anc in ["crates/", "  goose/", "    src/", "      agents/"] {
            assert!(out.contains(anc), "missing ancestor {anc:?}");
        }
        assert!(!out.contains("unrelated.rs"), "non-matching sibling leaked");
        assert!(!out.contains("other/"), "non-ancestor directory leaked");
    }

    /// "receipts" (plural) must find `execution_receipt.rs` (singular).
    #[test]
    fn map_query_trailing_plural_matches_singular_lines() {
        let map = "src/\n  execution_receipt.rs [200L, 8F]\n  main.rs [10L, 1F]\n";
        let out = render_map_query(Some(map), "receipts", "\"P\" (slug: p)", "pid-1");
        assert!(out.contains("execution_receipt.rs"));
        assert!(!out.contains("main.rs"));
        // The mechanism: the plural term itself survives normalization.
        assert!(goal_terms("execution receipts").contains(&"receipts".to_string()));
    }

    /// An unindexed project must say exactly that and point at the index
    /// endpoint — not return an empty result that reads as "no matches".
    #[test]
    fn map_query_without_an_indexed_map_says_so_and_suggests_indexing() {
        let out = render_map_query(None, "receipts", "\"P\" (slug: p)", "pid-1");
        assert!(out.contains("No code map is indexed"));
        assert!(out.contains("POST /api/projects/pid-1/index-code"));
        // Whitespace-only content is the same case as absent.
        let blank = render_map_query(Some("   \n  "), "receipts", "\"P\" (slug: p)", "pid-1");
        assert!(blank.contains("No code map is indexed"));
    }

    /// A term matching nothing is a distinct, honest answer.
    #[test]
    fn map_query_with_no_matching_lines_says_so() {
        let map = "src/\n  main.rs [10L, 1F]\n";
        let out = render_map_query(Some(map), "zzzz qqqq", "\"P\" (slug: p)", "pid-1");
        assert!(out.contains("match"), "must speak about matching");
        assert!(out.contains("No lines"));
        assert!(!out.contains("main.rs"));
    }

    /// An over-broad term is cut at a LINE boundary within budget, and the
    /// answer says so — a mid-line cut reads as a mangled filename.
    #[test]
    fn map_query_output_is_budget_cut_on_line_boundaries() {
        let line_len = "widget0000.rs [10L, 1F]".len();
        let big: String = (0..500)
            .map(|i| format!("widget{i:04}.rs [10L, 1F]\n"))
            .collect();
        let out = render_map_query(Some(&big), "widget", "\"P\" (slug: p)", "pid-1");
        assert!(out.contains("[output cut"), "cut must be announced");
        assert!(
            out.chars().count() < MAP_QUERY_MAX_CHARS + 400,
            "answer must stay near its budget, got {} chars",
            out.chars().count()
        );
        for line in out.lines().filter(|l| l.starts_with("widget")) {
            assert_eq!(line.len(), line_len, "cut mid-line: {line:?}");
        }
    }

    /// Injection slice: surface the DEEP path a goal names, with its ancestry,
    /// ahead of the generic tree top (ported from orchestrator.rs with the
    /// extraction).
    #[test]
    fn goal_aware_slice_surfaces_the_deep_path_with_its_ancestry() {
        // A tree where the goal-relevant file sits far past any flat 4k cut.
        let mut map = String::from("9999 files, 1L, 1F, 0C (unlimited)\n\n");
        for i in 0..400 {
            map.push_str(&format!(
                "padding{:03}/\n  filler{:03}.rs [10L, 1F]\n",
                i, i
            ));
        }
        map.push_str(
            "crates/\n  goose/\n    src/\n      agents/\n        execution_receipt.rs [200L, 8F]\n",
        );

        let block = format_code_map_block(
            &map,
            "Record the dispatching machine's hostname on execution receipts",
        )
        .unwrap();

        assert!(
            block.contains("execution_receipt.rs"),
            "the goal's deep path must be in the slice"
        );
        // Ancestry, not a bare basename: an orphan filename is not navigable.
        for anc in ["crates/", "  goose/", "    src/", "      agents/"] {
            assert!(block.contains(anc), "missing ancestor {anc:?}");
        }
        assert!(
            block.contains("## Paths matching this goal"),
            "the subtree section must be labeled"
        );
        assert!(
            block.chars().count() < CODE_MAP_INJECT_MAX_CHARS + 600,
            "budget still holds: {}",
            block.chars().count()
        );
    }

    /// A goal whose words match nothing must degrade to the flat slice — a map
    /// with an empty "matching paths" section is worse than no framing at all.
    #[test]
    fn unmatched_goal_falls_back_to_the_flat_slice() {
        let map = "5 files, 1L, 1F, 0C\n\nsrc/\n  main.rs [10L, 1F]\n";
        let block = format_code_map_block(map, "zzz qqqq wwww").unwrap();
        assert!(!block.contains("## Paths matching this goal"));
        assert!(block.contains("main.rs"));
    }

    /// The injected block stays navigable, bounded, and honest about being a
    /// snapshot (ported from orchestrator.rs with the extraction).
    #[test]
    fn code_map_block_is_bounded_and_cut_on_line_boundaries() {
        // Small map: injected whole, with the navigation and staleness framing.
        let small =
            format_code_map_block("src/\n  main.rs (120 loc, 3 fn)", "irrelevant goal").unwrap();
        assert!(small.contains("main.rs"));
        assert!(small.contains("instead of exploring"));
        assert!(small.contains("confirm a path exists"));
        assert!(!small.contains("[map truncated"));

        // Oversized map: cut at a LINE boundary within budget, and says so.
        let long_line = format!("dir{:04}/file.rs (10 loc)", 0);
        let big: String = (0..2000)
            .map(|i| format!("dir{:04}/file.rs (10 loc)\n", i))
            .collect();
        let block = format_code_map_block(&big, "no matching words").unwrap();
        assert!(block.contains("[map truncated"));
        assert!(
            block.chars().count() < CODE_MAP_INJECT_MAX_CHARS + 400,
            "the injected map must stay near its budget, got {} chars",
            block.chars().count()
        );
        // Every map line that survived is complete.
        for line in block.lines().filter(|l| l.starts_with("dir")) {
            assert_eq!(line.len(), long_line.len(), "cut mid-line: {line:?}");
        }

        // Empty map: no block at all rather than framing around nothing.
        assert!(format_code_map_block("   ", "anything").is_none());
    }
}
