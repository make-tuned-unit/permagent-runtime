//! Observation masking: old tool-result bodies are replaced with a short
//! placeholder in the conversation SENT to the model. The stored transcript is
//! never touched.
//!
//! Evidence: JetBrains, "The Complexity Trap" (arXiv 2508.21433) — replacing
//! observations older than the last 10 turns with a placeholder cut SWE-agent's
//! cost per instance ~52-56% at equal or better solve rate, and beat LLM
//! summarisation.
//!
//! Why masking moves in FIXED BLOCKS, not a sliding window. Every provider we
//! use caches an exact prefix at ~0.1x the input price. Masking rewrites an old
//! message, so the prefix breaks at the first newly-masked result and everything
//! after it is re-billed at full price (1.25x on an Anthropic cache write). A
//! window that slides by one turn every turn therefore breaks the cache EVERY
//! turn. Replayed against this machine's 25 longest sessions since 2026-09-10
//! (research_notes/Cheap coding harness on Mac minis/context_cost_measurement.md),
//! with the prefix cache working, a sliding window of 10 cost +67% vs no masking,
//! while a block of 10 (the boundary advances only every 10 turns) came out
//! slightly cheaper (-1.5%) and cut the tokens sent by 9%. The boundary here is
//! a pure function of the number of assistant turns, so between advances the
//! masked conversation is byte-identical turn to turn.
//!
//! Pairing: a masked result keeps its `ToolResponse` item and its id, and only
//! its content is swapped for one text item, so every provider format still sees
//! each tool call answered exactly once, in place.

use crate::conversation::message::{Message, MessageContent};
use rmcp::model::{CallToolResult, Content, Role};
use std::collections::HashMap;

/// Config key (and env var) for the number of recent assistant turns whose tool
/// results are always sent in full. `0` disables masking.
pub const OBS_MASK_TURNS_KEY: &str = "PERMAGENT_OBS_MASK_TURNS";

/// Default window, from the Complexity Trap's M=10.
pub const DEFAULT_OBS_MASK_TURNS: usize = 10;

/// Results shorter than this are sent in full: the placeholder would save
/// almost nothing and the short result (an exit code, a one-line error) is
/// often the failure signal the model needs to keep seeing.
pub const MIN_MASKABLE_CHARS: usize = 400;

/// The configured window: `PERMAGENT_OBS_MASK_TURNS` from config/env, else
/// [`DEFAULT_OBS_MASK_TURNS`]. `0` disables masking.
pub fn configured_keep_turns() -> usize {
    crate::config::Config::global()
        .get_param::<usize>(OBS_MASK_TURNS_KEY)
        .unwrap_or(DEFAULT_OBS_MASK_TURNS)
}

/// The number of oldest assistant turns whose tool results are masked, given
/// `assistant_turns` in the conversation and a window of `keep_turns`.
///
/// Advances in blocks of `keep_turns`: with 10, turns 0-19 mask nothing, 20-29
/// mask the first 10, 30-39 the first 20, and so on. So between 10 and 19 of
/// the most recent turns are always in full, and the masked prefix changes only
/// once every `keep_turns` turns.
pub fn mask_boundary(assistant_turns: usize, keep_turns: usize) -> usize {
    if keep_turns == 0 || assistant_turns < 2 * keep_turns {
        return 0;
    }
    ((assistant_turns - keep_turns) / keep_turns) * keep_turns
}

/// The placeholder that replaces a masked result. Deterministic (no clock, no
/// counters that change later) so a masked message is byte-identical on every
/// later turn.
pub fn placeholder(tool_name: &str, omitted_chars: usize, keep_turns: usize) -> String {
    format!(
        "[observation masked: output of `{tool_name}` ({omitted_chars} chars) omitted from \
         context because it is older than the last {keep_turns} turns. Re-run the tool if \
         you need it again.]"
    )
}

