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
//! a pure function of the number of model turns, so between advances the
//! masked conversation is byte-identical turn to turn.
//!
//! A "turn" is one model response, however many tool calls it made in
//! parallel — the unit the Complexity Trap's M=10 is calibrated in. It is NOT
//! one assistant message: the agent persists a response with N tool calls as N
//! `assistant(call) → user(result)` pairs, so counting assistant messages
//! counted tool calls, and a model making 4 parallel reads per step saw its
//! window shrink to ~2.5 steps while the placeholder claimed 10. See
//! [`model_turns`] for how the pairs are grouped back into responses.
//!
//! What is masked: only the results of tools that declare themselves read-only
//! (see [`repeatable_tools`]). The model can get such a result back, as of now,
//! by calling the tool again, at no cost beyond the call. Anything else can be
//! the only record of work that cannot be repeated for free, such as a
//! delegated worker's report, a job id, or the output of a command that changed
//! state. Those results stay in full however old they are. The replay above
//! masked every tool, so it overstates what this saves.
//!
//! Pairing: a masked result keeps its `ToolResponse` item and its id, and only
//! its content is swapped for one text item, so every provider format still sees
//! each tool call answered exactly once, in place.

use crate::conversation::message::{Message, MessageContent};
use rmcp::model::{CallToolResult, Content, Role, Tool};
use std::collections::{HashMap, HashSet};

/// Config key (and env var) for the number of recent model turns whose tool
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

/// The number of oldest model turns whose tool results are masked, given
/// `model_turns` in the conversation and a window of `keep_turns`.
///
/// Advances in blocks of `keep_turns`: with 10, turns 0-19 mask nothing, 20-29
/// mask the first 10, 30-39 the first 20, and so on. So between 10 and 19 of
/// the most recent turns are always in full, and the masked prefix changes only
/// once every `keep_turns` turns.
pub fn mask_boundary(model_turns: usize, keep_turns: usize) -> usize {
    if keep_turns == 0 || model_turns < 2 * keep_turns {
        return 0;
    }
    ((model_turns - keep_turns) / keep_turns) * keep_turns
}

/// The tools whose results may be masked: those that declare themselves
/// read-only (`readOnlyHint: true`), by name as the model calls them.
///
/// Calling a read-only tool again has no side effects, so the model can always
/// get a masked result back (as of now), which is what the [`placeholder`]
/// tells it. Every other tool keeps its results in full. A delegated worker's
/// report, a job id or the output of a command that changed something may be
/// the only record the model has, and running the call again would repeat its
/// side effects and its cost. A tool with no annotation counts as NOT
/// read-only, which is the MCP spec's default for `readOnlyHint`.
pub fn repeatable_tools<'a>(tools: impl IntoIterator<Item = &'a Tool>) -> HashSet<&'a str> {
    tools
        .into_iter()
        .filter(|tool| {
            tool.annotations
                .as_ref()
                .and_then(|annotations| annotations.read_only_hint)
                == Some(true)
        })
        .map(|tool| &*tool.name)
        .collect()
}

