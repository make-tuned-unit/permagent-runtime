//! The trust ladder that governs model-authored `command_exit_zero` checks.
//!
//! A completion check is a sentence the model wrote, compiled into a shell
//! command, and then handed to `/bin/sh -c` with the user's own privileges.
//! Before this module there was nothing in between — no allowlist, no consent,
//! no sandbox. This is the something in between.
//!
//! Three tiers, and who decides:
//!
//! | tier | what it means | who says yes |
//! |------|---------------|--------------|
//! | [`Tier::Auto`] | allowlisted runner, deny table clean | nobody — it runs |
//! | [`Tier::AgentTrust`] | unknown first token, deny table clean | the agent, if it has earned it |
//! | [`Tier::User`] | a deny row fired, or unearned | the user, in the Decision Inbox |
//!
//! **Privilege is earned per project, and it is earned by being right.** Every
//! gated check that runs and whose goal later passes typed verification without
//! being flagged adds one to `clean_runs`. Cross a threshold and the agent may
//! start self-approving unknown commands: first the read-only-looking ones, then
//! any of them. Get it wrong — a user deny, a flagged goal — and the count drops
//! back below the threshold it just cleared. One level at a time, with the
//! reason written down.
//!
//! **Nothing here is silent.** Every self-approval writes an [`AuditRow`], and
//! that row travels two ways: onto the check result (so it appears in the goal
//! card's verification section, next to the command it authorised) and into the
//! project's own capped audit list (so Settings can show the history).
//!
//! Two properties this module is required to hold, and which its tests assert:
//!
//! - **Absence is not consent.** A goal whose `completion_checks_source` is
//!   missing is gated. Only an explicit `user` stamp bypasses.
//! - **The agent can never widen its own allowlist.** Self-approval is per
//!   command, per run. Only a person, answering a Decision Inbox card, adds a
//!   token to the allowlist.

pub mod command_gate;

pub use command_gate::{
    classify, default_allowlist, first_token_of, Classification, DenyCategory, GateConfig, Tier,
};

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

/// Where this project's ladder state lives inside `projects.metadata_json`.
///
/// The bag is shared with `build_command`, `brand`, `strategy` and friends, so
/// every write here is a read-merge-write of that one key and never a
/// replacement of the bag.
pub const METADATA_KEY: &str = "verification_approval";

/// Clean runs needed before the agent may self-approve a read-only-looking
/// unknown command. Config: `readOnlyThreshold`.
pub const DEFAULT_READ_ONLY_THRESHOLD: u32 = 5;

/// Clean runs needed before the agent may self-approve any Tier-1 command in
/// this project. Config: `fullThreshold`.
pub const DEFAULT_FULL_THRESHOLD: u32 = 20;

/// How many audit rows a project keeps. Oldest are dropped — the per-goal copy
/// on the check result is the durable one.
pub const MAX_AUDIT_ROWS: usize = 50;

/// Longest command recorded in an audit row.
const MAX_AUDIT_COMMAND_CHARS: usize = 500;

fn default_read_only_threshold() -> u32 {
    DEFAULT_READ_ONLY_THRESHOLD
}
fn default_full_threshold() -> u32 {
    DEFAULT_FULL_THRESHOLD
}

// ── Privilege ───────────────────────────────────────────────────────────────

/// What the agent is currently allowed to wave through on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeLevel {
    /// Nothing. Every unknown command goes to the user.
    None,
    /// Unknown commands that carry no write-looking flag and no redirect.
    ReadOnly,
    /// Any Tier-1 command in this project.
    Full,
}

impl PrivilegeLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PrivilegeLevel::None => "none",
            PrivilegeLevel::ReadOnly => "read_only",
            PrivilegeLevel::Full => "full",
        }
    }
}

/// What the gate decided, for the audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecision {
    /// Allowlisted and clean — no approval was needed.
    Auto,
    /// The check was authored by the user, so the ladder does not apply.
    UserAuthored,
    /// The agent spent its earned privilege on this one command.
    AgentApproved,
    /// A person approved this exact command once.
    ApprovedOnce,
    /// A person approved it and added its first token to the allowlist.
    ApprovedAndAllowlisted,
    /// Sent to the Decision Inbox; the command did not run.
    Parked,
    /// A person said no. Counts against privilege.
    Denied,
}

