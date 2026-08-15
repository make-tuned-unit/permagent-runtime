use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

/// Matches shown per file before the remainder collapses into a "… more" note.
const DEFAULT_MAX_PER_FILE: usize = 5;
/// Files shown before the remainder collapses into the trailing summary line.
const DEFAULT_MAX_FILES: usize = 20;
/// Hard ceiling on a single rendered context line, after whitespace collapse.
const MAX_LINE_WIDTH: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Regular expression to search for (ripgrep syntax).
    pub pattern: String,
    /// Directory or file to search. Relative paths resolve against the working
    /// directory; defaults to the working directory.
    pub path: Option<String>,
    /// Only search files matching this glob, e.g. "*.rs" or "src/**/*.ts".
    pub glob: Option<String>,
    /// Restrict to a ripgrep file type, e.g. "rust", "py", "js".
    pub file_type: Option<String>,
    /// Max matches shown per file before the rest are summarized (default 5).
    pub max_per_file: Option<usize>,
    /// Max files shown before the rest are summarized (default 20).
    pub max_files: Option<usize>,
}

/// One ripgrep hit: the file it was found in, the 1-based line, and the raw line text.
#[derive(Debug, Clone, PartialEq)]
struct MatchRecord {
    file: String,
    line: u64,
    text: String,
}

struct SummaryOptions {
    max_per_file: usize,
    max_files: usize,
}

pub struct SearchTool;

impl SearchTool {
    pub fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub async fn search(&self, params: SearchParams) -> CallToolResult {
        self.search_with_cwd(params, None).await
    }

    pub async fn search_with_cwd(
        &self,
        params: SearchParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        if params.pattern.trim().is_empty() {
            return error_result("Search pattern cannot be empty.");
        }

        let opts = SummaryOptions {
            max_per_file: params.max_per_file.unwrap_or(DEFAULT_MAX_PER_FILE).max(1),
            max_files: params.max_files.unwrap_or(DEFAULT_MAX_FILES).max(1),
        };

        let base_dir = working_dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        // ripgrep is the engine: it respects .gitignore, resolves the regex, and
        // emits one JSON event per line so paths/line numbers parse unambiguously.
        let mut command = rg_command(&params, &base_dir, &["--json"], true, true);

        let output = match command.output().await {
            Ok(output) => output,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound {
                    return error_result(
                        "ripgrep (rg) was not found on PATH. Install ripgrep, or use \
                         the shell tool to search another way.",
                    );
                }
                return error_result(&format!("Failed to run ripgrep: {error}"));
            }
        };

        // ripgrep exit codes: 0 = matches found, 1 = no matches, 2 = error.
        //
        // Except one exit-2 is not an error. When a --glob or --type filter
        // excludes every candidate, ripgrep exits 2 with "No files were
        // searched" — the exact case this tool exists to explain, reported as a
        // failure. Treating it as one turns "your glob matched nothing, retry
        // without it" into "Search failed", which is the empty-vs-broken
        // confusion the zero-match message was written to prevent.
        //
        // Narrow on purpose: a bad regex also exits 2 and must stay an error.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let filtered_everything = stderr.contains("No files were searched");
        if output.status.code() == Some(2) && !filtered_everything {
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                "ripgrep reported an error (check the pattern or path)."
            } else {
                detail
            };
            return error_result(&format!(
                "Search failed: {detail}\n(searched from {})",
                base_dir.display()
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let records = parse_rg_json(&stdout);
        if records.is_empty() {
            let message = zero_match_message(&params, &base_dir).await;
            return CallToolResult::success(vec![Content::text(message).with_priority(0.0)]);
        }
        let summary = summarize_matches(&records, &opts, &params.pattern);
        CallToolResult::success(vec![Content::text(summary).with_priority(0.0)])
    }
}

/// Build a ripgrep invocation for the given search parameters. `extra_flags`
/// come first (e.g. `--json`, `--count-matches`, `-i`); `with_glob` /
/// `with_type` let the zero-match probes drop the caller's filters.
fn rg_command(
    params: &SearchParams,
    base_dir: &Path,
    extra_flags: &[&str],
    with_glob: bool,
    with_type: bool,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("rg");
    for flag in extra_flags {
        command.arg(flag);
    }
    command.arg("-e").arg(&params.pattern);
    if with_glob {
        if let Some(glob) = &params.glob {
            command.arg("--glob").arg(glob);
        }
    }
    if with_type {
        if let Some(file_type) = &params.file_type {
            command.arg("--type").arg(file_type);
        }
    }
    // End of flags, then the optional search target. Omitting the target
    // lets ripgrep default to the working directory with clean relative paths.
    command.arg("--");
    if let Some(path) = &params.path {
        command.arg(path);
    }
    command.current_dir(base_dir);
    command.stdin(Stdio::null());
    command
}

/// Run a `--count-matches` probe and sum the per-file counts. Any failure
/// (spawn error, rg error, unparseable output) counts as zero — probes only
/// ever add suggestions, never errors.
async fn probe_match_count(mut command: tokio::process::Command) -> usize {
    match command.output().await {
        Ok(output) if output.status.code() == Some(0) => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                // Lines are `path:count` (or a bare count for a single-file
                // target); the count is always the segment after the last colon.
                .filter_map(|line| line.rsplit(':').next()?.trim().parse::<usize>().ok())
                .sum()
        }
        _ => 0,
    }
}