/// The placeholder that replaces a masked result. Deterministic (no clock, no
/// counters that change later) so a masked message is byte-identical on every
/// later turn.
///
/// Only a read-only tool's results are ever masked ([`repeatable_tools`]), so
/// the placeholder can tell the model that calling it again is safe.
pub fn placeholder(tool_name: &str, omitted_chars: usize, keep_turns: usize) -> String {
    format!(
        "[observation masked: output of `{tool_name}` ({omitted_chars} chars) omitted from \
         context because it is older than the last {keep_turns} turns. The tool is \
         read-only: call it again if you need this output.]"
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

/// True for a user message holding tool results and nothing else — what the
/// agent persists for each answered call.
fn is_tool_results_only(message: &Message) -> bool {
    message.role == Role::User
        && !message.content.is_empty()
        && message
            .content
            .iter()
            .all(|c| matches!(c, MessageContent::ToolResponse(_)))
}

/// True for an assistant message shaped like one of the agent's per-call
/// splits of a model response: tool requests plus, at most, the reasoning the
/// agent clones onto each. A response's text only ever rides on its first
/// message, so an assistant message with text always starts a new turn.
fn is_split_call_shape(message: &Message) -> bool {
    message.role == Role::Assistant
        && message.is_tool_call()
        && message.content.iter().all(|c| {
            matches!(
                c,
                MessageContent::ToolRequest(_)
                    | MessageContent::Thinking(_)
                    | MessageContent::RedactedThinking(_)
            )
        })
}

/// The model turn each message belongs to (`None` before the first assistant
/// message), and how many model turns the conversation holds.
///
/// The agent persists ONE model response that made N tool calls as N
/// `assistant(call) → user(result)` pairs (the per-request loop after tool
/// execution in `agent.rs`), and nothing merges them back: they alternate with
/// "tool"-role user messages. An assistant message continues the previous
/// model turn when all of these hold:
///
/// - only tool-result messages sit between it and the previous assistant
///   message (no user text, no nudge — the loop never left the response),
/// - it is split-shaped ([`is_split_call_shape`]), and
/// - its result message carries the same `created` stamp as the previous
///   pair's results. The agent creates every result message of one response
///   together, in one pass before any tool runs; the next response's results
///   are created only once that response has arrived.
///
/// `created` has one-second resolution, so two separate responses that both
/// arrive within the same second read as one turn. That errs toward masking
/// LESS, never more. Grouping depends only on a message, what precedes it and
/// its own result, so appending turns never regroups earlier messages and the
/// masked prefix stays byte-stable between block advances.
fn model_turns(messages: &[Message]) -> (Vec<Option<usize>>, usize) {
    let mut turn_of = Vec::with_capacity(messages.len());
    let mut turns = 0usize;
    // `created` of the tool results since the last assistant message, kept only
    // while nothing but tool results has followed that message.
    let mut results_created: Option<i64> = None;
    for (i, message) in messages.iter().enumerate() {
        if message.role == Role::Assistant {
            let own_results_created = messages
                .get(i + 1)
                .filter(|next| is_tool_results_only(next))
                .map(|next| next.created);
            let continues = results_created.is_some()
                && is_split_call_shape(message)
                && own_results_created == results_created;
            if !continues {
                turns += 1;
            }
            results_created = None;
            turn_of.push(Some(turns - 1));
            continue;
        }
        if is_tool_results_only(message) {
            // The first result message after an assistant message sets the
            // stamp; a later one in the same run must agree with it.
            let previous_assistant = i > 0 && messages[i - 1].role == Role::Assistant;
            results_created = match results_created {
                None if previous_assistant => Some(message.created),
                Some(created) if created == message.created => Some(created),
                _ => None,
            };
        } else {
            results_created = None;
        }
        turn_of.push(turns.checked_sub(1));
    }
    (turn_of, turns)
}

/// Replace the bodies of tool results that answer the oldest
/// [`mask_boundary`] model turns with a [`placeholder`]. Pure: same input,
/// same output. Returns the input unchanged when nothing qualifies.
///
/// A tool result "answers" model turn `k` (0-based) when it sits in a user
/// message after the first assistant message of turn `k` and before the first
/// assistant message of turn `k + 1` (see [`model_turns`]). Only results of
/// the tools named in `repeatable` ([`repeatable_tools`]) are ever masked; a
/// result whose call cannot be found (so its tool is unknown) is kept too.
/// Errors (`Err`) and results under [`MIN_MASKABLE_CHARS`] are kept in full; a
/// successful result that reported `is_error` keeps that flag on its
/// placeholder.
///
/// Which results are masked depends on `repeatable` as well as the turn count,
/// so the masked prefix stays byte-stable between block advances only while
/// the tool set does. It changes when an extension comes or goes.
pub fn mask_old_observations(
    messages: &[Message],
    keep_turns: usize,
    repeatable: &HashSet<&str>,
) -> Vec<Message> {
    let (turn_of, turns) = model_turns(messages);
    let boundary = mask_boundary(turns, keep_turns);
    if boundary == 0 || repeatable.is_empty() {
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
    for (message, &turn) in messages.iter().zip(turn_of.iter()) {
        if message.role == Role::Assistant {
            out.push(message.clone());
            continue;
        }
        // Results in this message answer model turn `turn`.
        let answers_masked_turn = turn.is_some_and(|t| t < boundary);
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
            // Only a read-only tool's output can be had again for free.
            let Some(name) = tool_names.get(response.id.as_str()).copied() else {
                continue;
            };
            if !repeatable.contains(name) {
                continue;
            }
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
    use rmcp::model::{CallToolRequestParams, ToolAnnotations};

    /// The fixtures' tool: a read-only one, so every old result of it is
    /// maskable and the windowing tests can be about WHICH results are old
    /// enough. Which tools may be masked at all is tested on its own below.
    const READ_TOOL: &str = "developer__read";

    fn repeatable() -> HashSet<&'static str> {
        HashSet::from([READ_TOOL])
    }

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
                    .with_tool_request(&id, Ok(CallToolRequestParams::new(READ_TOOL))),
            );
            messages.push(Message::user().with_tool_response(
                &id,
                Ok(CallToolResult::success(vec![Content::text(big(&id))])),
            ));
        }
        messages
    }

    /// `steps` model responses after an opening user message, each making
    /// `calls` parallel tool calls, persisted the way the agent does: one
    /// `assistant(call) → user(result)` pair per call, the response's text on
    /// its first message, its reasoning cloned onto every one, and every
    /// result of one response sharing a `created` stamp.
    fn parallel_session(steps: usize, calls: usize) -> Vec<Message> {
        let mut messages = vec![Message::user().with_text("fix the bug")];
        for s in 0..steps {
            for c in 0..calls {
                let id = format!("call_{s}_{c}");
                let mut request = Message::assistant();
                if c == 0 {
                    request = request.with_text(format!("step {s}"));
                }
                messages.push(
                    request
                        .with_thinking(format!("plan {s}"), "")
                        .with_tool_request(&id, Ok(CallToolRequestParams::new(READ_TOOL))),
                );
                let mut result = Message::user().with_tool_response(
                    &id,
                    Ok(CallToolResult::success(vec![Content::text(big(&id))])),
                );
                result.created = s as i64;
                messages.push(result);
            }
        }
        messages
    }

    /// One bare tool round: an assistant call (with optional text) and its
    /// result, stamped `created`.
    fn round(id: &str, text: Option<&str>, created: i64) -> Vec<Message> {
        let mut request = Message::assistant();
        if let Some(text) = text {
            request = request.with_text(text);
        }
        let mut result = Message::user().with_tool_response(
            id,
            Ok(CallToolResult::success(vec![Content::text(big(id))])),
        );
        result.created = created;
        vec![
            request.with_tool_request(id, Ok(CallToolRequestParams::new(READ_TOOL))),
            result,
        ]
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
        assert_eq!(
            mask_old_observations(&messages, 10, &repeatable()),
            messages
        );
        let long = session(50);
        assert_eq!(mask_old_observations(&long, 0, &repeatable()), long);
    }

    #[test]
    fn old_results_become_placeholders_naming_tool_and_size() {
        let messages = session(20);
        let masked = mask_old_observations(&messages, 10, &repeatable());
        assert_eq!(masked.len(), messages.len());

        // User message after assistant turn k sits at index 2k + 2.
        let first = result_text(&masked[2]).unwrap();
        assert!(first.starts_with("[observation masked: output of `developer__read`"));
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
        let masked = mask_old_observations(&session(35), 10, &repeatable());
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
            let now = mask_old_observations(at(t), 10, &repeatable());
            let next = mask_old_observations(at(t + 1), 10, &repeatable());
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
                    .with_tool_request(&id, Ok(CallToolRequestParams::new(READ_TOOL))),
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
            // 25 SEQUENTIAL turns: each result is stamped when its own response
            // arrived. Bare calls whose results share one stamp are one
            // response's parallel calls (see `model_turns`), which is not what
            // this fixture means.
            let mut result = Message::user().with_tool_response(&id, response);
            result.created = t as i64;
            messages.push(result);
        }
        let masked = mask_old_observations(&messages, 10, &repeatable());
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

    /// A result whose call cannot be found has no known tool, so nothing says
    /// it is safe to call again: it is kept in full. (This test used to
    /// expect a placeholder naming a generic `tool`, which is the unsafe
    /// masking the read-only rule removes.)
    #[test]
    fn results_of_unknown_tool_ids_are_kept() {
        let mut messages = session(20);
        // Orphan the first result's name by renaming its request id.
        if let MessageContent::ToolRequest(r) = &mut messages[1].content[1] {
            r.id = "renamed".into();
        }
        let masked = mask_old_observations(&messages, 10, &repeatable());
        assert_eq!(masked[2], messages[2], "orphaned result kept in full");
        // Its neighbours in the masked block are still masked.
        assert!(result_text(&masked[4])
            .unwrap()
            .starts_with("[observation masked"));
    }

    /// The defect: 5 responses of 4 parallel calls are 20 assistant messages
    /// but only 5 model turns, so nothing is old enough to mask. Counting
    /// assistant messages masked the first 10 calls.
    #[test]
    fn parallel_calls_of_one_response_are_one_turn() {
        let messages = parallel_session(5, 4);
        assert_eq!(model_turns(&messages).1, 5);
        assert_eq!(
            mask_old_observations(&messages, 10, &repeatable()),
            messages
        );
    }

    /// 20 responses of 4 parallel calls: the oldest 10 TURNS are masked and the
    /// newest 10 are sent whole — every call of a turn treated alike.
    #[test]
    fn the_window_keeps_whole_recent_turns_when_calls_run_in_parallel() {
        let (steps, calls) = (20, 4);
        let messages = parallel_session(steps, calls);
        let masked = mask_old_observations(&messages, 10, &repeatable());
        for s in 0..steps {
            for c in 0..calls {
                // Result of call k sits at 2k + 2 (after the opening message).
                let at = 2 * (s * calls + c) + 2;
                let text = result_text(&masked[at]).unwrap();
                if s < 10 {
                    assert!(text.starts_with("[observation masked"), "step {s} call {c}");
                } else {
                    assert_eq!(text, big(&format!("call_{s}_{c}")), "step {s} call {c}");
                }
            }
        }
    }

    /// Only a bare call whose result was created with the previous call's
    /// result, with nothing but results in between, continues a turn.
    #[test]
    fn only_a_split_of_the_same_response_continues_a_turn() {
        let turns = |rounds: Vec<Vec<Message>>| {
            let mut messages = vec![Message::user().with_text("go")];
            messages.extend(rounds.into_iter().flatten());
            model_turns(&messages).1
        };
        // Same stamp, bare calls, nothing in between: one response.
        assert_eq!(turns(vec![round("a", None, 7), round("b", None, 7)]), 1);
        // A later stamp: the next response.
        assert_eq!(turns(vec![round("a", None, 7), round("b", None, 8)]), 2);
        // Text on the call: a new response (the agent's splits never carry it).
        assert_eq!(
            turns(vec![round("a", None, 7), round("b", Some("next"), 7)]),
            2
        );
        // The user (or a nudge) spoke in between: a new turn.
        let mut interrupted = round("a", None, 7);
        interrupted.push(Message::user().with_text("also check the tests"));
        assert_eq!(turns(vec![interrupted, round("b", None, 7)]), 2);
    }

    /// Growing the conversation never regroups the messages already in it, so
    /// the masked prefix can stay byte-stable. (Requests end on a user
    /// message; a split is persisted with all its results at once.)
    #[test]
    fn appending_turns_never_regroups_earlier_messages() {
        let full = parallel_session(12, 3);
        let (all, _) = model_turns(&full);
        for end in (1..=full.len()).filter(|&end| full[end - 1].role == Role::User) {
            let (prefix, _) = model_turns(&full[..end]);
            assert_eq!(prefix[..], all[..end], "end={end}");
        }
    }

    /// The block-stability property of `masked_prefix_is_stable_between_block_boundaries`,
    /// with parallel calls: the prefix changes exactly when the TURN count
    /// crosses a block boundary, not when the call count does.
    #[test]
    fn parallel_masked_prefix_is_stable_between_block_boundaries() {
        let calls = 3;
        let full = parallel_session(36, calls);
        let at = |steps: usize| &full[..1 + 2 * calls * steps];
        for t in 20..35 {
            let now = mask_old_observations(at(t), 10, &repeatable());
            let next = mask_old_observations(at(t + 1), 10, &repeatable());
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

    /// Only read-only tools are masked, so the placeholder can say that
    /// calling the tool again is safe, and nothing else.
    #[test]
    fn the_placeholder_says_the_masked_call_is_safe_to_repeat() {
        let text = placeholder(READ_TOOL, 5_000, 10);
        assert!(text.starts_with("[observation masked: output of `developer__read` (5000 chars)"));
        assert!(text.contains("older than the last 10 turns"));
        assert!(text.contains("The tool is read-only: call it again"));
    }

    /// The coordinator case: a delegated worker's report, then 25 turns of
    /// read-only reads. The reads' old results are masked. The report, from a
    /// tool that is not declared read-only, never is: it can be the only
    /// record of work that cannot be repeated for free. Before the read-only
    /// rule, masking replaced it after 20 turns like everything else.
    #[test]
    fn results_of_tools_not_declared_read_only_are_never_masked() {
        let tool_round = |id: &str, tool: &str, created: i64| {
            let mut result = Message::user().with_tool_response(
                id,
                Ok(CallToolResult::success(vec![Content::text(big(id))])),
            );
            result.created = created;
            vec![
                Message::assistant()
                    .with_text(format!("turn {created}"))
                    .with_tool_request(id, Ok(CallToolRequestParams::new(tool.to_string()))),
                result,
            ]
        };
        let mut messages = vec![Message::user().with_text("coordinate the fix")];
        messages.extend(tool_round("dispatch", "orchestrator__delegate", 0));
        for t in 1..=25 {
            messages.extend(tool_round(&format!("read_{t}"), READ_TOOL, t));
        }
        assert_eq!(model_turns(&messages).1, 26);

        let masked = mask_old_observations(&messages, 10, &repeatable());
        assert_eq!(
            masked[2], messages[2],
            "the worker's report is kept in full"
        );
        // Turns 1..10 are reads in the masked block; 10.. are recent.
        for t in 1..=25 {
            let text = result_text(&masked[2 * t + 2]).unwrap();
            if t < 10 {
                assert!(text.starts_with("[observation masked"), "turn {t}");
            } else {
                assert_eq!(text, big(&format!("read_{t}")), "turn {t}");
            }
        }
        // With no read-only tool at all, nothing is masked.
        assert_eq!(
            mask_old_observations(&messages, 10, &HashSet::new()),
            messages
        );
    }

    /// Read-only means `readOnlyHint: true` and nothing less: `false`, an
    /// annotation without the hint, and no annotation at all (the MCP default
    /// is not read-only) all keep a tool's results in full.
    #[test]
    fn repeatable_tools_are_those_declared_read_only() {
        let tool = |name: &'static str, annotations: Option<ToolAnnotations>| {
            let plain = Tool::new(name, "a tool", serde_json::Map::new());
            match annotations {
                Some(annotations) => plain.annotate(annotations),
                None => plain,
            }
        };
        let tools = [
            tool(
                "developer__read",
                Some(ToolAnnotations::new().read_only(true)),
            ),
            tool(
                "developer__shell",
                Some(ToolAnnotations::new().read_only(false)),
            ),
            tool(
                "notes__write",
                Some(ToolAnnotations::with_title("Write").destructive(false)),
            ),
            tool("orchestrator__delegate", None),
        ];
        assert_eq!(
            repeatable_tools(tools.iter()),
            HashSet::from(["developer__read"])
        );
    }
}
