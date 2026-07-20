//! Per-project publish sequence (#457) — the ordered post-push steps a project
//! needs before a change is actually LIVE (e.g. Reckonize: seed prod DB, then
//! `vercel --prod` so static pages regenerate). A git push alone runs NONE of
//! them, so a worker that pushed has not shipped anything user-visible.
//!
//! Storage: `projects.metadata_json.publish_sequence` (the schema-v26 metadata
//! bag that already hosts `build_command`, §3d ruling 3 in
//! docs/architecture/GOAL_COMPLETION_AND_VERIFICATION.md). Canonical entry
//! shape, as written by the command-center UI (publishSequence.ts):
//!
//! ```json
//! { "publish_sequence": [
//!     {"order": 1, "command": "set -a; source .env.local; set +a; npx tsx scripts/reseed-threads.ts", "timeout_secs": 300},
//!     {"order": 2, "command": "vercel --prod", "timeout_secs": 600}
//! ] }
//! ```
//!
//! The parser is tolerant of the bag being agent-writable: bare-string entries
//! are accepted as commands, malformed entries are skipped (never errored on),
//! and explicit `order` wins over array position.
//!
//! This module is the CAPTURE + SURFACE slice of #457: the sequence has a home,
//! dispatched workers are told push ≠ live, and the Review decision says so
//! when a sequence exists but was not run. The daemon-side publish RUNNER
//! (execute steps post-push with `.env.local` secrets + redacted output,
//! rulings 4/6) is deliberately NOT here — it lands with the evidence-record
//! redaction guard.

/// `projects.metadata_json` key holding the ordered publish steps.
pub const PUBLISH_SEQUENCE_KEY: &str = "publish_sequence";

/// One post-push publish step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishStep {
    /// Shell command, run in the project context. May reference `.env.local`
    /// by path — the config stores commands, never secret values (ruling 4).
    pub command: String,
    /// Optional per-step timeout; the eventual runner clamps like checks.rs.
    pub timeout_secs: Option<u64>,
}

/// Parse a project metadata bag into its ordered publish steps.
///
/// Absent key, non-array value, or an empty array ⇒ `vec![]` (no sequence —
/// push IS live for this project). Entries may be:
/// * an object with a non-empty string `command`, optional integer `order`
///   and `timeout_secs`;
/// * a bare non-empty string (treated as the command).
///
/// Malformed or blank entries are skipped. Ordering: explicit `order` first
/// (array index tiebreak, stable); entries without `order` sort at their
/// array position.
pub fn parse_publish_sequence(project_meta: &serde_json::Value) -> Vec<PublishStep> {
    let Some(arr) = project_meta
        .get(PUBLISH_SEQUENCE_KEY)
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };

    let mut keyed: Vec<(i64, usize, PublishStep)> = Vec::new();
    for (idx, entry) in arr.iter().enumerate() {
        let (command, order, timeout_secs) = match entry {
            serde_json::Value::String(s) => (s.trim().to_string(), None, None),
            serde_json::Value::Object(o) => {
                let command = o
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let order = o.get("order").and_then(|v| v.as_i64());
                let timeout_secs = o.get("timeout_secs").and_then(|v| v.as_u64());
                (command, order, timeout_secs)
            }
            _ => continue,
        };
        if command.is_empty() {
            continue;
        }
        keyed.push((
            order.unwrap_or(idx as i64),
            idx,
            PublishStep {
                command,
                timeout_secs,
            },
        ));
    }

    keyed.sort_by_key(|(order, idx, _)| (*order, *idx));
    keyed.into_iter().map(|(_, _, step)| step).collect()
}

