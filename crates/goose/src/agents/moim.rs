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
        let mut messages = conversation.messages().clone();
        let idx = messages
            .iter()
            .rposition(|m| m.role == Role::Assistant)
            .unwrap_or(0);
        messages.insert(idx, Message::user().with_text(moim));

        let (fixed, issues) = fix_conversation(Conversation::new_unvalidated(messages));

        let has_unexpected_issues = issues.iter().any(|issue| {
            !issue.contains("Merged consecutive user messages")
                && !issue.contains("Merged consecutive assistant messages")
                && !issue.contains("Added placeholder to empty tool result")
        });

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

/// MOIM inserts exactly ONE synthetic user message, so it can create at most
/// one consecutive-same-role pair. Anything beyond this was already in the
/// conversation before MOIM touched it.
const MAX_MERGES_CAUSED_BY_MOIM: usize = 1;

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
}