impl GateDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            GateDecision::Auto => "auto",
            GateDecision::UserAuthored => "user_authored",
            GateDecision::AgentApproved => "agent_approved",
            GateDecision::ApprovedOnce => "approved_once",
            GateDecision::ApprovedAndAllowlisted => "approved_and_allowlisted",
            GateDecision::Parked => "parked",
            GateDecision::Denied => "denied",
        }
    }

    /// Did the command actually run?
    pub fn ran(self) -> bool {
        !matches!(self, GateDecision::Parked | GateDecision::Denied)
    }

    /// Does this decision earn privilege when the goal later verifies clean?
    /// Auto and agent-approved runs do — those are the ladder proving itself.
    /// A user-authored check was never gated, so it earns nothing.
    pub fn counts_toward_privilege(self) -> bool {
        matches!(
            self,
            GateDecision::Auto | GateDecision::AgentApproved | GateDecision::ApprovedOnce
        )
    }
}

// ── Audit ───────────────────────────────────────────────────────────────────

/// One line of the visible record: what was about to run, what the gate said,
/// and how much privilege was standing behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuditRow {
    pub at: String,
    /// The command as written, truncated to [`MAX_AUDIT_COMMAND_CHARS`].
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub tier: Tier,
    pub decision: GateDecision,
    /// `clean_runs` at the moment of the decision — the "privilege" column.
    pub privilege: u32,
    pub level: PrivilegeLevel,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<DenyCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

// ── Settings ────────────────────────────────────────────────────────────────

/// Per-project ladder state. Every field has a serde default so a project that
/// has never been gated deserializes from `{}`, and so a field added later does
/// not invalidate a stored bag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalSettings {
    /// First tokens this project trusts, on top of [`default_allowlist`].
    /// Only a person can add to this.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Gated checks that ran and whose goal later verified clean.
    #[serde(default)]
    pub clean_runs: u32,
    #[serde(default = "default_read_only_threshold")]
    pub read_only_threshold: u32,
    #[serde(default = "default_full_threshold")]
    pub full_threshold: u32,
    /// Exact commands a person approved once. Consumed on use — an approve-once
    /// that is never used stays until the user clears it, but it can only ever
    /// authorise the one command string it names.
    #[serde(default)]
    pub once_grants: Vec<String>,
    /// Most recent [`MAX_AUDIT_ROWS`] decisions, newest last.
    #[serde(default)]
    pub audit: Vec<AuditRow>,
}

impl Default for ApprovalSettings {
    fn default() -> Self {
        Self {
            allowlist: Vec::new(),
            clean_runs: 0,
            read_only_threshold: DEFAULT_READ_ONLY_THRESHOLD,
            full_threshold: DEFAULT_FULL_THRESHOLD,
            once_grants: Vec::new(),
            audit: Vec::new(),
        }
    }
}

impl ApprovalSettings {
    /// Read the ladder state out of a project metadata bag. A missing or
    /// unreadable key yields defaults — a project with no state is a project
    /// with no privilege, which is the safe reading.
    pub fn from_metadata(meta: &serde_json::Value) -> Self {
        meta.get(METADATA_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Current standing.
    pub fn level(&self) -> PrivilegeLevel {
        if self.full_threshold > 0 && self.clean_runs >= self.full_threshold {
            PrivilegeLevel::Full
        } else if self.read_only_threshold > 0 && self.clean_runs >= self.read_only_threshold {
            PrivilegeLevel::ReadOnly
        } else {
            PrivilegeLevel::None
        }
    }

    /// Reward `n` clean gated runs.
    pub fn promote(&mut self, n: u32) {
        self.clean_runs = self.clean_runs.saturating_add(n);
    }

    /// Drop exactly one level. From Full the count lands on the read-only
    /// threshold (still ReadOnly, one short of Full); from ReadOnly it lands on
    /// zero; from None it stays at zero. Losing a level costs the same whether
    /// the agent had 20 clean runs or 200 — privilege is a standing, not a
    /// balance to be hoarded.
    pub fn demote(&mut self) {
        self.clean_runs = match self.level() {
            PrivilegeLevel::Full => self.read_only_threshold,
            PrivilegeLevel::ReadOnly => 0,
            PrivilegeLevel::None => 0,
        };
    }

    /// Add a first token to the allowlist. Idempotent, and sorted so the
    /// stored bag is stable.
    pub fn allowlist_token(&mut self, token: &str) {
        let token = token.trim();
        if token.is_empty() || self.allowlist.iter().any(|t| t == token) {
            return;
        }
        self.allowlist.push(token.to_string());
        self.allowlist.sort();
    }

    /// Record a one-command grant.
    pub fn grant_once(&mut self, cmd: &str) {
        if !self.once_grants.iter().any(|c| c == cmd) {
            self.once_grants.push(cmd.to_string());
        }
    }

    /// Spend a one-command grant if one is standing for this exact command.
    pub fn take_once_grant(&mut self, cmd: &str) -> bool {
        match self.once_grants.iter().position(|c| c == cmd) {
            Some(i) => {
                self.once_grants.remove(i);
                true
            }
            None => false,
        }
    }

    /// Append an audit row, keeping the list capped.
    pub fn push_audit(&mut self, row: AuditRow) {
        self.audit.push(row);
        let overflow = self.audit.len().saturating_sub(MAX_AUDIT_ROWS);
        if overflow > 0 {
            self.audit.drain(0..overflow);
        }
    }

    /// Everything the classifier needs, for a project rooted at `root`.
    pub fn gate_config(
        &self,
        root: impl Into<std::path::PathBuf>,
        build_command: Option<&str>,
    ) -> GateConfig {
        GateConfig::new(root, self.allowlist.iter().cloned(), build_command)
    }
}

// ── The decision ────────────────────────────────────────────────────────────

/// Provenance of a goal's completion checks, as stamped by the orchestrator.
///
/// The important case is [`ChecksSource::Unknown`]: a goal card with no stamp
/// is **gated**, not trusted. Older cards predate the stamp, and treating a
/// missing field as consent would hand every one of them a free pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksSource {
    /// The user wrote these checks. The ladder does not apply.
    User,
    /// The model compiled them from acceptance criteria, or they are the
    /// project default build check.
    Model,
    /// No stamp. Gated, same as model-authored.
    Unknown,
}

impl ChecksSource {
    /// Read the stamp off a goal card's metadata.
    pub fn from_metadata(meta: &serde_json::Value) -> Self {
        match meta
            .get("completion_checks_source")
            .and_then(|v| v.as_str())
        {
            Some(USER_CHECKS_SOURCE) => ChecksSource::User,
            Some(_) => ChecksSource::Model,
            None => ChecksSource::Unknown,
        }
    }