/// Numbered `1. cmd` lines for prompts/details, in publish order.
fn numbered_steps(steps: &[PublishStep]) -> String {
    steps
        .iter()
        .enumerate()
        .map(|(i, s)| match s.timeout_secs {
            Some(t) => format!("{}. {}   (timeout {}s)", i + 1, s.command, t),
            None => format!("{}. {}", i + 1, s.command),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Instructions block appended to a dispatched worker's goal instructions when
/// the project declares a publish sequence. `None` when there is no sequence.
///
/// This is the dispatch-side surface of "push ≠ live": the worker learns what
/// remains after its push and is told never to claim the change is live.
pub fn dispatch_instructions_block(steps: &[PublishStep]) -> Option<String> {
    if steps.is_empty() {
        return None;
    }
    Some(format!(
        "PUBLISH SEQUENCE — for this project a git push is NOT live. After \
         commit + push, these ordered steps are still required before the \
         change is user-visible:\n{}\n\
         These steps run in the project checkout and may source secrets from \
         the project's .env.local — never inline secret values into commands \
         or output. The daemon does not run these steps automatically yet. \
         Unless you have actually run them and can show their output, report \
         the work as 'pushed — publish sequence pending', and never claim it \
         is live or deployed.",
        numbered_steps(steps)
    ))
}

/// Review-time note appended to the `approve_review` decision detail when the
/// project declares a publish sequence (which the daemon has not run). `None`
/// when there is no sequence.
pub fn review_pending_note(steps: &[PublishStep]) -> Option<String> {
    if steps.is_empty() {
        return None;
    }
    Some(format!(
        "PUSH ≠ LIVE for this project: it declares a {}-step publish sequence \
         that the daemon has NOT run:\n{}\n\
         Approving marks the goal Complete, but the change is not user-visible \
         until the publish sequence has been run.",
        steps.len(),
        numbered_steps(steps)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_key_is_empty() {
        assert!(parse_publish_sequence(&json!({})).is_empty());
        assert!(parse_publish_sequence(&json!({"build_command": "npm run build"})).is_empty());
    }

    #[test]
    fn non_array_is_empty() {
        assert!(parse_publish_sequence(&json!({"publish_sequence": "vercel --prod"})).is_empty());
        assert!(parse_publish_sequence(&json!({"publish_sequence": {"command": "x"}})).is_empty());
        assert!(parse_publish_sequence(&json!({"publish_sequence": null})).is_empty());
    }

    #[test]
    fn canonical_objects_parse_in_order() {
        let meta = json!({"publish_sequence": [
            {"order": 2, "command": "vercel --prod", "timeout_secs": 600},
            {"order": 1, "command": "npx tsx scripts/reseed-threads.ts", "timeout_secs": 300},
        ]});
        let steps = parse_publish_sequence(&meta);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].command, "npx tsx scripts/reseed-threads.ts");
        assert_eq!(steps[0].timeout_secs, Some(300));
        assert_eq!(steps[1].command, "vercel --prod");
        assert_eq!(steps[1].timeout_secs, Some(600));
    }

    #[test]
    fn bare_strings_accepted_and_kept_in_array_order() {
        let meta = json!({"publish_sequence": ["seed.sh", "vercel --prod"]});
        let steps = parse_publish_sequence(&meta);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].command, "seed.sh");
        assert_eq!(steps[0].timeout_secs, None);
        assert_eq!(steps[1].command, "vercel --prod");
    }

    #[test]
    fn malformed_and_blank_entries_skipped() {
        let meta = json!({"publish_sequence": [
            42,
            "",
            "   ",
            {"order": 1},
            {"command": "   "},
            {"command": "real-step"},
            null,
        ]});
        let steps = parse_publish_sequence(&meta);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].command, "real-step");
    }

    #[test]
    fn explicit_order_wins_with_index_tiebreak() {
        let meta = json!({"publish_sequence": [
            {"order": 5, "command": "b"},
            {"order": 5, "command": "c"},
            {"order": 0, "command": "a"},
        ]});
        let steps: Vec<String> = parse_publish_sequence(&meta)
            .into_iter()
            .map(|s| s.command)
            .collect();
        assert_eq!(steps, vec!["a", "b", "c"]);
    }

    #[test]
    fn command_trimmed() {
        let meta = json!({"publish_sequence": [{"command": "  vercel --prod  "}]});
        assert_eq!(parse_publish_sequence(&meta)[0].command, "vercel --prod");
    }

    #[test]
    fn instructions_block_none_when_empty() {
        assert!(dispatch_instructions_block(&[]).is_none());
        assert!(review_pending_note(&[]).is_none());
    }

    #[test]
    fn instructions_block_lists_steps_and_forbids_live_claim() {
        let steps = vec![
            PublishStep {
                command: "npx tsx scripts/reseed-threads.ts".into(),
                timeout_secs: Some(300),
            },
            PublishStep {
                command: "vercel --prod".into(),
                timeout_secs: None,
            },
        ];
        let block = dispatch_instructions_block(&steps).unwrap();
        assert!(block.contains("git push is NOT live"));
        assert!(block.contains("1. npx tsx scripts/reseed-threads.ts   (timeout 300s)"));
        assert!(block.contains("2. vercel --prod"));
        assert!(block.contains("pushed — publish sequence pending"));
        assert!(block.contains("never claim it is live"));
    }

    #[test]
    fn review_note_counts_steps_and_says_not_user_visible() {
        let steps = vec![
            PublishStep {
                command: "seed".into(),
                timeout_secs: None,
            },
            PublishStep {
                command: "deploy".into(),
                timeout_secs: None,
            },
        ];
        let note = review_pending_note(&steps).unwrap();
        assert!(note.contains("PUSH ≠ LIVE"));
        assert!(note.contains("2-step publish sequence"));
        assert!(note.contains("NOT run"));
        assert!(note.contains("1. seed"));
        assert!(note.contains("2. deploy"));
        assert!(note.contains("not user-visible"));
    }

    #[test]
    fn end_to_end_from_project_meta() {
        // The exact Reckonize shape from #457 / §3d.
        let meta = json!({
            "build_command": "npm run build",
            "publish_sequence": [
                {"order": 1, "command": "set -a; source .env.local; set +a; npx tsx scripts/reseed-threads.ts", "timeout_secs": 300},
                {"order": 2, "command": "vercel --prod", "timeout_secs": 600}
            ]
        });
        let steps = parse_publish_sequence(&meta);
        let note = review_pending_note(&steps).unwrap();
        assert!(note.contains("vercel --prod"));
        // Sibling keys (build_command) are ignored, not consumed.
        assert_eq!(steps.len(), 2);
    }
}