/// Regex metacharacters whose presence suggests the pattern may have been
/// meant literally.
fn has_regex_metachars(pattern: &str) -> bool {
    pattern.chars().any(|c| r"\.+*?()|[]{}^$".contains(c))
}

/// A zero-match result with nothing else to say leaves the model guessing.
/// Probe a few concrete relaxations (case-insensitive, literal, unfiltered,
/// ignored files) and report which of them WOULD have matched, so the next
/// call can be a fix instead of a stab in the dark. Probes only run on the
/// zero-match path, so the extra subprocess cost is bounded and rare.
async fn zero_match_message(params: &SearchParams, base_dir: &Path) -> String {
    let scope = match &params.path {
        Some(path) => format!(" under `{path}`"),
        None => String::new(),
    };
    let mut message = format!("No matches for `{}`{}.", params.pattern, scope);

    let mut suggestions: Vec<String> = Vec::new();

    if params.pattern.chars().any(|c| c.is_alphabetic()) {
        let count = probe_match_count(rg_command(
            params,
            base_dir,
            &["--count-matches", "-i"],
            true,
            true,
        ))
        .await;
        if count > 0 {
            suggestions.push(format!(
                "Case-insensitive search finds {count} {} — retry with a `(?i)` prefix, e.g. \
                 `(?i){}`.",
                plural(count, "match", "matches"),
                params.pattern
            ));
        }
    }

    if has_regex_metachars(&params.pattern) {
        let count = probe_match_count(rg_command(
            params,
            base_dir,
            &["--count-matches", "--fixed-strings"],
            true,
            true,
        ))
        .await;
        if count > 0 {
            suggestions.push(format!(
                "The pattern contains regex metacharacters, but searching for it as a literal \
                 string finds {count} {} — escape the special characters (e.g. `\\(`, `\\.`).",
                plural(count, "match", "matches"),
            ));
        }
    }

    if params.glob.is_some() || params.file_type.is_some() {
        let count = probe_match_count(rg_command(
            params,
            base_dir,
            &["--count-matches"],
            false,
            false,
        ))
        .await;
        if count > 0 {
            let filters: Vec<String> = [
                params.glob.as_ref().map(|g| format!("glob=`{g}`")),
                params
                    .file_type
                    .as_ref()
                    .map(|t| format!("file_type=`{t}`")),
            ]
            .into_iter()
            .flatten()
            .collect();
            suggestions.push(format!(
                "{count} {} exist outside your {} filter — retry without it.",
                plural(count, "match", "matches"),
                filters.join(" and "),
            ));
        }
    }

    let count = probe_match_count(rg_command(
        params,
        base_dir,
        &["--count-matches", "--no-ignore", "--hidden"],
        true,
        true,
    ))
    .await;
    if count > 0 {
        suggestions.push(format!(
            "{count} {} exist in gitignored or hidden files — rerun via the shell tool with \
             `rg --no-ignore --hidden` if those are relevant.",
            plural(count, "match", "matches"),
        ));
    }

    if suggestions.is_empty() {
        message.push_str(
            "\nAutomatic fallbacks (case-insensitive, literal, unfiltered, ignored files) also \
             found nothing here. Try a shorter or broader pattern (e.g. one distinctive word), \
             or search from a higher-level directory.",
        );
    } else {
        message.push_str("\nSuggestions:");
        for suggestion in &suggestions {
            message.push_str(&format!("\n- {suggestion}"));
        }
    }
    message
}

