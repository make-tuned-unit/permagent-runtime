//! Specialist role briefs for dispatched goal workers.
//!
//! A role is a mandate block prepended to a worker's brief — it shapes HOW the
//! worker approaches the goal (debugger: reproduce first; security: adversarial
//! read; architect: design before code). Roles are deliberately orthogonal to
//! the worker roster: any worker can wear any brief, so roles never duplicate
//! engine configs and cost-ranked worker selection stays untouched.
//!
//! Selection rides `metadata_json.dispatch_role` on the goal card (set by the
//! `goal_advance` tool's `role` argument, or by a decision effect such as the
//! review-fail → debugger proposal). The key is sticky: a goal keeps its role
//! across re-dispatches until it is changed or cleared.

/// Card metadata key naming the role for the next dispatch.
pub const DISPATCH_ROLE_KEY: &str = "dispatch_role";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerRole {
    Debugger,
    Security,
    Architect,
}

impl WorkerRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "debugger" => Some(Self::Debugger),
            "security" => Some(Self::Security),
            "architect" => Some(Self::Architect),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debugger => "debugger",
            Self::Security => "security",
            Self::Architect => "architect",
        }
    }

    /// The mandate block a worker sees, placed between its persona and the
    /// goal instructions so it reads as mandate-then-task.
    pub fn brief_block(self) -> &'static str {
        match self {
            Self::Debugger => {
                "Role for this dispatch: DEBUGGER.\n\
                 Reproduce the failure before changing anything — a fix without a \
                 reproduction is a guess. Read the failing evidence in the goal \
                 description first, trace the mechanism to its root cause, and state \
                 the mechanism in your summary. Prefer the minimal diff that removes \
                 the cause over a broad rewrite, and leave behind a check or test \
                 that fails on the old behavior so the regression cannot return \
                 silently."
            }
            Self::Security => {
                "Role for this dispatch: SECURITY REVIEWER-IMPLEMENTER.\n\
                 Read the change surface adversarially before editing: untrusted \
                 input paths, authentication/authorization seams, injection sinks, \
                 secrets handling, and failure modes that fail open. Fix what the \
                 goal names, flag what it missed in your summary rather than \
                 silently expanding scope, and never weaken an existing guard to \
                 make something pass."
            }
            Self::Architect => {
                "Role for this dispatch: ARCHITECT.\n\
                 Design before code: identify the seams the change touches, state \
                 the shape of the solution and its alternatives in your summary, \
                 and keep implementation to the minimum that proves the design \
                 (interfaces, types, wiring). Flag any load-bearing invariant your \
                 design relies on so reviewers can hold it."
            }
        }
    }
}

/// Resolve the role brief for a dispatch from card metadata. Unknown or absent
/// role values resolve to None — dispatch proceeds unroled rather than failing.
pub fn role_brief_from_metadata(meta: &serde_json::Value) -> Option<&'static str> {
    meta.get(DISPATCH_ROLE_KEY)
        .and_then(|v| v.as_str())
        .and_then(WorkerRole::parse)
        .map(WorkerRole::brief_block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_and_rejects_unknown() {
        for role in [
            WorkerRole::Debugger,
            WorkerRole::Security,
            WorkerRole::Architect,
        ] {
            assert_eq!(WorkerRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(WorkerRole::parse("Debugger"), Some(WorkerRole::Debugger));
        assert_eq!(WorkerRole::parse("cowboy"), None);
        assert_eq!(WorkerRole::parse(""), None);
    }

    #[test]
    fn briefs_are_distinct_mandates() {
        let blocks = [
            WorkerRole::Debugger.brief_block(),
            WorkerRole::Security.brief_block(),
            WorkerRole::Architect.brief_block(),
        ];
        for b in blocks {
            assert!(b.starts_with("Role for this dispatch:"));
        }
        assert_ne!(blocks[0], blocks[1]);
        assert_ne!(blocks[1], blocks[2]);
    }

    #[test]
    fn metadata_resolution_is_lenient() {
        let meta = serde_json::json!({ DISPATCH_ROLE_KEY: "debugger" });
        assert_eq!(
            role_brief_from_metadata(&meta),
            Some(WorkerRole::Debugger.brief_block())
        );
        assert_eq!(
            role_brief_from_metadata(&serde_json::json!({ DISPATCH_ROLE_KEY: "cowboy" })),
            None
        );
        assert_eq!(role_brief_from_metadata(&serde_json::json!({})), None);
    }
}
