//! S4 (#430, epic #399): CC gate → `action_class` → risk tier classification.
//!
//! The gate parser (S2, `terminal_supervision.rs`) turns a supervised Claude
//! Code session's `control_request`/`can_use_tool` line into a
//! [`super::terminal_supervision::PendingGate`] carrying the CC `tool_name` and
//! its `input`. The inbox bridge (S3) files each gate as a `session_gate`
//! decision. This module is the layer between them: it maps a gate to a
//! **`risk_policy` action_class**, and the existing tier machinery
//! ([`crate::decisions::tier_for_action_class`]) resolves that class to a risk
//! **tier (0/1/2)** off the `risk_policy` trust dial.
//!
//! ## Why action_class, not a hardcoded tier
//!
//! The whole rest of Permagent keys authorization on `risk_policy`: a tier is
//! never baked into code, it is looked up from a user-tunable table (the "trust
//! dial"), and an **unknown class fails closed to Tier 2**
//! (`decisions.rs:tier_for_action_class`). S4 plugs into that seam rather than
//! inventing a parallel tier authority — the classifier returns only the
//! action_class string; the DB owns the tier. That means Jesse can re-tier any
//! CC action class (e.g. demote `cc_workspace_edit` to Tier 0, or promote
//! `cc_read_only` to Tier 1) by editing one `risk_policy` row, with no code
//! change.
//!
//! ## The mapping (Fork-2 policy, conservative / escalate-when-unsure)
//!
//! Epic #399's core constraint is the EXPLICIT NON-GOAL of Henry
//! blanket-advancing all gates: build the escalate-when-unsure version. So the
//! only gates that resolve below Tier 2 are ones with a *provably bounded,
//! reversible* effect surface:
//!
//! | CC tool(s) | action_class | seeded tier | why |
//! |---|---|---|---|
//! | `Read` `Glob` `Grep` `LS` `NotebookRead` `BashOutput` `TodoWrite` | `cc_read_only` | 0 | no effect outside the session's own reasoning — reads + in-memory todo state |
//! | `Write` `Edit` `MultiEdit` `NotebookEdit` | `cc_workspace_edit` | 1 | file mutation, but confined + git-reversible in a supervised worktree; recorded, Henry-clearable |
//! | `Bash` `KillBash` `KillShell` | `cc_shell` | 2 | arbitrary shell — the `rm -rf` / `git push` / `curl` surface; Jesse-only |
//! | `WebFetch` `WebSearch` | `network_external` | 2 | egress off the machine (reuses the existing seed) |
//! | `Task` `SlashCommand`, `mcp__*`, **anything else** | `cc_unclassified` | 2 (fail-closed) | unbounded / unrecognized surface — deliberately UNSEEDED so it fails closed |
//!
//! **Fail-closed default.** [`classify_gate`] returns [`UNCLASSIFIED`] for any
//! tool it does not explicitly recognize (a new CC tool, an MCP tool, a typo, a
//! crafted gate). `cc_unclassified` is intentionally **not** seeded into
//! `risk_policy`, so `tier_for_action_class` resolves it to Tier 2 — the most
//! restrictive tier, Jesse-only. Adding a tool to a lower tier is therefore an
//! explicit, reviewable act; the absence of a mapping is never permissive.
//!
//! **`input` is accepted but not yet consumed.** The signature takes the tool
//! input so a future refinement (e.g. tiering a `Write` outside the worktree
//! root, or a `Bash` matching a known-safe allowlist, differently) is a
//! non-breaking change. Today the policy keys on `tool_name` alone: a per-input
//! allowlist is an attack surface (easy to bypass, e.g. `$(…)` substitution),
//! and fail-closed-by-tool is the safe floor. This is deliberate, not a stub.

use serde_json::Value;

/// `risk_policy` action_class for CC tools with no effect outside the session's
/// own reasoning (filesystem reads + in-memory todo state). Seeded at Tier 0.
pub const READ_ONLY: &str = "cc_read_only";

/// `risk_policy` action_class for CC tools that mutate files in the supervised
/// worktree (confined, git-reversible). Seeded at Tier 1.
pub const WORKSPACE_EDIT: &str = "cc_workspace_edit";

/// `risk_policy` action_class for CC tools that run arbitrary shell commands.
/// Seeded at Tier 2 (Jesse-only) — the irreversible `rm`/`push`/`curl` surface.
pub const SHELL: &str = "cc_shell";

/// `risk_policy` action_class for CC tools that reach the network. Reuses the
/// existing `network_external` seed (Tier 2).
pub const NETWORK_EXTERNAL: &str = "network_external";

/// Fail-closed action_class for any tool the classifier does not recognize.
/// Deliberately NOT seeded into `risk_policy` so it resolves to Tier 2 (the
/// most restrictive tier) via [`crate::decisions::tier_for_action_class`].
pub const UNCLASSIFIED: &str = "cc_unclassified";

