use std::fs;
use std::path::{Path, PathBuf};

use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::agents::platform_extensions::analyze::parser::{syntax_check, SyntaxStatus};

const NO_MATCH_PREVIEW_LINES: usize = 20;
/// Cap on how many differing lines the no-match diff renders.
const MAX_DIFF_LINES: usize = 20;

/// Default cap on an UNPAGED `file_read` (no `line`/`limit` given): return at
/// most this many lines. Mirrors the shell tool's 2000-line clamp — a single
/// unpaged read of a huge file (a lockfile, a bundle) would otherwise blow a
/// small local model's context in one shot. Explicit `line`/`limit` args page
/// through anything and bypass this cap.
const DEFAULT_READ_MAX_LINES: usize = 2000;
/// Companion byte cap for an unpaged read — binds first for a file that is few
/// lines but enormous (e.g. a minified one-line bundle).
const DEFAULT_READ_MAX_BYTES: usize = 100_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileReadParams {
    /// Absolute path to the file to read.
    pub path: String,
    /// Line number to start reading from (1-based).
    #[schemars(range(min = 1))]
    pub line: Option<u32>,
    /// Maximum number of lines to read.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileWriteParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileEditParams {
    pub path: String,
    pub before: String,
    pub after: String,
}

pub struct EditTools;

impl EditTools {
    pub fn new() -> Self {
        Self
    }

    pub fn file_read_with_cwd(
        &self,
        params: FileReadParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        let path = resolve_path(&params.path, working_dir);

        match fs::read_to_string(&path) {
            Ok(content) => {
                // Explicit line/limit paginates through anything; an unpaged read
                // is capped (with a paging notice) so one huge file can't flood
                // the model's context.
                let body = if params.line.is_none() && params.limit.is_none() {
                    cap_unpaged(&content)
                } else {
                    apply_line_limit(&content, params.line, params.limit)
                };
                CallToolResult::success(vec![Content::text(body).with_priority(0.0)])
            }
            Err(error) => CallToolResult::error(vec![Content::text(format!(
                "Failed to read {}: {}",
                params.path, error
            ))
            .with_priority(0.0)]),
        }
    }

    pub fn file_write(&self, params: FileWriteParams) -> CallToolResult {
        self.file_write_with_cwd(params, None)
    }

    pub fn file_write_with_cwd(
        &self,
        params: FileWriteParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        // An empty path resolves to the working directory itself, so the write
        // reached the filesystem and came back as "Is a directory (os error
        // 21)" -- an error about a directory the model never named, which sent
        // it looking for a path bug it did not have. Refuse here, and say which
        // argument was wrong.
        if params.path.trim().is_empty() {
            return CallToolResult::error(vec![Content::text(
                "The `path` argument was empty. Pass the file to write, relative \
                 to the working directory (for example `src/main.rs`).",
            )
            .with_priority(0.0)]);
        }
        let path = resolve_path(&params.path, working_dir);

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                if let Err(error) = fs::create_dir_all(parent) {
                    return CallToolResult::error(vec![Content::text(format!(
                        "Failed to create directory {}: {}",
                        parent.display(),
                        error
                    ))
                    .with_priority(0.0)]);
                }
            }
        }

        let is_new = !path.exists();
        let previous_len = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);

        match fs::write(&path, &params.content) {
            Ok(()) => {
                // Trust but verify: read the file back and compare, so a silent
                // filesystem failure (full disk, interfering watcher/sync tool)
                // surfaces here instead of as a confusing downstream error.
                match fs::read(&path) {
                    Ok(on_disk) if on_disk == params.content.as_bytes() => {
                        let line_count = params.content.lines().count();
                        let action = if is_new { "Created" } else { "Wrote" };
                        // Emptying a file that had content is a legal write, so
                        // this stays a success -- but it must not read like an
                        // ordinary one. A local run erased its own already-correct
                        // solution and got back "Wrote fizzbuzz.py (0 lines,
                        // verified on disk)", which says nothing about the work
                        // that just disappeared, and spent every remaining turn
                        // verifying an empty file.
                        let erased = previous_len > 0 && params.content.is_empty();
                        let body = if erased {
                            format!(
                                "Wrote {} -- but the content was EMPTY, so this \
                                 replaced {} bytes with an empty file and that work is \
                                 now gone. If you meant to change the file, read it and \
                                 write the full intended content; if you meant to clear \
                                 it, continue.",
                                params.path, previous_len
                            )
                        } else {
                            format!(
                                "{} {} ({} lines, verified on disk)",
                                action, params.path, line_count
                            )
                        };
                        CallToolResult::success(vec![Content::text(body).with_priority(0.0)])
                    }
                    Ok(on_disk) => CallToolResult::error(vec![Content::text(format!(
                        "Write verification failed for {}: the write reported success but the \
                         file on disk does not match the intended content (intended {} bytes, \
                         found {} bytes). Something may be modifying the file concurrently — \
                         read the file to see its current state, then retry the write.",
                        params.path,
                        params.content.len(),
                        on_disk.len()
                    ))
                    .with_priority(0.0)]),
                    Err(error) => CallToolResult::error(vec![Content::text(format!(
                        "Wrote {} but could not read it back to verify: {}. Treat the write as \
                         unverified — read the file to confirm its content before relying on it.",
                        params.path, error
                    ))
                    .with_priority(0.0)]),
                }
            }
            Err(error) => CallToolResult::error(vec![Content::text(format!(
                "Failed to write {}: {}",
                params.path, error
            ))
            .with_priority(0.0)]),
        }
    }

    pub fn file_edit(&self, params: FileEditParams) -> CallToolResult {
        self.file_edit_with_cwd(params, None)
    }

    pub fn file_edit_with_cwd(
        &self,
        params: FileEditParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        let path = resolve_path(&params.path, working_dir);

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(error) => {
                return CallToolResult::error(vec![Content::text(format!(
                    "Failed to read {}: {}",
                    params.path, error
                ))
                .with_priority(0.0)]);
            }
        };

        let outcome = match flexible_replace(&content, &params.before, &params.after) {
            Ok(o) => o,
            Err(msg) => {
                // Recovery: if the search text is gone but the replacement is
                // already present, the edit was almost certainly applied in an
                // earlier (possibly retried) call — succeed as a no-op instead
                // of sending the model into a rewrite spiral.
                if msg.starts_with("No match found")
                    && edit_already_applied(&content, &params.after)
                {
                    return CallToolResult::success(vec![Content::text(format!(
                        "No changes needed: {} already contains the replacement text, so this \
                         edit appears to have been applied previously. The file was left \
                         unchanged.",
                        params.path
                    ))
                    .with_priority(0.0)]);
                }
                return CallToolResult::error(vec![Content::text(msg).with_priority(0.0)]);
            }
        };

        // Lint-on-edit guardrail: reject an edit that introduces a syntax error the
        // pre-edit file did not have. Only NET-NEW breakage blocks the write — edits
        // to already-broken files, and files in unsupported languages, pass through.
        if let (
            SyntaxStatus::Clean,
            SyntaxStatus::Error {
                line,
                column,
                snippet,
            },
        ) = (
            syntax_check(&path, &content),
            syntax_check(&path, &outcome.new_content),
        ) {
            return CallToolResult::error(vec![Content::text(format!(
                "Edit rejected: applying it would introduce a syntax error the file did not \
                 have before, so the file was left unchanged.\n\n\
                 New syntax error at line {line}, column {column}:\n    {snippet}\n\n\
                 Revise the replacement so the result parses, then try again."
            ))
            .with_priority(0.0)]);
        }

        match fs::write(&path, &outcome.new_content) {
            Ok(()) => {
                let old_lines = params.before.lines().count();
                let new_lines = params.after.lines().count();
                let note = match outcome.tier.label() {
                    Some(tier) => format!(" (matched via {tier} fuzzy match)"),
                    None => String::new(),
                };
                CallToolResult::success(vec![Content::text(format!(
                    "Edited {} ({} lines -> {} lines){}",
                    params.path, old_lines, new_lines, note
                ))
                .with_priority(0.0)])
            }
            Err(error) => CallToolResult::error(vec![Content::text(format!(
                "Failed to write {}: {}",
                params.path, error
            ))
            .with_priority(0.0)]),
        }
    }
}