    fn bypasses_gate(self) -> bool {
        matches!(self, ChecksSource::User)
    }
}

/// The stamp the orchestrator writes when it finds checks the user authored.
pub const USER_CHECKS_SOURCE: &str = "user";

/// What the gate decided about one command, ready to be acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateOutcome {
    pub decision: GateDecision,
    pub classification: Classification,
    pub level: PrivilegeLevel,
    pub reason: String,
}

impl GateOutcome {
    pub fn allowed(&self) -> bool {
        self.decision.ran()
    }

    /// The row to attach to the check result and to the project's audit list.
    pub fn audit_row(
        &self,
        cmd: &str,
        cwd: Option<&str>,
        clean_runs: u32,
        goal_id: Option<&str>,
    ) -> AuditRow {
        AuditRow {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            command: truncate(cmd, MAX_AUDIT_COMMAND_CHARS),
            cwd: cwd.map(|c| c.to_string()),
            tier: self.classification.tier,
            decision: self.decision,
            privilege: clean_runs,
            level: self.level,
            reason: self.reason.clone(),
            deny: self.classification.deny,
            goal_id: goal_id.map(|g| g.to_string()),
        }
    }
}

/// **The gate.** Given a command, who wrote the check, and what this project's
/// ladder looks like, decide whether it may run.
///
/// Pure, and deliberately so: the caller performs the effect (running it, or
/// parking the goal), and the tests can enumerate the whole decision space
/// without a shell or a database.
///
/// `settings` is taken by value-mutation for the one-shot grant: spending a
/// grant is part of the decision, so the caller must persist `settings`
/// afterwards or the grant survives its own use.
pub fn decide(
    cmd: &str,
    cwd: Option<&str>,
    source: ChecksSource,
    settings: &mut ApprovalSettings,
    cfg: &GateConfig,
) -> GateOutcome {
    let level = settings.level();

    if source.bypasses_gate() {
        return GateOutcome {
            decision: GateDecision::UserAuthored,
            classification: Classification {
                tier: Tier::Auto,
                deny: None,
                unknown_token: None,
                reason: "the user wrote this check".to_string(),
                read_only_looking: false,
            },
            level,
            reason: "you wrote this check yourself, so the ladder does not apply".to_string(),
        };
    }

    let run_dir = match cwd {
        Some(rel) => cfg.project_root.join(rel),
        None => cfg.project_root.clone(),
    };
    let classification = classify(cmd, &run_dir, cfg);

    // A standing one-command grant beats everything except the deny table.
    // A person approved this exact string; they did not approve a category.
    if classification.tier != Tier::Auto && settings.take_once_grant(cmd) {
        if classification.tier == Tier::User {
            // The world changed since the approval, or the approval was for a
            // command that is now denied. Re-parking is the second look.
            settings.grant_once(cmd);
        } else {
            return GateOutcome {
                decision: GateDecision::ApprovedOnce,
                classification,
                level,
                reason: "you approved this exact command once, and this is that once".to_string(),
            };
        }
    }

    match classification.tier {
        Tier::Auto => GateOutcome {
            decision: GateDecision::Auto,
            reason: classification.reason.clone(),
            classification,
            level,
        },
        Tier::User => GateOutcome {
            decision: GateDecision::Parked,
            reason: classification.reason.clone(),
            classification,
            level,
        },
        Tier::AgentTrust => {
            let earned = match level {
                PrivilegeLevel::Full => true,
                PrivilegeLevel::ReadOnly => classification.read_only_looking,
                PrivilegeLevel::None => false,
            };
            if earned {
                let reason = format!(
                    "self-approved: {} clean run{} in this project earned {} privilege, and {}",
                    settings.clean_runs,
                    if settings.clean_runs == 1 { "" } else { "s" },
                    level.as_str(),
                    classification.reason,
                );
                GateOutcome {
                    decision: GateDecision::AgentApproved,
                    classification,
                    level,
                    reason,
                }
            } else {
                let reason = format!(
                    "{} — {} clean run{} is not enough to self-approve {}",
                    classification.reason,
                    settings.clean_runs,
                    if settings.clean_runs == 1 { "" } else { "s" },
                    if classification.read_only_looking {
                        "a read-only-looking command"
                    } else {
                        "a command that may write"
                    },
                );
                GateOutcome {
                    decision: GateDecision::Parked,
                    classification,
                    level,
                    reason,
                }
            }
        }
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Read a project's ladder state. A missing project yields defaults, so a
/// caller never has to special-case it into an approval.
pub async fn load(pool: &Pool<Sqlite>, project_id: &str) -> Result<ApprovalSettings, String> {
    let Some(project) = crate::projects::get_project(pool, project_id).await? else {
        return Ok(ApprovalSettings::default());
    };
    Ok(ApprovalSettings::from_metadata(&project.metadata_json))
}

/// Merge-write the ladder state back into the project metadata bag.
///
/// Read-merge-write against a **fresh** read, not against whatever the caller
/// loaded earlier: the bag is shared with `build_command`, `brand` and the
/// rest, and a stale replacement would silently drop a sibling's write.
pub async fn save(
    pool: &Pool<Sqlite>,
    project_id: &str,
    settings: &ApprovalSettings,
) -> Result<(), String> {
    let Some(project) = crate::projects::get_project(pool, project_id).await? else {
        return Err(format!("project '{project_id}' not found"));
    };
    let mut meta = match project.metadata_json {
        serde_json::Value::Object(map) => serde_json::Value::Object(map),
        _ => serde_json::json!({}),
    };
    let obj = meta
        .as_object_mut()
        .ok_or_else(|| "project metadata is not an object".to_string())?;
    obj.insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    crate::projects::update_project(
        pool,
        project_id,
        crate::projects::UpdateProject {
            metadata_json: Some(meta),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Apply a change to a project's ladder state under a fresh read.
///
/// Every mutation goes through here so nobody hand-rolls a read-modify-write
/// and gets the merge wrong.
pub async fn update<F>(
    pool: &Pool<Sqlite>,
    project_id: &str,
    f: F,
) -> Result<ApprovalSettings, String>
where
    F: FnOnce(&mut ApprovalSettings),
{
    let mut settings = load(pool, project_id).await?;
    f(&mut settings);
    save(pool, project_id, &settings).await?;
    Ok(settings)
}

/// Persist everything one verification run decided: the audit rows, the
/// approve-once grants it spent, and the privilege it earned.
///
/// `spent_grants` matters as much as the rest. [`decide`] removes a grant from
/// the settings it was handed, but that copy is a snapshot — this function
/// re-reads the project and must remove the grant there too, or a
/// once-and-only-once approval quietly becomes a standing one.
pub async fn record_run(
    pool: &Pool<Sqlite>,
    project_id: &str,
    rows: Vec<AuditRow>,
    clean_runs_earned: u32,
    spent_grants: Vec<String>,
) -> Result<ApprovalSettings, String> {
    update(pool, project_id, move |s| {
        for cmd in &spent_grants {
            s.take_once_grant(cmd);
        }
        for row in rows {
            s.push_audit(row);
        }
        if clean_runs_earned > 0 {
            s.promote(clean_runs_earned);
        }
    })
    .await
}

#[cfg(test)]
mod tests;
