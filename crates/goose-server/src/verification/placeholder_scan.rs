//! Standing placeholder scan over the lines a goal actually added.
//!
//! `orchestrator::try_grep_absent` already knows the marker vocabulary
//! (TODO/FIXME/`todo!()`/…), but it only ever fires when the goal's own text
//! both declares an absence *and* names a file — a pathless "no TODOs left" is
//! skipped, and a goal that never mentions placeholders is never checked at
//! all. So the one claim a harness most needs to make — "nothing was stubbed
//! out to make this look finished" — was never made.
//!
//! This module makes it standing: every code goal is scanned, whether or not it
//! asked to be.
//!
//! **Added lines only.** The scan reads `git diff -U0` over the same range the
//! verifier already diffed and looks at `+` lines. Scanning whole changed files
//! would fail a goal for a pre-existing marker in a file it merely touched —
//! manufacturing false-fails, which is the one thing the verification ruling
//! forbids. A goal is answerable for what it wrote, and only that.
//!
//! **Tracked markers are not placeholders.** `TODO(#123)` / `FIXME(jesse)`
//! names who or what will close it; it is a filed item, not a stub. Only the
//! bare form counts.
//!
//! **Source files only.** Prose files carry TODO lists legitimately.

use std::path::Path;
use std::sync::LazyLock;

use super::checks::{CheckEvidence, CheckResult, CheckStatus};

/// `check_type` for the synthesized result row.
pub const CHECK_TYPE: &str = "placeholder_scan";

/// Human-readable summary carried on the result (there is no declared
/// `CompletionCheck` behind this row).
pub const SUMMARY: &str = "no placeholder markers in the lines this goal added";

/// Cap on reported hits (the rest are counted, not listed).
const MAX_HITS: usize = 40;
/// Cap on a single reported line.
const MAX_HIT_LINE_CHARS: usize = 300;
/// Refuse to scan an implausibly large diff rather than buffer it all.
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;

/// Extensions the scan applies to. Prose, config and data files are excluded on
/// purpose: a TODO list in `README.md` or a `"TODO"` string in a fixture JSON is
/// not a stubbed implementation.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts", "swift", "rb",
    "c", "h", "cc", "cpp", "hh", "hpp", "cs", "php", "sh", "bash", "zsh", "sql", "scala", "m",
    "mm", "dart", "lua", "vue", "svelte", "ex", "exs", "erl", "hs", "zig",
];

/// Bare tracker markers. Counted only when NOT followed by `(` — see the
/// module docs on tracked markers.
const TRACKER_MARKERS: &[&str] = &["TODO", "FIXME", "XXX", "HACK"];

/// Unambiguous stub bodies: constructs whose entire purpose is to stand in for
/// an implementation that was not written.
static STUB_PATTERNS: LazyLock<Vec<(&'static str, regex::Regex)>> = LazyLock::new(|| {
    vec![
        ("todo!()", regex::Regex::new(r"\btodo!\s*[\(\[\{]").unwrap()),
        (
            "unimplemented!()",
            regex::Regex::new(r"\bunimplemented!\s*[\(\[\{]").unwrap(),
        ),
        (
            "NotImplementedError",
            regex::Regex::new(r"\bNotImplementedError\b").unwrap(),
        ),
        (
            "not-implemented throw",
            regex::Regex::new(
                r#"(?i)throw\s+new\s+\w*Error\s*\(\s*["'`][^"'`]*not[ _-]?implemented"#,
            )
            .unwrap(),
        ),
        (
            "not-implemented panic",
            regex::Regex::new(r#"(?i)panic!\s*\(\s*"[^"]*(not implemented|todo)"#).unwrap(),
        ),
    ]
});

/// Prose a model leaves behind when it elides work it did not do.
const LAZINESS_PHRASES: &[&str] = &[
    "rest of the code unchanged",
    "rest of the file unchanged",
    "rest of the function unchanged",
    "rest unchanged",
    "remainder unchanged",
    "unchanged from above",
    "... existing code",
    "existing code ...",
    "existing code here",
    "implementation omitted",
    "implementation left out",
    "your code here",
    "code goes here",
    "fill this in",
    "fill in later",
    "left as an exercise",
];

/// One placeholder found on one added line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderHit {
    pub path: String,
    pub marker: String,
    pub line: String,
}

impl PlaceholderHit {
    fn render(&self) -> String {
        let line: String = self.line.chars().take(MAX_HIT_LINE_CHARS).collect();
        format!("{}: {} — {}", self.path, self.marker, line.trim())
    }
}

/// True when `path`'s extension is one the scan applies to.
pub fn is_source_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => SOURCE_EXTENSIONS
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext)),
        _ => false,
    }
}

/// Markers present on one added line of source.
pub fn markers_in_line(line: &str) -> Vec<String> {
    let mut hits: Vec<String> = Vec::new();

    for (name, re) in STUB_PATTERNS.iter() {
        if re.is_match(line) && !hits.iter().any(|h| h == name) {
            hits.push((*name).to_string());
        }
    }

    for marker in TRACKER_MARKERS {
        if has_bare_marker(line, marker) && !hits.iter().any(|h| h == marker) {
            hits.push((*marker).to_string());
        }
    }

    let lower = line.to_ascii_lowercase();
    for phrase in LAZINESS_PHRASES {
        if lower.contains(phrase) && !hits.iter().any(|h| h == phrase) {
            hits.push((*phrase).to_string());
        }
    }

    hits
}