impl Default for EditTools {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimum non-whitespace chars in `after` before its presence in the file
/// counts as evidence the edit already landed. Tiny fragments (`}`, `);`)
/// appear everywhere and would produce false "already applied" successes.
const ALREADY_APPLIED_MIN_CHARS: usize = 8;

/// Whether a failed edit looks like a retry of one that already landed: the
/// replacement text is already in the file byte-for-byte (a replayed edit
/// re-sends identical `after` text, so exact containment is the right test).
/// Deletions (empty `after`) and trivial fragments are excluded — for those,
/// containment is not evidence.
fn edit_already_applied(content: &str, after: &str) -> bool {
    after.chars().filter(|c| !c.is_whitespace()).count() >= ALREADY_APPLIED_MIN_CHARS
        && content.contains(after)
}

// ── Flexible (fuzzy) matcher ────────────────────────────────────────────

/// Which normalization tier produced a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTier {
    /// Byte-exact substring match (identical to the legacy behavior).
    Exact,
    /// Matched after normalizing internal/trailing whitespace; leading indentation preserved.
    WhitespaceNormalized,
    /// Matched after additionally ignoring leading indentation; replacement is re-indented.
    IndentationNormalized,
}

impl MatchTier {
    /// Human label for the fuzzy tiers; `None` for exact (so the success message
    /// stays byte-identical to the legacy behavior).
    fn label(self) -> Option<&'static str> {
        match self {
            MatchTier::Exact => None,
            MatchTier::WhitespaceNormalized => Some("whitespace-normalized"),
            MatchTier::IndentationNormalized => Some("indentation-normalized"),
        }
    }
}

/// A successfully computed edit: the new file content and which tier matched.
#[derive(Debug, Clone)]
pub struct EditOutcome {
    pub new_content: String,
    pub tier: MatchTier,
}