/// Size of a tool result's content in characters (text and embedded-resource
/// text; any other content by its serialized size).
fn result_chars(result: &CallToolResult) -> usize {
    result
        .content
        .iter()
        .map(|c| {
            if let Some(t) = c.as_text() {
                t.text.chars().count()
            } else {
                serde_json::to_string(c).map(|s| s.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// Replace the bodies of tool results that answer the oldest
/// [`mask_boundary`] assistant turns with a [`placeholder`]. Pure: same input,
/// same output. Returns the input unchanged when nothing qualifies.
///
/// A tool result "answers" assistant turn `k` (0-based) when it sits in a user
/// message after the `k`-th assistant message and before the next one. Errors
/// (`Err`) and results under [`MIN_MASKABLE_CHARS`] are kept in full; a
/// successful result that reported `is_error` keeps that flag on its
/// placeholder.
pub fn mask_old_observations(messages: &[Message], keep_turns: usize) -> Vec<Message> {
    let assistant_turns = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    let boundary = mask_boundary(assistant_turns, keep_turns);
    if boundary == 0 {
        return messages.to_vec();
    }

    let mut tool_names: HashMap<&str, &str> = HashMap::new();
    for message in messages.iter().filter(|m| m.role == Role::Assistant) {
        for content in &message.content {
            if let MessageContent::ToolRequest(request) = content {
                if let Ok(call) = &request.tool_call {
                    tool_names.insert(request.id.as_str(), &*call.name);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(messages.len());
    let mut assistants_seen = 0usize;
    for message in messages {
        if message.role == Role::Assistant {
            assistants_seen += 1;
            out.push(message.clone());
            continue;
        }
        // Results in this message answer assistant turn `assistants_seen - 1`.
        let answers_masked_turn = assistants_seen >= 1 && assistants_seen <= boundary;
        if !answers_masked_turn {
            out.push(message.clone());
            continue;
        }
        let mut masked = message.clone();
        for content in masked.content.iter_mut() {
            let MessageContent::ToolResponse(response) = content else {
                continue;
            };
            let Ok(result) = &response.tool_result else {
                continue;
            };
            let chars = result_chars(result);
            if chars < MIN_MASKABLE_CHARS {
                continue;
            }
            let name = tool_names
                .get(response.id.as_str())
                .copied()
                .unwrap_or("tool");
            let mut replacement =
                CallToolResult::success(vec![Content::text(placeholder(name, chars, keep_turns))]);
            replacement.is_error = result.is_error;
            response.tool_result = Ok(replacement);
        }
        out.push(masked);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;

    fn big(label: &str) -> String {
        format!("{label}: {}", "x".repeat(MIN_MASKABLE_CHARS))
    }

    /// `turns` tool rounds after an opening user message.
    fn session(turns: usize) -> Vec<Message> {
        let mut messages = vec![Message::user().with_text("fix the bug")];
        for t in 0..turns {
            let id = format!("call_{t}");
            messages.push(
                Message::assistant()
                    .with_text(format!("step {t}"))
                    .with_tool_request(&id, Ok(CallToolRequestParams::new("developer__shell"))),
            );
            messages.push(Message::user().with_tool_response(
                &id,
                Ok(CallToolResult::success(vec![Content::text(big(&id))])),
            ));
        }
        messages
    }

    fn result_text(message: &Message) -> Option<String> {
        message.content.iter().find_map(|c| match c {
            MessageContent::ToolResponse(r) => r
                .tool_result
                .as_ref()
                .ok()
                .and_then(|res| res.content.first())
                .and_then(|c| c.as_text())
                .map(|t| t.text.clone()),
            _ => None,
        })
    }

    #[test]
    fn boundary_advances_in_whole_blocks() {
        assert_eq!(mask_boundary(0, 10), 0);
        assert_eq!(mask_boundary(19, 10), 0);
        assert_eq!(mask_boundary(20, 10), 10);
        assert_eq!(mask_boundary(29, 10), 10);
        assert_eq!(mask_boundary(30, 10), 20);
        assert_eq!(mask_boundary(45, 10), 30);
        // 0 disables.
        assert_eq!(mask_boundary(1_000, 0), 0);
        // Always at least `keep` recent turns in full.
        for turns in 0..200 {
            let b = mask_boundary(turns, 10);
            assert!(turns - b >= 10 || b == 0, "turns={turns} boundary={b}");
            assert_eq!(b % 10, 0);
        }
    }

    #[test]
    fn short_sessions_are_untouched() {
        let messages = session(19);
        assert_eq!(mask_old_observations(&messages, 10), messages);
        let long = session(50);
        assert_eq!(mask_old_observations(&long, 0), long);
    }

    #[test]
    fn old_results_become_placeholders_naming_tool_and_size() {
        let messages = session(20);
        let masked = mask_old_observations(&messages, 10);
        assert_eq!(masked.len(), messages.len());

        // User message after assistant turn k sits at index 2k + 2.
        let first = result_text(&masked[2]).unwrap();
        assert!(first.starts_with("[observation masked: output of `developer__shell`"));
        let original_chars = big("call_0").chars().count();
        assert!(first.contains(&format!("({original_chars} chars)")));
        assert!(first.contains("last 10 turns"));

        // Turns 0..10 masked, 10..20 kept verbatim.
        for k in 0..20 {
            let text = result_text(&masked[2 * k + 2]).unwrap();
            if k < 10 {
                assert!(text.starts_with("[observation masked"), "turn {k}");
            } else {
                assert_eq!(text, big(&format!("call_{k}")), "turn {k}");
            }
        }
        // Assistant messages and the opening user message are untouched.
        assert_eq!(masked[0], messages[0]);
        for k in 0..20 {
            assert_eq!(masked[2 * k + 1], messages[2 * k + 1]);
        }
    }

    #[test]
    fn pairing_is_preserved_for_every_call() {
        let masked = mask_old_observations(&session(35), 10);
        let requests: Vec<String> = masked
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::ToolRequest(r) => Some(r.id.clone()),
                _ => None,
            })
            .collect();
        let responses: Vec<String> = masked
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::ToolResponse(r) => Some(r.id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(requests, responses, "every call answered once, in order");
        // And the conversation still passes the provider-facing validator
        // unchanged.
        let (fixed, issues) = crate::conversation::fix_conversation(
            crate::conversation::Conversation::new_unvalidated(masked.clone()),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(fixed.messages(), &masked);
    }

    /// Cache friendliness: between block advances, turn t's masked request is
    /// a byte-prefix of turn t+1's.
    #[test]
    fn masked_prefix_is_stable_between_block_boundaries() {
        // One transcript, sliced per turn: `Message::created` is a wall-clock
        // stamp, so building each turn separately could differ by a second.
        let full = session(46);
        let at = |turns: usize| &full[..1 + 2 * turns];
        for t in 20..45 {
            let now = mask_old_observations(at(t), 10);
            let next = mask_old_observations(at(t + 1), 10);
            let same_block = mask_boundary(t, 10) == mask_boundary(t + 1, 10);
            let now_json = serde_json::to_string(&now).unwrap();
            let next_prefix = serde_json::to_string(&next[..now.len()]).unwrap();
            assert_eq!(
                now_json == next_prefix,
                same_block,
                "t={t}: prefix must change exactly when the block advances"
            );
        }
    }

    #[test]
    fn small_results_and_errors_are_kept() {
        let mut messages = vec![Message::user().with_text("go")];
        for t in 0..25 {
            let id = format!("c{t}");
            messages.push(
                Message::assistant()
                    .with_tool_request(&id, Ok(CallToolRequestParams::new("developer__shell"))),
            );
            let response = match t {
                0 => Ok(CallToolResult::success(vec![Content::text("exit 1")])),
                1 => Err(rmcp::model::ErrorData::internal_error(big("boom"), None)),
                2 => {
                    let mut r = CallToolResult::success(vec![Content::text(big("failed"))]);
                    r.is_error = Some(true);
                    Ok(r)
                }
                _ => Ok(CallToolResult::success(vec![Content::text(big(&id))])),
            };
            messages.push(Message::user().with_tool_response(&id, response));
        }
        let masked = mask_old_observations(&messages, 10);
        assert_eq!(masked[2], messages[2], "short result kept");
        assert_eq!(masked[4], messages[4], "Err result kept");
        // A long is_error result is masked but keeps its error flag.
        let MessageContent::ToolResponse(r) = &masked[6].content[0] else {
            panic!("expected a tool response");
        };
        let res = r.tool_result.as_ref().unwrap();
        assert_eq!(res.is_error, Some(true));
        assert!(result_text(&masked[6])
            .unwrap()
            .starts_with("[observation masked"));
    }

    #[test]
    fn unknown_tool_ids_fall_back_to_a_generic_name() {
        let mut messages = session(20);
        // Orphan the first result's name by renaming its request id.
        if let MessageContent::ToolRequest(r) = &mut messages[1].content[1] {
            r.id = "renamed".into();
        }
        let masked = mask_old_observations(&messages, 10);
        assert!(result_text(&masked[2])
            .unwrap()
            .contains("output of `tool`"));
    }
}