impl Default for SearchTool {
    fn default() -> Self {
        Self::new()
    }
}

fn error_result(message: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.to_string()).with_priority(0.0)])
}

// --- ripgrep JSON parsing ---------------------------------------------------

#[derive(Deserialize)]
struct RgEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: RgData,
}

#[derive(Default, Deserialize)]
struct RgData {
    path: Option<RgText>,
    line_number: Option<u64>,
    lines: Option<RgText>,
}

#[derive(Deserialize)]
struct RgText {
    text: Option<String>,
}

/// Parse ripgrep's `--json` stream into flat match records, ignoring every
/// non-`match` event (begin/end/summary) and any line that fails to parse.
fn parse_rg_json(stdout: &str) -> Vec<MatchRecord> {
    let mut records = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let envelope: RgEnvelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(_) => continue,
        };
        if envelope.kind != "match" {
            continue;
        }
        let file = match envelope.data.path.and_then(|p| p.text) {
            Some(file) => file.strip_prefix("./").unwrap_or(&file).to_string(),
            None => continue,
        };
        let line_number = envelope.data.line_number.unwrap_or(0);
        let text = envelope.data.lines.and_then(|l| l.text).unwrap_or_default();
        records.push(MatchRecord {
            file,
            line: line_number,
            text,
        });
    }
    records
}

// --- summarization (pure) ---------------------------------------------------

/// Render match records as an LLM-friendly summary: grouped by file (first-seen
/// order), a per-file count and the top matches per file, capped across files,
/// with trailing notes for whatever was omitted. Pure over the record slice.
fn summarize_matches(records: &[MatchRecord], opts: &SummaryOptions, pattern: &str) -> String {
    if records.is_empty() {
        return format!("No matches for `{pattern}`.");
    }

    // Group by file while preserving the order files were first seen.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&MatchRecord>> = HashMap::new();
    for record in records {
        let entry = groups.entry(record.file.clone()).or_insert_with(|| {
            order.push(record.file.clone());
            Vec::new()
        });
        entry.push(record);
    }

    let total_matches = records.len();
    let total_files = order.len();

    let mut out = format!(
        "Found {} {} in {} {} for `{}`:\n",
        total_matches,
        plural(total_matches, "match", "matches"),
        total_files,
        plural(total_files, "file", "files"),
        pattern,
    );

    let shown_files = total_files.min(opts.max_files);
    for file in order.iter().take(shown_files) {
        let matches = &groups[file];
        out.push('\n');
        out.push_str(&format!(
            "{} ({} {}):\n",
            file,
            matches.len(),
            plural(matches.len(), "match", "matches"),
        ));

        let shown = matches.len().min(opts.max_per_file);
        for record in matches.iter().take(shown) {
            out.push_str(&format!(
                "  {}: {}\n",
                record.line,
                collapse_ws(&record.text)
            ));
        }
        if matches.len() > shown {
            let hidden = matches.len() - shown;
            out.push_str(&format!(
                "  ... and {} more {} in this file\n",
                hidden,
                plural(hidden, "match", "matches"),
            ));
        }
    }

    if total_files > shown_files {
        let hidden_files = total_files - shown_files;
        let hidden_matches: usize = order
            .iter()
            .skip(shown_files)
            .map(|file| groups[file].len())
            .sum();
        out.push_str(&format!(
            "\n... and {} more {} in {} more {}\n",
            hidden_matches,
            plural(hidden_matches, "match", "matches"),
            hidden_files,
            plural(hidden_files, "file", "files"),
        ));
    }

    out
}