/// Whether `marker` appears as a whole word and is NOT immediately followed by
/// a `(`-delimited reference (`TODO(#123)`), which marks it as tracked.
fn has_bare_marker(line: &str, marker: &str) -> bool {
    let mut prev_end = 0usize;
    for (idx, _) in line.match_indices(marker) {
        // Left word boundary.
        let before_ok = line
            .get(prev_end..idx)
            .and_then(|s| s.chars().next_back())
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        prev_end = idx;
        if !before_ok {
            continue;
        }
        let rest = line.get(idx + marker.len()..).unwrap_or("");
        let mut chars = rest.chars();
        match chars.next() {
            // Right word boundary: `TODOS` is not a marker.
            Some(c) if c.is_alphanumeric() || c == '_' => continue,
            // Tracked: `TODO(#123)`, `TODO (jesse)`.
            Some('(') => continue,
            Some(c) if c.is_whitespace() => {
                if rest.trim_start().starts_with('(') {
                    continue;
                }
                return true;
            }
            _ => return true,
        }
    }
    false
}

/// Scan a unified diff for placeholders on added lines of source files.
///
/// Pure: takes the diff text, returns hits. `git diff -U0` output is expected
/// (context lines would otherwise be scanned as if the goal had written them —
/// it did not, and `-U0` is what the runner passes).
pub fn scan_unified_diff(diff: &str) -> Vec<PlaceholderHit> {
    let mut hits: Vec<PlaceholderHit> = Vec::new();
    let mut current: Option<String> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let path = rest
                .split('\t')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            current = match path {
                "/dev/null" | "" => None,
                p => {
                    let p = p.strip_prefix("b/").unwrap_or(p);
                    is_source_path(p).then(|| p.to_string())
                }
            };
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("diff --git") {
            continue;
        }
        let Some(path) = current.as_deref() else {
            continue;
        };
        // `+++` is already handled above, so any remaining `+` line is content.
        let Some(added) = line.strip_prefix('+') else {
            continue;
        };
        for marker in markers_in_line(added) {
            hits.push(PlaceholderHit {
                path: path.to_string(),
                marker,
                line: added.to_string(),
            });
        }
    }

    hits
}

