//! Spill an oversized tool result to a file and leave a useful stub in context.
//!
//! Always-on, on the general dispatch path: every tool result resolved by
//! `Agent::dispatch_tool_call` passes through [`process_tool_response`], so this
//! is not a per-tool opt-in. That much Permagent already had.
//!
//! What it did NOT have is the half that makes the pattern pay, and which
//! deepagents' `FilesystemMiddleware.wrap_tool_call` gets right:
//!
//! * **A stub with content in it.** The old stub was a bare "stored in the file"
//!   sentence — the model learned the size and a path and nothing else, so even
//!   answering "did the build pass?" cost a second tool call to read back the
//!   last line it had just been handed and thrown away. The stub now carries a
//!   head and a tail, which between them hold the two places anything useful
//!   lives: what the output *is*, and how it *ended*.
//! * **A threshold measured against the context budget.** 200,000 characters is
//!   ~50k tokens — a quarter of a large window spent on one tool result, and it
//!   was applied PER CONTENT BLOCK, so three 150k blocks in one result (450k
//!   characters) all passed through untouched. The budget is now measured over
//!   the whole result, at deepagents' ~20k tokens.
//! * **A path that says what it is.** Files are keyed by session and tool
//!   request instead of a bare timestamp, so a path in the transcript can be
//!   traced back to the call that produced it.
//!
//! One property is load-bearing and unchanged: **a failure here never loses a
//! result**. If the spill file cannot be written, the original content is
//! returned whole with a warning. An offload that could silently eat a tool
//! result would be worse than no offload at all.

use rmcp::model::{CallToolResult, Content, ErrorData};
use std::path::PathBuf;

/// Results estimated above this many tokens are spilled. deepagents uses
/// ~20,000: well below a modern context window, well above any result a model
/// reads end to end.
pub const OFFLOAD_TOKEN_THRESHOLD: usize = 20_000;

/// Characters per token for the estimate. A real tokenizer would cost a pass
/// over every tool result on the hot path to sharpen a threshold that is
/// approximate by design; 4 is the usual English ratio and errs high on code,
/// which is most of what we carry.
const CHARS_PER_TOKEN: usize = 4;

/// Characters kept from the head of a spilled result — enough to see what it is.
const HEAD_CHARS: usize = 2_000;
/// Characters kept from the tail — where the summary line, the error and the
/// totals live.
const TAIL_CHARS: usize = 2_000;

/// Estimated tokens for a character count.
pub fn estimated_tokens(chars: usize) -> usize {
    chars / CHARS_PER_TOKEN
}