/// Collapse all runs of whitespace to single spaces, trim, and cap the width so
/// a minified or very long line can't blow the token budget.
fn collapse_ws(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_LINE_WIDTH {
        let truncated: String = collapsed.chars().take(MAX_LINE_WIDTH).collect();
        format!("{truncated}…")
    } else {
        collapsed
    }
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        one
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;

    fn rec(file: &str, line: u64, text: &str) -> MatchRecord {
        MatchRecord {
            file: file.to_string(),
            line,
            text: text.to_string(),
        }
    }

    fn opts(max_per_file: usize, max_files: usize) -> SummaryOptions {
        SummaryOptions {
            max_per_file,
            max_files,
        }
    }

    fn extract_text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(text) => &text.text,
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn no_matches_returns_clean_message() {
        let summary = summarize_matches(&[], &opts(5, 20), "needle");
        assert_eq!(summary, "No matches for `needle`.");
    }

    #[test]
    fn single_file_groups_and_counts() {
        let records = vec![
            rec("src/lib.rs", 3, "let x = 1;"),
            rec("src/lib.rs", 7, "let y = 2;"),
        ];
        let summary = summarize_matches(&records, &opts(5, 20), "let");

        assert!(summary.starts_with("Found 2 matches in 1 file for `let`:\n"));
        assert!(summary.contains("src/lib.rs (2 matches):\n"));
        assert!(summary.contains("  3: let x = 1;\n"));
        assert!(summary.contains("  7: let y = 2;\n"));
        // Nothing was truncated, so no trailing summary.
        assert!(!summary.contains("more in"));
    }

    #[test]
    fn singular_labels_for_one_match_one_file() {
        let records = vec![rec("a.rs", 1, "solo")];
        let summary = summarize_matches(&records, &opts(5, 20), "solo");
        assert!(summary.starts_with("Found 1 match in 1 file for `solo`:\n"));
        assert!(summary.contains("a.rs (1 match):\n"));
    }

    #[test]
    fn per_file_truncation_reports_correct_remainder() {
        let records: Vec<MatchRecord> = (1..=8)
            .map(|i| rec("big.rs", i, &format!("hit {i}")))
            .collect();
        let summary = summarize_matches(&records, &opts(3, 20), "hit");

        assert!(summary.contains("big.rs (8 matches):\n"));
        // Only the first 3 lines are shown.
        assert!(summary.contains("  1: hit 1\n"));
        assert!(summary.contains("  3: hit 3\n"));
        assert!(!summary.contains("  4: hit 4\n"));
        // 8 - 3 = 5 remaining, pluralized.
        assert!(summary.contains("  ... and 5 more matches in this file\n"));
    }

    #[test]
    fn per_file_truncation_singular_remainder() {
        let records: Vec<MatchRecord> = (1..=6).map(|i| rec("f.rs", i, "x")).collect();
        let summary = summarize_matches(&records, &opts(5, 20), "x");
        assert!(summary.contains("  ... and 1 more match in this file\n"));
    }

    #[test]
    fn file_cap_appends_trailing_summary() {
        // 4 files, 2 matches each; show only 2 files.
        let mut records = Vec::new();
        for f in 0..4 {
            for line in 1..=2 {
                records.push(rec(&format!("file{f}.rs"), line, "z"));
            }
        }
        let summary = summarize_matches(&records, &opts(5, 2), "z");

        assert!(summary.starts_with("Found 8 matches in 4 files for `z`:\n"));
        assert!(summary.contains("file0.rs (2 matches):\n"));
        assert!(summary.contains("file1.rs (2 matches):\n"));
        // Files beyond the cap are not rendered as blocks.
        assert!(!summary.contains("file2.rs (2 matches):\n"));
        assert!(!summary.contains("file3.rs (2 matches):\n"));
        // 2 hidden files * 2 matches = 4 hidden matches.
        assert!(summary.contains("... and 4 more matches in 2 more files\n"));
    }

    #[test]
    fn trailing_summary_singular_file_and_match() {
        // 2 files; the hidden one has exactly one match.
        let records = vec![rec("shown.rs", 1, "a"), rec("hidden.rs", 9, "a")];
        let summary = summarize_matches(&records, &opts(5, 1), "a");
        assert!(summary.contains("... and 1 more match in 1 more file\n"));
    }

    #[test]
    fn grouping_preserves_first_seen_file_order() {
        let records = vec![
            rec("zeta.rs", 1, "a"),
            rec("alpha.rs", 1, "a"),
            rec("zeta.rs", 2, "a"),
        ];
        let summary = summarize_matches(&records, &opts(5, 20), "a");
        let zeta = summary.find("zeta.rs").unwrap();
        let alpha = summary.find("alpha.rs").unwrap();
        assert!(zeta < alpha, "zeta was seen first and should render first");
    }

    #[test]
    fn context_lines_collapse_whitespace() {
        let records = vec![rec("ws.rs", 1, "\t  let    a =\t 1;   ")];
        let summary = summarize_matches(&records, &opts(5, 20), "let");
        assert!(summary.contains("  1: let a = 1;\n"));
    }

    #[test]
    fn long_context_line_is_truncated() {
        let long = "x".repeat(MAX_LINE_WIDTH + 50);
        let collapsed = collapse_ws(&long);
        assert_eq!(collapsed.chars().count(), MAX_LINE_WIDTH + 1); // +1 for the ellipsis
        assert!(collapsed.ends_with('…'));
    }

    #[test]
    fn huge_result_stays_bounded() {
        // 100 files, 10 matches each = 1000 matches.
        let mut records = Vec::new();
        for f in 0..100 {
            for line in 1..=10 {
                records.push(rec(&format!("file{f:03}.rs"), line, "needle"));
            }
        }
        let summary = summarize_matches(&records, &opts(5, 20), "needle");

        // At most: header + 20 files * (header + 5 lines + 1 "more") + trailing.
        let rendered_files = summary.matches(".rs (10 matches):").count();
        assert_eq!(rendered_files, 20);
        // 80 hidden files * 10 = 800 hidden matches.
        assert!(summary.contains("... and 800 more matches in 80 more files\n"));
        // Bounded output despite 1000 matches.
        assert!(
            summary.len() < 4000,
            "summary should stay compact, was {} bytes",
            summary.len()
        );
    }

    #[test]
    fn parse_rg_json_extracts_matches_only() {
        // A realistic slice of ripgrep --json: begin, two matches, end, summary.
        let stdout = concat!(
            r#"{"type":"begin","data":{"path":{"text":"src/main.rs"}}}"#,
            "\n",
            r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"fn main() {\n"},"line_number":1,"absolute_offset":0,"submatches":[{"match":{"text":"main"},"start":3,"end":7}]}}"#,
            "\n",
            r#"{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"    main2();\n"},"line_number":2,"absolute_offset":13,"submatches":[]}}"#,
            "\n",
            r#"{"type":"end","data":{"path":{"text":"src/main.rs"},"stats":{}}}"#,
            "\n",
            r#"{"type":"summary","data":{"stats":{"matched_lines":2}}}"#,
            "\n",
        );

        let records = parse_rg_json(stdout);
        assert_eq!(
            records,
            vec![
                rec("src/main.rs", 1, "fn main() {\n"),
                rec("src/main.rs", 2, "    main2();\n"),
            ]
        );
    }

    #[test]
    fn parse_rg_json_strips_leading_dot_slash_and_ignores_garbage() {
        let stdout = concat!(
            "not json at all\n",
            r#"{"type":"match","data":{"path":{"text":"./a/b.rs"},"lines":{"text":"hit\n"},"line_number":5}}"#,
            "\n",
        );
        let records = parse_rg_json(stdout);
        assert_eq!(records, vec![rec("a/b.rs", 5, "hit\n")]);
    }

    #[tokio::test]
    async fn empty_pattern_is_rejected() {
        let tool = SearchTool::new();
        let result = tool
            .search(SearchParams {
                pattern: "   ".to_string(),
                path: None,
                glob: None,
                file_type: None,
                max_per_file: None,
                max_files: None,
            })
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(extract_text(&result).contains("cannot be empty"));
    }

    // Integration smoke test that actually shells out to ripgrep. Skipped when
    // rg is not installed so the pure-logic suite above remains the gate.
    #[tokio::test]
    async fn search_runs_ripgrep_when_available() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "fn alpha_two() {}\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "alpha".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(
            text.starts_with("Found 2 matches in 2 files"),
            "got: {text}"
        );
        assert!(text.contains("src/a.rs (1 match):"));
        assert!(text.contains("src/b.rs (1 match):"));
        assert!(text.contains("alpha"));
    }

    #[tokio::test]
    async fn search_reports_no_matches_cleanly_when_available() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "nothing here\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "zzzznotpresent".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(
            text.starts_with("No matches for `zzzznotpresent`."),
            "got: {text}"
        );
        // A total miss must still coach the next attempt, not dead-end.
        assert!(
            text.contains("broader pattern") || text.contains("Suggestions:"),
            "zero-match must carry recovery guidance, got: {text}"
        );
    }

    #[tokio::test]
    async fn zero_match_suggests_case_insensitive_retry() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn HandleRequest() {}\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "handlerequest".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(text.contains("Case-insensitive"), "got: {text}");
        assert!(text.contains("(?i)"), "got: {text}");
    }

    #[tokio::test]
    async fn zero_match_suggests_dropping_filters() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "unique_needle here\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "unique_needle".to_string(),
                    path: None,
                    glob: Some("*.rs".to_string()),
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(
            text.contains("outside your glob=`*.rs` filter"),
            "got: {text}"
        );
        assert!(text.contains("retry without it"), "got: {text}");
    }

    /// A genuinely broken pattern must still be an error, so the exemption
    /// above cannot swallow real ripgrep failures.
    ///
    /// Both cases exit 2; only the filtered-everything one is benign. Without
    /// this, "treat exit 2 as no-matches" would silently report an unparseable
    /// regex as a clean zero-result — trading one empty-vs-broken confusion for
    /// its mirror image.
    #[tokio::test]
    async fn an_invalid_regex_is_still_an_error() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "anything\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "(unclosed".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(true), "{}", extract_text(&result));
    }

    #[tokio::test]
    async fn zero_match_suggests_literal_search_for_metachar_patterns() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // "foo(bar)" as a regex means "foo" with a capture group — it does NOT
        // match the literal text "foo(bar)".
        std::fs::write(dir.path().join("a.txt"), "call site: zzqx(bar) done\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "zzqx(qux)".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        // The literal probe for "zzqx(qux)" also misses (file has "zzqx(bar)"),
        // so this lands on the generic guidance path.
        let text = extract_text(&result);
        assert!(text.starts_with("No matches"), "got: {text}");
        assert!(
            text.contains("broader pattern") || text.contains("Suggestions:"),
            "got: {text}"
        );
    }

    #[tokio::test]
    async fn zero_match_literal_probe_suggests_escaping() {
        if which::which("rg").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // As a regex, "zzqx(bar)" matches the text "zzqxbar" (capture group),
        // which is absent — but the literal string "zzqx(bar)" IS in the file.
        std::fs::write(dir.path().join("a.txt"), "value: zzqx(bar) done\n").unwrap();

        let tool = SearchTool::new();
        let result = tool
            .search_with_cwd(
                SearchParams {
                    pattern: "zzqx(bar)".to_string(),
                    path: None,
                    glob: None,
                    file_type: None,
                    max_per_file: None,
                    max_files: None,
                },
                Some(dir.path()),
            )
            .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(text.starts_with("No matches"), "got: {text}");
        assert!(text.contains("literal string finds 1 match"), "got: {text}");
        assert!(
            text.contains("escape the special characters"),
            "got: {text}"
        );
    }

    #[test]
    fn has_regex_metachars_classifies() {
        assert!(has_regex_metachars("foo(bar)"));
        assert!(has_regex_metachars("a.b"));
        assert!(has_regex_metachars("x{2}"));
        assert!(!has_regex_metachars("plain_word"));
        assert!(!has_regex_metachars("snake_case name"));
    }
}