/// Run the scan over the goal's own diff range and synthesize a result row.
///
/// `diff_range_args` is the exact range `analyze_diff` resolved (`[base, head]`
/// or `[baseline]`), so the scan covers precisely the range the verifier graded
/// and nothing else. Returns
/// `None` — scan not run, no verdict effect — when there is no usable range or
/// git itself fails: the range already diffed cleanly once, so a failure here
/// is a machine problem, and a machine problem must not be dressed up as a
/// finding against the work.
pub async fn run(
    working_dir: &Path,
    diff_range_args: &[String],
    check_index: usize,
) -> Option<CheckResult> {
    if diff_range_args.is_empty() {
        return None;
    }
    let started_at = chrono::Utc::now().to_rfc3339();
    let start = std::time::Instant::now();

    let mut args: Vec<String> = vec![
        "diff".to_string(),
        "-U0".to_string(),
        "--no-color".to_string(),
    ];
    args.extend(diff_range_args.iter().cloned());

    let output = tokio::process::Command::new("git")
        .args(&args)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await;

    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            tracing::warn!(
                target: "permagentd::verification",
                args = ?args,
                "placeholder scan: git exited {:?} ({}) — scan skipped, no verdict effect",
                o.status.code(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
            return None;
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::verification",
                args = ?args,
                "placeholder scan: could not run git ({e}) — scan skipped, no verdict effect"
            );
            return None;
        }
    };
    if stdout.len() > MAX_DIFF_BYTES {
        tracing::warn!(
            target: "permagentd::verification",
            bytes = stdout.len(),
            "placeholder scan: diff exceeds {MAX_DIFF_BYTES} bytes — scan skipped"
        );
        return None;
    }

    let hits = scan_unified_diff(&String::from_utf8_lossy(&stdout));
    let total = hits.len();
    let listed: Vec<String> = hits.iter().take(MAX_HITS).map(|h| h.render()).collect();

    let message = if total == 0 {
        "no placeholder markers on any line this goal added".to_string()
    } else {
        format!(
            "{} placeholder marker(s) on lines this goal added — a goal cannot be \
             verified complete while its own diff still carries stubs. Tracked markers \
             (`TODO(#123)`) and prose files are exempt; these are not.",
            total
        )
    };

    Some(CheckResult {
        check_index,
        check_type: CHECK_TYPE.to_string(),
        status: if total == 0 {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        started_at,
        duration_ms: start.elapsed().as_millis() as u64,
        evidence: CheckEvidence {
            matches: Some(listed.clone()),
            message: Some(message),
            ..Default::default()
        },
        truncated: total > listed.len(),
        summary: Some(SUMMARY.to_string()),
        lint: None,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str, added: &[&str]) -> String {
        let mut s =
            format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,0 +1,1 @@\n");
        for a in added {
            s.push('+');
            s.push_str(a);
            s.push('\n');
        }
        s
    }

    #[test]
    fn added_todo_bang_is_a_hit() {
        let hits = scan_unified_diff(&diff("src/lib.rs", &["    todo!(\"wire this up\")"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].marker, "todo!()");
        assert_eq!(hits[0].path, "src/lib.rs");
    }

    #[test]
    fn every_marker_family_is_detected() {
        let cases: &[(&str, &str)] = &[
            ("unimplemented!()", "    unimplemented!()"),
            ("NotImplementedError", "    raise NotImplementedError"),
            ("TODO", "// TODO: come back to this"),
            ("FIXME", "// FIXME this is wrong"),
            ("XXX", "# XXX"),
            ("HACK", "// HACK"),
            (
                "not-implemented throw",
                "  throw new Error('not implemented');",
            ),
            (
                "not-implemented panic",
                r#"    panic!("not implemented yet")"#,
            ),
            (
                "rest of the code unchanged",
                "// ... rest of the code unchanged",
            ),
            ("... existing code", "// ... existing code ..."),
            ("your code here", "# your code here"),
        ];
        for (marker, line) in cases {
            let hits = scan_unified_diff(&diff("src/lib.rs", &[line]));
            assert!(
                hits.iter().any(|h| h.marker == *marker),
                "expected marker {marker:?} from line {line:?}, got {hits:?}"
            );
        }
    }

    /// The doc's stated negative: a tracked TODO in a prose file is not a
    /// placeholder — neither the tracking nor the file type alone is relied on.
    #[test]
    fn tracked_todo_in_a_doc_file_is_not_a_hit() {
        assert!(scan_unified_diff(&diff("docs/plan.md", &["- TODO(#123): follow up"])).is_empty());
        assert!(scan_unified_diff(&diff("docs/plan.md", &["- TODO: follow up"])).is_empty());
        assert!(
            scan_unified_diff(&diff("src/lib.rs", &["// TODO(#123): follow up"])).is_empty(),
            "a tracked marker in source names who closes it — not a stub"
        );
        assert!(scan_unified_diff(&diff("src/lib.rs", &["// FIXME (jesse) later"])).is_empty());
    }

    #[test]
    fn removed_and_context_lines_are_never_hits() {
        let d = "diff --git a/src/lib.rs b/src/lib.rs\n\
                 --- a/src/lib.rs\n\
                 +++ b/src/lib.rs\n\
                 @@ -1,2 +1,1 @@\n\
                 -    todo!()\n\
                 // TODO leftover context\n";
        assert!(
            scan_unified_diff(d).is_empty(),
            "{:?}",
            scan_unified_diff(d)
        );
    }

    #[test]
    fn deleted_file_header_clears_the_current_path() {
        let d = "diff --git a/src/gone.rs b/src/gone.rs\n--- a/src/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n+todo!()\n";
        assert!(scan_unified_diff(d).is_empty());
    }

    #[test]
    fn non_source_extensions_are_skipped() {
        for p in [
            "README.md",
            "fixtures/data.json",
            "config.yaml",
            "Cargo.toml",
            "notes.txt",
        ] {
            assert!(
                scan_unified_diff(&diff(p, &["  todo!() TODO FIXME"])).is_empty(),
                "{p} must be skipped"
            );
        }
        assert!(is_source_path("crates/a/src/b.rs"));
        assert!(is_source_path("ui/src/App.tsx"));
        assert!(!is_source_path("Makefile"));
        assert!(!is_source_path("README.md"));
    }

    #[test]
    fn word_boundaries_are_respected() {
        assert!(!has_bare_marker("let TODOS = 1;", "TODO"));
        assert!(!has_bare_marker("let myTODO = 1;", "TODO"));
        assert!(has_bare_marker("// TODO", "TODO"));
        assert!(has_bare_marker("// TODO: x", "TODO"));
        assert!(!has_bare_marker("// TODO(#1): x", "TODO"));
        assert!(!has_bare_marker("// TODO (#1): x", "TODO"));
    }

    #[test]
    fn hits_across_several_files_all_report() {
        let d = format!(
            "{}{}",
            diff("src/a.rs", &["    todo!()"]),
            diff("ui/src/b.ts", &["  // FIXME"])
        );
        let hits = scan_unified_diff(&d);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "src/a.rs");
        assert_eq!(hits[1].path, "ui/src/b.ts");
    }

    #[test]
    fn clean_diff_produces_no_hits() {
        let hits = scan_unified_diff(&diff(
            "src/lib.rs",
            &[
                "pub fn add(a: u32, b: u32) -> u32 {",
                "    a + b",
                "}",
                "// the todo list module is unrelated",
            ],
        ));
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[tokio::test]
    async fn run_returns_none_without_a_range() {
        assert!(run(Path::new("/"), &[], 0).await.is_none());
    }
}