/// Replace the first UNIQUE occurrence of `before` with `after`, cascading through
/// increasingly permissive matchers and applying the FIRST tier that yields a
/// unique match:
///   1. exact byte-for-byte substring (never regresses the legacy behavior);
///   2. whitespace-normalized, leading indentation preserved (line-oriented);
///   3. leading-indentation-normalized, replacement re-indented (line-oriented).
///
/// Safety rules (err toward a clean error over a wrong write):
///   * multiple *exact* matches short-circuit with the classic "add more context"
///     error and never fall through to fuzzier tiers;
///   * an ambiguous fuzzy tier (more than one candidate region) never guesses — it
///     returns an actionable error instead of risking a wrong edit.
pub fn flexible_replace(content: &str, before: &str, after: &str) -> Result<EditOutcome, String> {
    if before.is_empty() {
        return Err(
            "The search text is empty. Provide the exact text you want to replace.".to_string(),
        );
    }

    // ── Tier 1: exact substring ─────────────────────────────────────
    let exact: Vec<_> = content.match_indices(before).collect();
    match exact.len() {
        1 => {
            return Ok(EditOutcome {
                new_content: content.replacen(before, after, 1),
                tier: MatchTier::Exact,
            });
        }
        0 => {} // fall through to the fuzzy tiers
        n => return Err(multiple_exact_error(content, &exact, n)),
    }

    let content_segments: Vec<&str> = content.split_inclusive('\n').collect();
    let content_lines: Vec<&str> = content_segments.iter().map(|&s| strip_eol(s)).collect();
    let before_lines: Vec<&str> = before.lines().collect();

    // ── Tier 2: whitespace-normalized, indentation preserved ────────
    match line_match(&content_lines, &before_lines, normalize_ws_keep_indent) {
        LineMatch::Unique(start) => {
            let new_content = splice_lines(
                &content_segments,
                &content_lines,
                &before_lines,
                start,
                after,
                false,
            );
            return Ok(EditOutcome {
                new_content,
                tier: MatchTier::WhitespaceNormalized,
            });
        }
        LineMatch::Ambiguous(starts) => {
            return Err(ambiguous_line_error(
                "whitespace-normalized",
                &content_lines,
                &starts,
                before_lines.len(),
            ));
        }
        LineMatch::None => {}
    }

    // ── Tier 3: leading-indentation-normalized ──────────────────────
    match line_match(&content_lines, &before_lines, normalize_ignore_indent) {
        LineMatch::Unique(start) => {
            let new_content = splice_lines(
                &content_segments,
                &content_lines,
                &before_lines,
                start,
                after,
                true,
            );
            return Ok(EditOutcome {
                new_content,
                tier: MatchTier::IndentationNormalized,
            });
        }
        LineMatch::Ambiguous(starts) => {
            return Err(ambiguous_line_error(
                "indentation-normalized",
                &content_lines,
                &starts,
                before_lines.len(),
            ));
        }
        LineMatch::None => {}
    }

    Err(no_match_error(content, &content_lines, &before_lines))
}

enum LineMatch {
    Unique(usize),
    Ambiguous(Vec<usize>),
    None,
}

/// Slide a window of `before_lines.len()` over `content_lines` and record every
/// start index whose window equals `before_lines` under `normalize`.
fn line_match(
    content_lines: &[&str],
    before_lines: &[&str],
    normalize: fn(&str) -> String,
) -> LineMatch {
    let n = before_lines.len();
    if n == 0 || n > content_lines.len() {
        return LineMatch::None;
    }
    let norm_before: Vec<String> = before_lines.iter().map(|&l| normalize(l)).collect();
    let mut starts = Vec::new();
    for start in 0..=(content_lines.len() - n) {
        if (0..n).all(|i| normalize(content_lines[start + i]) == norm_before[i]) {
            starts.push(start);
        }
    }
    match starts.len() {
        0 => LineMatch::None,
        1 => LineMatch::Unique(starts[0]),
        _ => LineMatch::Ambiguous(starts),
    }
}

/// Rebuild the file with the matched line window (`start .. start + before_lines.len()`)
/// replaced by `after`, preserving the file's original line endings. When `reindent`
/// is set (tier 3), `after` is rebased from the model's indentation onto the file's.
fn splice_lines(
    content_segments: &[&str],
    content_lines: &[&str],
    before_lines: &[&str],
    start: usize,
    after: &str,
    reindent: bool,
) -> String {
    let end = start + before_lines.len();
    let prefix: String = content_segments[..start].concat();
    let suffix: String = content_segments[end..].concat();

    let last_seg = content_segments[end - 1];
    let region_has_trailing_nl = last_seg.ends_with('\n');
    let eol = if last_seg.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let after_lines: Vec<String> = if reindent {
        reindent_after(after, before_lines, &content_lines[start..end])
    } else {
        after.lines().map(str::to_string).collect()
    };

    let mut block = after_lines.join(eol);
    if region_has_trailing_nl && !block.is_empty() {
        block.push_str(eol);
    }

    format!("{prefix}{block}{suffix}")
}

/// Rebase `after` from the model's base indentation (that of `before_lines`) onto the
/// file's actual base indentation (that of the matched region), preserving each line's
/// indentation *relative* to that base. Best-effort: lines that don't share the model's
/// base prefix (e.g. dedented closers) are left untouched.
fn reindent_after(after: &str, before_lines: &[&str], matched_lines: &[&str]) -> Vec<String> {
    let before_base = leading_ws(first_nonblank(before_lines));
    let file_base = leading_ws(first_nonblank(matched_lines));
    if before_base == file_base {
        return after.lines().map(str::to_string).collect();
    }
    after
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else if let Some(rest) = line.strip_prefix(before_base) {
                format!("{file_base}{rest}")
            } else {
                line.to_string()
            }
        })
        .collect()
}

fn first_nonblank<'a>(lines: &[&'a str]) -> &'a str {
    lines
        .iter()
        .copied()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

fn leading_ws(line: &str) -> &str {
    // `split_at` (not `&line[..n]`) to satisfy clippy::string_slice; `n` is a valid
    // char boundary because it is the byte length of the whitespace `trim_start` removed.
    line.split_at(line.len() - line.trim_start().len()).0
}

/// Strip a single trailing line ending (`\n`, `\r\n`) from a `split_inclusive('\n')` segment.
fn strip_eol(seg: &str) -> &str {
    let seg = seg.strip_suffix('\n').unwrap_or(seg);
    seg.strip_suffix('\r').unwrap_or(seg)
}