/// The CC action classes S4 seeds into `risk_policy` (fresh installs +
/// migration). `network_external` is NOT here — it predates S4 — and
/// `cc_unclassified` is NOT here by design (fail-closed). Each entry is
/// `(action_class, tier, rationale)`.
pub const SEEDED_CLASSES: &[(&str, i64, &str)] = &[
    (
        READ_ONLY,
        0,
        "Supervised CC read-only tool (Read/Glob/Grep/LS/NotebookRead/BashOutput/TodoWrite) — no effect outside the session",
    ),
    (
        WORKSPACE_EDIT,
        1,
        "Supervised CC file edit (Write/Edit/MultiEdit/NotebookEdit) — confined, git-reversible; recorded decision",
    ),
    (
        SHELL,
        2,
        "Supervised CC shell (Bash/KillBash) — arbitrary command surface; Jesse-only",
    ),
];

/// Map a supervised CC gate's tool name to its `risk_policy` action_class.
///
/// Pure and deterministic. `input` is accepted for forward-compatible
/// input-aware refinement (see the module docs) but is not consulted today.
/// Unknown tools return [`UNCLASSIFIED`] → Tier 2 fail-closed.
pub fn classify_gate(tool_name: &str, _input: &Value) -> &'static str {
    match tool_name {
        // Tier 0 — no effect outside the session's own reasoning.
        "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" | "BashOutput" | "TodoWrite" => READ_ONLY,
        // Tier 1 — confined, reversible file mutation.
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => WORKSPACE_EDIT,
        // Tier 2 — arbitrary shell.
        "Bash" | "KillBash" | "KillShell" => SHELL,
        // Tier 2 — network egress.
        "WebFetch" | "WebSearch" => NETWORK_EXTERNAL,
        // Everything else (Task, SlashCommand, mcp__*, new/unknown tools) →
        // fail closed. Never guess a lower tier for an unrecognized surface.
        _ => UNCLASSIFIED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn classify(tool: &str) -> &'static str {
        classify_gate(tool, &json!({}))
    }

    #[test]
    fn read_only_tools_map_to_tier0_class() {
        for tool in [
            "Read",
            "Glob",
            "Grep",
            "LS",
            "NotebookRead",
            "BashOutput",
            "TodoWrite",
        ] {
            assert_eq!(classify(tool), READ_ONLY, "tool: {tool}");
        }
    }

    #[test]
    fn edit_tools_map_to_workspace_edit_class() {
        for tool in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
            assert_eq!(classify(tool), WORKSPACE_EDIT, "tool: {tool}");
        }
    }

    #[test]
    fn shell_tools_map_to_shell_class() {
        for tool in ["Bash", "KillBash", "KillShell"] {
            assert_eq!(classify(tool), SHELL, "tool: {tool}");
        }
    }

    #[test]
    fn network_tools_map_to_network_external_class() {
        for tool in ["WebFetch", "WebSearch"] {
            assert_eq!(classify(tool), NETWORK_EXTERNAL, "tool: {tool}");
        }
    }

    #[test]
    fn unknown_and_unbounded_tools_fail_closed() {
        for tool in [
            "Task",                      // spawns arbitrary autonomous work
            "SlashCommand",              // runs a user command
            "mcp__github__create_issue", // an MCP tool
            "mcp__anything",
            "SomeBrandNewToolClaudeAdds", // a tool that doesn't exist yet
            "",                           // empty / malformed
            "bash",                       // wrong case must NOT match Bash
            "read",                       // wrong case must NOT match Read
            "Write ",                     // trailing space must NOT match Write
        ] {
            assert_eq!(
                classify(tool),
                UNCLASSIFIED,
                "unrecognized tool must fail closed: {tool:?}"
            );
        }
    }

    #[test]
    fn classification_ignores_input_today() {
        // Same tool, wildly different inputs → same class (input is not yet
        // consulted; documented forward-compat hook).
        assert_eq!(
            classify_gate("Bash", &json!({"command": "ls"})),
            classify_gate("Bash", &json!({"command": "rm -rf / --no-preserve-root"})),
        );
        assert_eq!(
            classify_gate("Write", &json!({"path": "/etc/passwd"})),
            classify_gate("Write", &json!({"path": "src/foo.rs"})),
        );
    }

    #[test]
    fn seeded_classes_are_internally_consistent() {
        // The seed table must match the constants + intended tiers, and must
        // never accidentally seed the fail-closed sentinel.
        let classes: Vec<&str> = SEEDED_CLASSES.iter().map(|(c, _, _)| *c).collect();
        assert!(classes.contains(&READ_ONLY));
        assert!(classes.contains(&WORKSPACE_EDIT));
        assert!(classes.contains(&SHELL));
        assert!(
            !classes.contains(&UNCLASSIFIED),
            "cc_unclassified must stay UNSEEDED so it fails closed to Tier 2"
        );
        assert!(
            !classes.contains(&NETWORK_EXTERNAL),
            "network_external predates S4 and is seeded by the base schema"
        );
        for (_, tier, _) in SEEDED_CLASSES {
            assert!((0..=2).contains(tier), "tier out of range: {tier}");
        }
    }
}
