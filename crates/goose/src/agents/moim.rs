use crate::agents::extension_manager::ExtensionManager;
use crate::conversation::message::Message;
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

/// Insert the synthetic MOIM user message in front of the last assistant
/// turn. Adjacent text/thinking parts are coalesced *first* so historical
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
    let idx = messages
        .iter()
        .rposition(|m| m.role == Role::Assistant)
        .unwrap_or(0);
    messages.insert(idx, Message::user().with_text(moim));
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

        let merged_content = msgs[0]
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

        assert_eq!(msgs.len(), 6);

        let moim_msg = &msgs[3];
        let has_moim = moim_msg
            .content
            .iter()
            .any(|c| c.as_text().is_some_and(|t| t.contains("<info-msg>")));

        assert!(
            has_moim,
            "MOIM should be in message before latest assistant message"
        );
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
