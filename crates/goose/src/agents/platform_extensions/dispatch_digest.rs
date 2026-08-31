//! Dispatch-time context digest: the small, always-relevant slice of board and
//! notes state that rides the existing dispatch-context string into a worker.
//!
//! ## Why a digest and not a dump
//!
//! An external-CLI goal worker has no Brain, no `PromptManager`, and (before
//! the read-only bridge) no tools back into Permagent: everything it knows
//! arrives as one flat string built once at dispatch. The temptation is to put
//! the whole board in that string. The 2026-08-10 goals A/B measurement on the
//! code map says why that fails — injected bulk does not change behaviour, it
//! only costs tokens; what changes behaviour is a small always-relevant slice
//! plus a tool to ask for the rest ([`super::goal_context_mcp`]).
//!
//! So this module injects one slice and nothing more:
//!
//! - **sibling-goal status** — the other non-terminal goals on this project's
//!   board, so a worker stops duplicating or contradicting work in flight; and
//! - **a notes summary** — the first lines of `project.notes`, a DB field that
//!   until now nothing on any path read.
//!
//! Everything else is *linked, not inlined*: the digest names the bridge tool
//! that returns the full list.
//!
//! ## Staleness is stated, not implied
//!
//! Both halves are a snapshot as of dispatch. A worker that treats a snapshot
//! as live state acts on a board that moved under it, so every rendered block
//! carries its own age label and the name of the tool that answers live.

use std::fmt::Write as _;

/// Character budget for the whole digest. ~400 tokens at the usual 4 chars/token
/// heuristic, which is the number a3's B2 design costed: small enough to be
/// worth paying on EVERY dispatch, since unlike a tool definition it is not
/// amortized over a session.
pub(crate) const DIGEST_MAX_CHARS: usize = 1_600;

/// Per-sibling cap. Ten siblings at this width fill most of the budget, which
/// is the intended shape: the digest is a roll-call, not a briefing. A title
/// longer than this is cut on a char boundary with an ellipsis — the worker
/// can read the full one from `board_query`.
const SIBLING_TITLE_MAX_CHARS: usize = 72;

/// How many siblings to name. Past ten, a worker skims rather than reads, and
/// the marginal duplicate-work saved is small; the block says how many more
/// exist so nothing is silently hidden.
pub(crate) const MAX_SIBLINGS: usize = 10;

/// Character cap on the notes summary.
const NOTES_MAX_CHARS: usize = 480;

/// One sibling goal, reduced to the three facts that prevent collision: what it
/// is, where it is in the lifecycle, and who has it.
///
/// Deliberately NOT carrying `metadata_json`: goal-state keys there are
/// guarded (`cards::check_protected_metadata`) and a worker reading them would
/// be reading control-plane state it must never act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingGoal {
    pub title: String,
    /// Human-readable lifecycle state (the board column's name).
    pub state: String,
    pub assigned_to: Option<String>,
}

/// Everything the digest renders, already fetched. Split from the DB read so
/// the formatting — budget, ordering, labels — is testable without a daemon.
#[derive(Debug, Clone, Default)]
pub struct DigestInput {
    /// Non-terminal sibling goals, in the order they should be shown. The
    /// dispatching goal itself is excluded by the caller.
    pub siblings: Vec<SiblingGoal>,
    /// How many non-terminal siblings existed before [`MAX_SIBLINGS`] truncation.
    pub sibling_total: usize,
    /// `project.notes`, verbatim; empty when unset.
    pub notes: String,
}

