use crate::agents::extension_manager::ExtensionManager;
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::{fix_conversation, Conversation};
use rmcp::model::Role;
use std::path::Path;

// Test-only utility. Do not use in production code. No `test` directive due to call outside crate.
thread_local! {
    pub static SKIP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub async fn inject_moim(
    session_id: &str,
    conversation: Conversation,
    extension_manager: &ExtensionManager,
    working_dir: &Path,
) -> Conversation {
    if SKIP.with(|f| f.get()) {
        return conversation;
    }

    if let Some(moim) = extension_manager
        .collect_moim(session_id, working_dir)
        .await
    {
        let before_len = conversation.messages().len();
        let messages = with_moim_inserted(&conversation, moim);
        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        let has_unexpected_issues = issues.iter().any(|issue| !is_expected_moim_issue(issue));

        if has_unexpected_issues {
            tracing::warn!(
                session_id = %session_id,
                messages_before = before_len,
                messages_after = fixed.messages().len(),
                issues = ?issues,
                "MOIM injection caused unexpected issues; dropping the injection"
            );
            return conversation;
        }

        // ── Why this is logged, and why not at warn ──────────────────────
        //
        // The 2026-08-24 health review flagged four "Merged consecutive
        // messages" lines in session 20260823_4 as possible silent corruption
        // of the model's message sequence. It is not: MOIM CAUSES this merge on
        // purpose. It inserts a synthetic user message immediately before the
        // last assistant message; whenever the slot before that is also a user
        // message — the ordinary `user → assistant` shape — the two are merged,
        // which is exactly how the MOIM text is meant to reach the model,
        // appended to the user's own turn. `test_moim_injection_before_assistant`
        // asserts that merged content. Nothing is dropped: `merge_consecutive_messages`
        // extends the earlier message's content with the later one's, in order.
        //
        // So logging every one of these at warn would fire on every normal turn,
        // which is how the original storm-of-noise problem gets recreated. The
        // expected case is debug and carries the session id + counts so the
        // review can still see it. What IS worth a warning is more merges than
        // MOIM's single insertion can explain — that means the conversation
        // arrived already holding consecutive same-role messages, which is the
        // corruption the review was actually looking for.
        //
        // (MOIM now rides at the tail — see `with_moim_inserted` — so only its
        // fallback placement, for a conversation that does not end on a user
        // message, can still cause the merge described above.)
        let merge_count = issues
            .iter()
            .filter(|issue| issue.contains("Merged consecutive"))
            .count();
        let after_len = fixed.messages().len();

        if merge_count > MAX_MERGES_CAUSED_BY_MOIM {
            tracing::warn!(
                session_id = %session_id,
                merge_count,
                messages_before = before_len,
                messages_after = after_len,
                issues = ?issues,
                "MOIM injection merged more consecutive messages than its own insertion can \
                 explain — the conversation already held consecutive same-role messages"
            );
        } else if merge_count > 0 {
            tracing::debug!(
                session_id = %session_id,
                merge_count,
                messages_before = before_len,
                messages_after = after_len,
                "MOIM injection merged into the adjacent same-role message (expected)"
            );
        }

        return fixed;
    }
    conversation
}

/// Place the MOIM text at the very END of the request. Every agent-loop turn
/// ends on a user message — the user's words or the tool results just
/// returned — and the two take MOIM differently:
///
/// - The user's words: MOIM is appended as the last content block of that
///   message.
/// - Tool results: MOIM goes in its OWN user message right after them, never
///   inside the tool-result message. A text block inside a tool-result message
///   is a shape the OpenAI chat/completions formatter cannot order: it turns
///   the message into a `user` text item plus `role: tool` items, and the
///   `assistant(tool_calls)` → `tool` adjacency OpenAI-compatible APIs enforce
///   broke on every tool-loop turn (HTTP 400 from OpenAI, DeepSeek, Azure and
///   xAI; strict local chat templates fail the same way). `effective_role`
///   calls the result message "tool", so `fix_conversation` never merges MOIM
///   into it. The request then holds two user messages in a row, and each
///   format has to accept that. Chat/completions, Responses and Databricks
///   send it as `tool` item(s) followed by a `user` item, which is legal. The
///   Anthropic API folds the two into one turn with the `tool_result` blocks
///   first. Bedrock Converse rejects roles that do not alternate, so its
///   formatter folds consecutive same-role messages itself
///   (`formats::bedrock::to_bedrock_messages`).
///
/// Either way MOIM is the last thing in the request and nothing before it
/// changes, so the cacheable prefix stays byte-stable.
///
/// It used to be inserted in front of the last assistant turn, i.e. merged
/// into the user message TWO back from the end. MOIM carries a clock and a
/// context-fill line, and the slot it occupied moved forward every turn, so
/// the message it had sat in changed on the next turn: on Anthropic no
/// message-level cache breakpoint could ever be read back (measured
/// 2026-09-25: cache reads flat at the ~8.9k-token tools+system prefix while
/// the whole history was re-written at 1.25x each turn), and on the
/// automatic-prefix providers the last two messages were always re-billed.
/// At the tail it is the only thing that differs from the previous turn.
///
/// A conversation whose last visible message is not a user message keeps the old
/// placement (before the last assistant turn).
///
/// Adjacent text/thinking parts are coalesced *first* so historical
/// streamed deltas do not show up as MOIM issues. Session 20260827_1's
/// issue list grew by ~2 items per turn because `fix_conversation` reported
/// one `"Merged text content"` per un-coalesced assistant message in the
/// live in-memory history — not because a list was persisted or a merge ran
/// recursively. Coalescing here keeps that list at MOIM's own insertion.
fn with_moim_inserted(conversation: &Conversation, moim: String) -> Vec<Message> {
    let mut messages: Vec<Message> = conversation
        .messages()
        .iter()
        .cloned()
        .map(Message::coalesce_adjacent_text_and_thinking)
        .collect();
    let last_visible = messages.iter().rposition(|m| m.is_agent_visible());
    match last_visible {
        // Messages after it (if any) are not agent-visible and are filtered out
        // before the provider call, so this IS the tail the model sees.
        Some(i) if messages[i].role == Role::User && messages[i].is_tool_response() => {
            messages.insert(i + 1, Message::user().with_text(moim));
        }
        Some(i) if messages[i].role == Role::User => {
            messages[i].content.push(MessageContent::text(moim));
        }
        _ => {
            let idx = messages
                .iter()
                .rposition(|m| m.role == Role::Assistant)
                .unwrap_or(0);
            messages.insert(idx, Message::user().with_text(moim));
        }
    }
    messages
}

/// MOIM inserts exactly ONE synthetic user message, so it can create at most
/// one consecutive-same-role pair. Anything beyond this was already in the
/// conversation before MOIM touched it.
const MAX_MERGES_CAUSED_BY_MOIM: usize = 1;

/// `fix_conversation` reports every coalesce it performs. Most of those are
/// lossless and happen on ordinary turns: MOIM's own insertion sits next to the
/// last user message, streamed assistant text often has trailing whitespace,
/// and adjacent text parts inside one message are joined in order.
///
/// Session 20260827_1 logged twelve `dropping the injection` warnings because
/// `"Merged text content"` was treated as unexpected. That dropped Top-of-Mind
/// on every turn of a healthy conversation. These strings are the expected
/// set; anything else still drops the injection.
fn is_expected_moim_issue(issue: &str) -> bool {
    issue.contains("Merged consecutive user messages")
        || issue.contains("Merged consecutive assistant messages")
        || issue.contains("Added placeholder to empty tool result")
        || issue.contains("Added placeholder user message to empty conversation")
        || issue.contains("Merged text content")
        || issue.contains("Trimmed trailing whitespace from assistant message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use std::path::PathBuf;

    /// The shape the 2026-08-24 health review saw in session 20260823_4.
    ///
    /// An ordinary `user → assistant` conversation: MOIM inserts its synthetic
    /// user message directly before the last assistant message, which puts it
    /// next to the real user message. The merge that follows is MOIM's own doing
    /// and it is LOSSLESS — the user's words survive verbatim, in order, with the
    /// MOIM text appended. This test is the proof that "Merged consecutive
    /// messages" is not silent corruption.
    #[tokio::test]
    async fn moim_merge_is_lossless_for_the_reviewed_shape() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("first question"),
            Message::assistant().with_text("first answer"),
            Message::user().with_text("second question"),
            Message::assistant().with_text("second answer"),
        ]);
        let before_roles: Vec<Role> = conv.messages().iter().map(|m| m.role.clone()).collect();

        let result = inject_moim("20260823_4", conv, &em, &working_dir).await;
        let msgs = result.messages();

        // Roles still strictly alternate — no consecutive same-role pair is left
        // for the provider to reject.
        for pair in msgs.windows(2) {
            assert_ne!(
                pair[0].role, pair[1].role,
                "roles must alternate after MOIM injection"
            );
        }
        assert_eq!(
            before_roles.len(),
            msgs.len(),
            "the merge must not change the message count for this shape"
        );

        // Every original user word is still present, in order.
        let all_text: String = msgs
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        let first = all_text
            .find("first question")
            .expect("first question kept");
        let second = all_text
            .find("second question")
            .expect("second question kept");
        assert!(first < second, "user turns must stay in order");
        assert!(all_text.contains("first answer"));
        assert!(all_text.contains("second answer"));
    }

    #[tokio::test]
    async fn test_moim_injection_before_assistant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi"),
            Message::user().with_text("Bye"),
        ]);
        let result = inject_moim("test-session-id", conv, &em, &working_dir).await;
        let msgs = result.messages();

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content[0].as_text().unwrap(), "Hello");
        assert_eq!(msgs[1].content[0].as_text().unwrap(), "Hi");

        // MOIM rides at the tail of the newest user message, as its own block…
        assert_eq!(msgs[2].content.len(), 2);
        assert_eq!(msgs[2].content[0].as_text().unwrap(), "Bye");
        let moim = msgs[2].content[1].as_text().unwrap();
        assert!(moim.starts_with("<info-msg>"));
        assert!(moim.contains("Working directory: /test/dir"));

        // …and the earlier history is untouched, so it replays byte-identically
        // as next turn's cached prefix.
        assert_eq!(msgs[0].content.len(), 1);
    }

    /// Prefix stability across two consecutive turns: every message of turn
    /// N's request except the MOIM block reappears unchanged, in place, at the
    /// front of turn N+1's request.
    #[tokio::test]
    async fn moim_never_touches_the_history_a_later_turn_replays() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");
        let tool_round = |id: &str| {
            vec![
                Message::assistant()
                    .with_tool_request(id, Ok(CallToolRequestParams::new("search"))),
                Message::user().with_tool_response(
                    id,
                    Ok(rmcp::model::CallToolResult::success(vec![
                        rmcp::model::Content::text(format!("result {id}")),
                    ])),
                ),
            ]
        };
        let mut history = vec![Message::user().with_text("Search")];
        history.extend(tool_round("s1"));
        let turn_n = inject_moim(
            "s",
            Conversation::new_unvalidated(history.clone()),
            &em,
            &working_dir,
        )
        .await;
        history.extend(tool_round("s2"));
        let turn_n1 = inject_moim(
            "s",
            Conversation::new_unvalidated(history.clone()),
            &em,
            &working_dir,
        )
        .await;

        let n = turn_n.messages();
        let n1 = turn_n1.messages();
        // Turn N minus its trailing MOIM block…
        // (Both turns end on a tool result, so MOIM is its own trailing
        // message there, not a block inside the result message.)
        let mut n_without_moim = n.clone();
        let tail = n_without_moim.pop().unwrap();
        assert_eq!(tail.role, Role::User);
        assert!(
            tail.content.len() == 1
                && tail.content[0]
                    .as_text()
                    .is_some_and(|t| t.starts_with("<info-msg>")),
            "the tail must be MOIM alone: {tail:?}"
        );
        // …is exactly the head of turn N+1.
        assert_eq!(&n1[..n_without_moim.len()], &n_without_moim[..]);
        // Exactly one MOIM block per request, always in the last message.
        for msgs in [n, n1] {
            let count = msgs
                .iter()
                .flat_map(|m| m.content.iter())
                .filter(|c| c.as_text().is_some_and(|t| t.contains("<info-msg>")))
                .count();
            assert_eq!(count, 1);
            assert!(conversation_has_moim(&Conversation::new_unvalidated(vec![
                msgs.last().unwrap().clone()
            ])));
        }
    }

    #[tokio::test]
    async fn test_moim_injection_no_assistant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![Message::user().with_text("Hello")]);
        let result = inject_moim("test-session-id", conv, &em, &working_dir).await;

        assert_eq!(result.messages().len(), 1);

        let merged_content = result.messages()[0]
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert!(merged_content.contains("Hello"));
        assert!(merged_content.contains("<info-msg>"));
        assert!(merged_content.contains("Working directory: /test/dir"));
    }

    #[tokio::test]
    async fn test_moim_with_tool_calls() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Search for something"),
            Message::assistant()
                .with_text("I'll search for you")
                .with_tool_request("search_1", Ok(CallToolRequestParams::new("search"))),
            Message::user()
                .with_tool_response("search_1", Ok(rmcp::model::CallToolResult::success(vec![]))),
            Message::assistant()
                .with_text("I need to search more")
                .with_tool_request("search_2", Ok(CallToolRequestParams::new("search"))),
            Message::user()
                .with_tool_response("search_2", Ok(rmcp::model::CallToolResult::success(vec![]))),
        ]);

        let result = inject_moim("test-session-id", conv, &em, &working_dir).await;
        let msgs = result.messages();

        // MOIM is its own user message right AFTER the last tool result, never
        // a block inside it: the OpenAI chat format emits a tool-result
        // message's text ahead of its `role: tool` item, and the request 400s.
        assert_eq!(msgs.len(), 6);

        let results = &msgs[4];
        assert!(
            results
                .content
                .iter()
                .all(|c| c.as_tool_response().is_some()),
            "the tool-result message must carry only tool results: {results:?}"
        );
        let moim_msg = &msgs[5];
        assert_eq!(moim_msg.role, Role::User);
        assert_eq!(moim_msg.content.len(), 1);
        assert!(
            moim_msg.content[0]
                .as_text()
                .is_some_and(|t| t.starts_with("<info-msg>")),
            "MOIM should be the last message: {moim_msg:?}"
        );
        assert!(
            !msgs[..5].iter().any(|m| m
                .content
                .iter()
                .any(|c| c.as_text().is_some_and(|t| t.contains("<info-msg>")))),
            "MOIM must not touch the history"
        );
    }

    /// End to end on the chat/completions wire, where the old shape broke: a
    /// tool loop with MOIM injected goes through the OpenAI formatter for a
    /// CurrentTurnOnly family (GLM), with a volatile system tail. Every
    /// `tool_calls` must be answered in place, MOIM must follow the results,
    /// and the in-flight call must keep its reasoning.
    #[tokio::test]
    async fn a_tool_loop_with_moim_reaches_the_chat_completions_wire_in_order() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Search for something"),
            Message::assistant()
                .with_thinking("LIVE REASONING", "")
                .with_tool_request("search_1", Ok(CallToolRequestParams::new("search"))),
            Message::user().with_tool_response(
                "search_1",
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("one hit"),
                ])),
            ),
        ]);
        let with_moim = inject_moim("s", conv, &em, &working_dir).await;
        assert!(conversation_has_moim(&with_moim));

        let payload = chat_completions_wire(with_moim.messages(), "glm-5.3", "- 1 worker");
        let wire = payload.as_array().unwrap();
        let roles: Vec<&str> = wire.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "user", "system"],
            "{wire:#?}"
        );
        assert_eq!(wire[2]["tool_calls"][0]["id"], "search_1");
        assert_eq!(wire[3]["tool_call_id"], "search_1");
        assert!(wire[4]["content"]
            .as_str()
            .is_some_and(|t| t.starts_with("<info-msg>")));
        assert_eq!(wire[2]["reasoning_content"], "LIVE REASONING");
    }

    /// Prefix stability on the chat/completions wire across two tool turns:
    /// turn N's messages minus its tail (the MOIM message and the volatile
    /// system block) are exactly the head of turn N+1's.
    #[tokio::test]
    async fn moim_keeps_the_chat_completions_prefix_stable_across_tool_turns() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");
        let tool_round = |id: &str| {
            vec![
                Message::assistant()
                    .with_tool_request(id, Ok(CallToolRequestParams::new("search"))),
                Message::user().with_tool_response(
                    id,
                    Ok(rmcp::model::CallToolResult::success(vec![
                        rmcp::model::Content::text(format!("result {id}")),
                    ])),
                ),
            ]
        };
        let mut history = vec![Message::user().with_text("Search")];
        history.extend(tool_round("s1"));
        let turn_n = inject_moim(
            "s",
            Conversation::new_unvalidated(history.clone()),
            &em,
            &working_dir,
        )
        .await;
        history.extend(tool_round("s2"));
        let turn_n1 = inject_moim(
            "s",
            Conversation::new_unvalidated(history.clone()),
            &em,
            &working_dir,
        )
        .await;

        let a = chat_completions_wire(turn_n.messages(), "deepseek-chat", "- worker running");
        let b = chat_completions_wire(turn_n1.messages(), "deepseek-chat", "- worker done");
        let (a, b) = (a.as_array().unwrap(), b.as_array().unwrap());
        assert!(a[a.len() - 2]["content"]
            .as_str()
            .is_some_and(|t| t.starts_with("<info-msg>")));
        let shared = &a[..a.len() - 2];
        assert_eq!(&b[..shared.len()], shared);
    }

    /// Bedrock Converse rejects a conversation whose roles do not alternate.
    /// MOIM as its own message after the tool results is two user messages in
    /// a row, so on Bedrock every tool-loop turn failed until its formatter
    /// folded them. They reach Converse as one user turn, tool result first
    /// and MOIM last.
    #[cfg(feature = "aws-providers")]
    #[tokio::test]
    async fn a_tool_loop_with_moim_alternates_roles_on_bedrock_converse() {
        use aws_sdk_bedrockruntime::types::ContentBlock;

        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Search for something"),
            Message::assistant()
                .with_tool_request("search_1", Ok(CallToolRequestParams::new("search"))),
            Message::user().with_tool_response(
                "search_1",
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("one hit"),
                ])),
            ),
        ]);
        let with_moim = inject_moim("s", conv, &em, &working_dir).await;
        assert!(conversation_has_moim(&with_moim));

        let visible: Vec<&Message> = with_moim
            .messages()
            .iter()
            .filter(|m| m.is_agent_visible())
            .collect();
        let converse =
            crate::providers::formats::bedrock::to_bedrock_messages(&visible, 0).unwrap();
        let roles: Vec<_> = converse.iter().map(|m| m.role.clone()).collect();
        assert!(roles.windows(2).all(|w| w[0] != w[1]), "{roles:?}");
        assert_eq!(converse.len(), 3, "{converse:#?}");
        let tail = &converse[2].content;
        assert!(matches!(tail.first(), Some(ContentBlock::ToolResult(_))));
        assert!(
            matches!(tail.last(), Some(ContentBlock::Text(t)) if t.starts_with("<info-msg>")),
            "{tail:?}"
        );
    }

    /// The `messages` array a chat/completions provider receives.
    fn chat_completions_wire(
        messages: &[Message],
        model: &str,
        live_status: &str,
    ) -> serde_json::Value {
        let payload = crate::providers::formats::openai::create_request_split(
            &crate::model::ModelConfig::new_or_fail(model),
            &crate::cost_router::cache::SystemPromptParts::new(
                "You are a coding agent.\n".to_string(),
                format!("\n# Live Status\n{live_status}\n"),
            ),
            messages,
            &[],
            &crate::providers::utils::ImageFormat::OpenAi,
            false,
        )
        .unwrap();
        payload["messages"].clone()
    }

    fn conversation_has_moim(conv: &Conversation) -> bool {
        conv.messages().iter().any(|m| {
            m.content
                .iter()
                .any(|c| c.as_text().is_some_and(|t| t.contains("<info-msg>")))
        })
    }

    /// Session 20260827_1's shape: the current turn ends on a user message,
    /// the previous assistant has adjacent streamed text parts (coalesced as
    /// `Merged text content`), and MOIM sits next to the prior user message
    /// (`Merged consecutive user messages`). Coalesce only runs on assistant
    /// content; a fixture that ends on assistant also trips
    /// `Removed trailing assistant message` and drops the injection.
    #[tokio::test]
    async fn moim_keeps_injection_when_text_content_is_merged() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("first question"),
            Message::assistant()
                .with_text("first")
                .with_text(" answer  "),
            Message::user().with_text("second question"),
        ]);

        let issues = {
            let mut messages = conv.messages().clone();
            let idx = messages
                .iter()
                .rposition(|m| m.role == Role::Assistant)
                .unwrap();
            messages.insert(idx, Message::user().with_text("placeholder"));
            let (_, issues) =
                crate::conversation::fix_conversation(Conversation::new_unvalidated(messages));
            issues
        };
        assert!(
            issues.iter().any(|i| i.contains("Merged text content")),
            "fixture must reproduce the 20260827_1 coalescing issue, got {issues:?}"
        );
        assert!(
            issues
                .iter()
                .any(|i| i.contains("Merged consecutive user messages")),
            "fixture must reproduce the consecutive-user merge, got {issues:?}"
        );
        assert!(
            issues.iter().all(|i| is_expected_moim_issue(i)),
            "every issue in the 20260827_1 list must be expected, got {issues:?}"
        );

        let result = inject_moim("20260827_1", conv, &em, &working_dir).await;
        assert!(
            conversation_has_moim(&result),
            "MOIM must not drop the injection for Merged text content"
        );
        let all_text: String = result
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all_text.contains("first question"));
        assert!(all_text.contains("second question"));
        assert!(all_text.contains("first") && all_text.contains("answer"));
    }

    /// Growing the conversation must not start dropping MOIM. Each turn of
    /// session 20260827_1 re-ran the sanitizer on a longer history that still
    /// ended on the user's current message. Ten turns with split assistant
    /// text and trailing whitespace is the shape that used to warn.
    #[tokio::test]
    async fn moim_keeps_injection_across_ten_turns_with_coalescing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let mut messages = vec![Message::user().with_text("question 0")];
        for turn in 0..10 {
            let conv = Conversation::new_unvalidated(messages.clone());
            let before = conv.messages().len();
            let result = inject_moim("20260827_1", conv, &em, &working_dir).await;
            assert!(
                conversation_has_moim(&result),
                "turn {turn} (messages_before={before}) dropped MOIM"
            );
            messages.push(
                Message::assistant()
                    .with_text(format!("answer {turn} part a"))
                    .with_text(" part b  "),
            );
            messages.push(Message::user().with_text(format!("question {}", turn + 1)));
        }
    }

    /// The health-watch "runaway loop": issue count growing with turn count.
    /// That list is recomputed each turn from un-coalesced history, not
    /// persisted. Pre-coalescing must keep `"Merged text content"` at zero
    /// even as the conversation lengthens.
    #[test]
    fn moim_issue_list_does_not_grow_with_split_text_history() {
        let mut messages = vec![Message::user().with_text("question 0")];
        let mut last_text_merges = None;
        for turn in 0..10 {
            messages.push(
                Message::assistant()
                    .with_text(format!("answer {turn} part a"))
                    .with_text(" part b  "),
            );
            messages.push(Message::user().with_text(format!("question {}", turn + 1)));
            let conv = Conversation::new_unvalidated(messages.clone());
            let prepared = with_moim_inserted(&conv, "<info-msg>placeholder</info-msg>".into());
            let (_, issues) =
                crate::conversation::fix_conversation(Conversation::new_unvalidated(prepared));
            let text_merges = issues
                .iter()
                .filter(|i| i.contains("Merged text content"))
                .count();
            assert_eq!(
                text_merges, 0,
                "turn {turn} re-reported historical coalesces: {issues:?}"
            );
            let consecutive = issues
                .iter()
                .filter(|i| i.contains("Merged consecutive"))
                .count();
            assert!(
                consecutive <= MAX_MERGES_CAUSED_BY_MOIM,
                "turn {turn} merged more than MOIM's own insertion: {issues:?}"
            );
            if let Some(prev) = last_text_merges {
                assert_eq!(prev, text_merges, "issue count escalated at turn {turn}");
            }
            last_text_merges = Some(text_merges);
        }
    }

    #[test]
    fn expected_moim_issues_match_the_health_watch_list() {
        for issue in [
            "Merged text content",
            "Merged consecutive user messages",
            "Merged consecutive assistant messages",
            "Added placeholder to empty tool result",
            "Added placeholder user message to empty conversation",
            "Trimmed trailing whitespace from assistant message",
        ] {
            assert!(is_expected_moim_issue(issue), "{issue}");
        }
        assert!(!is_expected_moim_issue("Removed empty message"));
        assert!(!is_expected_moim_issue("Fixed lead/trail"));
    }
}