/// Collapse internal runs of whitespace to single spaces and trim trailing
/// whitespace, but preserve the exact leading indentation.
fn normalize_ws_keep_indent(line: &str) -> String {
    // `split_at` at the whitespace/rest boundary avoids clippy::string_slice; the
    // split point is the byte length of the leading whitespace (a valid boundary).
    let (indent, rest) = line.split_at(line.len() - line.trim_start().len());
    let body = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{indent}{body}")
}

/// Fully whitespace-insensitive: ignore leading indentation and collapse all
/// internal whitespace.
fn normalize_ignore_indent(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Error rendering ─────────────────────────────────────────────────────

fn multiple_exact_error(content: &str, matches: &[(usize, &str)], n: usize) -> String {
    let mut msg = format!(
        "Found {n} exact matches for the search text. Add more surrounding context so exactly one location matches:\n"
    );
    for (i, (pos, _)) in matches.iter().enumerate().take(2) {
        let line_num = count_lines_before(content, *pos);
        let context = get_line_context(content, line_num, 1);
        msg.push_str(&format!(
            "\nMatch {} (line {}):\n```\n{}\n```",
            i + 1,
            line_num,
            context
        ));
    }
    if n > 2 {
        msg.push_str(&format!("\n\n...and {} more", n - 2));
    }
    msg
}

fn ambiguous_line_error(
    tier: &str,
    content_lines: &[&str],
    starts: &[usize],
    len: usize,
) -> String {
    let mut msg = format!(
        "Found {} regions that match under {tier} matching (the exact text was not found). This \
         fuzzy match is ambiguous, so no edit was made — add more surrounding context so exactly \
         one region matches:\n",
        starts.len()
    );
    for (i, &start) in starts.iter().enumerate().take(3) {
        let end = (start + len).min(content_lines.len());
        msg.push_str(&format!(
            "\nMatch {} (line {}):\n```\n{}\n```",
            i + 1,
            start + 1,
            render_region(content_lines, start, end)
        ));
    }
    if starts.len() > 3 {
        msg.push_str(&format!("\n\n...and {} more", starts.len() - 3));
    }
    msg
}

fn no_match_error(content: &str, content_lines: &[&str], before_lines: &[&str]) -> String {
    let mut msg = String::from(
        "No match found for the search text.\nTried exact, whitespace-normalized, and \
         indentation-normalized matching; none produced a unique match.",
    );

    if let Some((start, len)) = closest_window(content_lines, before_lines) {
        let end = (start + len).min(content_lines.len());
        msg.push_str(&format!(
            "\n\nClosest region (lines {}-{}):\n```\n{}\n```",
            start + 1,
            end,
            render_region(content_lines, start, end)
        ));
        let diff = render_diff(before_lines, &content_lines[start..end]);
        if !diff.is_empty() {
            msg.push_str(&format!(
                "\n\nDifferences (- your search text, + file content):\n```\n{diff}\n```"
            ));
        }
    }

    let preview = build_file_preview(content, NO_MATCH_PREVIEW_LINES);
    msg.push_str(&format!("\n\nFile preview:\n```\n{preview}\n```"));
    msg
}

/// Find the content window (of `before_lines.len()` lines) most textually similar
/// to `before_lines`, so the failure message can point at the closest near-match.
/// Scored by summed per-line token overlap (not just full-line equality, so a
/// single differing token still surfaces the right region).
fn closest_window(content_lines: &[&str], before_lines: &[&str]) -> Option<(usize, usize)> {
    let n = before_lines.len();
    if n == 0 || content_lines.is_empty() {
        return None;
    }
    let window = n.min(content_lines.len());
    let mut best: Option<(usize, usize)> = None; // (start, score)
    for start in 0..=(content_lines.len() - window) {
        let score: usize = (0..window)
            .map(|i| line_similarity(before_lines[i], content_lines[start + i]))
            .sum();
        if score > 0 && best.is_none_or(|(_, b)| score > b) {
            best = Some((start, score));
        }
    }
    best.map(|(start, _)| (start, window))
}

/// Multiset token-overlap between two lines — a cheap textual similarity used only
/// to surface the closest near-match region in error messages.
fn line_similarity(a: &str, b: &str) -> usize {
    let mut a_tokens: Vec<&str> = a.split_whitespace().collect();
    let mut score = 0;
    for tok in b.split_whitespace() {
        if let Some(pos) = a_tokens.iter().position(|t| *t == tok) {
            a_tokens.remove(pos);
            score += 1;
        }
    }
    score
}

fn render_region(content_lines: &[&str], start: usize, end: usize) -> String {
    content_lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>4}: {}", start + i + 1, l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Aligned line diff: lines equal under full normalization are context; the rest
/// are shown as `-` (search text) / `+` (file), capped at `MAX_DIFF_LINES`.
fn render_diff(before_lines: &[&str], file_lines: &[&str]) -> String {
    let mut out = Vec::new();
    let max = before_lines.len().max(file_lines.len());
    let mut shown = 0;
    for i in 0..max {
        match (before_lines.get(i).copied(), file_lines.get(i).copied()) {
            (Some(b), Some(f)) => {
                if normalize_ignore_indent(b) == normalize_ignore_indent(f) {
                    out.push(format!("  {f}"));
                } else {
                    out.push(format!("- {b}"));
                    out.push(format!("+ {f}"));
                    shown += 1;
                }
            }
            (Some(b), None) => {
                out.push(format!("- {b}"));
                shown += 1;
            }
            (None, Some(f)) => {
                out.push(format!("+ {f}"));
                shown += 1;
            }
            (None, None) => {}
        }
        if shown >= MAX_DIFF_LINES {
            out.push("  ...".to_string());
            break;
        }
    }
    if shown == 0 {
        return String::new();
    }
    out.join("\n")
}

/// Cap an UNPAGED read (no `line`/`limit`) at `DEFAULT_READ_MAX_LINES` lines or
/// `DEFAULT_READ_MAX_BYTES` bytes, whichever binds first, appending a notice that
/// tells the model to page the rest with `line`/`limit`. A file already within
/// both limits is returned byte-for-byte with no notice, so small reads are
/// unchanged.
fn cap_unpaged(content: &str) -> String {
    let total_lines = content.split_inclusive('\n').count();
    if content.len() <= DEFAULT_READ_MAX_BYTES && total_lines <= DEFAULT_READ_MAX_LINES {
        return content.to_string();
    }

    // Accumulate whole lines until either cap would be exceeded.
    let mut body = String::new();
    let mut kept = 0usize;
    for seg in content.split_inclusive('\n') {
        if kept >= DEFAULT_READ_MAX_LINES || body.len() + seg.len() > DEFAULT_READ_MAX_BYTES {
            break;
        }
        body.push_str(seg);
        kept += 1;
    }

    // Degenerate case: the first line alone exceeds the byte cap (a minified file
    // on one line). Take whole chars up to the byte cap — boundary-safe by
    // construction (never a mid-char cut), avoiding a raw string slice
    // (`clippy::string_slice` is denied workspace-wide).
    if body.is_empty() {
        body = content
            .char_indices()
            .take_while(|(i, ch)| i + ch.len_utf8() <= DEFAULT_READ_MAX_BYTES)
            .map(|(_, ch)| ch)
            .collect();
    }

    let next = kept + 1;
    format!(
        "{body}\n\n[truncated: showing {shown} of {total_bytes} bytes / {kept} of {total_lines} \
         lines. This file is large — read the rest in pages with the `line` and `limit` \
         arguments, e.g. line={next}, limit={DEFAULT_READ_MAX_LINES}.]",
        shown = body.len(),
        total_bytes = content.len(),
    )
}

fn apply_line_limit(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let start = line
        .map(|l| (l as usize).saturating_sub(1))
        .unwrap_or(0)
        .min(lines.len());
    let end = limit
        .map(|l| start + l as usize)
        .unwrap_or(lines.len())
        .min(lines.len());
    lines[start..end].concat()
}

pub fn resolve_path(path: &str, working_dir: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        working_dir
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join(path)
    }
}

