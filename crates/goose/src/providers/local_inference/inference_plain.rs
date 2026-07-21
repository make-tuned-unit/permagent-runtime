//! Plain (tool-free) generation path.
//!
//! llama-cpp-2 0.1.147 removed the entire OpenAI-compat chat layer when
//! upstream llama.cpp dropped that surface (utilityai/llama-cpp-rs#1037):
//! the `openai` module, `apply_chat_template_oaicompat`,
//! `apply_chat_template_with_tools_oaicompat`, `ChatTemplateResult` (tool
//! grammar + triggers + additional stops), and the chat-format-aware streaming
//! parser. There is no upstream replacement.
//!
//! Consequences for this provider until an equivalent is reconstructed:
//!
//! - Requests WITH tools always run through `inference_emulated_tools` (text
//!   protocol), even for models whose GGUF templates support native tool
//!   calling.
//! - Tool-free requests run through this module: the prompt is rendered with
//!   the model's chat template via llama.cpp's built-in (non-Jinja) template
//!   engine, output streams as text, and `<think>…</think>` reasoning blocks
//!   are split into thinking content by [`ThinkingSplitter`] (the removed
//!   upstream parser previously did this per chat format).
//! - Grammar-constrained tool-call decoding is gone: the grammar was derived
//!   by the removed template API, so there is no grammar source anymore.

use crate::conversation::message::Message;
use crate::providers::errors::ProviderError;
use llama_cpp_2::model::AddBos;

use super::inference_engine::{
    create_and_prefill_context, create_and_prefill_multimodal, generation_loop,
    validate_and_compute_context, GenerationContext, TokenAction,
};
use super::{finalize_usage, StreamSender};

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// A piece of model output classified by the [`ThinkingSplitter`].
#[derive(Debug, PartialEq, Eq)]
enum SplitPiece {
    Text(String),
    Thinking(String),
}

/// State machine for the splitter.
enum SplitState {
    /// Nothing emitted yet; deciding whether the output opens with `<think>`.
    Start,
    /// Inside a `<think>…</think>` block.
    InThinking,
    /// Plain text passthrough (after the think block, or when none was found).
    Text,
}

/// Streaming splitter that separates a leading `<think>…</think>` block from
/// the rest of the output.
///
/// Reasoning models (Qwen3, DeepSeek-R1 distills, …) open their response with a
/// `<think>` block. The removed llama.cpp oaicompat parser used to strip it into
/// `reasoning_content`; this is the minimal replacement so thinking still
/// arrives as thinking content instead of polluting the chat text. Only a
/// leading tag (optionally preceded by whitespace) is recognized — matching how
/// these models emit it — so `<think>` appearing mid-text is left untouched.
struct ThinkingSplitter {
    enabled: bool,
    state: SplitState,
    buffer: String,
}

impl ThinkingSplitter {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            state: SplitState::Start,
            buffer: String::new(),
        }
    }

    /// Feed a chunk; returns zero or more classified pieces ready to emit.
    fn push(&mut self, chunk: &str) -> Vec<SplitPiece> {
        if !self.enabled {
            return vec![SplitPiece::Text(chunk.to_string())];
        }
        self.buffer.push_str(chunk);
        let mut out = Vec::new();
        loop {
            match self.state {
                SplitState::Start => {
                    let trimmed = self.buffer.trim_start();
                    if trimmed.starts_with(THINK_OPEN) {
                        // Drop the (pure whitespace) prefix and the tag itself.
                        let after = trimmed
                            .split_at(THINK_OPEN.len())
                            .1
                            .trim_start_matches('\n')
                            .to_string();
                        self.buffer = after;
                        self.state = SplitState::InThinking;
                        continue;
                    }
                    // Keep waiting while the (trimmed) buffer could still grow
                    // into the opening tag; otherwise it is plain text.
                    if trimmed.is_empty() || THINK_OPEN.starts_with(trimmed) {
                        break;
                    }
                    self.state = SplitState::Text;
                    continue;
                }
                SplitState::InThinking => {
                    if let Some(idx) = self.buffer.find(THINK_CLOSE) {
                        let (thinking, rest) = self.buffer.split_at(idx);
                        if !thinking.is_empty() {
                            out.push(SplitPiece::Thinking(thinking.to_string()));
                        }
                        // Skip the close tag and any whitespace right after it.
                        let rest = rest.split_at(THINK_CLOSE.len()).1.trim_start().to_string();
                        self.buffer = rest;
                        self.state = SplitState::Text;
                        continue;
                    }
                    // Emit everything except a suffix that could still be the
                    // start of the close tag.
                    let hold = longest_suffix_that_prefixes(&self.buffer, THINK_CLOSE);
                    if self.buffer.len() > hold {
                        let rest = self.buffer.split_off(self.buffer.len() - hold);
                        let thinking = std::mem::replace(&mut self.buffer, rest);
                        if !thinking.is_empty() {
                            out.push(SplitPiece::Thinking(thinking));
                        }
                    }
                    break;
                }
                SplitState::Text => {
                    if !self.buffer.is_empty() {
                        out.push(SplitPiece::Text(std::mem::take(&mut self.buffer)));
                    }
                    break;
                }
            }
        }
        out
    }

    /// Flush anything still buffered at end of generation.
    fn flush(&mut self) -> Vec<SplitPiece> {
        let mut out = Vec::new();
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            match self.state {
                // An unterminated think block still counts as thinking.
                SplitState::InThinking => out.push(SplitPiece::Thinking(remaining)),
                SplitState::Start | SplitState::Text => out.push(SplitPiece::Text(remaining)),
            }
        }
        self.state = SplitState::Text;
        out
    }
}