/// Reduce an id to one safe path component (no traversal, no separators).
fn sanitize_component(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// Head + tail of `text`, stating in the middle exactly how much was elided.
pub fn preview(text: &str) -> String {
    let total = text.chars().count();
    if total <= HEAD_CHARS + TAIL_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(HEAD_CHARS).collect();
    let tail: String = text.chars().skip(total - TAIL_CHARS).collect();
    let elided = total - HEAD_CHARS - TAIL_CHARS;
    format!("{head}\n\n… [{elided} characters elided — the full output is in the file named above] …\n\n{tail}")
}

/// The in-context replacement for a spilled result.
pub fn stub(tool_name: &str, text: &str, path: &std::path::Path) -> String {
    let chars = text.chars().count();
    format!(
        "[The `{tool_name}` result was {chars} characters (~{tokens} tokens) — too large to \
         carry in context. The COMPLETE output, nothing removed, is saved at:\n\n  {path}\n\n\
         If the head and tail below do not answer your question, read the file rather than \
         guessing: `search` it for a term, or shell `head`/`tail`/`sed -n '100,200p'` it.]\n\n\
         {preview}",
        tool_name = tool_name,
        chars = chars,
        tokens = estimated_tokens(chars),
        path = path.display(),
        preview = preview(text),
    )
}

/// Where a spilled result is written: namespaced by session and keyed by the
/// tool request, so a path in the transcript traces back to its call.
pub fn spill_path(session_id: &str, request_id: &str, tool_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("permagent_tool_results")
        .join(sanitize_component(session_id))
        .join(format!(
            "{}-{}.txt",
            sanitize_component(tool_name),
            sanitize_component(request_id)
        ))
}

/// Process a resolved tool result, spilling it to a file when it is too large
/// to carry in context.
///
/// Non-text content (images, embedded resources) is passed through untouched —
/// it is not what overruns the window here, and routing it through a text file
/// would corrupt it. `is_error` and every other field are preserved: this
/// changes how much of a result the model sees, never what the result *was*.
pub fn process_tool_response(
    response: Result<CallToolResult, ErrorData>,
    tool_name: &str,
    request_id: &str,
    session_id: &str,
) -> Result<CallToolResult, ErrorData> {
    let Ok(mut result) = response else {
        return response;
    };

    // Budget is measured over the WHOLE result, not per block — three blocks
    // under the limit can still be far over it together.
    let text_chars: usize = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.chars().count()))
        .sum();
    if estimated_tokens(text_chars) <= OFFLOAD_TOKEN_THRESHOLD {
        return Ok(result);
    }

    let full_text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");

    let path = spill_path(session_id, request_id, tool_name);
    let write_result = std::fs::create_dir_all(path.parent().unwrap_or(&path))
        .and_then(|_| std::fs::write(&path, full_text.as_bytes()));

    let replacement = match write_result {
        Ok(()) => {
            tracing::info!(
                tool = %tool_name,
                chars = text_chars,
                path = %path.display(),
                "tool result offloaded to a file; a head+tail stub carries the path into context"
            );
            stub(tool_name, &full_text, &path)
        }
        Err(e) => {
            // Never lose the result to a disk problem.
            tracing::warn!(
                tool = %tool_name,
                path = %path.display(),
                "could not write the tool-result spill file ({e}) — returning the result whole \
                 rather than losing it"
            );
            format!(
                "Warning: failed to save this large response to a file: {e}. Showing the full \
                 content instead.\n\n{full_text}"
            )
        }
    };

    // Collapse every text block into the one replacement, in the position of
    // the first text block; non-text blocks keep their relative order.
    let mut rebuilt: Vec<Content> = Vec::with_capacity(result.content.len());
    let mut placed = false;
    for content in result.content.into_iter() {
        if content.as_text().is_some() {
            if !placed {
                rebuilt.push(Content::text(replacement.clone()));
                placed = true;
            }
        } else {
            rebuilt.push(content);
        }
    }
    result.content = rebuilt;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{Content, ErrorCode, ErrorData, RawContent};
    use std::borrow::Cow;

    /// Characters comfortably over the threshold.
    fn over_threshold() -> usize {
        OFFLOAD_TOKEN_THRESHOLD * CHARS_PER_TOKEN + 4_000
    }

    fn process(response: Result<CallToolResult, ErrorData>) -> Result<CallToolResult, ErrorData> {
        process_tool_response(response, "search", "req-1", "sess-1")
    }

    fn text_of(result: &CallToolResult, i: usize) -> String {
        result.content[i].as_text().unwrap().text.clone()
    }

    #[test]
    fn a_small_response_passes_through_byte_for_byte() {
        let small = "This is a small text response";
        let processed = process(Ok(CallToolResult::success(vec![Content::text(small)]))).unwrap();
        assert_eq!(processed.content.len(), 1);
        assert_eq!(text_of(&processed, 0), small);
    }

    #[test]
    fn a_response_at_the_threshold_is_left_alone() {
        let exactly = "a".repeat(OFFLOAD_TOKEN_THRESHOLD * CHARS_PER_TOKEN);
        let processed =
            process(Ok(CallToolResult::success(vec![Content::text(&exactly)]))).unwrap();
        assert_eq!(text_of(&processed, 0), exactly);
    }

    #[test]
    fn a_large_response_is_written_whole_and_stubbed_in_context() {
        let body = format!("FIRSTLINE{}LASTLINE", "a".repeat(over_threshold()));
        let processed = process_tool_response(
            Ok(CallToolResult::success(vec![Content::text(&body)])),
            "shell",
            "req-42",
            "sess-9",
        )
        .unwrap();

        assert_eq!(processed.content.len(), 1);
        let stub_text = text_of(&processed, 0);

        // The stub is small, names the tool, and carries BOTH ends — the old
        // stub carried neither.
        assert!(
            stub_text.chars().count() < body.chars().count() / 4,
            "stub was {} chars",
            stub_text.chars().count()
        );
        assert!(stub_text.starts_with("[The `shell` result"));
        assert!(stub_text.contains("FIRSTLINE"), "head must survive");
        assert!(stub_text.contains("LASTLINE"), "tail must survive");
        assert!(stub_text.contains("characters elided"));

        // The file holds the result verbatim.
        let path = spill_path("sess-9", "req-42", "shell");
        assert!(stub_text.contains(&path.display().to_string()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
    }

    /// The per-block bug: blocks that are individually small can be far over
    /// the budget together, and used to pass through untouched.
    #[test]
    fn the_budget_is_measured_over_the_whole_result_not_per_block() {
        let block = "b".repeat(over_threshold() / 2 + 1_000);
        let processed = process_tool_response(
            Ok(CallToolResult::success(vec![
                Content::text(&block),
                Content::text(&block),
            ])),
            "search",
            "req-multi",
            "sess-multi",
        )
        .unwrap();
        assert_eq!(
            processed.content.len(),
            1,
            "text blocks collapse into one stub"
        );
        assert!(text_of(&processed, 0).contains("too large to carry in context"));
        let _ = std::fs::remove_file(spill_path("sess-multi", "req-multi", "search"));
    }

    #[test]
    fn is_error_and_non_text_content_survive_the_offload() {
        let mut original = CallToolResult::success(vec![
            Content::text("c".repeat(over_threshold())),
            Content::image("aGk=".to_string(), "image/png".to_string()),
        ]);
        original.is_error = Some(true);

        let processed = process_tool_response(Ok(original), "browser", "req-7", "sess-7").unwrap();
        assert_eq!(processed.is_error, Some(true));
        assert_eq!(processed.content.len(), 2);
        assert!(processed.content[0].as_text().is_some());
        assert!(matches!(processed.content[1].raw, RawContent::Image(_)));
        let _ = std::fs::remove_file(spill_path("sess-7", "req-7", "browser"));
    }

    #[test]
    fn an_error_response_is_passed_through_untouched() {
        let err = Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from("boom"),
            data: None,
        });
        assert!(process(err).is_err());
    }

    #[test]
    fn a_result_with_no_text_is_never_spilled() {
        let processed = process(Ok(CallToolResult::success(vec![]))).unwrap();
        assert!(processed.content.is_empty());
    }

    #[test]
    fn the_preview_returns_short_text_unchanged() {
        assert_eq!(preview("short"), "short");
    }

    #[test]
    fn a_hostile_id_cannot_escape_the_spill_dir() {
        let p = spill_path("../../etc", "../passwd", "sh");
        assert!(!p.to_string_lossy().contains(".."));
        assert!(p.starts_with(std::env::temp_dir().join("permagent_tool_results")));
    }
}