fn truncate_chars(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let kept: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// Render the dispatch digest, or `None` when there is nothing to say.
///
/// Deterministic: the same input renders byte-identical output, so a dispatch
/// brief can be snapshot-tested and a diff in it means a real change.
pub fn format_dispatch_digest(input: &DigestInput) -> Option<String> {
    let notes = truncate_chars(&input.notes, NOTES_MAX_CHARS);
    if input.siblings.is_empty() && notes.is_empty() {
        return None;
    }

    let mut out = String::from(
        "Project context digest (snapshot taken at dispatch — NOT live; \
         call the `board_query` / `project_get` bridge tools for current state):\n",
    );

    if !input.siblings.is_empty() {
        let _ = writeln!(out, "## Other goals on this board");
        for s in input.siblings.iter().take(MAX_SIBLINGS) {
            let title = truncate_chars(&s.title, SIBLING_TITLE_MAX_CHARS);
            let line = match s.assigned_to.as_deref().filter(|a| !a.is_empty()) {
                Some(who) => format!("- {title} — {} — {who}\n", s.state),
                None => format!("- {title} — {} — unassigned\n", s.state),
            };
            if out.len() + line.len() > DIGEST_MAX_CHARS {
                break;
            }
            out.push_str(&line);
        }
        let shown = input.siblings.len().min(MAX_SIBLINGS);
        if input.sibling_total > shown {
            let _ = writeln!(
                out,
                "({} more not shown — `board_query` returns the full board.)",
                input.sibling_total - shown
            );
        }
        out.push_str(
            "Do not duplicate or contradict work already in flight above; if your goal \
             overlaps one, say so instead of doing it twice.\n",
        );
    }

    if !notes.is_empty() {
        let block = format!("## Project notes (as of dispatch)\n{notes}\n");
        if out.len() + block.len() <= DIGEST_MAX_CHARS {
            out.push_str(&block);
        }
    }

    // Hard backstop: the per-section checks above bound the common path, but a
    // budget that can be exceeded by ANY input is not a budget.
    if out.chars().count() > DIGEST_MAX_CHARS {
        out = out.chars().take(DIGEST_MAX_CHARS).collect();
    }
    Some(out.trim_end().to_string())
}

// ── DB loading ───────────────────────────────────────────────────────────────

/// Board columns whose `state_binding` means the goal is finished with — a
/// sibling there is history, not a collision risk.
const TERMINAL_BINDINGS: &[&str] = &["complete", "cancelled"];

/// Build the digest for a dispatching goal from live DB state.
///
/// Best-effort by construction: every failure path yields `None` rather than an
/// error, because a digest that can fail a dispatch is worse than no digest.
pub(crate) async fn load_dispatch_digest(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project: &crate::projects::Project,
    dispatching_card_id: &str,
) -> Option<String> {
    let columns = crate::cards::list_columns(pool, &project.id).await.ok()?;
    let cards = crate::cards::list_cards(pool, &project.id, Some("goal"), None)
        .await
        .ok()?;

    let state_of = |column_id: &str| -> Option<(String, bool)> {
        let col = columns.iter().find(|c| c.id == column_id)?;
        let binding = col.state_binding.as_deref().unwrap_or("");
        Some((col.name.clone(), TERMINAL_BINDINGS.contains(&binding)))
    };

    let mut siblings: Vec<SiblingGoal> = Vec::new();
    for card in &cards {
        if card.id == dispatching_card_id {
            continue;
        }
        let Some((state, terminal)) = state_of(&card.column_id) else {
            continue;
        };
        if terminal {
            continue;
        }
        siblings.push(SiblingGoal {
            title: card.title.clone(),
            state,
            assigned_to: card.assigned_to.clone(),
        });
    }
    // `list_cards` orders by (column_id, position); re-sort on the lifecycle
    // order the columns declare so the digest reads Triage→Ready→…, and is
    // stable regardless of the id ordering SQLite happened to return.
    let column_rank = |name: &str| -> i32 {
        columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.position)
            .unwrap_or(i32::MAX)
    };
    siblings.sort_by(|a, b| {
        column_rank(&a.state)
            .cmp(&column_rank(&b.state))
            .then_with(|| a.title.cmp(&b.title))
    });

    let sibling_total = siblings.len();
    siblings.truncate(MAX_SIBLINGS);

    format_dispatch_digest(&DigestInput {
        siblings,
        sibling_total,
        notes: project.notes.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sib(title: &str, state: &str, who: Option<&str>) -> SiblingGoal {
        SiblingGoal {
            title: title.to_string(),
            state: state.to_string(),
            assigned_to: who.map(str::to_string),
        }
    }

    /// Nothing to say → no block, rather than a header framing an empty list.
    /// (Same shape as `code_map::format_code_map_block`'s empty-map case.)
    #[test]
    fn absent_when_there_is_no_sibling_and_no_note() {
        assert!(format_dispatch_digest(&DigestInput::default()).is_none());
        assert!(format_dispatch_digest(&DigestInput {
            notes: "   \n ".into(),
            ..Default::default()
        })
        .is_none());
    }

    /// The three collision-relevant facts per sibling, and the anti-duplication
    /// instruction that is the whole point of injecting them.
    #[test]
    fn names_each_sibling_with_state_and_assignee() {
        let out = format_dispatch_digest(&DigestInput {
            siblings: vec![
                sib("Wire the voice relay", "In Progress", Some("claude_code")),
                sib("Fix the picker scanner", "Ready", None),
            ],
            sibling_total: 2,
            notes: String::new(),
        })
        .unwrap();
        assert!(out.contains("Wire the voice relay"));
        assert!(out.contains("In Progress"));
        assert!(out.contains("claude_code"));
        assert!(out.contains("Fix the picker scanner"));
        assert!(out.contains("unassigned"));
        assert!(out.contains("Do not duplicate"));
    }

    /// Staleness is stated on the block itself and names the live alternative —
    /// the contract `code_map`'s SNAPSHOT_DISCLAIMER already sets for injected
    /// state. A snapshot a worker mistakes for live state is the failure.
    #[test]
    fn carries_a_staleness_label_and_names_the_live_tool() {
        let out = format_dispatch_digest(&DigestInput {
            siblings: vec![sib("A goal", "Ready", None)],
            sibling_total: 1,
            notes: "Deploys go through the publish sequence.".into(),
        })
        .unwrap();
        assert!(out.contains("snapshot taken at dispatch"));
        assert!(out.contains("NOT live"));
        assert!(out.contains("board_query"));
        assert!(
            out.contains("as of dispatch"),
            "the notes half is labeled too"
        );
    }

    /// The budget is an ACTUAL cap, not a target: 50 siblings with long titles
    /// and a 10 KB note must still fit. Mirrors
    /// `code_map::code_map_block_is_bounded_and_cut_on_line_boundaries`.
    #[test]
    fn respects_the_token_budget_under_adversarial_input() {
        let siblings: Vec<SiblingGoal> = (0..50)
            .map(|i| {
                sib(
                    &format!("{} {i}", "a very long goal title ".repeat(12)),
                    "Ready",
                    Some("someone"),
                )
            })
            .collect();
        let out = format_dispatch_digest(&DigestInput {
            siblings,
            sibling_total: 50,
            notes: "n".repeat(10_000),
        })
        .unwrap();
        assert!(
            out.chars().count() <= DIGEST_MAX_CHARS,
            "digest must stay within its budget, got {} chars",
            out.chars().count()
        );
        // And it must say what it hid rather than silently dropping 40 goals.
        assert!(out.contains("more not shown"));
    }

    /// Same input, same bytes — a dispatch brief that varies run to run cannot
    /// be snapshot-tested and its diffs cannot be reviewed.
    #[test]
    fn rendering_is_deterministic() {
        let input = DigestInput {
            siblings: vec![
                sib("Beta", "Ready", Some("codex")),
                sib("Alpha", "Review", None),
            ],
            sibling_total: 2,
            notes: "Some note".into(),
        };
        let first = format_dispatch_digest(&input).unwrap();
        // Non-vacuity: an empty render is trivially deterministic, so the
        // invariant is only meaningful once there is content to vary.
        assert!(first.contains("Alpha") && first.contains("Beta"));
        for _ in 0..5 {
            assert_eq!(format_dispatch_digest(&input).unwrap(), first);
        }
    }

    /// Only title/state/assignee cross the boundary. `metadata_json` carries
    /// guarded goal-state keys (`cards::check_protected_metadata`) and control
    /// -plane fields like `rlm_state`; a worker must never receive them here.
    #[test]
    fn never_leaks_card_metadata() {
        let out = format_dispatch_digest(&DigestInput {
            siblings: vec![sib("Some goal", "Ready", None)],
            sibling_total: 1,
            notes: String::new(),
        })
        .unwrap();
        // Non-vacuity: the block must actually carry the sibling, or "no
        // metadata leaked" is satisfied by rendering nothing at all.
        assert!(out.contains("Some goal"));
        for leaked in [
            "metadata_json",
            "rlm_state",
            "attempt_count",
            "verify_escalation",
        ] {
            assert!(!out.contains(leaked), "{leaked} leaked into the digest");
        }
    }

    /// A notes-only project still gets its half — the `project.notes` field was
    /// dead on every path before this.
    #[test]
    fn renders_notes_alone_when_there_are_no_siblings() {
        let out = format_dispatch_digest(&DigestInput {
            notes: "Build with `just build`; never touch ~/.permagent.".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(out.contains("just build"));
        assert!(!out.contains("Other goals on this board"));
    }

    // ── The DB read ─────────────────────────────────────────────────────

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    /// The selection rules that decide what a worker sees: its OWN goal is not
    /// a sibling (it already has the brief), and a Complete/Cancelled goal is
    /// history rather than a collision risk. Getting either wrong produces a
    /// digest that reads as advice to redo finished work.
    #[tokio::test]
    async fn excludes_the_dispatching_goal_and_terminal_siblings() {
        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Digest Project".into(),
                slug: None,
                description: None,
                root_path: None,
                site_url: None,
                repo_url: None,
                notes: Some("House rule: run fmt before you push.".into()),
                tags: None,
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let columns = crate::cards::list_columns(&pool, &project.id)
            .await
            .unwrap();
        let col = |binding: &str| -> String {
            columns
                .iter()
                .find(|c| c.state_binding.as_deref() == Some(binding))
                .unwrap()
                .id
                .clone()
        };

        let make = |title: &str, binding: &str| {
            let pool = pool.clone();
            let input = crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: title.to_string(),
                description: None,
                card_type: Some("goal".into()),
                column_id: Some(col(binding)),
                created_by: None,
                metadata_json: None,
            };
            async move { crate::cards::create_card(&pool, input).await.unwrap() }
        };
        let me = make("The goal being dispatched", "ready").await;
        make("A sibling in flight", "in_progress").await;
        make("Something already shipped", "complete").await;
        make("Something abandoned", "cancelled").await;

        let out = load_dispatch_digest(&pool, &project, &me.id).await.unwrap();
        assert!(out.contains("A sibling in flight"));
        assert!(
            !out.contains("The goal being dispatched"),
            "a worker must not be told about its own goal as a sibling"
        );
        assert!(
            !out.contains("Something already shipped"),
            "Complete leaked"
        );
        assert!(!out.contains("Something abandoned"), "Cancelled leaked");
        // The dead `project.notes` field now reaches the worker.
        assert!(out.contains("run fmt before you push"));
    }
}