/// Byte length of the longest suffix of `buf` that is a prefix of `tag`.
/// `tag` must be ASCII; the returned length always falls on a char boundary of
/// `buf` because every matched prefix is pure ASCII.
fn longest_suffix_that_prefixes(buf: &str, tag: &str) -> usize {
    let max = tag.len().min(buf.len());
    for len in (1..=max).rev() {
        let (prefix, _) = tag.split_at(len);
        if buf.ends_with(prefix) {
            return len;
        }
    }
    0
}

/// Send one classified piece to the stream. Returns `false` if the receiver hung up.
fn send_piece(piece: &SplitPiece, message_id: &str, tx: &StreamSender) -> bool {
    let mut msg = match piece {
        SplitPiece::Thinking(t) => Message::assistant().with_thinking(t, ""),
        SplitPiece::Text(t) => Message::assistant().with_text(t),
    };
    msg.id = Some(message_id.to_string());
    tx.blocking_send(Ok((Some(msg), None))).is_ok()
}

/// Generate a tool-free chat completion: render the chat template, stream the
/// output, and split a leading `<think>` block into thinking content.
pub(super) fn generate_plain(ctx: &mut GenerationContext<'_>) -> Result<(), ProviderError> {
    // `true` = add_generation_prompt (append the assistant opening tag).
    let prompt = ctx
        .loaded
        .model
        .apply_chat_template(&ctx.loaded.template, ctx.chat_messages, true)
        .map_err(|e| {
            ProviderError::ExecutionError(format!("Failed to apply chat template: {}", e))
        })?;

    let _ = ctx
        .log
        .write(&serde_json::json!({"applied_prompt": &prompt}), None);

    let (mut llama_ctx, prompt_token_count, effective_ctx) = if !ctx.images.is_empty() {
        create_and_prefill_multimodal(
            ctx.loaded,
            ctx.runtime,
            &prompt,
            ctx.images,
            ctx.context_limit,
            ctx.settings,
        )?
    } else {
        let tokens = ctx
            .loaded
            .model
            .str_to_token(&prompt, AddBos::Never)
            .map_err(|e| ProviderError::ExecutionError(e.to_string()))?;
        let (ptc, ectx) = validate_and_compute_context(
            ctx.loaded,
            ctx.runtime,
            tokens.len(),
            ctx.context_limit,
            ctx.settings,
        )?;
        let lctx =
            create_and_prefill_context(ctx.loaded, ctx.runtime, &tokens, ectx, ctx.settings)?;
        (lctx, ptc, ectx)
    };

    let message_id = ctx.message_id;
    let tx = ctx.tx;
    let mut generated_text = String::new();
    let mut splitter = ThinkingSplitter::new(ctx.settings.enable_thinking);

    let output_token_count = generation_loop(
        &ctx.loaded.model,
        &mut llama_ctx,
        ctx.settings,
        prompt_token_count,
        effective_ctx,
        // No grammar sampler: tool grammars came from the removed template API
        // and this path serves tool-free requests only.
        None,
        |piece| {
            generated_text.push_str(piece);
            for part in splitter.push(piece) {
                if !send_piece(&part, message_id, tx) {
                    return Ok(TokenAction::Stop);
                }
            }
            Ok(TokenAction::Continue)
        },
    )?;

    for part in splitter.flush() {
        let _ = send_piece(&part, message_id, tx);
    }

    let provider_usage = finalize_usage(
        ctx.log,
        std::mem::take(&mut ctx.model_name),
        "plain",
        prompt_token_count,
        output_token_count,
        Some(("generated_text", &generated_text)),
    );
    let _ = ctx.tx.blocking_send(Ok((None, Some(provider_usage))));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed chunks and collect merged (thinking, text) strings.
    fn run(enabled: bool, chunks: &[&str]) -> (String, String) {
        let mut splitter = ThinkingSplitter::new(enabled);
        let mut thinking = String::new();
        let mut text = String::new();
        let mut collect = |pieces: Vec<SplitPiece>| {
            for p in pieces {
                match p {
                    SplitPiece::Thinking(t) => thinking.push_str(&t),
                    SplitPiece::Text(t) => text.push_str(&t),
                }
            }
        };
        for chunk in chunks {
            collect(splitter.push(chunk));
        }
        collect(splitter.flush());
        (thinking, text)
    }

    #[test]
    fn test_disabled_passes_everything_through() {
        let (thinking, text) = run(false, &["<think>secret</think>", "answer"]);
        assert_eq!(thinking, "");
        assert_eq!(text, "<think>secret</think>answer");
    }

    #[test]
    fn test_no_think_tag_is_plain_text() {
        let (thinking, text) = run(true, &["Hello", " world"]);
        assert_eq!(thinking, "");
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_single_chunk_think_then_text() {
        let (thinking, text) = run(true, &["<think>reasoning here</think>\nThe answer is 4."]);
        assert_eq!(thinking, "reasoning here");
        assert_eq!(text, "The answer is 4.");
    }

    #[test]
    fn test_leading_whitespace_before_think() {
        let (thinking, text) = run(true, &["\n\n<think>hm</think>ok"]);
        assert_eq!(thinking, "hm");
        assert_eq!(text, "ok");
    }

    #[test]
    fn test_char_by_char_streaming() {
        let full = "<think>a+b</think>done";
        let chunks: Vec<String> = full.chars().map(|c| c.to_string()).collect();
        let chunk_refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let (thinking, text) = run(true, &chunk_refs);
        assert_eq!(thinking, "a+b");
        assert_eq!(text, "done");
    }

    #[test]
    fn test_tag_split_across_chunks() {
        let (thinking, text) = run(true, &["<th", "ink>\nplan", "ning</th", "ink>\nresult"]);
        assert_eq!(thinking, "planning");
        assert_eq!(text, "result");
    }

    #[test]
    fn test_partial_open_tag_that_never_completes() {
        // "<thinking about it" must not be swallowed as a tag.
        let (thinking, text) = run(true, &["<think", "ing about it"]);
        assert_eq!(thinking, "");
        assert_eq!(text, "<thinking about it");
    }

    #[test]
    fn test_unclosed_think_block_flushes_as_thinking() {
        let (thinking, text) = run(true, &["<think>never ", "closed"]);
        assert_eq!(thinking, "never closed");
        assert_eq!(text, "");
    }

    #[test]
    fn test_mid_text_think_tag_is_left_alone() {
        let (thinking, text) = run(true, &["The tag <think> is special."]);
        assert_eq!(thinking, "");
        assert_eq!(text, "The tag <think> is special.");
    }

    #[test]
    fn test_close_tag_lookalike_inside_thinking() {
        let (thinking, text) = run(true, &["<think>a </thin fake b</think>c"]);
        assert_eq!(thinking, "a </thin fake b");
        assert_eq!(text, "c");
    }

    #[test]
    fn test_multibyte_text_streams_safely() {
        let (thinking, text) = run(true, &["<think>héllo — ünïcode</think>", "résultat ✓"]);
        assert_eq!(thinking, "héllo — ünïcode");
        assert_eq!(text, "résultat ✓");
    }

    #[test]
    fn test_whitespace_after_close_tag_is_stripped() {
        let (thinking, text) = run(true, &["<think>x</think>\n\n  Answer"]);
        assert_eq!(thinking, "x");
        assert_eq!(text, "Answer");
    }

    #[test]
    fn test_longest_suffix_that_prefixes() {
        assert_eq!(longest_suffix_that_prefixes("abc", "</think>"), 0);
        assert_eq!(longest_suffix_that_prefixes("abc<", "</think>"), 1);
        // "</thin" — 6 bytes — is the longest suffix that prefixes the tag.
        assert_eq!(longest_suffix_that_prefixes("abc</thin", "</think>"), 6);
        assert_eq!(longest_suffix_that_prefixes("</think", "</think>"), 7);
        assert_eq!(longest_suffix_that_prefixes("", "</think>"), 0);
        // Multibyte buffer content must not panic.
        assert_eq!(longest_suffix_that_prefixes("héllo—", "</think>"), 0);
    }
}