fn count_lines_before(content: &str, byte_pos: usize) -> usize {
    content
        .char_indices()
        .take_while(|(i, _)| *i < byte_pos)
        .filter(|(_, c)| *c == '\n')
        .count()
        + 1
}

fn get_line_context(content: &str, target_line: usize, context: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = target_line.saturating_sub(context + 1);
    let end = (target_line + context).min(lines.len());

    lines[start..end].join("\n")
}

fn build_file_preview(content: &str, max_lines: usize) -> String {
    if content.is_empty() {
        return "(file is empty)".to_string();
    }

    let lines: Vec<&str> = content.lines().collect();
    let preview_end = lines.len().min(max_lines);
    let mut preview = lines[..preview_end]
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{:>4}: {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    if lines.len() > preview_end {
        preview.push_str(&format!("\n... ({} more lines)", lines.len() - preview_end));
    }

    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;
    use std::fs;
    use tempfile::TempDir;
    use test_case::test_case;

    fn setup() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn extract_text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(text) => &text.text,
            _ => panic!("expected text"),
        }
    }

    #[test_case(None, None, "line1\nline2\nline3" ; "full content")]
    #[test_case(Some(2), None, "line2\nline3" ; "from line 2")]
    #[test_case(None, Some(2), "line1\nline2\n" ; "limit 2")]
    #[test_case(Some(2), Some(1), "line2\n" ; "line 2 limit 1")]
    #[test_case(Some(99), None, "" ; "beyond eof")]
    fn test_apply_line_limit(line: Option<u32>, limit: Option<u32>, expected: &str) {
        assert_eq!(
            apply_line_limit("line1\nline2\nline3", line, limit),
            expected
        );
    }

    #[test]
    fn test_file_read() {
        let dir = setup();
        let path = dir.path().join("read.txt");
        fs::write(&path, "line1\nline2\nline3").unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: None,
                limit: None,
            },
            None,
        );

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(extract_text(&result), "line1\nline2\nline3");
    }

    #[test]
    fn test_file_read_partial() {
        let dir = setup();
        let path = dir.path().join("read.txt");
        fs::write(&path, "line1\nline2\nline3").unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: Some(2),
                limit: Some(1),
            },
            None,
        );

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(extract_text(&result), "line2\n");
    }

    // ── F1.4: unpaged file_read is capped (regression guard) ─────────

    #[test]
    fn unpaged_read_of_a_big_file_is_clamped_with_a_paging_notice() {
        let dir = setup();
        let path = dir.path().join("big.txt");
        // Well beyond the 2000-line cap.
        let content: String = (0..5000).map(|i| format!("line{i}\n")).collect();
        fs::write(&path, &content).unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: None,
                limit: None,
            },
            None,
        );

        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        // The body is clamped to at most the line cap …
        assert!(
            text.lines().count() <= DEFAULT_READ_MAX_LINES + 4,
            "unpaged read must be clamped, got {} lines",
            text.lines().count()
        );
        // … and the model is told how to page the rest.
        assert!(
            text.contains("[truncated"),
            "a truncation notice must be present"
        );
        assert!(text.contains("line="), "the notice must show how to page");
        assert!(!text.contains("line4999"), "the tail must be dropped");
    }

    #[test]
    fn unpaged_read_is_clamped_by_bytes_for_a_few_huge_lines() {
        let dir = setup();
        let path = dir.path().join("huge_lines.txt");
        // Only ~60 lines, but each is 4 KB → ~240 KB, over the byte cap.
        let content: String = (0..60).map(|_| format!("{}\n", "x".repeat(4096))).collect();
        fs::write(&path, &content).unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: None,
                limit: None,
            },
            None,
        );

        let text = extract_text(&result);
        assert!(
            text.contains("[truncated"),
            "byte-capped read must carry a notice"
        );
        // Body (before the notice) must respect the byte cap.
        let body = text.split("\n\n[truncated").next().unwrap();
        assert!(
            body.len() <= DEFAULT_READ_MAX_BYTES,
            "byte cap must bind, body was {} bytes",
            body.len()
        );
    }

    #[test]
    fn unpaged_read_hard_caps_a_single_giant_line_at_a_char_boundary() {
        let dir = setup();
        let path = dir.path().join("minified.txt");
        // One line of 3-byte chars, no newline, well over the byte cap. The byte
        // cap (100_000) is NOT a char boundary for 3-byte chars, so a naive byte
        // slice would panic — the cap must land on a boundary, never mid-char.
        let content = "→".repeat(40_000); // 120_000 bytes, a single line
        fs::write(&path, &content).unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: None,
                limit: None,
            },
            None,
        );

        let text = extract_text(&result);
        assert!(
            text.contains("[truncated"),
            "a giant single line must be capped"
        );
        let body = text.split("\n\n[truncated").next().unwrap();
        assert!(
            body.len() <= DEFAULT_READ_MAX_BYTES,
            "byte cap must bind, body was {} bytes",
            body.len()
        );
        // Landed on a char boundary — the body is whole chars, nothing split.
        assert!(
            body.chars().all(|c| c == '→'),
            "the cap must not split a char"
        );
    }

    #[test]
    fn unpaged_read_of_a_small_file_is_byte_identical_and_unnotified() {
        let dir = setup();
        let path = dir.path().join("small.txt");
        fs::write(&path, "alpha\nbeta\ngamma").unwrap();
        let tools = EditTools::new();

        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: None,
                limit: None,
            },
            None,
        );

        let text = extract_text(&result);
        assert_eq!(
            text, "alpha\nbeta\ngamma",
            "small files pass through verbatim"
        );
        assert!(!text.contains("[truncated"), "no notice for a small file");
    }

    #[test]
    fn explicit_range_read_bypasses_the_unpaged_cap() {
        let dir = setup();
        let path = dir.path().join("big.txt");
        let content: String = (0..5000).map(|i| format!("line{i}\n")).collect();
        fs::write(&path, &content).unwrap();
        let tools = EditTools::new();

        // Page deep past the 2000-line cap: explicit args must not be clamped or
        // annotated — they page through anything.
        let result = tools.file_read_with_cwd(
            FileReadParams {
                path: path.to_string_lossy().to_string(),
                line: Some(4900),
                limit: Some(3),
            },
            None,
        );

        let text = extract_text(&result);
        assert_eq!(text, "line4899\nline4900\nline4901\n");
        assert!(
            !text.contains("[truncated"),
            "explicit-range reads are never capped"
        );
    }

    #[test]
    fn test_file_write_new() {
        let dir = setup();
        let path = dir.path().join("new_file.txt");
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: "Hello, world!\nLine 2".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hello, world!\nLine 2");
    }

    #[test]
    fn write_names_the_work_an_empty_content_erased() {
        // Observed in a local harness run: the model wrote a correct solution,
        // then wrote empty content over it and was told "Wrote fizzbuzz.py
        // (0 lines, verified on disk)". It spent every remaining turn verifying
        // an empty file, never told that its own answer had been erased.
        let dir = setup();
        let path = dir.path().join("solution.py");
        fs::write(
            &path,
            "def fizzbuzz(n):
    return str(n)
",
        )
        .unwrap();
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: String::new(),
        });

        // Still a success: emptying a file is a legal write.
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(
            text.contains("EMPTY") && text.contains("gone"),
            "an erasing write must name what it destroyed, got: {text}"
        );
        assert!(
            !text.contains("0 lines, verified on disk"),
            "an erasing write must not read like an ordinary one, got: {text}"
        );
    }

    #[test]
    fn write_keeps_its_ordinary_message_when_creating_an_empty_file() {
        // The warning is about destruction, not about emptiness: deliberately
        // creating an empty file is unremarkable and must stay quiet.
        let dir = setup();
        let path = dir.path().join("placeholder.txt");
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: String::new(),
        });

        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("verified on disk"), "got: {text}");
        assert!(!text.contains("EMPTY"), "got: {text}");
    }

    #[test]
    fn write_refuses_an_empty_path_instead_of_writing_the_directory() {
        // The same run then sent , which resolved to the working
        // directory and failed with "Is a directory (os error 21)" -- an error
        // naming a directory the model never mentioned. Refuse on the argument.
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: "   ".to_string(),
            content: "anything".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("`path`"), "got: {text}");
        assert!(
            !text.contains("Is a directory"),
            "the filesystem must not be reached at all, got: {text}"
        );
    }

    #[test]
    fn test_file_write_overwrite() {
        let dir = setup();
        let path = dir.path().join("existing.txt");
        fs::write(&path, "old content").unwrap();
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: "new content".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn test_file_write_reports_verified_success() {
        let dir = setup();
        let path = dir.path().join("verified.txt");
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: "content to verify".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert!(extract_text(&result).contains("verified on disk"));
    }

    #[test]
    fn edit_already_applied_requires_distinctive_after() {
        let content = "fn main() {\n    let value = compute();\n}\n";
        assert!(edit_already_applied(content, "let value = compute();"));
        assert!(!edit_already_applied(content, "let value = other();"));
        // Trivial fragments and deletions never count.
        assert!(!edit_already_applied(content, "}"));
        assert!(!edit_already_applied(content, "();"));
        assert!(!edit_already_applied(content, ""));
        assert!(!edit_already_applied(content, "   \n  "));
    }

    #[test]
    fn test_file_write_creates_dirs() {
        let dir = setup();
        let path = dir.path().join("a/b/c/file.txt");
        let tools = EditTools::new();

        let result = tools.file_write(FileWriteParams {
            path: path.to_string_lossy().to_string(),
            content: "nested".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert!(path.exists());
    }

    #[test]
    fn test_file_edit_single_match() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        fs::write(&path, "fn foo() {\n    println!(\"hello\");\n}").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "println!(\"hello\");".to_string(),
            after: "println!(\"world\");".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("println!(\"world\");"));
        assert!(!content.contains("println!(\"hello\");"));
    }

    #[test]
    fn test_file_edit_no_match() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        fs::write(&path, "some content").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "nonexistent".to_string(),
            after: "replacement".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("No match found"));
        assert!(text.contains("File preview:"));
        assert!(text.contains("some content"));
    }

    #[test]
    fn test_file_edit_already_applied_is_success_noop() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        let content = "fn main() {\n    println!(\"updated value\");\n}\n";
        fs::write(&path, content).unwrap();
        let tools = EditTools::new();

        // Replay of an edit that already landed: `before` is gone, `after` present.
        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "println!(\"original value\");".to_string(),
            after: "println!(\"updated value\");".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("No changes needed"), "got: {text}");
        assert!(text.contains("applied previously"), "got: {text}");
        // File untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn test_file_edit_trivial_after_still_errors() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        // The file contains "}" everywhere; that must not count as "already applied".
        fs::write(&path, "fn main() {\n    body();\n}\n").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "nonexistent();".to_string(),
            after: "}".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        assert!(extract_text(&result).contains("No match found"));
    }

    #[test]
    fn test_file_edit_absent_before_and_after_still_errors() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        fs::write(&path, "some content").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "nonexistent before".to_string(),
            after: "also absent replacement".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        assert!(extract_text(&result).contains("No match found"));
    }

    #[test]
    fn test_file_edit_multiple_matches() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        fs::write(&path, "foo\nbar\nfoo\nbaz").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "foo".to_string(),
            after: "qux".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), "foo\nbar\nfoo\nbaz");
    }

    #[test]
    fn test_file_edit_delete() {
        let dir = setup();
        let path = dir.path().join("edit.txt");
        fs::write(&path, "keep\ndelete me\nkeep").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "\ndelete me".to_string(),
            after: "".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep\nkeep");
    }

    #[test]
    fn test_file_write_resolves_relative_paths_from_working_dir() {
        let dir = setup();
        let tools = EditTools::new();

        let result = tools.file_write_with_cwd(
            FileWriteParams {
                path: "relative.txt".to_string(),
                content: "relative write".to_string(),
            },
            Some(dir.path()),
        );

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(
            fs::read_to_string(dir.path().join("relative.txt")).unwrap(),
            "relative write"
        );
    }

    #[test]
    fn test_file_edit_resolves_relative_paths_from_working_dir() {
        let dir = setup();
        fs::write(dir.path().join("relative-edit.txt"), "before").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit_with_cwd(
            FileEditParams {
                path: "relative-edit.txt".to_string(),
                before: "before".to_string(),
                after: "after".to_string(),
            },
            Some(dir.path()),
        );

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(
            fs::read_to_string(dir.path().join("relative-edit.txt")).unwrap(),
            "after"
        );
    }

    // ── Flexible matcher (pure function) ─────────────────────────────

    #[test]
    fn flex_exact_hit_uses_exact_tier() {
        let out =
            flexible_replace("let x = 1;\nlet y = 2;\n", "let x = 1;", "let x = 42;").unwrap();
        assert_eq!(out.tier, MatchTier::Exact);
        assert_eq!(out.new_content, "let x = 42;\nlet y = 2;\n");
    }

    #[test]
    fn flex_no_regression_on_working_mid_line_exact_edit() {
        // A partial (mid-line) exact match must still work exactly as before.
        let content = "fn foo() {\n    println!(\"hello\");\n}\n";
        let out =
            flexible_replace(content, "println!(\"hello\");", "println!(\"world\");").unwrap();
        assert_eq!(out.tier, MatchTier::Exact);
        assert_eq!(out.new_content, "fn foo() {\n    println!(\"world\");\n}\n");
    }

    #[test]
    fn flex_whitespace_only_difference_resolves() {
        // File has different *internal* spacing; leading indentation matches.
        let content = "fn foo() {\n    let  x   =    1;\n}\n";
        let out = flexible_replace(content, "    let x = 1;", "    let x = 2;").unwrap();
        assert_eq!(out.tier, MatchTier::WhitespaceNormalized);
        assert_eq!(out.new_content, "fn foo() {\n    let x = 2;\n}\n");
    }

    #[test]
    fn flex_indentation_only_difference_resolves_and_reindents() {
        // Search block has no leading indent; the file block is indented 4 spaces.
        // The replacement must be rebased onto the file's indentation.
        let content = "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n";
        let out =
            flexible_replace(content, "let x = 1;\nlet y = 2;", "let x = 1;\nlet z = 3;").unwrap();
        assert_eq!(out.tier, MatchTier::IndentationNormalized);
        assert_eq!(
            out.new_content,
            "fn foo() {\n    let x = 1;\n    let z = 3;\n}\n"
        );
    }

    #[test]
    fn flex_reindent_preserves_relative_nesting() {
        // Nested structure whose absolute indent is wrong but relative indent is right.
        let content = "class A:\n    def run(self):\n        if x:\n            go()\n";
        let out = flexible_replace(content, "if x:\n    go()", "if x:\n    stop()").unwrap();
        assert_eq!(out.tier, MatchTier::IndentationNormalized);
        assert_eq!(
            out.new_content,
            "class A:\n    def run(self):\n        if x:\n            stop()\n"
        );
    }

    #[test]
    fn flex_absent_search_is_clean_error_and_no_content() {
        let err = flexible_replace("alpha\nbeta\ngamma\n", "nonexistent line", "x").unwrap_err();
        assert!(err.contains("No match found"));
    }

    #[test]
    fn flex_ambiguous_exact_prefers_exact_error() {
        // Two exact matches → classic "add more context" error, never a fuzzy guess.
        let err = flexible_replace("foo\nbar\nfoo\n", "foo", "qux").unwrap_err();
        assert!(err.contains("exact matches"));
        assert!(err.contains("more surrounding context"));
    }

    #[test]
    fn flex_ambiguous_fuzzy_is_clear_error_no_guess() {
        // Same block at two different indentations: no exact match, tier-3 is ambiguous.
        let content =
            "fn a() {\n    step()\n    stop()\n}\nfn b() {\n        step()\n        stop()\n}\n";
        let err = flexible_replace(content, "step()\nstop()", "go()\nhalt()").unwrap_err();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("indentation-normalized"));
        assert!(err.contains("more surrounding context"));
    }

    #[test]
    fn flex_error_quality_shows_tiers_tried_near_match_and_diff() {
        let content = "fn compute() {\n    let total = a + b;\n}\n";
        let err = flexible_replace(content, "    let total = a - b;", "x").unwrap_err();
        assert!(err.contains("Tried exact, whitespace-normalized, and indentation-normalized"));
        assert!(err.contains("Closest region"));
        assert!(err.contains("Differences"));
        assert!(err.contains("let total = a + b;"));
    }

    #[test]
    fn flex_empty_search_is_error() {
        let err = flexible_replace("anything", "", "x").unwrap_err();
        assert!(err.contains("empty"));
    }

    // ── Lint-on-edit guardrail (integration through file_edit) ────────

    #[test]
    fn lint_valid_edit_writes() {
        let dir = setup();
        let path = dir.path().join("ok.rs");
        fs::write(&path, "fn main() {\n    let x = 1;\n}\n").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "let x = 1;".to_string(),
            after: "let x = 2;".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert!(fs::read_to_string(&path).unwrap().contains("let x = 2;"));
    }

    #[test]
    fn lint_rejects_net_new_syntax_error_and_leaves_file_unchanged() {
        let dir = setup();
        let path = dir.path().join("break.rs");
        let original = "fn main() {\n    let x = 1;\n}\n";
        fs::write(&path, original).unwrap();
        let tools = EditTools::new();

        // Deleting the closing brace introduces a syntax error the file didn't have.
        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "    let x = 1;\n}".to_string(),
            after: "    let x = 1;".to_string(),
        });

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("Edit rejected"));
        assert!(text.contains("syntax error"));
        // File must be untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn lint_does_not_reject_edit_to_already_broken_file() {
        let dir = setup();
        let path = dir.path().join("already_broken.rs");
        // Pre-edit file is already unparseable (missing closing brace).
        let original = "fn main() {\n    let x = 1;\n";
        fs::write(&path, original).unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "let x = 1;".to_string(),
            after: "let x = 2;".to_string(),
        });

        // Still broken after, but NOT spuriously rejected — only net-new breakage blocks.
        assert!(!result.is_error.unwrap_or(false));
        assert!(fs::read_to_string(&path).unwrap().contains("let x = 2;"));
    }

    #[test]
    fn lint_passes_through_unsupported_language() {
        let dir = setup();
        let path = dir.path().join("data.txt");
        fs::write(&path, "hello (world").unwrap();
        let tools = EditTools::new();

        // The result is "unbalanced", but .txt has no grammar → no lint, edit applies.
        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "hello (world".to_string(),
            after: "hello ((world".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello ((world");
    }

    #[test]
    fn lint_allows_edit_that_fixes_a_broken_file() {
        let dir = setup();
        let path = dir.path().join("fixup.rs");
        fs::write(&path, "fn main() {\n    let x = 1;\n").unwrap();
        let tools = EditTools::new();

        // Add the missing brace: broken → clean is always allowed.
        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "    let x = 1;\n".to_string(),
            after: "    let x = 1;\n}\n".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "fn main() {\n    let x = 1;\n}\n"
        );
    }

    #[test]
    fn fuzzy_edit_writes_through_file_edit_with_tier_note() {
        let dir = setup();
        let path = dir.path().join("fuzzy.rs");
        // Statement uses odd internal spacing; model supplies the tidy form.
        fs::write(&path, "fn main() {\n    let  y   =   9;\n}\n").unwrap();
        let tools = EditTools::new();

        let result = tools.file_edit(FileEditParams {
            path: path.to_string_lossy().to_string(),
            before: "    let y = 9;".to_string(),
            after: "    let y = 10;".to_string(),
        });

        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("whitespace-normalized"));
        assert!(text.contains("fuzzy match"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "fn main() {\n    let y = 10;\n}\n"
        );
    }
}
