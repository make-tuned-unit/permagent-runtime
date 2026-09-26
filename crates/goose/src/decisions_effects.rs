//! Durable, retryable application of answered Decision Inbox effects.

use crate::decisions::{self, Decision, DecisionProof};
use crate::goal_state::GoalAction;
use crate::goal_transition::{self, GuardError, TransitionEffects};
use sqlx::{Pool, Row, Sqlite};

const OUTBOX_BATCH_SIZE: i64 = 32;
const RUNNING_LEASE_SECONDS: i64 = 300;

/// The result displayed by the inline answer path.
pub type EffectResult = (Option<String>, Option<String>);

/// Where approved regressions are materialised: the `permagent-eval` crate's
/// own `tasks/` directory in the checkout this daemon was built from, which is
/// exactly the directory the eval runner reads by default (its
/// `DEFAULT_TASKS_DIR` is anchored on the same `CARGO_MANIFEST_DIR`).
///
/// Never relative to the process's working directory: launchd starts the
/// daemon with cwd `/` (the plist sets no `WorkingDirectory`, and the app's
/// sidecar inherits `/` too), which on macOS is the read-only system volume —
/// every approval failed with os error 30 and the outbox retried it to death.
/// A daemon whose build checkout is gone writes under its data directory
/// instead, so an approval always lands somewhere writable.
fn eval_tasks_dir() -> std::path::PathBuf {
    let checkout = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../permagent-eval/tasks");
    if checkout.is_dir() {
        checkout
    } else {
        crate::config::paths::Paths::data_dir().join("eval-tasks")
    }
}

/// Write an approved regression as a `permagent-eval` task directory under
/// [`eval_tasks_dir`].
fn write_regression_task(
    payload: &crate::decisions::RegressionProposalPayload,
) -> Result<String, GuardError> {
    write_regression_task_in(&eval_tasks_dir(), payload)
}

/// [`write_regression_task`] into an explicit tasks directory.
///
/// Re-validates the slug and filename rather than trusting the payload: these
/// values reach the filesystem, and the approval gate authorises WHAT is
/// written, not WHERE. Refuses to overwrite an existing task — a proposal that
/// silently replaced a passing regression would be a way to delete coverage
/// through the approval path.
///
/// Safe to retry, which the effect outbox does: the task is written into a
/// `.{task_id}.partial` directory and renamed into place, so a write that
/// fails halfway never leaves a half-task every retry then refuses to
/// overwrite, and a retry that finds exactly the files it would write (the
/// write landed but the outbox row was not marked applied) reports success.
fn write_regression_task_in(
    tasks_dir: &std::path::Path,
    payload: &crate::decisions::RegressionProposalPayload,
) -> Result<String, GuardError> {
    if !crate::decisions::is_safe_task_id(&payload.task_id) {
        return Err(GuardError::Invalid(format!(
            "unsafe regression task_id '{}'",
            payload.task_id
        )));
    }
    if !crate::decisions::is_safe_oracle_filename(&payload.oracle_filename) {
        return Err(GuardError::Invalid(format!(
            "unsafe oracle filename '{}'",
            payload.oracle_filename
        )));
    }

    let spec = serde_yaml::to_string(
        &serde_yaml::from_str::<serde_yaml::Value>(&format!(
            "id: {}\ntitle: {}\ncategory: {}\ntest: {}\nprompt: |\n{}\n",
            serde_yaml::to_string(&payload.task_id)
                .unwrap_or_default()
                .trim(),
            serde_yaml::to_string(&payload.title)
                .unwrap_or_default()
                .trim(),
            serde_yaml::to_string(payload.category.as_deref().unwrap_or("regression"))
                .unwrap_or_default()
                .trim(),
            serde_yaml::to_string(&payload.test)
                .unwrap_or_default()
                .trim(),
            payload
                .prompt
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ))
        .map_err(|e| GuardError::Invalid(format!("could not build task.yaml: {e}")))?,
    )
    .map_err(|e| GuardError::Invalid(format!("could not serialize task.yaml: {e}")))?;

    let root = tasks_dir.join(&payload.task_id);
    if root.exists() {
        let oracle = root.join("oracle").join(&payload.oracle_filename);
        let already_written = std::fs::read_to_string(root.join("task.yaml"))
            .is_ok_and(|written| written == spec)
            && std::fs::read_to_string(oracle)
                .is_ok_and(|written| written == payload.oracle_source);
        if already_written {
            return Ok(root.display().to_string());
        }
        return Err(GuardError::Invalid(format!(
            "regression task '{}' already exists — refusing to overwrite existing coverage",
            payload.task_id
        )));
    }

    // The slug check above keeps this name inside `tasks_dir`. A leftover
    // from a crashed earlier attempt is this approval's own, so it goes.
    let partial = tasks_dir.join(format!(".{}.partial", payload.task_id));
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|e| {
            GuardError::Invalid(format!("could not clear an earlier partial write: {e}"))
        })?;
    }
    let written = write_task_files(&partial, &root, &spec, payload);
    if let Err(e) = written {
        if let Err(cleanup) = std::fs::remove_dir_all(&partial) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    task_id = %payload.task_id,
                    "could not remove a partial regression task write: {cleanup}"
                );
            }
        }
        return Err(e);
    }

    Ok(root.display().to_string())
}

/// Write a regression task's files into `partial`, then rename it to `root`.
fn write_task_files(
    partial: &std::path::Path,
    root: &std::path::Path,
    spec: &str,
    payload: &crate::decisions::RegressionProposalPayload,
) -> Result<(), GuardError> {
    use std::io::Write;

    let oracle_dir = partial.join("oracle");
    std::fs::create_dir_all(&oracle_dir)
        .map_err(|e| GuardError::Invalid(format!("could not create task dir: {e}")))?;
    let mut f = std::fs::File::create(partial.join("task.yaml"))
        .map_err(|e| GuardError::Invalid(format!("could not write task.yaml: {e}")))?;
    f.write_all(spec.as_bytes())
        .map_err(|e| GuardError::Invalid(format!("could not write task.yaml: {e}")))?;
    let mut o = std::fs::File::create(oracle_dir.join(&payload.oracle_filename))
        .map_err(|e| GuardError::Invalid(format!("could not write oracle: {e}")))?;
    o.write_all(payload.oracle_source.as_bytes())
        .map_err(|e| GuardError::Invalid(format!("could not write oracle: {e}")))?;
    std::fs::rename(partial, root)
        .map_err(|e| GuardError::Invalid(format!("could not move the task into place: {e}")))
}

async fn goal_state(pool: &Pool<Sqlite>, goal_id: &str) -> Result<Option<String>, GuardError> {
    sqlx::query_scalar(
        "SELECT col.state_binding
         FROM cards c JOIN board_columns col ON col.id = c.column_id
         WHERE c.id = ?",
    )
    .bind(goal_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| GuardError::Db(e.to_string()))
}

/// Durable verifier state consulted at BOTH the HTTP answer boundary and the
/// retryable effect boundary. The second check closes the race where a verdict
/// changes after the user taps Approve but before the outbox lands the work.
/// Missing evidence is pending, never an implicit pass.
pub async fn approve_review_verification_status(
    pool: &Pool<Sqlite>,
    goal_id: &str,
) -> Result<String, GuardError> {
    let metadata: Option<String> =
        sqlx::query_scalar("SELECT metadata_json FROM cards WHERE id = ?")
            .bind(goal_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| GuardError::Db(e.to_string()))?;
    let Some(metadata) = metadata else {
        return Err(GuardError::NotFound(format!("goal '{goal_id}' not found")));
    };
    let metadata: serde_json::Value = serde_json::from_str(&metadata)
        .map_err(|e| GuardError::Db(format!("goal metadata is invalid JSON: {e}")))?;
    Ok(metadata
        .pointer("/dispatch_evidence/verdict/status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("pending")
        .to_ascii_lowercase())
}

fn already_applied(message: impl Into<String>) -> Result<EffectResult, GuardError> {
    Ok((Some(message.into()), None))
}

/// On enrichment reject, persist a user-typed "how to find this person online"
/// hint onto the graph as a manual field so the next `enrich_person` briefing
/// can use it. An empty note is a plain decline — nothing is written.
async fn persist_find_online_hints_on_reject(
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let hint = decision
        .answer_note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(hint) = hint else {
        return already_applied("enrichment proposal declined; nothing was written");
    };

    let payload: decisions::EnrichmentProposalPayload =
        serde_json::from_value(decision.payload.clone()).map_err(|e| {
            GuardError::Invalid(format!("stored enrichment payload unreadable: {e}"))
        })?;

    let Some(brain) = crate::agents::platform_extensions::get_global_brain() else {
        return already_applied(format!(
            "enrichment proposal declined; find-online hint kept on the decision \
             (Brain unavailable — not written to \"{}\")",
            payload.person_name
        ));
    };
    let entity_id: spectral::core::entity_id::EntityId = payload
        .graph_entity_id
        .parse()
        .map_err(|e| GuardError::Invalid(format!("invalid graph_entity_id in payload: {e:?}")))?;
    let wrote = brain
        .set_entity_field(
            entity_id,
            "find_online_hints",
            hint,
            spectral::ingest::FieldSource::Manual,
            None,
        )
        .await
        .map_err(|e| GuardError::Db(format!("set_entity_field(find_online_hints): {e}")))?;
    if wrote {
        emit_person_updated(&payload);
        already_applied(format!(
            "enrichment proposal declined; find-online hint saved for \"{}\"",
            payload.person_name
        ))
    } else {
        already_applied(format!(
            "enrichment proposal declined; find-online hint was not written for \"{}\"",
            payload.person_name
        ))
    }
}

/// Drop this project's command-approval privilege one level, with the reason
/// recorded. Best-effort by design: this is a consequence of another decision,
/// never the decision itself, so a ladder write that fails must not fail it.
async fn demote_check_privilege(pool: &Pool<Sqlite>, decision: &Decision, why: &str) {
    use crate::verification_approval as approval;
    let Some(project_id) = decision.project_id.as_deref() else {
        return;
    };
    let reason = why.to_string();
    let goal_id = decision.goal_id.clone();
    if let Err(e) = approval::update(pool, project_id, move |s| {
        s.demote();
        s.push_audit(approval::AuditRow {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            command: String::new(),
            cwd: None,
            tier: approval::Tier::User,
            decision: approval::GateDecision::Denied,
            privilege: s.clean_runs,
            level: s.level(),
            reason,
            deny: None,
            goal_id,
        });
    })
    .await
    {
        tracing::warn!(
            target: "permagentd::decisions",
            project_id = %project_id, error = %e,
            "could not demote command-approval privilege after a rejection"
        );
    }
}

/// Apply the answer to a [`decisions::PROPOSAL_CHECK_APPROVAL`] card.
///
/// The three answers and what each one costs:
///
/// - **approve once** — a grant for this exact command string, spent the first
///   time it is seen. Widens nothing.
/// - **approve and allowlist** — adds the command's *first token* to this
///   project's allowlist, so every future check starting with that token is
///   Tier 0. This is the only way the allowlist ever grows, and only a person
///   can do it.
/// - **deny** — the command does not run, and the agent's privilege drops one
///   level with the reason written into the audit.
///
/// Approve of either kind unparks the goal so the checks run again with the new
/// state. Failed → Ready is the only unpark the goal state machine allows, so
/// this takes the same route the `unblock` decision takes — including raising
/// the attempt cap, because Ready means "another attempt" and a goal already at
/// its cap would re-park immediately without ever reaching the command the user
/// just approved.
async fn apply_check_approval(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
    acted_by: &str,
) -> Result<EffectResult, GuardError> {
    use crate::verification_approval as approval;

    let payload: decisions::ChoicePayload = serde_json::from_value(decision.payload.clone())
        .map_err(|e| GuardError::Invalid(format!("check-approval payload unreadable: {e}")))?;
    let Some(subject) = payload.check_approval else {
        return Err(GuardError::Invalid(
            "check-approval decision carries no command — nothing safe to apply".to_string(),
        ));
    };

    // `reject` is a deny however it was expressed.
    let chosen = match decision.answer.as_deref() {
        Some("reject") => decisions::CHECK_APPROVAL_DENY,
        _ => decision
            .answer_choice_id
            .as_deref()
            .unwrap_or(decisions::CHECK_APPROVAL_DENY),
    };

    let (row_decision, note) = match chosen {
        decisions::CHECK_APPROVAL_ONCE => (
            approval::GateDecision::ApprovedOnce,
            format!("approved once: `{}`", subject.command),
        ),
        decisions::CHECK_APPROVAL_ALLOWLIST => match subject.first_token.as_deref() {
            Some(tok) if !tok.trim().is_empty() => (
                approval::GateDecision::ApprovedAndAllowlisted,
                format!("`{tok}` added to this project's allowlist"),
            ),
            // Offered nothing to allowlist, so this answer cannot mean what it
            // says. Refuse rather than silently downgrade it to approve-once —
            // the user should see that their choice did not apply.
            _ => {
                return Err(GuardError::Invalid(
                    "this command has no first token to allowlist — approve it once instead"
                        .to_string(),
                ))
            }
        },
        decisions::CHECK_APPROVAL_DENY => (
            approval::GateDecision::Denied,
            "denied; the agent's approval privilege drops one level".to_string(),
        ),
        other => {
            return already_applied(format!(
                "unrecognized command-approval option '{other}' — no effect applied"
            ))
        }
    };

    let command = subject.command.clone();
    let first_token = subject.first_token.clone();
    let reason = format!("{} (decision {})", note, decision.id);
    let goal_id_for_row = decision.goal_id.clone();
    let cwd = subject.cwd.clone();
    let deny = subject.deny.clone();

    let settings = approval::update(pool, &subject.project_id, move |s| {
        match row_decision {
            approval::GateDecision::ApprovedOnce => s.grant_once(&command),
            approval::GateDecision::ApprovedAndAllowlisted => {
                if let Some(tok) = first_token.as_deref() {
                    s.allowlist_token(tok);
                }
            }
            approval::GateDecision::Denied => s.demote(),
            _ => {}
        }
        s.push_audit(approval::AuditRow {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            command,
            cwd: Some(cwd),
            tier: approval::Tier::User,
            decision: row_decision,
            privilege: s.clean_runs,
            level: s.level(),
            reason,
            deny: deny.as_deref().and_then(deny_category_from_str),
            goal_id: goal_id_for_row,
        });
    })
    .await
    .map_err(GuardError::Db)?;

    if row_decision == approval::GateDecision::Denied {
        return already_applied(format!(
            "{note} — the goal stays parked and the command will not run"
        ));
    }

    // Approved: send the goal back for another run, which will now clear the
    // gate. Reuses the `unblock` unpark shape so there is one way a parked goal
    // comes back, not two.
    let Some(goal_id) = decision.goal_id.as_deref() else {
        return already_applied(format!("{note} (no goal attached to re-run)"));
    };
    match goal_state(pool, goal_id).await?.as_deref() {
        Some("ready" | "in_progress" | "review" | "complete") => {
            return already_applied(format!("{note}; the goal had already moved on"))
        }
        None => return already_applied(format!("{note}; the goal was already gone")),
        _ => {}
    }
    let card = crate::cards::get_card(pool, goal_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
    let attempt_count = card
        .metadata_json
        .get("attempt_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let mut budget = card
        .metadata_json
        .get("budget")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let attempt_cap = attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP;
    budget.insert("attempt_cap".to_string(), serde_json::json!(attempt_cap));
    let mut patch = serde_json::Map::new();
    patch.insert(
        "needs_human_attention".to_string(),
        serde_json::json!(false),
    );
    patch.insert("budget".to_string(), serde_json::Value::Object(budget));
    goal_transition::advance_goal_checked(
        pool,
        goal_id,
        GoalAction::Ready,
        acted_by,
        Some(proof),
        TransitionEffects {
            metadata_patch: patch,
            ..Default::default()
        },
    )
    .await?;
    already_applied(format!(
        "{note}; the goal is unparked and its checks will run again \
         (privilege now {})",
        settings.level().as_str()
    ))
}

/// Parse a deny slug back into its category. An unrecognised slug records no
/// category rather than guessing one.
fn deny_category_from_str(s: &str) -> Option<crate::verification_approval::DenyCategory> {
    use crate::verification_approval::DenyCategory as D;
    Some(match s {
        "pipe_to_interpreter" => D::PipeToInterpreter,
        "network_tool" => D::NetworkTool,
        "destructive_outside_root" => D::DestructiveOutsideRoot,
        "git_mutating" => D::GitMutating,
        "privilege_escalation" => D::PrivilegeEscalation,
        "redirect_outside_root" => D::RedirectOutsideRoot,
        "command_substitution" => D::CommandSubstitution,
        "unparseable" => D::Unparseable,
        _ => return None,
    })
}

fn emit_person_updated(payload: &decisions::EnrichmentProposalPayload) {
    if let Some(uuid) = payload.entity_uuid.as_deref().filter(|s| !s.is_empty()) {
        crate::events::emit(crate::events::person_changed("", uuid, "updated"));
    }
}

/// Future returned by an installed [`PostLandingHook`].
pub type PostLandingFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Runs after an approval fast-forwards trunk, by project id. The daemon
/// installs one that refreshes the project's code map — an existing map now
/// describes the pre-landing tree.
///
/// Detached and best-effort by contract: the approval has already committed,
/// and must never fail or wait on a tree-sitter pass. Uninstalled, landing is
/// unaffected.
pub type PostLandingHook = Box<dyn Fn(Pool<Sqlite>, String) -> PostLandingFuture + Send + Sync>;

/// Process-global post-landing hook. Same shape and contract as
/// `orchestrator::GOAL_DISPATCH_HOOK`.
pub static POST_LANDING_HOOK: std::sync::OnceLock<PostLandingHook> = std::sync::OnceLock::new();

/// Install the post-landing hook. Idempotent (OnceLock) — a second call is a
/// no-op.
pub fn install_post_landing_hook(hook: PostLandingHook) {
    let _ = POST_LANDING_HOOK.set(hook);
}

/// H1 (G8 review, harness loop Lane E3 2026-09-10): the working directory
/// independently recorded for whatever session dispatched this goal —
/// `sessions.working_dir`, set at session creation and never re-derived
/// from the process's own live `current_dir()`. Read via the card's
/// `worker_session_id` (the same key R3/the requeue guard already use to
/// identify "the session that ran this goal"). `None` when the goal has no
/// recorded dispatching session, or that session's row cannot be found —
/// [`crate::goal_landing::verify_worktree_landing_target`] treats an
/// unknown working directory as a refusal, never as license to trust the
/// worktree's own claim about where it came from.
///
/// Lane R22d: `pub(crate)` (was private) so `orchestrator.rs`'s salvage path
/// can resolve the SAME trusted landing root this function's own caller
/// (`land_approved_goal`) actually lands onto — salvage's notion of
/// "already landed" must never disagree with the real landing decision.
pub(crate) async fn goal_dispatching_session_working_dir(
    pool: &Pool<Sqlite>,
    metadata: &serde_json::Value,
) -> Option<String> {
    let session_id = metadata.get("worker_session_id").and_then(|v| v.as_str())?;
    sqlx::query_scalar::<_, String>("SELECT working_dir FROM sessions WHERE id = ?")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Fast-forward an approved goal's work onto the project trunk.
///
/// The first member is `None` when there is nothing to land at all (no
/// evidence, no commits, or a project with no `root_path` — a project we
/// cannot locate on disk has no trunk to land onto, and the daemon's cwd is
/// emphatically not it). Otherwise it is `(resolved_project_id, outcome)`:
/// the project id comes back because the post-landing code-map refresh needs
/// the id this fn RESOLVED (`decision.project_id` may be absent; the card's is
/// the fallback). The outcome is reported on the approval in every case,
/// including refusals — a dirty tree or diverged trunk must be visible, not
/// silent. The second member surfaces a failure to persist the durable
/// NeedsMerge follow-up after the approval itself has already committed.
///
/// Lives here, in the OUTBOX effect path, because that is the only path an
/// approval actually takes: `approve_review` is outbox-eligible, so
/// `execute_effect` delegates before reaching its own inline arm. This code
/// spent its whole life in that unreachable arm (L7, 2026-08-31).
///
/// `headless_auto_land` (Lane R63b) is the `auto_land_allowed` the approval
/// arrived with — true only on the `PERMAGENT_HEADLESS_AUTOLAND` path. Only
/// then does a `NeedsMerge` get the rebase + fresh re-verify retry below;
/// interactive approvals keep reporting "needs merge" unchanged.
async fn land_approved_goal(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    project_id: Option<&str>,
    headless_auto_land: bool,
) -> (
    Option<(String, crate::goal_landing::LandOutcome)>,
    Option<String>,
) {
    let Some(card) = crate::cards::get_card(pool, goal_id).await.ok().flatten() else {
        return (None, None);
    };
    let project_id = project_id.unwrap_or(card.project_id.as_str()).to_string();
    let configured_root = crate::projects::get_project_by_id_or_slug(pool, &project_id)
        .await
        .ok()
        .flatten()
        .and_then(|project| project.root_path);

    // The ROLLING `dispatch_evidence.commits` only answers "did THIS attempt
    // commit anything new" — Y16b resets it to the prior attempt's own tip
    // on every re-dispatch, so once an earlier attempt commits a real fix,
    // every LATER attempt's rolling evidence reads empty even though the
    // fix already sits, unlanded, on the goal branch.
    let rolling_commits_empty = card
        .metadata_json
        .pointer("/dispatch_evidence/commits")
        .and_then(|v| v.as_array())
        .is_none_or(|commits| commits.is_empty());
    let worktree_path_hint = card
        .metadata_json
        .pointer("/dispatch_evidence/worktree_path")
        .and_then(|v| v.as_str());

    // R43a (A38 Miss 2): before refusing on an empty rolling gate, consult
    // the SAME trunk-relative question `maybe_hold_review` (R42a) and
    // `maybe_salvage_committed_work`/the read-only-landing refusal
    // (R22d/R29b) already trust — does the goal branch carry ANY commit the
    // trunk hasn't seen yet, regardless of which attempt made it. A goal
    // reviewed and approved on the strength of an earlier attempt's commit
    // must not be refused here just because THIS attempt's own rolling
    // evidence is empty.
    //
    // R43a-b (G31 review, HIGH-1): an unlanded commit ALONE is not evidence
    // it was ever verified — R28a's teardown checkpoint commits every dirty
    // tracked change unconditionally, with no build/test check. Mirror
    // `maybe_hold_review`'s own R42a-b discipline: only credit the
    // trunk-relative commit as landable when EITHER this goal's own
    // recorded verdict is a PASS bound to that exact head
    // (`dispatch_evidence.head_commit == worktree HEAD`), or a fresh,
    // time-boxed verify against the worktree genuinely passes right now
    // (`run_fresh_verify_for_hold`, the identical helper `maybe_hold_
    // review`'s override calls). Neither is attempted when the goal's own
    // `salvage_receipt` already recorded a verify FAILURE for this branch —
    // salvage already spent time proving this exact, still-unmoved range
    // broken.
    //
    // Falls back to the plain refusal below whenever no trusted trunk root
    // resolves, the worktree hint is missing, nothing is actually unlanded
    // there, or nothing verifies at that head — never widened beyond that.
    // `refusal_reason` names WHY, for the note below: `no_evidence` (no
    // worktree hint to even consult), `no_trunk` (no trusted trunk root
    // resolved), `nothing_unlanded` (trunk resolved; zero unlanded
    // commits), `unverified_head` (commits are unlanded but none verify at
    // that exact head).
    let (trunk_relative_head, refusal_reason) = if rolling_commits_empty {
        match worktree_path_hint {
            Some(worktree) => {
                let worktree_path = std::path::Path::new(worktree);
                match crate::agents::platform_extensions::orchestrator::commits_unlanded_at_trunk(
                    pool,
                    &card,
                    worktree_path,
                )
                .await
                {
                    Ok((unlanded, _trunk_head)) if unlanded > 0 => {
                        let salvage_already_failed = card
                            .metadata_json
                            .pointer("/salvage_receipt/outcome")
                            .and_then(|v| v.as_str())
                            == Some("salvage_verify_failed");
                        if salvage_already_failed {
                            (None, "unverified_head")
                        } else {
                            let current_head =
                                crate::agents::platform_extensions::orchestrator::worktree_head_commit(
                                    worktree_path,
                                )
                                .await;
                            let recorded_pass_at_head = card
                                .metadata_json
                                .pointer("/dispatch_evidence/verdict/status")
                                .and_then(|v| v.as_str())
                                == Some("pass")
                                && match (
                                    card.metadata_json
                                        .pointer("/dispatch_evidence/head_commit")
                                        .and_then(|v| v.as_str()),
                                    current_head.as_deref(),
                                ) {
                                    (Some(recorded), Some(current)) => {
                                        crate::agents::platform_extensions::orchestrator::sha_matches(
                                            recorded, current,
                                        )
                                    }
                                    _ => false,
                                };
                            let verified = recorded_pass_at_head
                                || crate::agents::platform_extensions::orchestrator::run_fresh_verify_for_hold(
                                    pool,
                                    &card,
                                    worktree_path,
                                )
                                .await;
                            if verified {
                                match current_head {
                                    Some(head) => (Some(head), ""),
                                    None => (None, "unverified_head"),
                                }
                            } else {
                                (None, "unverified_head")
                            }
                        }
                    }
                    Ok(_) => (None, "nothing_unlanded"),
                    Err(_) => (None, "no_trunk"),
                }
            }
            None => (None, "no_evidence"),
        }
    } else {
        (None, "")
    };

    // R62d (A61 Miss 5): a goal `maybe_hold_review` allowed as already
    // satisfied at its baseline (a genuine verify bound to this attempt,
    // nothing to commit, the trunk already carrying its files as verified)
    // has nothing to land by construction. Complete it with the receipt's
    // honest note and NO landing outcome — never a fabricated "landed" —
    // instead of parking it for "no commits past the baseline". Only when
    // the trunk check itself found nothing unlanded, and only while the
    // receipt still names this worker session at the worktree's current
    // head.
    if rolling_commits_empty
        && trunk_relative_head.is_none()
        && refusal_reason == "nothing_unlanded"
    {
        if let Some(worktree) = worktree_path_hint {
            if let Some(note) =
                crate::agents::platform_extensions::orchestrator::already_satisfied_receipt_still_holds(
                    &card,
                    std::path::Path::new(worktree),
                )
                .await
            {
                tracing::info!(
                    goal_id,
                    "land_skipped reason=already_satisfied: {note}"
                );
                return (None, Some(format!("note: {note}")));
            }
        }
    }

    // Missing evidence or empty history cannot establish committed work.
    // Preserve an actionable refusal even if the upstream hold was bypassed.
    if rolling_commits_empty && trunk_relative_head.is_none() {
        let worktree = worktree_path_hint.unwrap_or("unknown worktree");
        // R43a-c: the card note stays exactly the human sentence it always
        // was — `refusal_reason` is a machine diagnostic and goes to
        // tracing plus the persisted, structured landing outcome below,
        // never prepended onto user-visible text (two pre-existing tests,
        // `land_empty_commit_evidence_reports_note_and_unblock` and
        // `land_approved_goal_requires_a_verified_head_for_the_trunk_
        // relative_override`, want the same note shape for different
        // reasons — only the structured field can distinguish them).
        let note = format!(
            "note: not landed — goal {goal_id} has no commits past the baseline \
             (uncommitted work in {worktree})"
        );
        tracing::warn!(
            goal_id,
            reason = refusal_reason,
            "land_refused reason={refusal_reason}: {note}"
        );
        let outcome = crate::goal_landing::LandOutcome::Skipped(note.clone());
        persist_landing_outcome(pool, goal_id, &card, &outcome, Some(refusal_reason)).await;
        crate::agents::platform_extensions::orchestrator::park_refused_goal(
            pool,
            &card,
            &note,
            Some(decisions::UnblockCause::AutoLandRefused),
        )
        .await;
        return (Some((project_id, outcome)), Some(note));
    }

    // When the override above found real trunk-unlanded work despite empty
    // rolling evidence, hand the rest of this function a metadata view
    // corrected to match: `evidence_head_and_worktree`/`land_request_from_
    // metadata` both deliberately read the ROLLING `commits`/`head_commit`
    // pair as "nothing to land" whenever `commits` is empty (see their own
    // doc comment and `goal_landing::tests::a_worker_that_committed_
    // nothing_has_nothing_to_land`) — exactly the trap this override exists
    // to see past. Only `commits`/`head_commit` are substituted; every
    // other field (worktree_path, baseline_commit, ...) is the card's own.
    let landing_metadata = match &trunk_relative_head {
        Some(head) => {
            let mut evidence = card
                .metadata_json
                .get("dispatch_evidence")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            evidence.insert(
                "head_commit".to_string(),
                serde_json::Value::String(head.clone()),
            );
            evidence.insert(
                "commits".to_string(),
                serde_json::json!([
                    "(trunk-relative: unlanded commit(s) from an earlier attempt, R43a)"
                ]),
            );
            serde_json::json!({ "dispatch_evidence": evidence })
        }
        None => card.metadata_json.clone(),
    };

    let Some((worktree_path, _head_commit)) =
        crate::goal_landing::evidence_head_and_worktree(&landing_metadata)
    else {
        // A41 audit (row #23): `landing_metadata` is built above to always
        // carry a usable `head_commit`/non-empty `commits` pair once the
        // trunk-relative override fired (`trunk_relative_head.is_some()`),
        // so reaching here on that path would itself be a bug — but the
        // NON-override path (`rolling_commits_empty == false`) still hands
        // this the card's own raw metadata unmodified, and nothing enforces
        // `head_commit` being present/non-empty whenever `commits` merely
        // happens to be non-empty.
        //
        // R43a-b (M38 gate): two genuinely different shapes reach here and
        // they are NOT the same defect. A `head_commit` key present but
        // blank/empty (or `commits` present-but-empty) is a real,
        // inconsistent evidence shape — the worker's own session recorded
        // SOME commits yet left nothing usable to land from; that must stay
        // a reported, parked refusal (unchanged from the A41 hardening
        // below). But a `head_commit` key entirely ABSENT means this
        // evidence was never shaped to describe a land in the first place —
        // the sibling-credit (`try_credit_sibling_goals`) and
        // promoted-dependent (Y15) card shapes both stamp `commits` as a
        // marker/receipt with no `head_commit` field at all, because the
        // credited/promoted goal has no worktree of its OWN to land from
        // (the commit belongs to a different goal's dispatch). Parking
        // THOSE goals here re-fails a Review→Complete transition
        // `advance_goal_checked_with_auto_land` already made before this
        // function ever ran — the exact M38 gate regression (two
        // pre-existing landing-success tests: `approving_a_review_
        // dispatches_the_promoted_dependent`,
        // `try_credit_sibling_goals_auto_lands_the_credited_goal_under_
        // headless_auto_land`). Falls back to the ORIGINAL rolling-evidence
        // behaviour for that shape: report it honestly (never silent) but
        // never park a goal that has nothing of its own to land.
        let worktree = worktree_path_hint.unwrap_or("unknown worktree");
        let head_commit_key_present = landing_metadata
            .pointer("/dispatch_evidence/head_commit")
            .is_some();
        if !head_commit_key_present {
            let note = format!(
                "land_refused reason=no_evidence: note: not landed — goal {goal_id} has no \
                 worktree of its own to land from (worktree {worktree})"
            );
            tracing::info!(goal_id, "{note}");
            let outcome = crate::goal_landing::LandOutcome::Skipped(note.clone());
            persist_landing_outcome(pool, goal_id, &card, &outcome, None).await;
            return (Some((project_id, outcome)), Some(note));
        }
        let note = format!(
            "land_refused reason=no_evidence_head: goal {goal_id} has no usable head commit \
             to land (worktree {worktree})"
        );
        tracing::warn!(goal_id, "{note}");
        let outcome = crate::goal_landing::LandOutcome::Skipped(note.clone());
        persist_landing_outcome(pool, goal_id, &card, &outcome, None).await;
        crate::agents::platform_extensions::orchestrator::park_refused_goal(
            pool,
            &card,
            &note,
            Some(decisions::UnblockCause::AutoLandRefused),
        )
        .await;
        return (Some((project_id, outcome)), Some(note));
    };

    // R1 (harness loop, Lane D1, from Lane F8 forensics 2026-09-10): a
    // project seeded with no `root_path` (the eval harness's default
    // `personal` project, `session/spectral_schema.rs:610`, and any project
    // created with no local folder) used to make this function return
    // `(None, None)` right here — landing silently no-op'd, `Review ->
    // Complete` proceeded unconditionally, and the card read as landed while
    // the trunk never moved (F8's fx02/fx03: two genuinely-correct,
    // oracle-passing fixes — 7/7 and 13/13 tests re-run against the pristine
    // hidden oracle — lost this way, 100%-reproducing). When the project
    // itself doesn't know its root, fall back to what the goal's own
    // worktree knows about its git baseline before giving up.
    //
    // H1 (G8 review, harness loop Lane E3 2026-09-10): that fallback alone
    // is not enough — "git says this is where the worktree came from" is
    // not the same as "this is the project the user intended", and the
    // daemon's own cwd could be exactly what a NULL `root_path` fell back
    // to at dispatch time (`orchestrator.rs`, closed alongside this fix).
    // `verify_worktree_landing_target` requires the recovered candidate to
    // match an independently recorded working directory (the dispatching
    // session's own `sessions.working_dir`) AND for the goal's own dispatch
    // baseline to be a real ancestor of the candidate's HEAD before this
    // fallback is trusted — see that function's own doc comment for why
    // both checks are required. A mismatch is recorded and reported exactly
    // like every other landing refusal in this function; it is never a
    // silent no-op and never an attempted fast-forward.
    let root = match configured_root {
        Some(root) => Some(root),
        None => match crate::goal_landing::resolve_primary_checkout_from_worktree(&worktree_path)
            .await
        {
            Some(candidate) => {
                let baseline_commit = card
                    .metadata_json
                    .pointer("/dispatch_evidence/baseline_commit")
                    .and_then(|v| v.as_str());
                let known_working_dir =
                    goal_dispatching_session_working_dir(pool, &card.metadata_json).await;
                match crate::goal_landing::verify_worktree_landing_target(
                    &candidate,
                    baseline_commit,
                    known_working_dir.as_deref(),
                )
                .await
                {
                    Ok(()) => Some(candidate),
                    Err(reason) => {
                        let outcome = crate::goal_landing::LandOutcome::Skipped(format!(
                            "landing target is not the goal's repo ({reason})"
                        ));
                        tracing::warn!(
                            goal_id,
                            project_id = %project_id,
                            candidate_root = %candidate,
                            %reason,
                            "land_approved_goal: worktree-derived landing target failed \
                             verification; refusing to land rather than trust an unverified \
                             fallback"
                        );
                        persist_landing_outcome(pool, goal_id, &card, &outcome, None).await;
                        return (Some((project_id, outcome)), None);
                    }
                }
            }
            None => None,
        },
    };

    let Some(root) = root else {
        // There IS real work to land (we already confirmed a head commit and
        // a non-empty commit range above) but no root path could be
        // resolved by any means — never silently return `(None, None)` here.
        // Record the honest failure on the card and the approval response,
        // exactly as `goal_landing::land`'s own refusals do.
        let outcome = crate::goal_landing::LandOutcome::Skipped(format!(
            "project '{project_id}' has no root_path and the goal's own worktree \
             ('{worktree_path}') could not resolve a primary checkout via git — \
             landing needs a repo root to fast-forward onto, not silence"
        ));
        tracing::warn!(
            goal_id,
            project_id = %project_id,
            worktree_path = %worktree_path,
            "land_approved_goal: no landing target could be resolved; recording an \
             honest failure instead of the old silent no-op"
        );
        persist_landing_outcome(pool, goal_id, &card, &outcome, None).await;
        return (Some((project_id, outcome)), None);
    };

    let Some(req) = crate::goal_landing::land_request_from_metadata(&landing_metadata, Some(&root))
    else {
        // A41 audit (row #23), same hardening as the `evidence_head_and_
        // worktree` check above: `land_request_from_metadata` re-derives
        // the same `(worktree, head_commit)` pair from `landing_metadata`
        // (already proven resolvable a few lines up) and additionally
        // needs the worktree path's own file name to derive a branch name
        // — the only way to land here is a pathological worktree path
        // (empty, `/`, or ending in `..`). Report it rather than silently
        // returning `(None, None)`.
        let note = format!(
            "land_refused reason=no_evidence_head: goal {goal_id}'s worktree path \
             ('{worktree_path}') could not be turned into a landing request"
        );
        tracing::warn!(goal_id, "{note}");
        let outcome = crate::goal_landing::LandOutcome::Skipped(note.clone());
        persist_landing_outcome(pool, goal_id, &card, &outcome, None).await;
        crate::agents::platform_extensions::orchestrator::park_refused_goal(
            pool,
            &card,
            &note,
            Some(decisions::UnblockCause::AutoLandRefused),
        )
        .await;
        return (Some((project_id, outcome)), Some(note));
    };
    let mut req = req;
    let mut outcome = crate::goal_landing::land(&req).await;

    // Lane R63b (M47 r63 s2): R56a's derived disjoint write scopes dispatch
    // sibling goals in PARALLEL, so once the first lands the rest cannot
    // fast-forward — fast-forward-only landing was only enough while siblings
    // were serialized. Under headless auto-land, rebase the goal branch onto
    // the moved trunk (never a merge commit: history stays linear), re-verify
    // the rebased head, and land it through the SAME `land` call, so every
    // receipt, note and consumer below is unchanged. The helper refuses
    // unless the goal's write_scope and own changes are disjoint from what the
    // trunk changed, the worktree is clean and the rebase applies. A refusal
    // or conflict leaves the branch intact and keeps the honest `NeedsMerge`;
    // a failed re-verify parks the goal. Bounded, because a sibling can land
    // while this one re-verifies.
    let mut card = card;
    let mut rebase_refusal: Option<&'static str> = None;
    if headless_auto_land {
        for _ in 0..HEADLESS_REBASE_MAX_ATTEMPTS {
            let crate::goal_landing::LandOutcome::NeedsMerge { trunk, .. } = &outcome else {
                break;
            };
            let attempt =
                crate::agents::platform_extensions::orchestrator::rebase_goal_branch_for_headless_land(
                    pool,
                    &card,
                    std::path::Path::new(&worktree_path),
                    &root,
                    trunk,
                    &req.branch,
                    &req.head_commit,
                )
                .await;
            // The helper re-points metadata on a rebase; re-read so the
            // landing receipt below never writes a stale evidence copy back.
            if let Some(fresh) = crate::cards::get_card(pool, goal_id).await.ok().flatten() {
                card = fresh;
            }
            match attempt {
                crate::agents::platform_extensions::orchestrator::HeadlessRebase::Verified {
                    head,
                } => {
                    req.head_commit = head;
                    outcome = crate::goal_landing::land(&req).await;
                    rebase_refusal = Some("rebase_retries_exhausted");
                }
                crate::agents::platform_extensions::orchestrator::HeadlessRebase::NotRebased {
                    reason,
                } => {
                    rebase_refusal = Some(reason);
                    break;
                }
                crate::agents::platform_extensions::orchestrator::HeadlessRebase::ReverifyFailed {
                    detail,
                } => {
                    let note = format!("note: not landed — {detail}");
                    tracing::warn!(
                        goal_id,
                        reason = "rebase_reverify_failed",
                        "land_refused reason=rebase_reverify_failed: {note}"
                    );
                    let outcome = crate::goal_landing::LandOutcome::Skipped(note.clone());
                    persist_landing_outcome(
                        pool,
                        goal_id,
                        &card,
                        &outcome,
                        Some("rebase_reverify_failed"),
                    )
                    .await;
                    crate::agents::platform_extensions::orchestrator::park_refused_goal(
                        pool, &card, &note, Some(decisions::UnblockCause::AutoLandRefused),
                    )
                    .await;
                    // Feed the Brain here too (fix-as-found, 2026-09-25): this
                    // early return used to skip the landing-outcome feed
                    // entirely, so a headless rebase's failed re-verify — a
                    // real, actionable outcome — never reached memory. Same
                    // call as the fall-through path below.
                    crate::brain_feed::feed_for_project(
                        pool,
                        crate::brain_feed::FeedSource::Goal,
                        crate::brain_feed::feed_key("goal", goal_id, "landing"),
                        &card.project_id,
                        format!("Goal '{}' landing: {:?}", card.title, outcome),
                    )
                    .await;
                    return (Some((project_id, outcome)), Some(note));
                }
            }
        }
        if !matches!(outcome, crate::goal_landing::LandOutcome::NeedsMerge { .. }) {
            rebase_refusal = None;
        }
    }

    // YF2 (G6 review, Lane Y-fix 2026-09-09, finding #2): this outcome used
    // to be computed and then discarded — only folded into a one-shot
    // response string. Review -> Complete is unconditional, so a card could
    // reach Complete with real commits whose `land()` call returned
    // `NeedsMerge`/`Skipped`, and every later reader
    // (`goal_landed_head_commit`, the CLI drain summary) had no durable way
    // to tell that apart from a genuine landing — printing a fabricated
    // `landed <id> → <sha>` line whenever the trunk had moved out from
    // under the worker (e.g. a second goal in the same roadmap landing
    // first). Persist it onto the card so those readers can gate on whether
    // landing actually happened, not merely whether it was attempted.
    persist_landing_outcome(pool, goal_id, &card, &outcome, rebase_refusal).await;

    // Feed the Brain: what landing this goal actually did (or honestly why
    // it did not) — the outcome future dispatches should recall.
    crate::brain_feed::feed_for_project(
        pool,
        crate::brain_feed::FeedSource::Goal,
        crate::brain_feed::feed_key("goal", goal_id, "landing"),
        &card.project_id,
        format!("Goal '{}' landing: {:?}", card.title, outcome),
    )
    .await;

    let warning = if let crate::goal_landing::LandOutcome::NeedsMerge { branch, trunk } = &outcome {
        ensure_needs_merge_decision(pool, goal_id, &card.title, &card.project_id, branch, trunk)
            .await
            .err()
            .map(|error| format!("could not create manual-merge inbox item: {}", error))
    } else {
        None
    };

    (Some((project_id, outcome)), warning)
}

/// Lane R63b: how many rebase + re-verify rounds one headless landing may
/// spend when siblings keep moving the trunk underneath it.
const HEADLESS_REBASE_MAX_ATTEMPTS: usize = 3;

/// Shared `dispatch_evidence.landing` persistence (YF2, G6 review, Lane
/// Y-fix 2026-09-09) — factored out (R1, Lane D1) so both the ordinary git
/// fast-forward path and the "no landing target resolvable" honest-failure
/// path leave the same durable trail for `goal_landed_head_commit`/the CLI
/// drain summary to read back later. Best-effort: a metadata-write failure
/// is logged, never propagated — the landing itself (or its refusal) has
/// already happened by the time this runs.
async fn persist_landing_outcome(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    card: &crate::cards::Card,
    outcome: &crate::goal_landing::LandOutcome,
    refusal_reason: Option<&str>,
) {
    let landing_json = serde_json::json!({
        "outcome": outcome.outcome_key(),
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10):
        // explicit trunk-reached signal, independent of `outcome`'s coarser
        // grouping and of any external grading/overlay step's own verdict —
        // F9 found fx02/fx03 graded "solved" while `main`'s HEAD never
        // moved. So M10's analysis (and any other reader) can tell a
        // genuine land apart from a fix that stayed in the working tree.
        "trunk_state": outcome.trunk_state(),
        "landed_head": outcome.landed_head(),
        "describe": outcome.describe(),
        // R43a-c (M38 gate, 2026-09-15 20:20): machine diagnostics never go
        // into the human-readable card note (two pre-existing tests want
        // opposite note text depending only on WHY landing was refused) —
        // the reason lives here instead, as a durable, structured field a
        // reader can match on without re-parsing prose. `None` (persisted
        // as `null`) for every non-refusal outcome and for refusals that
        // predate this field's introduction.
        "refusal_reason": refusal_reason,
        "at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(mut evidence) = card
        .metadata_json
        .get("dispatch_evidence")
        .and_then(|value| value.as_object())
        .cloned()
    {
        evidence.insert("landing".to_string(), landing_json);
        if let Err(error) = crate::cards::merge_card_metadata(
            pool,
            goal_id,
            serde_json::json!({ "dispatch_evidence": serde_json::Value::Object(evidence) }),
            false,
        )
        .await
        {
            tracing::warn!(
                goal_id,
                %error,
                "could not persist the landing outcome onto dispatch_evidence"
            );
        }
    }
}

/// Registered program successors may start only after the approved source is
/// actually present on the project trunk. Re-run the existing fast-forward
/// check on an outbox retry so a crash between Review → Complete and landing
/// cannot make a successor run against stale trunk state.
async fn ensure_registered_goal_landed(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    headless_auto_land: bool,
) -> Result<(), GuardError> {
    let card = crate::cards::get_card(pool, goal_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
    let registered = card
        .metadata_json
        .get("program")
        .and_then(|program| program.get("manifest"))
        .is_some();
    if !registered {
        return Ok(());
    }
    if card
        .metadata_json
        .get("program")
        .and_then(|program| program.get("delivery"))
        .and_then(|delivery| delivery.as_str())
        == Some("no_write")
    {
        // The approved ProgramDag explicitly declares this node as research /
        // audit work. Its typed verifier receipts, not a missing commit, are
        // the completion authority; there is no trunk to fast-forward.
        return Ok(());
    }
    let (landing, warning) =
        land_approved_goal(pool, goal_id, Some(&card.project_id), headless_auto_land).await;
    let Some((_, outcome)) = landing else {
        return Err(GuardError::Db(format!(
            "registered continuation blocked: approved goal '{}' has no durable landing outcome{}",
            goal_id,
            warning
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default()
        )));
    };
    if !outcome.is_landed() {
        return Err(GuardError::Db(format!(
            "registered continuation blocked until source landing succeeds: {}",
            outcome.describe()
        )));
    }
    Ok(())
}

/// Leave one durable, copy-pasteable recovery item when automatic landing
/// cannot fast-forward. An existing open unblock for the goal wins: two open
/// requests for the same goal would compete for the human's answer.
async fn ensure_needs_merge_decision(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    goal_title: &str,
    project_id: &str,
    branch: &str,
    trunk: &str,
) -> Result<(), String> {
    if decisions::find_open_decision_for_goal(pool, goal_id, "unblock")
        .await?
        .is_some()
    {
        return Ok(());
    }

    let detail = format!(
        "Goal \"{}\" ({}) finished on branch `{}`, but it could not be fast-forwarded onto trunk `{}`.\n\nMerge it:\n```sh\ngit switch {}\ngit merge {}\n```\n\nOr rebase it:\n```sh\ngit switch {}\ngit rebase {}\ngit switch {}\ngit merge --ff-only {}\n```",
        goal_title, goal_id, branch, trunk, trunk, branch, branch, trunk, trunk, branch
    );
    decisions::create_decision(
        pool,
        decisions::NewDecision {
            kind: "unblock".to_string(),
            goal_id: Some(goal_id.to_string()),
            project_id: Some(project_id.to_string()),
            headline: Some(
                "Goal work could not land and needs a manual merge or rebase".to_string(),
            ),
            detail: Some(detail),
            payload: serde_json::to_value(decisions::UnblockPayload {
                reason: decisions::UnblockReason::Stuck,
                cause: Some(decisions::UnblockCause::MergeConflict),
                ..Default::default()
            })
            .map_err(|error| error.to_string())?,
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// What each allowed answer does, in one plain sentence (Decision Inbox
/// redesign D4). Lives here, beside [`apply_decision_effect_with_auto_land`],
/// so the copy the modal shows and the arm that runs cannot drift apart:
/// change an arm below, change its sentence here. Every sentence promises only
/// what the arm (or, for scheduled jobs, `scheduler::scheduled_job_resume`;
/// for tool/session gates, the waiter the answer route delivers to) actually
/// does; where the effect is only a record, the sentence says so.
///
/// Keyed on exactly [`decisions::allowed_answers`] for the kind — a unit test
/// pins that — so the UI never offers an answer with no sentence, or a
/// sentence for an answer the route would refuse.
///
/// Without the goal's state, a goal-bound unblock is read as the one its goal
/// is paused on — true of every park — except a merge-conflict unblock, which
/// is filed after the goal completed. A caller that read the goal's state
/// uses [`answer_effects_for_goal`].
pub fn answer_effects(
    kind: &str,
    payload: &serde_json::Value,
    has_goal: bool,
) -> std::collections::BTreeMap<String, String> {
    let merge_conflict = payload.get("cause").and_then(|v| v.as_str()) == Some("merge_conflict");
    answer_effects_for_goal(kind, payload, has_goal.then_some(!merge_conflict))
}

/// [`answer_effects`] for a known goal state. `goal_parked` is `None` when the
/// decision has no goal, else whether its goal is paused on this decision
/// right now ([`goal_state_is_parked`]). An unblock's answers only move a
/// paused goal: the approve arm leaves a running goal alone ("already
/// unparked"), the reject arm cancels only a paused one (D8), and
/// `resume_answered_decision` resumes only a paused one — so a running goal's
/// unblock (a worker's question, a stalled-gate check) gets record-only copy.
pub fn answer_effects_for_goal(
    kind: &str,
    payload: &serde_json::Value,
    goal_parked: Option<bool>,
) -> std::collections::BTreeMap<String, String> {
    fn str_of<'a>(payload: &'a serde_json::Value, key: &str) -> &'a str {
        payload.get(key).and_then(|v| v.as_str()).unwrap_or("")
    }
    let has_goal = goal_parked.is_some();
    let goal_parked = goal_parked.unwrap_or(false);
    let proposal = str_of(payload, "proposal");
    let reason = str_of(payload, "reason");
    let cause = str_of(payload, "cause");
    let action_class = str_of(payload, "action_class");
    let pairs: Vec<(&str, String)> =
        match kind {
            "approve_review" => vec![
                (
                    "approve",
                    "Marks the goal done and lands its work once its checks have passed.".into(),
                ),
                (
                    "reject",
                    "Sends the work back for another attempt, with your note as the reason. If it \
                 has no attempts left, the goal is paused for you instead."
                        .into(),
                ),
            ],
            // Scheduled-job unblocks never reach the arms below: the answer route
            // hands them to `scheduler::scheduled_job_resume` (D6).
            "unblock" if reason == decisions::CONSECUTIVE_FAILURE_LIMIT => vec![
                (
                    "approve",
                    "Turns the scheduled job back on; it runs again at its next scheduled time."
                        .into(),
                ),
                (
                    "reject",
                    "Keeps the job paused. You can resume it later from Automate.".into(),
                ),
            ],
            "unblock" if !has_goal => vec![
                (
                    "approve",
                    "Records your answer. Nothing else changes.".into(),
                ),
                (
                    "reject",
                    "Records your answer. Nothing else changes.".into(),
                ),
                (
                    "input",
                    "Saves your reply on this decision. Nothing else changes.".into(),
                ),
            ],
            // A merge-conflict unblock is filed after Review -> Complete: the
            // approve arm finds the goal "already unparked" and the reject arm
            // finds it "already complete". Neither merges anything.
            "unblock" if !goal_parked && cause == "merge_conflict" => vec![
                (
                    "approve",
                    "Closes this. Nothing is merged for you: the branch still needs the merge or \
                     rebase shown in the details."
                        .into(),
                ),
                (
                    "reject",
                    "Closes this. Nothing is merged or cancelled: the branch keeps its work."
                        .into(),
                ),
                (
                    "input",
                    "Saves your reply on this decision. Nothing is merged for you.".into(),
                ),
            ],
            "unblock" if !goal_parked => vec![
                (
                    "approve",
                    "Records your answer. The goal isn't paused on this, so nothing else changes."
                        .into(),
                ),
                (
                    "reject",
                    "Records your answer. The goal isn't paused on this, so it is not cancelled."
                        .into(),
                ),
                (
                    "input",
                    "Saves your reply on this decision. The goal isn't paused on this, so nothing \
                     else changes."
                        .into(),
                ),
            ],
            "unblock" => vec![
                (
                    "approve",
                    if reason == "wallclock_cap" {
                        // D7: the approve arm also clears the wall-budget anchor.
                        "Wakes the goal with a fresh time budget and more attempts.".into()
                    } else if reason == "token_budget" {
                        // The approve arm also grants a fresh token allowance,
                        // or the next dispatch re-parks on the spent budget.
                        "Wakes the goal with a fresh token allowance and more attempts.".into()
                    } else {
                        "Wakes the goal and gives it more attempts.".into()
                    },
                ),
                // D8: the goal-unblock reject arm cancels a goal paused on it.
                (
                    "reject",
                    "Cancels this goal for good. It will not run again.".into(),
                ),
                (
                    "input",
                    "Saves your reply on this decision and wakes the paused goal to try again."
                        .into(),
                ),
            ],
            "choice" if proposal == decisions::PROPOSAL_CHECK_APPROVAL => vec![
            (
                "choice",
                "Applies the option you pick: run the check once, run it and allow it from now \
                 on, or deny it."
                    .into(),
            ),
            ("reject", "Denies the command; the check does not run.".into()),
        ],
            "choice" if proposal == decisions::PROPOSAL_DEBUG_DISPATCH => {
                vec![
            (
                "choice",
                "Applies the option you pick: send the goal back with a debugger, or leave it \
                 in Review."
                    .into(),
            ),
            ("reject", "Records that neither fits. Nothing else changes.".into()),
        ]
            }
            "choice" if proposal == decisions::PROPOSAL_ROADMAP_CREATE => {
                vec![
                    (
                        "choice",
                        "Records the option you pick. The roadmap is created only if you approve \
                 it."
                        .into(),
                    ),
                    (
                        "reject",
                        "Records that none fit. No roadmap is created.".into(),
                    ),
                ]
            }
            "choice" if has_goal => {
                vec![
            (
                "choice",
                "Records the option you pick. If the goal is paused, it wakes up to continue."
                    .into(),
            ),
            ("reject", "Records that none fit. Nothing else changes.".into()),
        ]
            }
            "choice" => vec![
                ("choice", "Records the option you pick.".into()),
                (
                    "reject",
                    "Records that none fit. Nothing else changes.".into(),
                ),
            ],
            "risk_gate" => {
                let approve = if action_class == "user_data_deletion" && has_goal {
                    "Permanently deletes this goal. This can't be undone."
                } else if action_class == crate::steward::hygiene::ACTION_REPO_WORKTREE_REAP {
                    "Removes this worktree, after checking again that it is safe to."
                } else if action_class == crate::steward::hygiene::ACTION_REPO_BRANCH_DELETE {
                    "Deletes this branch, after checking again that it is safe to."
                } else if action_class == crate::decision_inbox::sink::CAPABILITY_ACTION_CLASS {
                    "Grants the request. If the goal is paused, it wakes up to continue."
                } else {
                    "Records your permission."
                };
                vec![
                    ("approve", approve.into()),
                    ("reject", "Records your refusal. Nothing is changed.".into()),
                ]
            }
            // D9: the approve arm only acknowledges; no automation is created.
            "automation_proposal" => vec![
                ("approve", "Records your approval.".into()),
                ("edit", "Records your approval with your edits.".into()),
                (
                    "reject",
                    if str_of(payload, "normalized_command").is_empty() {
                        "Declines it.".into()
                    } else {
                        "Declines it. This automation won't be suggested again.".into()
                    },
                ),
            ],
            "enrichment_proposal" => vec![
                (
                    "approve",
                    "Writes these details to the person's profile. Details you entered yourself \
                 are kept."
                        .into(),
                ),
                (
                    "reject",
                    "Nothing is written. A note you add is kept as a hint for next time.".into(),
                ),
            ],
            "person_merge_proposal" => vec![
                (
                    "approve",
                    "Merges the duplicate into the other person. The merge can be undone.".into(),
                ),
                (
                    "reject",
                    "Nothing changes; both people stay as they are.".into(),
                ),
            ],
            "person_delete_proposal" => vec![
                (
                    "approve",
                    "Deletes this person, with their meetings and project links. This can't be \
                 undone."
                        .into(),
                ),
                ("reject", "Nothing changes.".into()),
            ],
            "project_intel_proposal" => vec![
                ("approve", "Adds these items to the project's notes.".into()),
                ("reject", "Nothing is written.".into()),
            ],
            "council_action" => vec![
                ("approve", "Files a card for this on the board.".into()),
                (
                    "reject",
                    "Nothing is filed, and the Council won't suggest it again.".into(),
                ),
            ],
            "file_to_project" => vec![
                ("approve", "Files this under the project.".into()),
                ("reject", "Nothing is filed.".into()),
            ],
            "config_change_proposal" => vec![
                ("approve", "Changes the setting as described.".into()),
                ("reject", "No setting is changed.".into()),
            ],
            "model_upgrade" => vec![
                (
                    "approve",
                    "Switches the active model to the proposed one.".into(),
                ),
                ("reject", "The active model stays the same.".into()),
            ],
            "tool_approval" => vec![
                ("approve", "Lets the tool call run this once.".into()),
                ("reject", "Refuses the tool call.".into()),
            ],
            "session_gate" => vec![
                (
                    "approve",
                    "Lets the terminal session continue past this step.".into(),
                ),
                ("reject", "Stops the terminal session at this step.".into()),
            ],
            "regression_proposal" => vec![
                ("approve", "Saves this as a regression test task.".into()),
                ("reject", "Nothing is written.".into()),
            ],
            // D5: no effect arm exists; both answers only close the card.
            "capability_gap" => vec![
                (
                    "approve",
                    "Records that you want this capability. Nothing is built automatically.".into(),
                ),
                (
                    "reject",
                    "Records that it isn't needed. Nothing else changes.".into(),
                ),
            ],
            "malformed" => vec![
                (
                    "approve",
                    "Removes it from your queue. Nothing else changes.".into(),
                ),
                (
                    "reject",
                    "Removes it from your queue. Nothing else changes.".into(),
                ),
            ],
            _ => Vec::new(),
        };
    pairs
        .into_iter()
        .map(|(answer, sentence)| (answer.to_string(), sentence))
        .collect()
}

/// Audit principal the chat `answer_decisions` tool records on every answer
/// it relays. That tool answers as [`decisions::ACTOR_HENRY`] — chat can never
/// stand in for the Tier-2 human — so this principal is what marks the answer
/// as the user's own words rather than a policy's.
pub const CHAT_RELAY_PRINCIPAL: &str = "henry-chat";

/// Whether an answered decision carries the USER's verdict: their tap in the
/// inbox ([`decisions::ACTOR_JESSE`]), or their words relayed by the chat
/// agent ([`decisions::ACTOR_HENRY`] audited under [`CHAT_RELAY_PRINCIPAL`]),
/// as opposed to a policy's, a timer's or the harness's. Only the user's
/// verdict may restart a goal's wall clock, grant it more tokens, or cancel it
/// from its unblock — and it gets those effects whichever channel carried it.
///
/// Read from the answer's durable audit row, so the inline run and an outbox
/// retry of the same answer always agree.
pub(crate) async fn is_users_verdict(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    acted_by: &str,
) -> Result<bool, GuardError> {
    if acted_by == decisions::ACTOR_JESSE {
        return Ok(true);
    }
    if acted_by != decisions::ACTOR_HENRY {
        return Ok(false);
    }
    let principal: Option<Option<String>> = sqlx::query_scalar(
        "SELECT principal FROM decision_audit \
         WHERE decision_id = ? AND acted_by = ? AND outcome = ? \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(&decision.id)
    .bind(acted_by)
    .bind(decision.answer.as_deref().unwrap_or_default())
    .fetch_optional(pool)
    .await
    .map_err(|e| GuardError::Db(format!("read the answer's audit principal: {e}")))?;
    Ok(principal.flatten().as_deref() == Some(CHAT_RELAY_PRINCIPAL))
}

/// Whether a goal in `state` is parked: the Failed column every park moves it
/// to, or the legacy Triage parks. The same reading the unblock approve arm
/// uses ("already unparked" for everything else).
pub fn goal_state_is_parked(state: Option<&str>) -> bool {
    matches!(state, Some("failed" | "triage"))
}

/// Whether the goal's wall clock is what would re-park it (D7). Keyed on the
/// goal's own park, not only on the answered decision's reason:
/// `exhaust_and_park` dedupes a new park onto any open unblock and keeps THAT
/// decision's original payload, so a wall-clock park that reused an earlier
/// worker question (reason `stuck`) was approved without the reset and
/// re-parked on the very next dispatch. True when the answered decision is a
/// wall-clock park, when the goal's last park was one, or when its wall
/// budget is spent right now.
fn wall_clock_blocks_goal(decision: &Decision, metadata: &serde_json::Value) -> bool {
    if decision.payload.get("reason").and_then(|v| v.as_str()) == Some("wallclock_cap") {
        return true;
    }
    let last_park_was_wall_clock = metadata
        .get("last_error")
        .and_then(|v| v.as_str())
        .is_some_and(|e| {
            e.starts_with("wallclock cap exhausted") || e.contains("wall budget exhausted")
        });
    if last_park_was_wall_clock {
        return true;
    }
    // `check_budget`'s wall-clock test, read without the pool.
    let Some(cap_secs) = goal_transition::goal_budget(metadata).wallclock_cap_secs else {
        return false;
    };
    metadata
        .get("first_dispatched_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .is_some_and(|first| {
            let elapsed = (chrono::Utc::now() - first.with_timezone(&chrono::Utc)).num_seconds();
            elapsed >= 0 && elapsed as u64 >= cap_secs
        })
}

/// Whether the goal's last park was on its token budget, by the answered
/// decision's reason or the goal's own last park (the same dedupe as above).
fn token_budget_park(decision: &Decision, metadata: &serde_json::Value) -> bool {
    decision.payload.get("reason").and_then(|v| v.as_str()) == Some("token_budget")
        || metadata
            .get("last_error")
            .and_then(|v| v.as_str())
            .is_some_and(|e| e.starts_with("token budget exhausted"))
}

/// Goal metadata key holding the token allowance a goal was first given, so a
/// token-budget park's approve grants that same allowance again on top of
/// what is spent (see [`unpark_budget_resets`]).
const TOKEN_ALLOWANCE_KEY: &str = "token_budget_allowance";

/// What a user's "wake this goal" answer to an unblock (approve, or an
/// `input` reply to a paused goal) must reset besides the attempt cap, or the
/// very next dispatch re-parks the goal on the same spent budget before any
/// attempt runs. Adds the resets to `patch` — extending, never replacing, a
/// `budget` entry the caller already put there — and returns the clause for
/// the effect message ("" when nothing was reset).
///
/// - Wall clock (D7): that cap is measured from `first_dispatched_at`, the
///   goal's FIRST dispatch, never per attempt. Removing the key (a JSON null
///   in a guarded metadata patch) lets the next dispatch stamp a fresh anchor,
///   exactly as a first dispatch does. See [`wall_clock_blocks_goal`].
/// - Token budget: `check_budget` sums every worker session the goal has
///   used, so the spent total never drops. A token-budget park's approve
///   raises `budget.token_budget` to what is spent plus the original
///   allowance: a fresh allowance of the same size, as the answer's effect
///   sentence says. Only for a token-budget park, so no other answer ever
///   raises spend without saying so.
///
/// The user's verdict only ([`is_users_verdict`]): an automated actor
/// restarting a goal's clock or raising its spend would defeat both budgets'
/// purpose (the headless harness sizes the wall budget to fit inside its own
/// kill deadline).
pub(crate) async fn unpark_budget_resets(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    metadata: &serde_json::Value,
    acted_by: &str,
    patch: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<String, GuardError> {
    if decision.kind != "unblock" || !is_users_verdict(pool, decision, acted_by).await? {
        return Ok(String::new());
    }
    let mut clauses: Vec<String> = Vec::new();
    if wall_clock_blocks_goal(decision, metadata) {
        patch.insert("first_dispatched_at".to_string(), serde_json::Value::Null);
        clauses.push("its wall-clock budget restarted".to_string());
    }
    if token_budget_park(decision, metadata) {
        if let Some(cap) = goal_transition::goal_budget(metadata).token_budget {
            // Unknown accounting (a missing worker total) is not "spent":
            // nothing is raised, and dispatch reports the accounting error.
            if let Ok(spent) = goal_transition::goal_spent_tokens(pool, metadata).await {
                if spent >= cap {
                    // The allowance the goal was first given, remembered at
                    // the first raise, so every raise grants the same size
                    // rather than compounding the previous raise.
                    let allowance = metadata
                        .get(TOKEN_ALLOWANCE_KEY)
                        .and_then(|v| v.as_u64())
                        .unwrap_or(cap);
                    let raised = spent.saturating_add(allowance);
                    let budget = patch.entry("budget".to_string()).or_insert_with(|| {
                        metadata
                            .get("budget")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}))
                    });
                    let lifted = match budget.as_object_mut() {
                        Some(budget) => {
                            budget.insert("token_budget".to_string(), serde_json::json!(raised));
                            true
                        }
                        None => false,
                    };
                    if lifted {
                        patch.insert(
                            TOKEN_ALLOWANCE_KEY.to_string(),
                            serde_json::json!(allowance),
                        );
                        clauses.push(format!(
                            "a fresh allowance of {allowance} tokens (budget now {raised})"
                        ));
                    }
                }
            }
        }
    }
    Ok(clauses.iter().map(|c| format!(" and {c}")).collect())
}

/// Apply an outbox-eligible decision effect without daemon `AppState`.
///
/// `tool_approval` and `session_gate` deliberately are not handled here.
///
/// Thin wrapper over [`apply_decision_effect_with_auto_land`] with headless
/// auto-land off — every pre-existing caller (`goose-server`'s
/// verification/decisions routes, `overbought`, `decision_inbox`, the effect
/// outbox drain below, and every test) keeps this exact signature and this
/// exact behavior; none of them needed to change for Lane V2.
pub async fn apply_decision_effect(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
    kind: &str,
) -> Result<EffectResult, GuardError> {
    apply_decision_effect_with_auto_land(pool, decision, proof, kind, false, None).await
}

/// What [`apply_answered_effect_now`] did with a freshly answered decision.
pub enum AnsweredEffect {
    /// The durable effect ran here. `Ok` is `(effect, warning)`; an `Err` is
    /// already marked failed on the outbox row (the drain retries it), and
    /// both an error and a warning are already audit-recorded.
    Ran(Result<EffectResult, GuardError>),
    /// The outbox drain claimed the effect first and applies it itself.
    ClaimedByDrain,
    /// Not an outbox kind: nothing ran, and the proof goes back to the caller
    /// for the channel's own inline effect (tool approvals, session gates).
    NotDurable(DecisionProof),
}

/// Run the durable effect of an answer a watching user just gave, now rather
/// than on the outbox's next tick: claim the answer's outbox row, apply the
/// effect, settle the row. The one path both user channels take — the inbox's
/// HTTP answer route and the chat `answer_decisions` tool — so a verdict the
/// user states in chat has exactly the effect of the same tap in the inbox:
/// the wall-clock reset (D7) and the reject-cancel (D8) key on
/// [`is_users_verdict`], never on which channel carried the answer.
pub async fn apply_answered_effect_now(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
) -> Result<AnsweredEffect, String> {
    let Some(claim_key) = decisions::effect_outbox_claim_key(&decision.id, &decision.kind) else {
        return Ok(AnsweredEffect::NotDurable(proof));
    };
    if !decisions::claim_effect_outbox(pool, &claim_key).await? {
        return Ok(AnsweredEffect::ClaimedByDrain);
    }
    let result = apply_decision_effect(pool, decision, proof, &decision.kind).await;
    match &result {
        Ok((_, warning)) => {
            if let Err(e) = decisions::mark_effect_outbox_applied(pool, &claim_key).await {
                tracing::error!(
                    "Decision {} effect outbox could not be marked applied: {}",
                    decision.id,
                    e
                );
            }
            if let Some(warning) = warning {
                record_answered_effect_failure(pool, decision, warning).await;
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if let Err(mark_err) =
                decisions::mark_effect_outbox_failed(pool, &claim_key, &msg).await
            {
                tracing::error!(
                    "Decision {} effect outbox could not be marked failed: {}",
                    decision.id,
                    mark_err
                );
            }
            record_answered_effect_failure(pool, decision, &msg).await;
        }
    }
    Ok(AnsweredEffect::Ran(result))
}

/// Log and audit-record an answered decision's effect failure (or a warning
/// from a follow-on step after the effect committed). If the audit write
/// itself fails, that is logged rather than discarded.
async fn record_answered_effect_failure(pool: &Pool<Sqlite>, decision: &Decision, error: &str) {
    tracing::error!(
        "Decision {} answered but effect failed: {}",
        decision.id,
        error
    );
    if let Err(audit_err) =
        decisions::record_effect_outcome(pool, decision, &format!("effect_error: {}", error)).await
    {
        tracing::error!(
            "Decision {} effect failure could not be audit-recorded: {} (original effect error: {})",
            decision.id,
            audit_err,
            error
        );
    }
}

/// [`apply_decision_effect`], plus the two arguments its `approve_review`
/// "approve" arm needs to pass into
/// `goal_transition::advance_goal_checked_with_auto_land` so
/// `check_proof`'s Tier-1 arm can ever accept
/// [`decisions::ACTOR_AUTO_LAUNCH`] (harness loop, Lane V2 — the user's ruling,
/// HARNESS_LOOP_LOG.md "JESSE RULING (2026-09-08 23:55): Tier-1 landing →
/// option 3"). `auto_land_allowed: false` (what [`apply_decision_effect`]
/// always passes) makes this byte-identical to the pre-V2 function. The ONLY
/// caller that ever passes `true` is
/// `orchestrator::auto_land_headless_goal`, itself reached only when a
/// headless Auto dispatch policy has `PERMAGENT_HEADLESS_AUTOLAND` on. Every
/// other match arm in this function ignores both new arguments entirely —
/// they are meaningful only to the `("approve_review", Some("approve"))` arm.
pub async fn apply_decision_effect_with_auto_land(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
    kind: &str,
    auto_land_allowed: bool,
    calling_session_id: Option<&str>,
) -> Result<EffectResult, GuardError> {
    if decision.kind != kind {
        return Err(GuardError::Invalid(format!(
            "outbox kind '{}' does not match decision kind '{}'",
            kind, decision.kind
        )));
    }
    let acted_by = proof.acted_by().to_string();
    match (kind, decision.answer.as_deref()) {
        // An approved regression becomes a real permagent-eval task on disk.
        // The path is re-validated HERE and not trusted from the payload: the
        // decision authorises the content, and a gate that also had to police
        // path traversal would be one mistake away from arbitrary file write.
        ("regression_proposal", Some("approve")) => {
            let payload: crate::decisions::RegressionProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("regression_proposal payload unreadable: {e}"))
                })?;
            // The written path is the effect itself, not a warning: a
            // warning is audit-recorded as an effect error.
            let path = write_regression_task(&payload)?;
            already_applied(format!("regression task written to {path}"))
        }
        // D5: a declined regression writes nothing.
        ("regression_proposal", Some("reject")) => {
            already_applied("regression proposal declined; nothing was written")
        }
        // D5: a capability request has no effect arm; the answer is the record.
        ("capability_gap", Some("approve" | "reject")) => {
            already_applied("capability request answered; recorded only")
        }
        ("approve_review", Some("approve")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match goal_state(pool, goal_id).await?.as_deref() {
                Some("complete") => {
                    // The approval transition may have committed before a
                    // registered continuation hit a transient storage or
                    // dispatch failure. Re-enter only that exact continuation
                    // seam; never replay the broad legacy promoter.
                    ensure_registered_goal_landed(pool, goal_id, auto_land_allowed).await?;
                    match crate::agents::platform_extensions::program_bridge::handoff_registered_goal(pool, goal_id).await {
                        Ok(Some(response))
                            if response.status
                                == crate::agents::platform_extensions::program_bridge::HandoffStatus::PendingDispatch =>
                        {
                            return Err(GuardError::Db(
                                "registered program continuation remains pending dispatch".to_string(),
                            ));
                        }
                        Ok(Some(_)) => return already_applied("goal was already Complete; registered continuation is applied"),
                        Ok(None) => return already_applied("goal was already Complete"),
                        Err(crate::agents::platform_extensions::program_bridge::ProgramHandoffError::Storage(error)) => {
                            return Err(GuardError::Db(format!("registered continuation retryable storage failure: {error}")));
                        }
                        Err(error) => {
                            return already_applied(format!(
                                "goal was already Complete; registered continuation is blocked: {error}"
                            ));
                        }
                    }
                }
                Some("review") => {}
                None => return already_applied("goal was already gone"),
                Some(state) => {
                    return already_applied(format!(
                        "goal already advanced out of Review (current state: {state})"
                    ))
                }
            }
            match approve_review_verification_status(pool, goal_id)
                .await?
                .as_str()
            {
                "pass" => {}
                "pending" => {
                    // Retryable by design: the user already approved, but the
                    // asynchronous verifier has not earned permission to land
                    // yet. Keeping the effect pending preserves that intent.
                    return Err(GuardError::Denied(
                        "verification is still pending; the goal remains in Review".to_string(),
                    ));
                }
                status => {
                    // A completed non-pass is terminal for THIS approval. The
                    // verifier/rework flow may later create a fresh review
                    // decision, but this stale approval can never land it.
                    return already_applied(format!(
                        "approval not applied: verifier verdict is {status}; goal remains in Review"
                    ));
                }
            }
            goal_transition::advance_goal_checked_with_auto_land(
                pool,
                goal_id,
                GoalAction::Approve,
                &acted_by,
                Some(proof),
                TransitionEffects {
                    review_notes: decision.answer_note.clone(),
                    ..Default::default()
                },
                auto_land_allowed,
                calling_session_id,
            )
            .await?;
            // Approving used to move the card and merge NOTHING, so the work
            // stayed on `goal/<run_id>` and the trunk never saw it — measured
            // 2026-08-09, fixed in the inline arm, and that fix never ran
            // because this path short-circuits it (L7). Land it here, where an
            // approval actually arrives. Fast-forward only, and the outcome is
            // ALWAYS reported: a landing that silently declined is exactly the
            // failure this replaces.
            let no_write_program = crate::cards::get_card(pool, goal_id)
                .await
                .ok()
                .flatten()
                .and_then(|card| card.metadata_json.get("program").cloned())
                .and_then(|program| program.get("delivery").cloned())
                .and_then(|delivery| delivery.as_str().map(str::to_owned))
                == Some("no_write".to_string());
            let (landing, landing_warning) = if no_write_program {
                (None, None)
            } else {
                land_approved_goal(
                    pool,
                    goal_id,
                    decision.project_id.as_deref(),
                    auto_land_allowed,
                )
                .await
            };
            if goal_state(pool, goal_id).await?.as_deref() == Some("failed") {
                return Ok((
                    Some("goal not landed — parked with an unblock decision".to_string()),
                    landing_warning,
                ));
            }
            let effect = match &landing {
                Some((_, outcome)) => {
                    format!("goal approved: Review → Complete; {}", outcome.describe())
                }
                None => "goal approved: Review → Complete".to_string(),
            };

            // The trunk just moved: an existing code map now describes the
            // pre-landing tree. Refresh it DETACHED and best-effort — the
            // approval response never waits on (and can never fail because of)
            // a tree-sitter pass. Fast-forward only: refusals and no-op
            // landings changed nothing, so there is nothing to re-index.
            if let Some((project_id, crate::goal_landing::LandOutcome::FastForwarded { .. })) =
                &landing
            {
                if let Some(hook) = POST_LANDING_HOOK.get() {
                    tokio::spawn(hook(pool.clone(), project_id.clone()));
                }
            }

            // D10: promote the dependents this goal was blocking AND start
            // them. Promotion alone left them in Ready with no worker until a
            // human called resume_roadmap by hand.
            //
            // AFTER landing, deliberately: a dependent starts against the
            // trunk, so it must not begin before its parent's work is on it.
            let mut program_lookup_warning = None;
            let program_registered = match crate::cards::get_card(pool, goal_id).await {
                Ok(Some(card)) => card
                    .metadata_json
                    .get("program")
                    .and_then(|program| program.get("manifest"))
                    .is_some(),
                Ok(None) => false,
                Err(error) => {
                    // Fail closed: if we cannot establish that a goal is a
                    // legacy roadmap card, never run the broad dependent
                    // nudge which could bypass a program's exact frontier.
                    program_lookup_warning =
                        Some(format!("registered program lookup unavailable: {error}"));
                    true
                }
            };
            let no_write_program = crate::cards::get_card(pool, goal_id)
                .await
                .ok()
                .flatten()
                .and_then(|card| card.metadata_json.get("program").cloned())
                .and_then(|program| program.get("delivery").cloned())
                .and_then(|delivery| delivery.as_str().map(str::to_owned))
                == Some("no_write".to_string());
            if program_registered && !no_write_program {
                let landed = landing
                    .as_ref()
                    .map(|(_, outcome)| outcome.is_landed())
                    .unwrap_or(false);
                if !landed {
                    return Err(GuardError::Db(
                        "registered continuation blocked until approved source is landed on trunk"
                            .to_string(),
                    ));
                }
            }
            // The program bridge owns the exact successor set for registered
            // programs. Do not also run the broad legacy nudge, which could
            // start an approval-gated or unrelated dependent before the
            // program manifest authorizes it.
            let promotion_warning = if program_registered {
                None
            } else {
                match decision.project_id.as_deref() {
                    Some(project_id) => {
                        crate::agents::platform_extensions::orchestrator::promote_and_dispatch_dependents(
                            pool, project_id, goal_id,
                        )
                        .await
                    }
                    None => None,
                }
            };
            // Registered program roadmaps use the same Complete transition,
            // typed evidence, and dispatcher as ordinary goals. Keep this
            // best-effort after the approval is durable: a continuity fault
            // must be visible and retryable, but cannot roll back a valid
            // human approval or silently approve a successor.
            let program_warning = match crate::agents::platform_extensions::program_bridge::handoff_registered_goal(pool, goal_id).await {
                Ok(Some(response)) => match response.status {
                    crate::agents::platform_extensions::program_bridge::HandoffStatus::PendingDispatch => {
                        return Err(GuardError::Db(
                            "registered program continuation remains pending dispatch".to_string(),
                        ));
                    }
                    _ => None,
                },
                Ok(None) => None,
                Err(crate::agents::platform_extensions::program_bridge::ProgramHandoffError::Storage(error)) => {
                    return Err(GuardError::Db(format!(
                        "registered continuation retryable storage failure: {error}"
                    )));
                }
                Err(error) => Some(format!("registered program continuation: {error}")),
            };
            crate::recognition::write_back_decision_outcome(pool, goal_id, true).await;
            let warning = [
                promotion_warning,
                landing_warning,
                program_lookup_warning,
                program_warning,
            ]
            .into_iter()
            .flatten()
            .reduce(|left, right| format!("{left}; {right}"));
            Ok((Some(effect), warning))
        }
        ("approve_review", Some("reject")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            if goal_state(pool, goal_id).await?.as_deref() != Some("review") {
                return already_applied("goal already advanced out of Review");
            }
            crate::recognition::write_back_decision_outcome(pool, goal_id, false).await;
            let card = crate::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let budget = goal_transition::goal_budget(&card.metadata_json);
            if attempt_count + 1 >= budget.attempt_cap {
                if let Some(existing) =
                    decisions::find_open_decision_for_goal(pool, goal_id, "unblock")
                        .await
                        .map_err(GuardError::Db)?
                {
                    return already_applied(format!(
                        "goal already parked with unblock decision {}",
                        existing.id
                    ));
                }
                let reason = decision
                    .answer_note
                    .clone()
                    .unwrap_or_else(|| "Rejected after maximum attempts".to_string());
                let already_bumped = card
                    .metadata_json
                    .get("last_rejection_decision_id")
                    .and_then(|value| value.as_str())
                    == Some(decision.id.as_str());
                let spent = attempt_count + u64::from(!already_bumped);
                if !already_bumped {
                    sqlx::query(
                        "UPDATE cards
                         SET metadata_json = json_set(
                             metadata_json,
                             '$.attempt_count', ?,
                             '$.last_rejection_decision_id', ?
                         )
                         WHERE id = ? AND column_id IN (
                             SELECT id FROM board_columns WHERE state_binding = 'review'
                         )",
                    )
                    .bind(spent as i64)
                    .bind(&decision.id)
                    .bind(goal_id)
                    .execute(pool)
                    .await
                    .map_err(|e| GuardError::Db(format!("record rejected attempt: {e}")))?;
                }
                let unblock_id = goal_transition::exhaust_and_park_with_cause(
                    pool,
                    goal_id,
                    &card.title,
                    &card.project_id,
                    goal_transition::BudgetExhaustion::AttemptCap {
                        spent,
                        cap: budget.attempt_cap,
                    },
                    Some(&reason),
                    Some(decisions::UnblockCause::ReviewRejected),
                )
                .await
                .map_err(GuardError::Db)?;
                Ok((
                    Some(format!(
                        "goal rejected at attempt cap: parked with unblock decision {unblock_id}"
                    )),
                    None,
                ))
            } else {
                let mut patch = serde_json::Map::new();
                patch.insert(
                    "attempt_count".to_string(),
                    serde_json::json!(attempt_count + 1),
                );
                // Y16c (G19 MED): stamp which session's work this reject
                // decision covers, so a later re-dispatch's
                // `prior_goal_branch_tip` (orchestrator.rs) can refuse to
                // rebuild on this exact branch even if the card later cycles
                // back through Ready with `worker_session_id` still pointing
                // at it (e.g. via a reconciliation path unrelated to review
                // outcome). Only meaningful when a worker actually ran;
                // `null` on a card rejected before any dispatch is harmless
                // (it can never equal a real session id).
                patch.insert(
                    "goal_branch_rejected_session_id".to_string(),
                    card.metadata_json
                        .get("worker_session_id")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
                goal_transition::advance_goal_checked(
                    pool,
                    goal_id,
                    GoalAction::Reject,
                    &acted_by,
                    Some(proof),
                    TransitionEffects {
                        review_notes: decision.answer_note.clone(),
                        metadata_patch: patch,
                        ..Default::default()
                    },
                )
                .await?;
                // A rejected review is the standing "the agent got it wrong"
                // signal, so the approval ladder loses a level here too. The
                // demotion is best-effort: a rejection must land whatever the
                // ladder does.
                demote_check_privilege(pool, decision, "a review of this goal was rejected").await;
                already_applied("goal rejected: Review → InProgress for rework")
            }
        }
        ("unblock", Some("approve")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match goal_state(pool, goal_id).await?.as_deref() {
                Some("ready" | "in_progress" | "review" | "complete") => {
                    return already_applied("goal was already unparked")
                }
                None => return already_applied("goal was already gone"),
                _ => {}
            }
            let card = crate::cards::get_card(pool, goal_id)
                .await
                .map_err(GuardError::Db)?
                .ok_or_else(|| GuardError::NotFound(format!("goal '{goal_id}' not found")))?;
            let attempt_count = card
                .metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let mut budget = card
                .metadata_json
                .get("budget")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let attempt_cap = attempt_count + goal_transition::DEFAULT_ATTEMPT_CAP;
            budget.insert("attempt_cap".to_string(), serde_json::json!(attempt_cap));
            let mut patch = serde_json::Map::new();
            patch.insert(
                "needs_human_attention".to_string(),
                serde_json::json!(false),
            );
            patch.insert("budget".to_string(), serde_json::Value::Object(budget));
            let resets =
                unpark_budget_resets(pool, decision, &card.metadata_json, &acted_by, &mut patch)
                    .await?;
            // QA 2026-09-26: a human's approve also clears the zero-progress
            // circuit breaker's streak and spent escalation, or the very next
            // dispatch reads them and parks the goal again before any worker
            // runs — the same reason D7 restarts the wall budget just above.
            goal_transition::clear_zero_progress_on_human_unblock(&mut patch, &acted_by);
            goal_transition::advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Ready,
                &acted_by,
                Some(proof),
                TransitionEffects {
                    metadata_patch: patch,
                    ..Default::default()
                },
            )
            .await?;
            already_applied(format!(
                "goal unparked to Ready with attempt_cap raised to {attempt_cap}{resets}"
            ))
        }
        // D8: rejecting the unblock a goal is parked on cancels the goal.
        // Before this arm the answer fell through to
        // `resume_answered_decision`, which ignores a reject, leaving a parked
        // goal with no open question at all. Same path as the board's Cancel
        // (`orchestrator::cancel_goal`): terminal transition, stop any worker,
        // supersede its other open decisions.
        //
        // Only a goal actually paused on THIS decision is cancelled. A
        // goal-bound unblock is also filed while the goal keeps running (an
        // interactive worker's `escalate` question, a stalled-gate check), and
        // one can outlive an unpark done by another path; rejecting those is
        // an answer to record — "no" to the worker's question — never a reason
        // to kill a live worker. And only the user's own verdict cancels: the
        // inbox tap or their words relayed by chat, never an automated actor.
        ("unblock", Some("reject")) if decision.goal_id.is_some() => {
            let goal_id = decision.goal_id.as_deref().expect("checked above");
            let state = goal_state(pool, goal_id).await?;
            match state.as_deref() {
                None => return already_applied("goal was already gone"),
                Some("cancelled") => return already_applied("goal was already cancelled"),
                Some("complete") => return already_applied("goal was already complete"),
                _ => {}
            }
            // The one-open-unblock-per-goal dedupe means a parked goal with
            // another unblock still open is paused on that one, not this.
            let parked_on_this = goal_state_is_parked(state.as_deref())
                && decisions::find_open_decision_for_goal(pool, goal_id, "unblock")
                    .await
                    .map_err(GuardError::Db)?
                    .is_none_or(|open| open.id == decision.id);
            if !is_users_verdict(pool, decision, &acted_by).await? {
                return already_applied(if parked_on_this {
                    format!("unblock declined by {acted_by}; the goal stays parked")
                } else {
                    format!("unblock declined by {acted_by}; recorded only")
                });
            }
            if !parked_on_this {
                return already_applied(
                    "answer recorded; the goal was not paused on this decision, so it was left \
                     as it is",
                );
            }
            crate::agents::platform_extensions::orchestrator::cancel_goal(pool, goal_id)
                .await
                .map_err(GuardError::Invalid)?;
            already_applied("goal cancelled at your request")
        }
        ("risk_gate", Some("approve"))
            if decision
                .payload
                .get("action_class")
                .and_then(|v| v.as_str())
                == Some("user_data_deletion")
                && decision.goal_id.is_some() =>
        {
            let goal_id = decision.goal_id.as_deref().expect("checked above");
            if goal_state(pool, goal_id).await?.is_none() {
                return already_applied(format!("goal {goal_id} was already gone"));
            }
            goal_transition::delete_goal_checked(pool, goal_id, proof).await?;
            already_applied(format!("goal {goal_id} deleted"))
        }
        // Steward git-health lane: an approved repo-hygiene cleanup (worktree
        // reap / branch delete). The mutation lives ONLY here, behind the
        // DecisionProof; `apply_repo_hygiene` RE-VERIFIES every safety
        // predicate at effect time (second-look contract) — "world changed"
        // resolves Ok-with-refusal so the outbox stops, "cannot determine"
        // resolves Err so it retries.
        ("risk_gate", Some("approve"))
            if decision
                .payload
                .get("action_class")
                .and_then(|v| v.as_str())
                .is_some_and(|c| {
                    c == crate::steward::hygiene::ACTION_REPO_WORKTREE_REAP
                        || c == crate::steward::hygiene::ACTION_REPO_BRANCH_DELETE
                }) =>
        {
            let payload: decisions::RiskGatePayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored risk_gate payload unreadable: {e}"))
                })?;
            let Some(target) = payload.repo_target.as_ref() else {
                return Err(GuardError::Invalid(
                    "repo-hygiene decision carries no repo_target — nothing safe to apply"
                        .to_string(),
                ));
            };
            crate::steward::hygiene::apply_repo_hygiene(&payload.action_class, target).await
        }
        ("automation_proposal", Some("reject")) => {
            let normalized = decision
                .payload
                .get("normalized_command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !normalized.is_empty() {
                crate::recognition::mark_observation_bounced(pool, normalized).await;
            }
            already_applied("automation proposal declined; will not re-pitch")
        }
        ("automation_proposal", Some("approve" | "edit")) => {
            already_applied(if decision.answer.as_deref() == Some("edit") {
                "automation proposal approved with edits"
            } else {
                "automation proposal approved"
            })
        }
        ("enrichment_proposal", Some("approve")) => {
            let payload: decisions::EnrichmentProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored enrichment payload unreadable: {e}"))
                })?;
            let brain =
                crate::agents::platform_extensions::get_global_brain().ok_or_else(|| {
                    GuardError::Invalid(
                        "Brain is not available — cannot write enriched fields".to_string(),
                    )
                })?;
            let entity_id: spectral::core::entity_id::EntityId =
                payload.graph_entity_id.parse().map_err(|e| {
                    GuardError::Invalid(format!("invalid graph_entity_id in payload: {e:?}"))
                })?;
            let (mut applied, mut protected, mut skipped) = (0usize, 0usize, 0usize);
            for field in &payload.fields {
                if !crate::people::ENRICHABLE_FIELD_NAMES.contains(&field.field_name.as_str()) {
                    skipped += 1;
                    continue;
                }
                let wrote = brain
                    .set_entity_field(
                        entity_id,
                        &field.field_name,
                        &field.value,
                        spectral::ingest::FieldSource::Enriched,
                        Some(&field.source_url),
                    )
                    .await
                    .map_err(|e| {
                        GuardError::Db(format!("set_entity_field({}): {e}", field.field_name))
                    })?;
                if wrote {
                    applied += 1;
                } else {
                    protected += 1;
                }
            }
            let mut message = format!(
                "enrichment applied to \"{}\": {applied} field(s) written with Enriched \
                 provenance, {protected} protected by manual provenance",
                payload.person_name
            );
            if skipped > 0 {
                message.push_str(&format!(", {skipped} skipped (not enrichable)"));
            }
            emit_person_updated(&payload);
            already_applied(message)
        }
        ("enrichment_proposal", Some("reject")) => {
            persist_find_online_hints_on_reject(decision).await
        }
        // Approved person merge (#1073 follow-up). The tool never merges
        // directly — it files this card and a person approves it. Everything
        // the merge does, and everything it deliberately keeps, lives in
        // `people_merge`; this arm only carries the approval across.
        ("person_merge_proposal", Some("approve")) => {
            let payload: decisions::PersonMergeProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored person merge payload unreadable: {e}"))
                })?;
            let brain = crate::agents::platform_extensions::get_global_brain();
            let report = crate::people_merge::merge_people(
                pool,
                brain.as_ref(),
                &payload.survivor_uuid,
                &payload.duplicate_uuid,
            )
            .await
            .map_err(GuardError::Db)?;
            already_applied(format!(
                "{} — undo with merge id {}",
                report.summary, report.merge_id
            ))
        }
        ("person_merge_proposal", Some("reject")) => {
            already_applied("merge declined; both people are unchanged")
        }
        // Approved person delete. Same gate, same reasoning.
        ("person_delete_proposal", Some("approve")) => {
            let payload: decisions::PersonDeleteProposalPayload =
                serde_json::from_value(decision.payload.clone()).map_err(|e| {
                    GuardError::Invalid(format!("stored person delete payload unreadable: {e}"))
                })?;
            let brain = crate::agents::platform_extensions::get_global_brain();
            let report =
                crate::people_merge::delete_person(pool, brain.as_ref(), &payload.entity_uuid)
                    .await
                    .map_err(GuardError::Db)?;
            already_applied(format!(
                "deleted \"{}\": {} meeting(s), {} project link(s), {} graph edge(s). {}",
                report.display_name,
                report.meetings_deleted,
                report.project_links_deleted,
                report.graph_edges_deleted,
                report.retained.join(" ")
            ))
        }
        ("person_delete_proposal", Some("reject")) => {
            already_applied("delete declined; the person is unchanged")
        }
        ("project_intel_proposal", Some("approve")) => apply_project_intel(pool, decision).await,
        ("project_intel_proposal", Some("reject")) => {
            already_applied("project intelligence proposal declined; nothing was written")
        }
        ("council_action", Some("approve")) => apply_council_action(pool, decision).await,
        ("council_action", Some("reject")) => {
            // Retained negative: the same extension point the Initiative layer
            // uses for a declined automation, so a dismissed council
            // recommendation is never re-pitched next Sunday. Keyed on the
            // payload title (what `council::deliver` files from), falling back
            // to the headline. The user's reason stays on the decision row,
            // which is what the next brief assembly reads.
            let subject = decision
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&decision.headline);
            crate::decision_inbox::negatives::record_decline(pool, "council_action", subject).await;
            already_applied("council action dismissed; nothing was filed on the board")
        }
        ("file_to_project", Some("approve")) => apply_file_to_project(pool, decision).await,
        ("file_to_project", Some("reject")) => {
            already_applied("file-to-project proposal declined; nothing was persisted")
        }
        ("config_change_proposal", Some("approve")) => apply_config_change(decision),
        // A rejection writes NOTHING. Said explicitly rather than falling to a
        // catch-all, so "reject changed a setting anyway" would be a visible
        // edit here rather than an accident of arm ordering.
        ("config_change_proposal", Some("reject")) => {
            already_applied("no setting was changed".to_string())
        }
        ("model_upgrade", Some("approve")) => apply_model_upgrade(decision).await,
        ("model_upgrade", Some("reject")) => {
            already_applied("model upgrade declined; the active model is unchanged")
        }
        // The verification approval ladder's Tier-2 answer. Three outcomes, and
        // only a person can produce any of them: the agent files this card, it
        // never answers one.
        ("choice", Some("choice"))
            if decision.payload.get("proposal").and_then(|v| v.as_str())
                == Some(decisions::PROPOSAL_CHECK_APPROVAL) =>
        {
            apply_check_approval(pool, decision, proof, &acted_by).await
        }
        // A rejected command-approval card is a deny, and a deny costs
        // privilege exactly as an explicit `deny` option does.
        ("choice", Some("reject"))
            if decision.payload.get("proposal").and_then(|v| v.as_str())
                == Some(decisions::PROPOSAL_CHECK_APPROVAL) =>
        {
            apply_check_approval(pool, decision, proof, &acted_by).await
        }
        // Review-fail → debugger proposal (the verification FAIL arm files it).
        // Only the debug_dispatch proposal has an effect; every other choice
        // decision stays a pure record — the answer itself is the outcome.
        ("choice", Some("choice"))
            if decision.payload.get("proposal").and_then(|v| v.as_str())
                == Some(decisions::PROPOSAL_DEBUG_DISPATCH) =>
        {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match decision.answer_choice_id.as_deref() {
                Some("dispatch-debugger") => {
                    if goal_state(pool, goal_id).await?.as_deref() != Some("review") {
                        return already_applied("goal already advanced out of Review");
                    }
                    // 1. Sticky debugger mandate — the next dispatch reads
                    //    dispatch_role when assembling the worker brief.
                    sqlx::query(
                        "UPDATE cards SET metadata_json = \
                             json_set(metadata_json, '$.dispatch_role', 'debugger') \
                         WHERE id = ?",
                    )
                    .bind(goal_id)
                    .execute(pool)
                    .await
                    .map_err(|e| GuardError::Db(format!("persist dispatch_role: {e}")))?;
                    // 2. Delegate the state change to the existing review
                    //    reject flow: answer the goal's open approve_review
                    //    decision as 'reject' under the same actor. That arm
                    //    owns the rework transition, attempt budget, and
                    //    at-cap parking — one source of truth, full audit
                    //    trail, and the tier-1 proof gate stays intact.
                    let Some(review) =
                        decisions::find_open_decision_for_goal(pool, goal_id, "approve_review")
                            .await
                            .map_err(GuardError::Db)?
                    else {
                        return already_applied(
                            "debugger mandate recorded, but the goal has no open \
                             approve_review decision to reject — advance it manually",
                        );
                    };
                    decisions::answer_decision(
                        pool,
                        &review.id,
                        &crate::decisions::DecisionAnswer {
                            answer: "reject".to_string(),
                            note: Some(format!(
                                "Debugger re-dispatch requested via decision {}",
                                decision.id
                            )),
                            choice_id: None,
                            input_text: None,
                        },
                        &acted_by,
                    )
                    .await
                    .map_err(|e| {
                        GuardError::Invalid(format!(
                            "debugger mandate recorded, but rejecting review {} failed: {e:?}",
                            review.id
                        ))
                    })?;
                    Ok((
                        Some(
                            "debugger mandate set; review rejected for rework — the goal \
                             returns to InProgress and its next dispatch carries the \
                             debugger brief"
                                .to_string(),
                        ),
                        None,
                    ))
                }
                Some("leave-for-review") => {
                    already_applied("noted — goal stays in Review for the normal approve flow")
                }
                other => already_applied(format!(
                    "unrecognized debug-dispatch option '{}' — no effect applied",
                    other.unwrap_or("(none)")
                )),
            }
        }
        _ => crate::decision_inbox::policy::resume_answered_decision(pool, decision, proof)
            .await
            .map(|effect| (effect, None)),
    }
}

/// Apply an approved `model_upgrade`: switch the active inference model
/// (`GOOSE_MODEL`) to the proposed one. The target must already be installed
/// (download stays in Settings → Models). Idempotent: if the active model is
/// already the target, it reads as applied.
/// An approved `config_change_proposal` becomes the write it described.
///
/// This is the whole point of the proposal path: the agent files a card, the
/// user approves, and the write happens HERE — server-side, on the same
/// `Config::set_param` the Settings pane uses, so it announces itself with
/// `config_changed` like any other. The agent never touched the key.
///
/// `apply_proposed_change` re-validates class, key and value shape before
/// writing. The approval authorises what the CARD said; it is not a blank
/// cheque for whatever the stored payload contains at apply time.
fn apply_config_change(decision: &Decision) -> Result<EffectResult, GuardError> {
    let payload: decisions::ConfigChangeProposalPayload =
        serde_json::from_value(decision.payload.clone()).map_err(|e| {
            GuardError::Invalid(format!(
                "stored config_change_proposal payload unreadable: {e}"
            ))
        })?;

    match crate::agents::platform_extensions::configure_app::apply_proposed_change(
        &payload.key_class,
        &payload.key,
        &payload.value,
    ) {
        Ok(summary) => Ok((Some(summary), None)),
        Err(why) => Err(GuardError::Invalid(format!(
            "could not apply the approved setting change: {why}"
        ))),
    }
}

async fn apply_model_upgrade(decision: &Decision) -> Result<EffectResult, GuardError> {
    let payload: decisions::ModelUpgradePayload = serde_json::from_value(decision.payload.clone())
        .map_err(|e| {
            GuardError::Invalid(format!("stored model_upgrade payload unreadable: {e}"))
        })?;

    // Only switch to a model that is actually installed — otherwise inference
    // would fail. Download is handled by the existing Settings → Models flow.
    #[cfg(feature = "local-inference")]
    {
        let installed = crate::providers::local_inference::local_model_registry::get_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_model(&payload.model_id)
            .is_some();
        if !installed {
            return already_applied(format!(
                "model \"{}\" is not installed — install it from Settings → Models, then propose \
                 the switch again",
                payload.model_id
            ));
        }
    }

    let config = crate::config::Config::global();
    if let Ok(current) = config.get_param::<String>("GOOSE_MODEL") {
        if current == payload.model_id {
            return already_applied(format!("active model is already \"{}\"", payload.model_id));
        }
    }
    config
        .set_param("GOOSE_MODEL", &payload.model_id)
        .map_err(|e| GuardError::Db(format!("failed to set active model: {e}")))?;

    Ok((
        Some(format!("active model switched to \"{}\"", payload.model_id)),
        None,
    ))
}

async fn apply_council_action(
    pool: &Pool<Sqlite>,
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let payload: decisions::CouncilActionPayload = serde_json::from_value(decision.payload.clone())
        .map_err(|e| {
            GuardError::Invalid(format!("stored council_action payload unreadable: {e}"))
        })?;
    if let Some(plan) = payload.plan.clone() {
        return apply_council_plan(pool, &payload.session_id, plan).await;
    }
    let project_id = decision
        .project_id
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let id = payload.project_id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            }
        })
        .ok_or_else(|| {
            GuardError::Invalid("council_action has no project to file a card on".to_string())
        })?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| GuardError::Db(format!("check project exists: {e}")))?;
    if !exists {
        return already_applied(format!(
            "project \"{}\" no longer exists; nothing was filed",
            if payload.project_name.is_empty() {
                project_id
            } else {
                payload.project_name.clone()
            }
        ));
    }
    let title = if payload.title.trim().is_empty() {
        decision.headline.clone()
    } else {
        payload.title.clone()
    };
    let card = crate::cards::create_card(
        pool,
        crate::cards::CreateCard {
            project_id,
            title,
            description: Some(payload.description.clone()),
            card_type: Some("standard".to_string()),
            column_id: None,
            created_by: Some("henry".to_string()),
            metadata_json: Some(serde_json::json!({
                "from": "council",
                "council_session_id": payload.session_id,
            })),
        },
    )
    .await
    .map_err(|e| GuardError::Db(format!("file council card: {e}")))?;
    Ok((Some(format!("filed board card \"{}\"", card.title)), None))
}

async fn apply_council_plan(
    pool: &Pool<Sqlite>,
    council_session_id: &str,
    proposal: crate::council::plan::CouncilProposal,
) -> Result<EffectResult, GuardError> {
    let capabilities = crate::council::plan::configured_worker_capabilities();
    let session = crate::council::plan::orchestrate(
        proposal,
        &capabilities,
        Some(crate::council::plan::ApprovalToken::explicit()),
    )
    .map_err(|error| GuardError::Invalid(format!("Council DAG refused: {error:?}")))?;
    let project_id = session.proposal.project.project_id.clone();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| GuardError::Db(format!("check Council project exists: {e}")))?;
    if !exists {
        return already_applied(format!(
            "project \"{}\" no longer exists; the Council DAG was not created",
            session.proposal.project.project_name
        ));
    }

    crate::cards::seed_goal_columns(pool, &project_id)
        .await
        .map_err(GuardError::Db)?;
    let triage = crate::cards::get_goal_column(pool, &project_id, "triage")
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| GuardError::Db("Triage column not found".to_string()))?;
    // The decision-effect outbox is retryable. Rehydrate any nodes written by
    // an interrupted attempt so a lease retry resumes the DAG instead of
    // duplicating cards.
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, json_extract(metadata_json, '$.council_node_id')
         FROM cards
         WHERE project_id = ? AND archived_at IS NULL
           AND json_extract(metadata_json, '$.council_plan_id') = ?
         ORDER BY created_at ASC",
    )
    .bind(&project_id)
    .bind(&session.proposal.proposal_id)
    .fetch_all(pool)
    .await
    .map_err(|error| GuardError::Db(format!("resume Council DAG: {error}")))?;
    let mut node_to_card = existing
        .into_iter()
        .map(|(card_id, node_id)| (node_id, card_id))
        .collect::<std::collections::BTreeMap<String, String>>();
    let mut roots = Vec::new();

    for node_id in &session.order {
        let node = session
            .proposal
            .nodes
            .iter()
            .find(|node| &node.id == node_id)
            .ok_or_else(|| {
                GuardError::Invalid(format!("validated node '{node_id}' disappeared"))
            })?;
        if let Some(card_id) = node_to_card.get(&node.id) {
            if node.dependencies.is_empty() {
                roots.push(card_id.clone());
            }
            continue;
        }
        let dependency_cards: Vec<String> = node
            .dependencies
            .iter()
            .map(|dependency| {
                node_to_card.get(dependency).cloned().ok_or_else(|| {
                    GuardError::Invalid(format!(
                        "dependency '{dependency}' was not created before '{}'",
                        node.id
                    ))
                })
            })
            .collect::<Result<_, _>>()?;
        let mut acceptance = node.acceptance_criteria.clone();
        if !node.verification.command.trim().is_empty() {
            acceptance.push(format!(
                "Verification command `{}` exits successfully",
                node.verification.command
            ));
        }
        let mut description = node.description.clone();
        if !node.files.is_empty() {
            description.push_str(&format!("\n\nFiles: {}", node.files.join(", ")));
        }
        if !node.symbols.is_empty() {
            description.push_str(&format!("\nSymbols: {}", node.symbols.join(", ")));
        }
        if !node.pattern_references.is_empty() {
            description.push_str(&format!(
                "\nFollow existing patterns: {}",
                node.pattern_references.join(", ")
            ));
        }
        let card = crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: project_id.clone(),
                title: node.title.clone(),
                description: Some(description),
                card_type: Some("goal".to_string()),
                column_id: Some(triage.id.clone()),
                created_by: Some("user".to_string()),
                metadata_json: Some(serde_json::json!({
                    "depends_on": dependency_cards,
                    "goal_state": "triage",
                    "attempt_count": 0,
                    "tags": node.required_capabilities,
                    "acceptance_criteria": acceptance,
                    "files": node.files,
                    "symbols": node.symbols,
                    "pattern_references": node.pattern_references,
                    "risk": node.risk,
                    "verification_required": node.verification.required,
                    "council_session_id": council_session_id,
                    "council_plan_id": session.proposal.proposal_id,
                    "council_node_id": node.id,
                    "project_memory_ids": session.proposal.project.memory_ids,
                })),
            },
        )
        .await
        .map_err(|e| GuardError::Db(format!("create Council goal '{}': {e}", node.title)))?;
        node_to_card.insert(node.id.clone(), card.id.clone());
        if node.dependencies.is_empty() {
            roots.push(card.id);
        }
    }

    for card_id in &roots {
        if goal_state(pool, card_id).await?.as_deref() != Some("triage") {
            continue;
        }
        goal_transition::advance_goal_checked(
            pool,
            card_id,
            GoalAction::Ready,
            decisions::ACTOR_SYSTEM,
            None,
            TransitionEffects::default(),
        )
        .await
        .map_err(|error| GuardError::Invalid(String::from(error)))?;
    }
    let dispatched = crate::agents::platform_extensions::orchestrator::nudge_dispatch_ready_goals(
        pool,
        &project_id,
        "approved Council DAG",
    )
    .await;
    Ok((
        Some(format!(
            "created {} Council DAG goals; {} root goal(s) released and {} dispatched",
            node_to_card.len(),
            roots.len(),
            dispatched
        )),
        None,
    ))
}

async fn apply_project_intel(
    pool: &Pool<Sqlite>,
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let payload: decisions::ProjectIntelProposalPayload =
        serde_json::from_value(decision.payload.clone()).map_err(|e| {
            GuardError::Invalid(format!(
                "stored project intelligence payload unreadable: {e}"
            ))
        })?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| GuardError::Db(e.to_string()))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&payload.project_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| GuardError::Db(format!("check project exists: {e}")))?;
    if !exists {
        tx.rollback()
            .await
            .map_err(|e| GuardError::Db(e.to_string()))?;
        return already_applied(format!(
            "project \"{}\" no longer exists; nothing was written",
            payload.project_name
        ));
    }
    for item in &payload.items {
        let candidates: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM project_intel WHERE project_id = ? AND kind = ?")
                .bind(&payload.project_id)
                .bind(&item.kind)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| GuardError::Db(format!("deduplicate project_intel: {e}")))?;
        let item_name_folded = item.name.to_lowercase();
        for (id, name) in candidates {
            // Full Unicode case-fold, not eq_ignore_ascii_case: the latter only
            // folds ASCII A-Z, so accented names ("CAFÉ" vs "café") would slip
            // past and duplicate instead of updating in place.
            if name.to_lowercase() == item_name_folded {
                sqlx::query("DELETE FROM project_intel WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| GuardError::Db(format!("deduplicate project_intel: {e}")))?;
            }
        }
        sqlx::query(
            "INSERT INTO project_intel
             (id, project_id, kind, name, note, source_url, created_at)
             VALUES (?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&payload.project_id)
        .bind(&item.kind)
        .bind(&item.name)
        .bind(&item.note)
        .bind(&item.source_url)
        .execute(&mut *tx)
        .await
        .map_err(|e| GuardError::Db(format!("insert project_intel: {e}")))?;
    }
    tx.commit()
        .await
        .map_err(|e| GuardError::Db(e.to_string()))?;

    // project_intel accumulates rows across every applied decision, but the
    // Brain feed key below is one evolving fact per project (feed_inner does
    // a wholesale content replace on upsert). Building the summary from only
    // this batch's payload.items would erase recall of every item fed by
    // earlier applications, so re-query the full current table instead.
    let all_items: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, kind FROM project_intel WHERE project_id = ? ORDER BY kind, name",
    )
    .bind(&payload.project_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let summary = all_items
        .iter()
        .map(|(name, kind)| format!("{name} ({kind})"))
        .collect::<Vec<_>>()
        .join(", ");
    crate::brain_feed::feed_for_project(
        pool,
        crate::brain_feed::FeedSource::Project,
        crate::brain_feed::feed_key("project", &payload.project_id, "intel"),
        &payload.project_id,
        format!(
            "Project '{}' intelligence: {}",
            payload.project_name, summary
        ),
    )
    .await;

    already_applied(format!(
        "project intelligence applied to \"{}\": {} item(s) added/updated with source citations",
        payload.project_name,
        payload.items.len()
    ))
}

enum PersonOutcome {
    Created,
    AssociatedExisting,
}

async fn file_person_addressless(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    name: &str,
) -> Result<PersonOutcome, String> {
    let candidates = crate::people::list_people(
        pool,
        &crate::people::PeopleFilter {
            company: None,
            role: None,
            query: Some(name.to_string()),
        },
    )
    .await?;
    // Full Unicode case-fold, not eq_ignore_ascii_case: the latter only folds
    // ASCII A-Z, so an accented name ("José" vs "JOSÉ") would fail to match an
    // existing person and create a duplicate instead of associating.
    let name_folded = name.to_lowercase();
    let exact: Vec<_> = candidates
        .iter()
        .filter(|person| person.display_name.to_lowercase() == name_folded)
        .collect();
    match exact.as_slice() {
        [person] => {
            crate::project_association::associate_person(
                pool,
                project_id,
                &person.entity_uuid,
                None,
            )
            .await?;
            Ok(PersonOutcome::AssociatedExisting)
        }
        [] => {
            // Conservative classification (memory-feed lane, 2026-09-25): a
            // name carrying a legal-entity suffix ("Acme Inc", "Doe & Partners
            // LLC") is an organisation, not a person — filing it here used to
            // mint an indistinguishable `Person` graph entity. Refuse rather
            // than guess; the caller can still file it as a project note.
            if crate::people_create::classify_person_or_org(name)
                == crate::people_create::PersonOrOrg::Organisation
            {
                return Err(format!(
                    "'{name}' looks like an organisation, not a person — not added as a person \
                     (never guess an identity)"
                ));
            }
            let brain = brain.ok_or_else(|| {
                "Brain is not available — minting a person writes a graph entity; add them later \
                 with create_person"
                    .to_string()
            })?;
            let person = crate::people_create::create_person(
                pool,
                brain,
                name,
                crate::people_provenance::Provenance::Runtime,
            )
            .await?;
            crate::project_association::associate_person(
                pool,
                project_id,
                &person.entity_uuid,
                None,
            )
            .await?;
            Ok(PersonOutcome::Created)
        }
        people => Err(format!(
            "matches {} people in the directory — not added (never guess an identity)",
            people.len()
        )),
    }
}

async fn apply_file_to_project(
    pool: &Pool<Sqlite>,
    decision: &Decision,
) -> Result<EffectResult, GuardError> {
    let payload: decisions::FileToProjectPayload = serde_json::from_value(decision.payload.clone())
        .map_err(|e| {
            GuardError::Invalid(format!("stored file_to_project payload unreadable: {e}"))
        })?;
    let project = crate::projects::get_project_by_id_or_slug(pool, &payload.project_id)
        .await
        .map_err(GuardError::Db)?
        .ok_or_else(|| {
            GuardError::NotFound(format!(
                "project '{}' ('{}') no longer exists — nothing was filed",
                payload.project_id, payload.project_name
            ))
        })?;
    let note_id = format!("decision-note:{}", decision.id);
    let brain = crate::agents::platform_extensions::get_global_brain();
    let note = if let Some(note) = crate::project_notes::get_note(pool, &project.id, &note_id)
        .await
        .map_err(GuardError::Db)?
    {
        note
    } else {
        crate::project_notes::create_note_indexed_with_id(
            pool,
            brain.as_ref(),
            &note_id,
            &project.id,
            payload.title.as_deref(),
            &payload.body,
        )
        .await
        .map_err(GuardError::Db)?
    };
    let (mut added, mut associated) = (0usize, 0usize);
    let mut warnings = Vec::new();
    for name in &payload.people {
        match file_person_addressless(pool, brain.as_ref(), &project.id, name).await {
            Ok(PersonOutcome::Created) => added += 1,
            Ok(PersonOutcome::AssociatedExisting) => associated += 1,
            Err(error) => warnings.push(format!("{name}: {error}")),
        }
    }
    let mut effect = format!(
        "filed note {} to project \"{}\"{}",
        note.id,
        project.name,
        if note.memory_key.is_some() {
            " (indexed into the Brain)"
        } else {
            " (Brain index skipped — note row persisted)"
        }
    );
    if added + associated > 0 {
        effect.push_str(&format!(
            "; people: {added} added, {associated} already in the directory — all address-less"
        ));
    }
    let warning = (!warnings.is_empty()).then(|| {
        format!(
            "note filed, but {} people step(s) did not apply: {}",
            warnings.len(),
            warnings.join("; ")
        )
    });
    Ok((Some(effect), warning))
}

/// How many dead effects are sitting in the outbox, and how old the oldest is.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DeadEffect {
    pub id: String,
    pub kind: String,
    pub headline: String,
    pub last_error: String,
    pub updated_at: String,
}

/// Every effect that has exhausted its retries: an approved, audited,
/// user-authorised action that never happened. Joined to the decision so the
/// surface can say WHICH decision, not just a count — Azure's dead-letter
/// guidance is explicit that a bare number is not a decision.
pub async fn dead_effects(pool: &Pool<Sqlite>) -> Result<Vec<DeadEffect>, String> {
    let rows = sqlx::query(
        "SELECT o.id AS id, o.kind AS kind,
                COALESCE(d.headline, '(the decision has been deleted)') AS headline,
                COALESCE(o.last_error, '') AS last_error, o.updated_at AS updated_at
         FROM effect_outbox o
         LEFT JOIN decisions d ON d.id = o.decision_id
         WHERE o.status = 'dead'
         ORDER BY o.updated_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| DeadEffect {
            id: r.get("id"),
            kind: r.get("kind"),
            headline: r.get("headline"),
            last_error: r.get("last_error"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

async fn dead_effect_count(pool: &Pool<Sqlite>) -> Result<i64, String> {
    sqlx::query_scalar("SELECT COUNT(*) FROM effect_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

/// Retire outbox rows whose decision no longer exists.
///
/// `effect_outbox.decision_id` has no foreign key while
/// `decisions.project_id REFERENCES projects(id) ON DELETE CASCADE`, so
/// deleting a project cascades the decision away and leaves the outbox row
/// pointing at nothing (two such rows were found live on 2026-08-31). A
/// non-terminal dangling row is not retryable — it would burn all five
/// attempts on `decision '…' no longer exists` and land as `dead` with an
/// error that reads like a bug. Retire it once, with an honest reason.
async fn retire_dangling_effects(pool: &Pool<Sqlite>) -> Result<u64, String> {
    let result = sqlx::query(
        "UPDATE effect_outbox
         SET status = 'dead', attempts = max_attempts,
             last_error = 'the decision was deleted (its project was removed) — \
                           this effect can never be applied',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status IN ('pending', 'running')
           AND (decision_id IS NULL
                OR NOT EXISTS (SELECT 1 FROM decisions d WHERE d.id = decision_id))",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected())
}

/// Claim and apply a bounded batch of due effects.
pub async fn drain_effect_outbox(pool: &Pool<Sqlite>) -> Result<(), String> {
    // Depth BEFORE this pass. A dead effect used to produce a `tracing::warn`
    // and nothing else — no inbox card, no HUD, no doctor check — so an
    // approved action could simply never happen and no surface said so. The
    // rule is the CloudWatch dead-letter-queue one: alert on the transition
    // INTO a non-empty queue, not per entry and not per tick.
    let dead_before = dead_effect_count(pool).await?;
    retire_dangling_effects(pool).await?;

    sqlx::query(
        "UPDATE effect_outbox
         SET status = CASE WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'pending' END,
             attempts = attempts + 1,
             last_error = 'effect claim lease expired before completion',
             next_attempt_at = strftime(
                 '%Y-%m-%dT%H:%M:%fZ', 'now',
                 '+' || MIN(3600, (1 << MIN(attempts, 12))) || ' seconds'
             ),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status = 'running'
           AND updated_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now', ?)",
    )
    .bind(format!("-{RUNNING_LEASE_SECONDS} seconds"))
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let rows = sqlx::query(
        "SELECT id, decision_id, kind FROM effect_outbox
         WHERE status = 'pending' AND next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')
         ORDER BY next_attempt_at, id LIMIT ?",
    )
    .bind(OUTBOX_BATCH_SIZE)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for row in rows {
        let id: String = row.get("id");
        let decision_id: Option<String> = row.get("decision_id");
        let kind: String = row.get("kind");
        let claimed = sqlx::query(
            "UPDATE effect_outbox SET status = 'running',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ? AND status = 'pending'",
        )
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        // Cloned so `decision_id` (the outer binding) survives past the async
        // block below — the failure arm needs it too, to write an audit row.
        let decision_id_for_apply = decision_id.clone();
        let result = async {
            let decision_id = decision_id_for_apply
                .ok_or_else(|| "effect outbox row has no decision_id".to_string())?;
            let decision = decisions::get_decision(pool, &decision_id)
                .await?
                .ok_or_else(|| format!("decision '{decision_id}' no longer exists"))?;
            let proof = DecisionProof::from_answered_row(&decision)
                .ok_or_else(|| format!("decision '{decision_id}' is not an answered row"))?;
            apply_decision_effect(pool, &decision, proof, &kind)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(_) => {
                sqlx::query(
                    "UPDATE effect_outbox SET status = 'applied', attempts = attempts + 1,
                         last_error = NULL,
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id = ? AND status = 'running'",
                )
                .bind(&id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE effect_outbox
                     SET status = CASE
                             WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'pending' END,
                         attempts = attempts + 1, last_error = ?,
                         next_attempt_at = strftime(
                             '%Y-%m-%dT%H:%M:%fZ', 'now',
                             '+' || MIN(3600, (1 << MIN(attempts, 12))) || ' seconds'
                         ),
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id = ? AND status = 'running'",
                )
                .bind(&error)
                .bind(&id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;

                // Audit trail (CASE A fix #4): before this, only the inline
                // HTTP-answer path (`record_effect_failure`,
                // routes/decisions.rs:915) ever wrote a decision_audit row for
                // an effect failure. A failure surfaced later by THIS
                // background drain — every retry, and the terminal dead-letter
                // — produced nothing: no audit row, and the decisions row kept
                // status=answered/answer=approve, indistinguishable from a
                // decision whose effect actually ran. Mirror the inline path
                // here so every failed attempt is on the record, and the final
                // one is nameable as a dead-letter (decision_history's
                // `outcome` column — a decision that dead-lettered carries an
                // `effect_dead: …` audit row a successful one never gets).
                if let Some(ref did) = decision_id {
                    if let Ok(Some(decision)) = decisions::get_decision(pool, did).await {
                        let status_now: String =
                            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE id = ?")
                                .bind(&id)
                                .fetch_one(pool)
                                .await
                                .unwrap_or_else(|_| "pending".to_string());
                        let outcome = if status_now == "dead" {
                            format!("effect_dead: {error}")
                        } else {
                            format!("effect_retry: {error}")
                        };
                        if let Err(audit_err) =
                            decisions::record_effect_outcome(pool, &decision, &outcome).await
                        {
                            tracing::error!(
                                target: "permagentd::decisions",
                                decision = %did,
                                "effect-outbox failure could not be audit-recorded: {audit_err} \
                                 (original effect error: {error})"
                            );
                        }
                    }
                }
            }
        }
    }

    if dead_before == 0 {
        let dead_now = dead_effects(pool).await?;
        if !dead_now.is_empty() {
            brief_dead_effects(pool, &dead_now).await;
        }
    }
    Ok(())
}

/// One briefing on the 0→nonzero transition, never one per entry. The standing
/// state is re-asserted by the daily job digest (`job_health`), which is what
/// stops a badge from being the only surface for something that has been
/// broken for a week.
async fn brief_dead_effects(pool: &Pool<Sqlite>, dead: &[DeadEffect]) {
    let mut detail = String::from(
        "These were approved and audited, and then failed every retry. Nothing will \
         re-attempt them.\n\n",
    );
    for d in dead.iter().take(10) {
        detail.push_str(&format!(
            "- {} — {}: {}\n",
            d.kind, d.headline, d.last_error
        ));
    }
    if dead.len() > 10 {
        detail.push_str(&format!("- …and {} more\n", dead.len() - 10));
    }
    crate::briefings::file_briefing(
        pool,
        crate::briefings::NewBriefing {
            from_agent: "decisions".to_string(),
            kind: "effect_dead_letter".to_string(),
            severity: crate::briefings::Severity::ActionRequired,
            summary: format!(
                "{} approved decision{} did not take effect",
                dead.len(),
                if dead.len() == 1 { "" } else { "s" }
            ),
            detail: Some(detail),
            ref_kind: Some("decision".to_string()),
            ref_id: None,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::{DecisionAnswer, NewDecision, ACTOR_JESSE};
    use crate::projects::PERSONAL_PROJECT_ID;
    use crate::session::spectral_schema::init_spectral_db;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn goal_in_review(pool: &Pool<Sqlite>) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let col = crate::cards::get_goal_column(pool, PERSONAL_PROJECT_ID, "review")
            .await
            .unwrap()
            .unwrap();
        let mut meta = serde_json::Map::new();
        meta.insert("attempt_count".to_string(), serde_json::json!(0));
        meta.insert("goal_state".to_string(), serde_json::json!("review"));
        meta.insert(
            "dispatch_evidence".to_string(),
            serde_json::json!({"verdict": {"status": "pass"}}),
        );
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Debug arm test goal".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(col.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn land_empty_commit_evidence_reports_note_and_unblock() {
        let pool = test_pool().await;
        let card = goal_in_review(&pool).await;
        crate::cards::set_goal_dispatch_evidence(
            &pool,
            &card.id,
            serde_json::json!({
                "worktree_path": "/missing/goal-worktree",
                "baseline_commit": "seed",
                "head_commit": "seed",
                "commits": [],
                "verdict": {"status": "pass"},
            }),
        )
        .await
        .unwrap();
        // Exercise the approval effect as well: a parked goal must not be
        // described as Complete or reach dependent promotion.
        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".into(),
                goal_id: Some(card.id.clone()),
                project_id: Some(card.project_id.clone()),
                // `decisions` CHECKs length(headline) > 0 AND length(detail) > 0,
                // and `create_decision` stores a validation failure as a
                // `malformed` row rather than erroring — without these two the
                // row comes back `malformed` and `answer_decision` refuses it
                // ("outbox kind 'approve_review' does not match decision kind
                // 'malformed'"), which is what this fixture hit.
                headline: Some("Review the finished work".into()),
                detail: Some("evidence".into()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".into(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &decision.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();
        let (effect, note) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();
        assert!(effect.unwrap().contains("parked"));
        let expected = format!(
            "note: not landed — goal {} has no commits past the baseline \
             (uncommitted work in /missing/goal-worktree)",
            card.id
        );
        assert_eq!(note.as_deref(), Some(expected.as_str()));
        // R43a-c: the machine reason behind an identically-shaped note lives
        // on the structured landing outcome, not the note text itself —
        // `/missing/goal-worktree` doesn't exist, so no trusted trunk root
        // resolves.
        let refreshed = crate::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        let landing = refreshed
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("the refusal must still be persisted onto dispatch_evidence.landing");
        assert_eq!(
            landing.get("refusal_reason").and_then(|v| v.as_str()),
            Some("no_trunk"),
            "landing: {landing:?}"
        );
        assert_eq!(goal_state_of(&pool, &card.id).await, "failed");
        let unblock = decisions::find_open_decision_for_goal(&pool, &card.id, "unblock")
            .await
            .unwrap()
            .unwrap();
        assert!(unblock.detail.contains(&expected));
        // Repeating the landing refusal reuses the same open unblock.
        let (_, repeated_note) = land_approved_goal(&pool, &card.id, None, false).await;
        assert_eq!(repeated_note, note);
        let repeated = decisions::find_open_decision_for_goal(&pool, &card.id, "unblock")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repeated.id, unblock.id);
    }

    #[tokio::test]
    async fn land_absent_dispatch_evidence_reports_note_and_unblock() {
        let pool = test_pool().await;
        let card = goal_in_review(&pool).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_remove(metadata_json, '$.dispatch_evidence') WHERE id = ?",
        )
        .bind(&card.id)
        .execute(&pool)
        .await
        .unwrap();

        let (landing, note) = land_approved_goal(&pool, &card.id, None, false).await;
        assert!(matches!(
            landing,
            Some((_, crate::goal_landing::LandOutcome::Skipped(_)))
        ));
        let note = note.unwrap();
        assert!(note.contains("note: not landed"));
        assert!(note.contains("unknown worktree"));
        assert_eq!(goal_state_of(&pool, &card.id).await, "failed");
        let unblock = decisions::find_open_decision_for_goal(&pool, &card.id, "unblock")
            .await
            .unwrap()
            .unwrap();
        assert!(unblock.detail.contains(&note));
    }

    /// A41 audit (row #23): outside the trunk-relative override (rolling
    /// `commits` is non-empty here, so `land_approved_goal`'s own empty-
    /// commits gate never refuses), an inconsistent `dispatch_evidence`
    /// shape — commits present, but no usable `head_commit` — used to make
    /// `evidence_head_and_worktree` return `None` and this function
    /// silently return `(None, None)`: no refusal note, no park, no
    /// persisted `dispatch_evidence.landing`, nothing for a human or the
    /// next A-series analysis to see. Pins the hardened, honest refusal.
    #[tokio::test]
    async fn land_approved_goal_reports_an_honest_refusal_when_evidence_has_no_usable_head_commit()
    {
        let pool = test_pool().await;
        let card = goal_in_review(&pool).await;
        crate::cards::set_goal_dispatch_evidence(
            &pool,
            &card.id,
            serde_json::json!({
                "worktree_path": "/tmp/.permagent-goal-worktrees/no-head-commit",
                "baseline_commit": "seed",
                "head_commit": "",
                "commits": ["abc123 the work"],
                "verdict": {"status": "pass"},
            }),
        )
        .await
        .unwrap();

        let (landing, note) = land_approved_goal(&pool, &card.id, None, false).await;
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::Skipped(_)))
            ),
            "the refusal must be reported, never a silent (None, None): {landing:?}"
        );
        let note = note.expect("a genuine refusal must report a note");
        assert!(
            note.contains("land_refused reason=no_evidence_head"),
            "note: {note}"
        );
        assert_eq!(goal_state_of(&pool, &card.id).await, "failed");

        let refreshed = crate::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        let landing = refreshed
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("the refusal must still be persisted onto dispatch_evidence.landing");
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("skipped")
        );
        let unblock = decisions::find_open_decision_for_goal(&pool, &card.id, "unblock")
            .await
            .unwrap()
            .expect("a refused land must leave an answerable unblock decision");
        assert!(unblock.detail.contains(&note));
    }

    async fn goal_state_of(pool: &Pool<Sqlite>, card_id: &str) -> String {
        let card = crate::cards::get_card(pool, card_id)
            .await
            .unwrap()
            .unwrap();
        let col = crate::cards::get_column(pool, &card.column_id)
            .await
            .unwrap()
            .unwrap();
        col.state_binding.unwrap_or_default()
    }

    #[tokio::test]
    async fn approved_no_write_program_does_not_require_git_landing() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        let mut metadata = goal.metadata_json.as_object().cloned().unwrap();
        metadata.insert(
            "program".to_string(),
            serde_json::json!({
                "program_id": "research-program",
                "node_id": "audit",
                "manifest_sha256": "approved",
                "manifest": "schema: 1\nprogram_id: research-program\nobjective: audit\nterminal_node: audit\nnodes:\n  - id: audit\n    child_dag: audit.md\n    status: active\n    entry_gate: [input captured]\n    exit_gate: [audit receipt]\n    worker_policy: cheap\n    delivery: no_write",
                "delivery": "no_write"
            }),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        ensure_registered_goal_landed(&pool, &goal.id, false)
            .await
            .expect("explicit no-write contract must not require a repository landing");
    }

    #[tokio::test]
    async fn approve_review_advances_registered_no_write_program_through_normal_effect() {
        use crate::agents::platform_extensions::{execution_receipt, goal_engine};
        use permagent_eval::{
            ApprovalPolicy, DeliveryMode, ProgramDag, ProgramNode, ProgramNodeStatus,
        };

        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        let program = ProgramDag {
            schema: 1,
            program_id: "normal-approval-research".to_string(),
            objective: "complete an approved audit without a repository write".to_string(),
            terminal_node: "audit".to_string(),
            nodes: vec![ProgramNode {
                id: "audit".to_string(),
                child_dag: "audit.md".to_string(),
                status: ProgramNodeStatus::Active,
                depends_on: Vec::new(),
                next_on_pass: Vec::new(),
                entry_gate: vec!["input captured".to_string()],
                exit_gate: vec!["audit receipt".to_string()],
                worker_policy: "cheap".to_string(),
                approval: ApprovalPolicy::None,
                delivery: DeliveryMode::NoWrite,
                blocked_reason: None,
            }],
        };
        let manifest = serde_yaml::to_string(&program).unwrap();
        let manifest_hash =
            crate::agents::platform_extensions::program_bridge::manifest_identity_hash(&program)
                .unwrap();
        let plan_id = "approved-normal-research";

        let mut receipt = execution_receipt::ExecutionReceipt::new(
            "worker",
            "session",
            serde_json::json!({}),
            "lifecycle",
            "2026-09-05T00:00:00Z",
            1,
        );
        receipt.finalize(
            execution_receipt::ReceiptState::Completed,
            "2026-09-05T00:01:00Z",
        );
        let mut evidence = serde_json::to_value(goal_engine::GoalEvidence::default()).unwrap();
        evidence["verdict"] = serde_json::json!({
            "status": "pass",
            "finished_at": "fixture-verification"
        });
        let mut metadata = goal.metadata_json.as_object().cloned().unwrap();
        metadata.insert("council_plan_id".to_string(), serde_json::json!(plan_id));
        metadata.insert("council_node_id".to_string(), serde_json::json!("audit"));
        metadata.insert("depends_on".to_string(), serde_json::json!([]));
        metadata.insert(
            "execution_receipt".to_string(),
            serde_json::to_value(receipt).unwrap(),
        );
        metadata.insert("dispatch_evidence".to_string(), evidence);
        metadata.insert(
            "program_receipts".to_string(),
            serde_json::json!([{
                "gate": "audit receipt",
                "passed": true,
                "verification_id": "fixture-verification"
            }]),
        );
        sqlx::query("UPDATE cards SET metadata_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&metadata).unwrap())
            .bind(&goal.id)
            .execute(&pool)
            .await
            .unwrap();

        let council = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "council_action".to_string(),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve research program".to_string()),
                detail: Some("fixture approval".to_string()),
                // A council_action payload that fails its schema is not an
                // error: create_decision stores it as a tier-2 `malformed`
                // decision so the user can resolve it. This fixture omitted
                // `title`, so it was filed as malformed and registration then
                // correctly found no approval for the plan.
                payload: serde_json::json!({
                    "session_id": "decisions-effects-fixture",
                    "title": "Approve research program",
                    "plan": {
                        "proposal_id": plan_id,
                        "program_manifest_sha256": manifest_hash,
                        "program_manifest": manifest.clone(),
                        "project": {
                            "project_id": PERSONAL_PROJECT_ID,
                            "project_name": "Personal",
                            "summary": "Decision effects fixture"
                        },
                        "budget_limit": 1,
                        "nodes": [{
                            "id": "audit",
                            "title": "Run the audit",
                            "description": "Produce the audit findings",
                            "acceptance_criteria": ["audit receipt"],
                            "estimated_budget": 1,
                            "risk": "low",
                            "verification": {"command": "true", "required": true}
                        }]
                    }
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &council.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        crate::agents::platform_extensions::program_bridge::register_program(
            &pool,
            crate::agents::platform_extensions::program_bridge::ProgramRegistrationRequest {
                manifest: manifest.clone(),
                node_to_goal: std::collections::BTreeMap::from([(
                    "audit".to_string(),
                    goal.id.clone(),
                )]),
            },
        )
        .await
        .unwrap();

        let review = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve the completed audit".to_string()),
                detail: Some("typed evidence is complete".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &review.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &review.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();
        apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(goal_state_of(&pool, &goal.id).await, "complete");
        let after = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(after.metadata_json.get("program_transition").is_some());
    }

    async fn answered_review_approval(
        pool: &Pool<Sqlite>,
        goal_id: &str,
    ) -> (Decision, DecisionProof) {
        let decision = decisions::create_decision(
            pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Approve reviewed work".to_string()),
                detail: Some("test".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn review_approval_waits_for_durable_verifier_pass() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_remove(metadata_json, '$.dispatch_evidence.verdict') WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();
        let (decision, proof) = answered_review_approval(&pool, &goal.id).await;

        let error = apply_decision_effect(&pool, &decision, proof, "approve_review")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("verification is still pending"));
        assert_eq!(goal_state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn non_passing_verdict_can_never_land_an_earlier_approval() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.dispatch_evidence.verdict.status', 'fail') WHERE id = ?",
        )
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();
        let (decision, proof) = answered_review_approval(&pool, &goal.id).await;

        let (effect, _) = apply_decision_effect(&pool, &decision, proof, "approve_review")
            .await
            .unwrap();
        assert!(effect.unwrap().contains("verifier verdict is fail"));
        assert_eq!(goal_state_of(&pool, &goal.id).await, "review");
    }

    /// Y16c (G19 MED): a rejected review under the attempt cap advances to
    /// InProgress for rework (unchanged), AND stamps
    /// `goal_branch_rejected_session_id` with the exact worker session whose
    /// branch was rejected — the marker `prior_goal_branch_tip`
    /// (orchestrator.rs) refuses to continue from, even if the card later
    /// cycles back through Ready with `worker_session_id` still intact.
    #[tokio::test]
    async fn rejected_review_stamps_the_rejected_session_for_prior_goal_branch_tip() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        sqlx::query(
            "UPDATE cards SET metadata_json = json_set(metadata_json, '$.worker_session_id', ?) WHERE id = ?",
        )
        .bind("worker-session-rejected")
        .bind(&goal.id)
        .execute(&pool)
        .await
        .unwrap();

        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Reject the reviewed work".to_string()),
                detail: Some("test".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (answered, proof) = decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "reject".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();
        assert_eq!(
            effect.as_deref(),
            Some("goal rejected: Review → InProgress for rework")
        );
        assert_eq!(goal_state_of(&pool, &goal.id).await, "in_progress");

        let after = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after
                .metadata_json
                .get("goal_branch_rejected_session_id")
                .and_then(|v| v.as_str()),
            Some("worker-session-rejected"),
            "the reject arm must stamp which session's branch was rejected so \
             `prior_goal_branch_tip` can refuse to continue from it"
        );
    }

    // I1 (harness loop, 2026-09-10, gate fix): GOAL_DISPATCH_HOOK is a
    // process-global OnceLock every test_dispatch_recorder::install() call
    // races to set — serialized against every other test that touches it
    // (see test_dispatch_recorder's own doc comment) so a losing race never
    // silently starves this test's own dispatch recorder.
    #[serial_test::serial]
    #[tokio::test]
    async fn council_plan_retry_reuses_existing_node_cards() {
        crate::agents::platform_extensions::orchestrator::test_dispatch_recorder::install();
        let pool = test_pool().await;
        let proposal = crate::council::plan::CouncilProposal {
            proposal_id: "council-retry-test".to_string(),
            project: crate::council::plan::ProjectContext {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                project_name: "Personal".to_string(),
                summary: "Frozen test context".to_string(),
                memory_ids: Vec::new(),
                playbook_briefing: None,
            },
            nodes: vec![crate::council::plan::DagNode {
                id: "edit".to_string(),
                title: "Make one surgical edit".to_string(),
                description: "Change the named symbol only.".to_string(),
                files: vec!["src/example.rs".to_string()],
                symbols: vec!["example".to_string()],
                pattern_references: vec!["src/existing.rs::existing".to_string()],
                acceptance_criteria: vec!["The requested behavior is present".to_string()],
                dependencies: Vec::new(),
                required_capabilities: Default::default(),
                estimated_budget: 1,
                risk: crate::council::plan::RiskLevel::Low,
                verification: crate::council::plan::VerificationSpec {
                    command: "cargo test -p permagent".to_string(),
                    required: true,
                },
            }],
            budget_limit: 1,
            program_manifest_sha256: None,
            program_manifest: None,
        };

        apply_council_plan(&pool, "session-retry", proposal.clone())
            .await
            .unwrap();
        apply_council_plan(&pool, "session-retry", proposal)
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cards
             WHERE json_extract(metadata_json, '$.council_plan_id') = 'council-retry-test'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "outbox retries must not duplicate DAG nodes");
    }

    async fn answered_debug_choice(
        pool: &Pool<Sqlite>,
        goal_id: &str,
        choice_id: &str,
    ) -> Decision {
        let d = decisions::create_decision(
            pool,
            NewDecision {
                kind: "choice".to_string(),
                goal_id: Some(goal_id.to_string()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Verification failed — dispatch the debugger?".to_string()),
                detail: Some("verifier fail".to_string()),
                payload: serde_json::json!({
                    "question": "Dispatch the debugger?",
                    "proposal": decisions::PROPOSAL_DEBUG_DISPATCH,
                    "options": [
                        {"id": "dispatch-debugger", "label": "Re-dispatch with the debugger mandate"},
                        {"id": "leave-for-review", "label": "Leave it in Review"}
                    ],
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            pool,
            &d.id,
            &DecisionAnswer {
                answer: "choice".to_string(),
                note: None,
                choice_id: Some(choice_id.to_string()),
                input_text: None,
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        decisions::get_decision(pool, &d.id).await.unwrap().unwrap()
    }

    /// D10, completion/land path: approving a goal's review promotes its
    /// dependents Triage→Ready — and, since the nudge, actually starts them.
    /// Before the nudge the dependent sat in Ready with no worker until a
    /// human called `resume_roadmap` by hand.
    #[serial_test::serial]
    #[tokio::test]
    async fn approving_a_review_dispatches_the_promoted_dependent() {
        use crate::agents::platform_extensions::orchestrator::test_dispatch_recorder;
        test_dispatch_recorder::install();

        let pool = test_pool().await;
        let parent = goal_in_review(&pool).await;
        // Y15 refuses to land a goal whose dispatch evidence carries no
        // commits past the baseline. This pin is about dependent promotion,
        // not about that refusal, so give the parent the one thing Y15's
        // precondition asks for. Deliberately no `worktree_path` /
        // `head_commit`: `evidence_head_and_worktree` then still returns
        // None, so `land_approved_goal` has no worktree of the parent's OWN
        // to land from — R43a-b: that shape is reported (never silent) but
        // never PARKED (parking would undo the Review→Complete transition
        // `advance_goal_checked_with_auto_land` already made), so this test
        // exercises the same promotion path it always did.
        crate::cards::set_goal_dispatch_evidence(
            &pool,
            &parent.id,
            serde_json::json!({
                "verdict": {"status": "pass"},
                "commits": ["deadbeef"],
            }),
        )
        .await
        .unwrap();
        let triage = crate::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "triage")
            .await
            .unwrap()
            .unwrap();
        let dependent = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Dependent of the approved goal".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(triage.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "depends_on": [parent.id],
                    "attempt_count": 0,
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(parent.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(goal_state_of(&pool, &parent.id).await, "complete");
        assert_eq!(
            goal_state_of(&pool, &dependent.id).await,
            "ready",
            "the dependent is promoted out of Triage"
        );
        assert!(
            test_dispatch_recorder::saw(&dependent.id),
            "approving a goal must ask the dispatcher to start the dependent it \
             just unblocked — not leave it waiting for a manual resume_roadmap"
        );
    }

    fn git_in(root: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git must be on PATH for this test");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A trunk on `main` and a `goal/run-1` branch one commit ahead of it.
    /// Returns (repo root, the branch's head commit).
    fn repo_with_finished_goal_branch() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_in(root, &["init", "-b", "main"]);
        git_in(root, &["config", "user.email", "test@example.com"]);
        git_in(root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("README.md"), "base\n").unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "base"]);
        git_in(root, &["checkout", "-b", "goal/run-1"]);
        std::fs::write(root.join("work.txt"), "the goal's work\n").unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "the work"]);
        let head = git_in(root, &["rev-parse", "HEAD"]);
        git_in(root, &["checkout", "main"]);
        (dir, head)
    }

    /// L7: approving a review is supposed to fast-forward the work onto trunk.
    /// It did not. `land_approved_goal` lived in `routes/decisions.rs`'s INLINE
    /// approve arm, and `execute_effect` short-circuits every outbox-eligible
    /// kind — `approve_review` among them — into `apply_decision_effect`
    /// before reaching it. The only call site of `goal_landing::land` was
    /// unreachable, so approved work sat on `goal/<run_id>` forever and the
    /// D10 nudge then started dependents against a trunk missing their parent.
    #[tokio::test]
    async fn approving_a_review_through_the_outbox_lands_the_branch_on_trunk() {
        let (repo, head) = repo_with_finished_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Landing test".to_string(),
                root_path: Some(root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Finished goal".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": "/tmp/.permagent-goal-worktrees/run-1",
                        "head_commit": head,
                        "commits": ["abc123 the work"],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(
            git_in(repo.path(), &["rev-parse", "HEAD"]),
            head,
            "approval did not fast-forward trunk onto the approved work"
        );
        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("main"),
            "the landing outcome must be reported on the approval, not left silent: {effect}"
        );

        // YF2 (G6 review, Lane Y-fix 2026-09-09): the outcome must also be
        // durably persisted on the card — a one-shot response string is not
        // enough for `goal_landed_head_commit`/the CLI drain summary to
        // read back later.
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("land_approved_goal must persist the outcome onto dispatch_evidence.landing");
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("landed")
        );
        assert_eq!(
            landing.get("landed_head").and_then(|v| v.as_str()),
            Some(head.as_str())
        );
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10):
        // pin the new explicit trunk-reached field. Pre-fix: the
        // `trunk_state` key does not exist (this assertion fails on `None`
        // != `Some("landed_on_trunk")`).
        assert_eq!(
            landing.get("trunk_state").and_then(|v| v.as_str()),
            Some("landed_on_trunk")
        );
    }

    /// R1 (harness loop, Lane D1, from Lane F8 forensics): the eval harness's
    /// default `personal` project (and any project created with no local
    /// folder) is seeded with `root_path` NULL
    /// (`session/spectral_schema.rs:610`). Before this lane,
    /// `land_approved_goal` bailed to `(None, None)` right there and the
    /// approval proceeded to Complete having landed nothing — F8 proved two
    /// of five measured fixtures were genuinely-correct, oracle-passing
    /// fixes lost exactly this way. Pre-fix, this test fails: `HEAD` stays
    /// on the base commit and `dispatch_evidence/landing` never exists.
    ///
    /// H1 (G8 review, Lane E3 2026-09-10): this is also G8's own pinning
    /// test 2 — "same setup [as the refusal below] with the session's
    /// project root = repo A → lands on A". The card now records a
    /// `worker_session_id` whose session's `working_dir` genuinely IS the
    /// worktree's origin repo, and `dispatch_evidence.baseline_commit` is
    /// the real pre-branch commit — exactly what a real dispatch leaves
    /// behind — so `verify_worktree_landing_target`'s two checks both pass
    /// and this fallback still lands correctly, not just before this lane.
    #[tokio::test]
    async fn a_project_with_no_root_path_lands_via_the_goal_worktree_git_baseline() {
        let (repo, head) = repo_with_finished_goal_branch();
        let baseline = git_in(repo.path(), &["rev-parse", "main"]);

        // A REAL linked worktree at `goal/run-1`, mirroring exactly what
        // `goal_engine::create_goal_worktree` does at dispatch time, so
        // `resolve_primary_checkout_from_worktree`'s `git rev-parse
        // --git-common-dir` has a genuine worktree to resolve.
        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("run-1");
        git_in(
            repo.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "goal/run-1",
            ],
        );

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "No root_path project".to_string(),
                root_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            crate::projects::get_project(&pool, &project.id)
                .await
                .unwrap()
                .unwrap()
                .root_path
                .is_none(),
            "precondition: this project must have no root_path, matching the eval seed"
        );

        // The independently recorded working directory `verify_worktree_
        // landing_target` requires — a REAL session whose own `working_dir`
        // is this goal's actual repo, exactly what a real dispatch leaves
        // behind (never re-derived from the daemon's own `current_dir()`).
        let session_id = "d1-worktree-baseline-session".to_string();
        sqlx::query("INSERT INTO sessions (id, working_dir) VALUES (?, ?)")
            .bind(&session_id)
            .bind(repo.path().to_string_lossy().into_owned())
            .execute(&pool)
            .await
            .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Finished goal, no project root".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "review",
                    "worker_session_id": session_id,
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "head_commit": head,
                        "baseline_commit": baseline,
                        "commits": ["abc123 the work"],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(
            git_in(repo.path(), &["rev-parse", "HEAD"]),
            head,
            "a NULL root_path must not silently no-op landing when the goal's own \
             worktree can resolve its primary checkout via git"
        );
        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("main"),
            "the landing outcome must be reported, not left silent: {effect}"
        );

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect(
                "a NULL root_path must not skip persisting the landing outcome onto \
                 dispatch_evidence",
            );
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("landed"),
            "outcome: {landing:?}"
        );
        assert_eq!(
            landing.get("landed_head").and_then(|v| v.as_str()),
            Some(head.as_str())
        );
    }

    /// R43a (A38 Miss 2): a goal reviewed and approved on the strength of an
    /// EARLIER attempt's commit must still land when THIS attempt's own
    /// rolling `dispatch_evidence.commits` reads empty (Y16b resets the
    /// rolling baseline to the prior attempt's own tip on every
    /// re-dispatch). Reproduces the confirmed shape verbatim:
    /// `dispatch_evidence.commits: []`, `baseline_commit == head_commit` ==
    /// the real fix an EARLIER attempt already committed to the goal
    /// branch. Before this lane the first gate in `land_approved_goal`
    /// refused outright and parked the goal (`goal_state: "failed"`) even
    /// though the fix was one fast-forward away.
    #[tokio::test]
    async fn land_approved_goal_lands_trunk_unlanded_work_despite_empty_rolling_evidence() {
        let (repo, head) = repo_with_finished_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();

        // A REAL linked worktree checked out on the goal branch — the
        // trunk-relative override needs somewhere to run its own `git`
        // calls against, exactly like a real dispatched worker's worktree.
        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("run-1");
        git_in(
            repo.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "goal/run-1",
            ],
        );

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Trunk-relative landing test".to_string(),
                root_path: Some(root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Goal reviewed on an earlier attempt's commit".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 3,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "baseline_commit": head,
                        "head_commit": head,
                        "commits": [],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, note) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(
            note, None,
            "a successful land must not also report a refusal note"
        );
        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("main"),
            "the fast-forward must be reported on the approval: {effect}"
        );
        assert_eq!(
            git_in(repo.path(), &["rev-parse", "HEAD"]),
            head,
            "the trunk must advance onto the earlier attempt's already-committed fix"
        );
        assert_eq!(goal_state_of(&pool, &goal.id).await, "complete");

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("landing outcome must be persisted");
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("landed")
        );
        assert!(
            decisions::find_open_decision_for_goal(&pool, &goal.id, "unblock")
                .await
                .unwrap()
                .is_none(),
            "a goal that actually lands must never also be left parked with an unblock decision"
        );
    }

    /// R62d (A61 Miss 5): the goal `maybe_hold_review` allowed as already
    /// satisfied at its baseline has nothing to land by construction. The
    /// approval must complete it with the receipt's honest note and no
    /// landing outcome, never park it for "no commits past the baseline" —
    /// but only while the receipt names the card's current worker session;
    /// a stale receipt still refuses and parks exactly as
    /// `land_approved_goal_still_refuses_when_nothing_is_unlanded_anywhere`.
    #[tokio::test]
    async fn approving_an_already_satisfied_goal_completes_it_without_a_landing() {
        struct Approved {
            pool: Pool<Sqlite>,
            goal_id: String,
            effect: Option<String>,
            note: Option<String>,
            _repo: tempfile::TempDir,
            _worktrees: tempfile::TempDir,
        }
        async fn approve_with_receipt(receipt_session: &str) -> Approved {
            let (repo, _head) = repo_with_finished_goal_branch();
            let root = repo.path().to_string_lossy().into_owned();
            let base_sha = git_in(repo.path(), &["rev-parse", "main"]);
            let worktrees_root = tempfile::tempdir().unwrap();
            let worktree_path = worktrees_root.path().join("already-satisfied");
            git_in(
                repo.path(),
                &[
                    "worktree",
                    "add",
                    "--detach",
                    worktree_path.to_str().unwrap(),
                    &base_sha,
                ],
            );
            let pool = test_pool().await;
            let project = crate::projects::create_project(
                &pool,
                crate::projects::CreateProject {
                    name: "Already satisfied test".to_string(),
                    root_path: Some(root.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            crate::cards::seed_goal_columns(&pool, &project.id)
                .await
                .unwrap();
            let review = crate::cards::get_goal_column(&pool, &project.id, "review")
                .await
                .unwrap()
                .unwrap();
            let goal = crate::cards::create_card(
                &pool,
                crate::cards::CreateCard {
                    project_id: project.id.clone(),
                    title: "Goal a sibling already satisfied".to_string(),
                    description: Some("test".to_string()),
                    card_type: Some("goal".to_string()),
                    column_id: Some(review.id),
                    created_by: None,
                    metadata_json: Some(serde_json::json!({
                        "attempt_count": 1,
                        "goal_state": "review",
                        "worker_session_id": "sess-current",
                        "dispatch_evidence": {
                            "worktree_path": worktree_path.to_string_lossy(),
                            "baseline_commit": base_sha,
                            "head_commit": base_sha,
                            "commits": [],
                            "verdict": {"status": "pass"},
                        },
                        "already_satisfied": {
                            "note": "already satisfied at baseline: test receipt — nothing was landed for this goal",
                            "trunk_head": base_sha,
                            "head": base_sha,
                            "worker_session_id": receipt_session,
                            "scope": [],
                        },
                    })),
                },
            )
            .await
            .unwrap();
            let d = decisions::create_decision(
                &pool,
                NewDecision {
                    kind: "approve_review".to_string(),
                    goal_id: Some(goal.id.clone()),
                    project_id: Some(project.id.clone()),
                    headline: Some("Review the finished work".to_string()),
                    detail: Some("evidence".to_string()),
                    payload: serde_json::json!({}),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            decisions::answer_decision(
                &pool,
                &d.id,
                &DecisionAnswer {
                    answer: "approve".to_string(),
                    ..Default::default()
                },
                ACTOR_JESSE,
            )
            .await
            .unwrap();
            let answered = decisions::get_decision(&pool, &d.id)
                .await
                .unwrap()
                .unwrap();
            let proof = DecisionProof::from_answered_row(&answered).unwrap();
            let (effect, note) = apply_decision_effect(&pool, &answered, proof, "approve_review")
                .await
                .unwrap();
            assert_eq!(
                git_in(repo.path(), &["rev-parse", "main"]),
                base_sha,
                "the trunk never moves: there is nothing to land"
            );
            Approved {
                pool,
                goal_id: goal.id,
                effect,
                note,
                _repo: repo,
                _worktrees: worktrees_root,
            }
        }

        let Approved {
            pool,
            goal_id,
            effect,
            note,
            _repo,
            _worktrees,
        } = approve_with_receipt("sess-current").await;
        let effect = effect.unwrap_or_default();
        assert!(effect.contains("Review → Complete"), "effect: {effect}");
        assert!(!effect.contains("parked"), "effect: {effect}");
        let note = note.expect("the already-satisfied note is reported");
        assert!(
            note.contains("already satisfied at baseline"),
            "note: {note}"
        );
        assert_eq!(goal_state_of(&pool, &goal_id).await, "complete");
        let card = crate::cards::get_card(&pool, &goal_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            card.metadata_json
                .pointer("/dispatch_evidence/landing")
                .is_none(),
            "no landing receipt of any kind is fabricated"
        );
        assert!(
            decisions::find_open_decision_for_goal(&pool, &goal_id, "unblock")
                .await
                .unwrap()
                .is_none()
        );

        let Approved {
            pool,
            goal_id,
            effect,
            note,
            _repo,
            _worktrees,
        } = approve_with_receipt("sess-an-earlier-attempt").await;
        assert!(effect.unwrap_or_default().contains("parked"));
        assert!(note
            .unwrap_or_default()
            .contains("no commits past the baseline"));
        assert_eq!(goal_state_of(&pool, &goal_id).await, "failed");
    }

    /// R43a (A38 Miss 2): the override above must still refuse when there is
    /// genuinely nothing to credit — the rolling gate is empty AND the
    /// trunk-relative check ALSO finds nothing unlanded (a real worktree
    /// pinned at the exact same commit as the trunk). Proves the override
    /// only ever widens the Allow when real trunk-unlanded work exists; it
    /// must never force a land merely because it ran.
    #[tokio::test]
    async fn land_approved_goal_still_refuses_when_nothing_is_unlanded_anywhere() {
        let (repo, _head) = repo_with_finished_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();
        let base_sha = git_in(repo.path(), &["rev-parse", "main"]);

        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("nothing-committed");
        git_in(
            repo.path(),
            &[
                "worktree",
                "add",
                "--detach",
                worktree_path.to_str().unwrap(),
                &base_sha,
            ],
        );

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Nothing unlanded test".to_string(),
                root_path: Some(root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Goal with genuinely nothing committed".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 3,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "baseline_commit": base_sha,
                        "head_commit": base_sha,
                        "commits": [],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, note) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert!(effect.unwrap_or_default().contains("parked"));
        let note = note.expect("a genuine refusal must report a note");
        assert!(
            note.contains("no commits past the baseline"),
            "note: {note}"
        );
        assert_eq!(
            git_in(repo.path(), &["rev-parse", "main"]),
            base_sha,
            "the trunk must never move on a genuine refusal"
        );
        assert_eq!(goal_state_of(&pool, &goal.id).await, "failed");

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card.metadata_json.pointer("/dispatch_evidence/landing");
        assert_ne!(
            landing
                .and_then(|l| l.get("outcome"))
                .and_then(|v| v.as_str()),
            Some("landed"),
            "a refused land must never be recorded as landed"
        );
        let unblock = decisions::find_open_decision_for_goal(&pool, &goal.id, "unblock")
            .await
            .unwrap()
            .expect("a refused land must leave an answerable unblock decision");
        assert!(unblock.detail.contains(&note));
    }

    /// R43a-b (G31 review, M38 lanes, HIGH-1): the trunk-relative override
    /// must not credit "committed work exists" from `commits_unlanded_at_
    /// trunk > 0` alone — an unlanded commit with no verify bound to its
    /// exact head is refused (mirrors `maybe_hold_review`'s own R42a-b
    /// discipline); the identical shape with a recorded PASS bound to that
    /// head lands normally. The refused half uses `salvage_receipt.outcome
    /// == "salvage_verify_failed"` rather than an actually-undetectable
    /// build stack — deterministic and fast (no subprocess), and the fresh-
    /// verify-subprocess path itself already has real-process coverage from
    /// R42a-b's own `maybe_hold_review` tests via the same shared
    /// `run_fresh_verify_for_hold` helper this override now also calls.
    #[tokio::test]
    async fn land_approved_goal_requires_a_verified_head_for_the_trunk_relative_override() {
        let (repo, head) = repo_with_finished_goal_branch();
        let root = repo.path().to_string_lossy().into_owned();

        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("run-1");
        git_in(
            repo.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "goal/run-1",
            ],
        );

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Verify-gated trunk-relative override".to_string(),
                root_path: Some(root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();

        // Unlanded at trunk, but no verify is bound to that head — salvage
        // already tried and failed this exact, still-unmoved range.
        let goal_unverified = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Unlanded but never verified".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id.clone()),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 3,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "baseline_commit": head,
                        "head_commit": head,
                        "commits": [],
                    },
                    "salvage_receipt": {"outcome": "salvage_verify_failed"},
                })),
            },
        )
        .await
        .unwrap();

        let (landing, note) = land_approved_goal(&pool, &goal_unverified.id, None, false).await;
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::Skipped(_)))
            ),
            "an unlanded commit with no verify bound to its head must be refused, not landed: \
             {landing:?}"
        );
        let note = note.expect("a genuine refusal must report a note");
        assert_eq!(
            note,
            format!(
                "note: not landed — goal {} has no commits past the baseline \
                 (uncommitted work in {})",
                goal_unverified.id,
                worktree_path.to_string_lossy()
            ),
            "the card note stays the human sentence — the machine reason lives on the \
             structured landing outcome, not prepended here (R43a-c)"
        );
        let refreshed = crate::cards::get_card(&pool, &goal_unverified.id)
            .await
            .unwrap()
            .unwrap();
        let landing = refreshed
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("the refusal must still be persisted onto dispatch_evidence.landing");
        assert_eq!(
            landing.get("refusal_reason").and_then(|v| v.as_str()),
            Some("unverified_head"),
            "landing: {landing:?}"
        );
        assert_eq!(goal_state_of(&pool, &goal_unverified.id).await, "failed");
        assert_ne!(
            git_in(repo.path(), &["rev-parse", "main"]),
            head,
            "the trunk must never move on an unverified-head refusal"
        );

        // Identical unlanded commit, but this goal's own recorded verdict is
        // a PASS bound to that exact head.
        let goal_verified = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Unlanded and verified at head".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 3,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "baseline_commit": head,
                        "head_commit": head,
                        "commits": [],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let (landing, note) = land_approved_goal(&pool, &goal_verified.id, None, false).await;
        assert_eq!(
            note, None,
            "a successful land must not also report a refusal note"
        );
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::FastForwarded { .. }))
            ),
            "a verified unlanded commit must land: {landing:?}"
        );
        assert_eq!(
            git_in(repo.path(), &["rev-parse", "main"]),
            head,
            "the trunk must advance once the unlanded commit is verified at its head"
        );
        // Note: `land_approved_goal` is called directly here (as the other
        // direct-call tests in this module do) rather than through
        // `apply_decision_effect` — the Review→Complete transition itself
        // is `advance_goal_checked_with_auto_land`'s job, called BEFORE
        // landing in the real approval flow, so this goal's card column is
        // left exactly where it started (Review); only the trunk-side
        // outcome asserted above is this function's own contract.
    }

    // ── Lane R63b: headless landing rebases a parallel sibling's branch ──────

    /// M47 r63 s2 shape: two sibling goal branches forked from the same
    /// trunk commit, each committed in its own linked worktree
    /// (`<tmp>/run-a`, `<tmp>/run-b` -> `goal/run-a`, `goal/run-b`).
    struct R63bSiblings {
        repo: tempfile::TempDir,
        _worktrees: tempfile::TempDir,
        wt_a: std::path::PathBuf,
        wt_b: std::path::PathBuf,
        base: String,
        head_a: String,
        head_b: String,
    }

    fn r63b_siblings(a_files: &[&str], b_files: &[&str]) -> R63bSiblings {
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path();
        git_in(root, &["init", "-b", "main"]);
        git_in(root, &["config", "user.email", "test@example.com"]);
        git_in(root, &["config", "user.name", "Test"]);
        git_in(root, &["config", "commit.gpgsign", "false"]);
        // No excludes from the machine's global config: a global `*.pyc`
        // ignore would change what `add -A` tracks and what a checkout may
        // overwrite, and the rebase-abort test depends on both.
        git_in(root, &["config", "core.excludesFile", "/dev/null"]);
        std::fs::write(root.join("README.md"), "base\n").unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "base"]);
        let base = git_in(root, &["rev-parse", "HEAD"]);

        let worktrees = tempfile::tempdir().unwrap();
        let commit_sibling = |name: &str, files: &[&str]| {
            let wt = worktrees.path().join(format!("run-{name}"));
            let branch = format!("goal/run-{name}");
            git_in(
                root,
                &["worktree", "add", "-b", &branch, wt.to_str().unwrap()],
            );
            for file in files {
                let path = wt.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, format!("goal {name}\n")).unwrap();
            }
            git_in(&wt, &["add", "-A"]);
            git_in(&wt, &["commit", "-m", &format!("goal {name}")]);
            let head = git_in(&wt, &["rev-parse", "HEAD"]);
            (wt, head)
        };
        let (wt_a, head_a) = commit_sibling("a", a_files);
        let (wt_b, head_b) = commit_sibling("b", b_files);
        R63bSiblings {
            repo,
            _worktrees: worktrees,
            wt_a,
            wt_b,
            base,
            head_a,
            head_b,
        }
    }

    async fn r63b_project(pool: &Pool<Sqlite>, root: &std::path::Path) -> String {
        let project = crate::projects::create_project(
            pool,
            crate::projects::CreateProject {
                name: format!("R63b {}", uuid::Uuid::new_v4()),
                root_path: Some(root.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        crate::cards::seed_goal_columns(pool, &project.id)
            .await
            .unwrap();
        project.id
    }

    /// A reviewed goal with a PASS verdict, a declared `write_scope`, and a
    /// worker session whose recorded genuine verify command is
    /// `verify_command` — what `run_fresh_verify_for_hold` re-runs.
    async fn r63b_goal(
        pool: &Pool<Sqlite>,
        project_id: &str,
        worktree: &std::path::Path,
        base: &str,
        head: &str,
        scope: &[&str],
        verify_command: &str,
    ) -> crate::cards::Card {
        use crate::conversation::message::MessageContent;
        use rmcp::model::{CallToolRequestParams, CallToolResult, Content};

        let review = crate::cards::get_goal_column(pool, project_id, "review")
            .await
            .unwrap()
            .unwrap();
        let session_id = format!("r63b-worker-{}", uuid::Uuid::new_v4());
        sqlx::query("INSERT INTO sessions (id, working_dir) VALUES (?, '/tmp')")
            .bind(&session_id)
            .execute(pool)
            .await
            .unwrap();
        let request = MessageContent::tool_request(
            "r63b-call",
            Ok(
                CallToolRequestParams::new("developer__verify".to_string()).with_arguments(
                    serde_json::Map::from_iter([(
                        "command".to_string(),
                        serde_json::json!(verify_command),
                    )]),
                ),
            ),
        );
        let mut result = CallToolResult::success(vec![Content::text("PASS")]);
        result.structured_content = Some(serde_json::json!({
            "kind": crate::agents::platform_extensions::developer::verify::VERIFICATION_OBSERVATION_KIND,
            "command": verify_command,
            "verdict": "pass",
            "evidence": "pass",
            "vacuous_reason": null,
        }));
        let response = MessageContent::tool_response("r63b-call", Ok(result));
        for (role, content) in [("assistant", request), ("user", response)] {
            sqlx::query(
                "INSERT INTO messages (message_id, session_id, role, content_json, \
                 created_timestamp) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&session_id)
            .bind(role)
            .bind(serde_json::to_string(&vec![content]).unwrap())
            .bind(chrono::Utc::now().timestamp())
            .execute(pool)
            .await
            .unwrap();
        }

        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: project_id.to_string(),
                title: format!("R63b sibling {}", worktree.display()),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 1,
                    "goal_state": "review",
                    "worker_session_id": session_id,
                    "write_scope": scope,
                    "dispatch_evidence": {
                        "worktree_path": worktree.to_string_lossy(),
                        "baseline_commit": base,
                        "head_commit": head,
                        "commits": [format!("{} the work", head.chars().take(7).collect::<String>())],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap()
    }

    fn r63b_landing(card: &crate::cards::Card) -> serde_json::Value {
        card.metadata_json
            .pointer("/dispatch_evidence/landing")
            .cloned()
            .expect("the landing outcome must be persisted")
    }

    /// Lane R63b (M47 r63 s2): two siblings with disjoint write scopes ran in
    /// parallel; the first fast-forwards, and the second, which used to be
    /// refused "needs merge", is rebased onto the moved trunk, re-verified on
    /// the rebased head (the recorded check only passes once goal a's file is
    /// present), and landed through the same `land` path.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn headless_landing_rebases_a_disjoint_sibling_and_lands_it_after_reverify() {
        let fx = r63b_siblings(&["a.txt"], &["b.txt"]);
        let pool = test_pool().await;
        let project_id = r63b_project(&pool, fx.repo.path()).await;
        let goal_a = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_a,
            &fx.base,
            &fx.head_a,
            &["a.txt"],
            "test -f a.txt",
        )
        .await;
        let goal_b = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_b,
            &fx.base,
            &fx.head_b,
            &["b.txt"],
            "test -f a.txt",
        )
        .await;

        let (landing_a, _) = land_approved_goal(&pool, &goal_a.id, None, true).await;
        assert!(
            matches!(
                landing_a,
                Some((_, crate::goal_landing::LandOutcome::FastForwarded { .. }))
            ),
            "{landing_a:?}"
        );
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), fx.head_a);

        let (landing_b, note_b) = land_approved_goal(&pool, &goal_b.id, None, true).await;
        let Some((_, crate::goal_landing::LandOutcome::FastForwarded { head, .. })) =
            landing_b.clone()
        else {
            panic!("the disjoint sibling must rebase and land: {landing_b:?} {note_b:?}");
        };
        assert_eq!(note_b, None);
        assert_ne!(head, fx.head_b, "a rebased head is a new commit");
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), head);
        assert_eq!(
            git_in(fx.repo.path(), &["rev-parse", "main^"]),
            fx.head_a,
            "linear history: goal b sits directly on goal a, no merge commit"
        );
        assert_eq!(git_in(&fx.wt_b, &["rev-parse", "HEAD"]), head);

        let card = crate::cards::get_card(&pool, &goal_b.id)
            .await
            .unwrap()
            .unwrap();
        let landing = r63b_landing(&card);
        assert_eq!(landing["outcome"], "landed", "{landing:?}");
        assert_eq!(landing["landed_head"], serde_json::json!(head));
        assert!(landing["refusal_reason"].is_null(), "{landing:?}");
        let evidence = &card.metadata_json["dispatch_evidence"];
        assert_eq!(evidence["head_commit"], serde_json::json!(head));
        assert_eq!(evidence["baseline_commit"], serde_json::json!(fx.head_a));
        assert_eq!(
            card.metadata_json["baseline_commit"],
            serde_json::json!(fx.head_a)
        );
        assert_eq!(evidence["commits"].as_array().map(Vec::len), Some(1));
        assert_eq!(evidence["rebased_onto_trunk"]["reverify"], "pass");
        assert_eq!(
            evidence["rebased_onto_trunk"]["from_head"],
            serde_json::json!(fx.head_b)
        );
    }

    /// The trunk changed a path inside the goal's declared scope (a directory
    /// claims its descendants). The rebase would even apply cleanly, but
    /// independence is unproven, so nothing is rebased: the branch is
    /// untouched and the existing "needs merge" refusal and unblock stand.
    #[tokio::test]
    async fn headless_landing_does_not_rebase_when_the_trunk_touched_the_goal_scope() {
        let fx = r63b_siblings(&["docs/a.md"], &["docs/b.md"]);
        let pool = test_pool().await;
        let project_id = r63b_project(&pool, fx.repo.path()).await;
        git_in(fx.repo.path(), &["merge", "--ff-only", "goal/run-a"]);
        let goal_b = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_b,
            &fx.base,
            &fx.head_b,
            &["docs"],
            "true",
        )
        .await;

        let (landing, _) = land_approved_goal(&pool, &goal_b.id, None, true).await;
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::NeedsMerge { .. }))
            ),
            "{landing:?}"
        );
        assert_eq!(git_in(&fx.wt_b, &["rev-parse", "HEAD"]), fx.head_b);
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), fx.head_a);
        let card = crate::cards::get_card(&pool, &goal_b.id)
            .await
            .unwrap()
            .unwrap();
        let landing = r63b_landing(&card);
        assert_eq!(landing["outcome"], "needs_merge");
        assert_eq!(landing["refusal_reason"], "rebase_trunk_overlap");
        assert!(card
            .metadata_json
            .pointer("/dispatch_evidence/rebased_onto_trunk")
            .is_none());
        assert_eq!(
            card.metadata_json["dispatch_evidence"]["head_commit"],
            serde_json::json!(fx.head_b)
        );
        let unblock = decisions::find_open_decision_for_goal(&pool, &goal_b.id, "unblock")
            .await
            .unwrap()
            .expect("needs merge still leaves its manual-merge unblock");
        assert!(unblock.detail.contains("goal/run-b"), "{}", unblock.detail);
    }

    /// Scope-disjoint, but `git rebase` cannot apply: the trunk now tracks a
    /// file the goal worktree holds untracked (a regenerated artefact the
    /// clean check tolerates). The rebase is aborted, the branch is left
    /// exactly as it was, and the goal keeps the honest "needs merge".
    #[tokio::test]
    async fn headless_landing_aborts_a_rebase_that_cannot_apply_and_leaves_the_branch_intact() {
        let fx = r63b_siblings(&["a.txt", "stray.pyc"], &["b.txt"]);
        std::fs::write(fx.wt_b.join("stray.pyc"), "local bytecode\n").unwrap();
        let pool = test_pool().await;
        let project_id = r63b_project(&pool, fx.repo.path()).await;
        git_in(fx.repo.path(), &["merge", "--ff-only", "goal/run-a"]);
        let goal_b = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_b,
            &fx.base,
            &fx.head_b,
            &["b.txt"],
            "true",
        )
        .await;

        let (landing, _) = land_approved_goal(&pool, &goal_b.id, None, true).await;
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::NeedsMerge { .. }))
            ),
            "{landing:?}"
        );
        assert_eq!(git_in(&fx.wt_b, &["rev-parse", "HEAD"]), fx.head_b);
        assert_eq!(
            git_in(&fx.wt_b, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "goal/run-b",
            "no rebase may be left in progress"
        );
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), fx.head_a);
        let card = crate::cards::get_card(&pool, &goal_b.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r63b_landing(&card)["refusal_reason"], "rebase_conflict");
        assert!(
            decisions::find_open_decision_for_goal(&pool, &goal_b.id, "unblock")
                .await
                .unwrap()
                .is_some()
        );
    }

    /// The rebase applies, but the worker's recorded check no longer passes
    /// on the rebased head. Nothing lands; the goal is parked with the reason
    /// and a `fail` verdict (so the old pass is never credited to the new
    /// head), and the rebased branch is left intact.
    #[tokio::test]
    #[serial_test::serial(receipts_global_registry)]
    async fn headless_landing_parks_when_the_rebased_head_fails_reverify() {
        let fx = r63b_siblings(&["a.txt"], &["b.txt"]);
        let pool = test_pool().await;
        let project_id = r63b_project(&pool, fx.repo.path()).await;
        git_in(fx.repo.path(), &["merge", "--ff-only", "goal/run-a"]);
        let goal_b = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_b,
            &fx.base,
            &fx.head_b,
            &["b.txt"],
            "test -f never-created.txt",
        )
        .await;

        let (landing, note) = land_approved_goal(&pool, &goal_b.id, None, true).await;
        assert!(
            matches!(
                landing,
                Some((_, crate::goal_landing::LandOutcome::Skipped(_)))
            ),
            "{landing:?}"
        );
        let note = note.expect("a refused landing reports its note");
        assert!(note.contains("did not genuinely pass"), "{note}");
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), fx.head_a);
        let rebased = git_in(&fx.wt_b, &["rev-parse", "goal/run-b"]);
        assert_ne!(rebased, fx.head_b);
        assert_eq!(git_in(&fx.wt_b, &["rev-parse", "goal/run-b^"]), fx.head_a);
        assert_eq!(goal_state_of(&pool, &goal_b.id).await, "failed");

        let card = crate::cards::get_card(&pool, &goal_b.id)
            .await
            .unwrap()
            .unwrap();
        let landing = r63b_landing(&card);
        assert_eq!(landing["outcome"], "skipped");
        assert_eq!(landing["refusal_reason"], "rebase_reverify_failed");
        let evidence = &card.metadata_json["dispatch_evidence"];
        assert_eq!(evidence["verdict"]["status"], "fail");
        assert_eq!(evidence["head_commit"], serde_json::json!(rebased));
        assert_eq!(evidence["rebased_onto_trunk"]["reverify"], "fail");

        // Fix-as-found (2026-09-25): the early return on this exact path used
        // to skip the landing-outcome Brain feed entirely, so a headless
        // rebase's failed re-verify — a real, actionable outcome — never
        // reached memory. It must be queued in the outbox like any other
        // landing outcome.
        let fed: Option<String> =
            sqlx::query_scalar("SELECT content FROM brain_feed_outbox WHERE memory_key = ?")
                .bind(crate::brain_feed::feed_key("goal", &goal_b.id, "landing"))
                .fetch_optional(&pool)
                .await
                .unwrap();
        let fed = fed.expect("the reverify-failed landing outcome must reach the Brain feed");
        assert!(fed.contains("Skipped"), "{fed}");

        assert!(
            decisions::find_open_decision_for_goal(&pool, &goal_b.id, "unblock")
                .await
                .unwrap()
                .is_some()
        );
    }

    /// Interactive approvals are unchanged: the same disjoint sibling shape
    /// that headless landing rebases stays "needs merge" for the human, with
    /// no rebase, no metadata rewrite and no machine refusal reason.
    #[tokio::test]
    async fn interactive_landing_still_reports_needs_merge_without_rebasing() {
        let fx = r63b_siblings(&["a.txt"], &["b.txt"]);
        let pool = test_pool().await;
        let project_id = r63b_project(&pool, fx.repo.path()).await;
        git_in(fx.repo.path(), &["merge", "--ff-only", "goal/run-a"]);
        let goal_b = r63b_goal(
            &pool,
            &project_id,
            &fx.wt_b,
            &fx.base,
            &fx.head_b,
            &["b.txt"],
            "test -f a.txt",
        )
        .await;

        let (landing, _) = land_approved_goal(&pool, &goal_b.id, None, false).await;
        let Some((_, outcome)) = landing else {
            panic!("an interactive landing still reports its outcome");
        };
        assert!(
            outcome.describe().contains("cannot fast-forward"),
            "{}",
            outcome.describe()
        );
        assert_eq!(git_in(&fx.wt_b, &["rev-parse", "HEAD"]), fx.head_b);
        assert_eq!(git_in(fx.repo.path(), &["rev-parse", "main"]), fx.head_a);
        let card = crate::cards::get_card(&pool, &goal_b.id)
            .await
            .unwrap()
            .unwrap();
        assert!(r63b_landing(&card)["refusal_reason"].is_null());
        assert!(card
            .metadata_json
            .pointer("/dispatch_evidence/rebased_onto_trunk")
            .is_none());
    }

    /// H1 (G8 review, harness loop Lane E3 2026-09-10), G8's own pinning
    /// test 1: "NULL root_path + goal worktree in repo A + process cwd in
    /// repo B → landing refused with the reason, repo B HEAD unchanged".
    /// This is the exact hazard G8 flagged: before this lane,
    /// `land_approved_goal` trusted `resolve_primary_checkout_from_worktree`
    /// on its own say-so — "git says this worktree came from here" — with
    /// no independent confirmation that the recovered repo was ever the
    /// goal's intended project. Here the worktree really is repo A's own
    /// linked worktree (so the OLD code would have fast-forwarded A
    /// correctly), but the goal has no `worker_session_id` at all — no
    /// independently recorded working directory exists to confirm A is
    /// legitimate — so the fallback must refuse outright. Repo B stands in
    /// for the daemon's own cwd / `permagent-runtime` checkout: present in
    /// this test only to prove landing never touches it either, by
    /// construction (this function never reads `current_dir()` at all) —
    /// its HEAD is asserted unchanged as direct evidence of that. Pre-fix,
    /// this test's assertion on `HEAD` staying on the base commit fails —
    /// the old code lands A unconditionally.
    #[tokio::test]
    async fn a_project_with_no_root_path_and_no_known_session_refuses_the_worktree_fallback() {
        let (repo_a, head_a) = repo_with_finished_goal_branch();
        let baseline_a = git_in(repo_a.path(), &["rev-parse", "main"]);
        let (repo_b, _head_b) = repo_with_finished_goal_branch();
        let repo_b_head_before = git_in(repo_b.path(), &["rev-parse", "HEAD"]);

        let worktrees_root = tempfile::tempdir().unwrap();
        let worktree_path = worktrees_root.path().join("run-1");
        git_in(
            repo_a.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "goal/run-1",
            ],
        );

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "No root_path, no known session".to_string(),
                root_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Finished goal, unverifiable landing target".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "review",
                    // No `worker_session_id` — no independently recorded
                    // working directory exists for this goal.
                    "dispatch_evidence": {
                        "worktree_path": worktree_path.to_string_lossy(),
                        "head_commit": head_a,
                        "baseline_commit": baseline_a,
                        "commits": ["abc123 the work"],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_eq!(
            git_in(repo_a.path(), &["rev-parse", "HEAD"]),
            baseline_a,
            "repo A must NOT be fast-forwarded without an independently recorded working \
             directory confirming it as this goal's own repo"
        );
        assert_eq!(
            git_in(repo_b.path(), &["rev-parse", "HEAD"]),
            repo_b_head_before,
            "repo B (standing in for the daemon's own cwd) must never be touched by landing"
        );
        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("landing target is not the goal's repo"),
            "the refusal must say why, not just fail: {effect}"
        );

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("a refused fallback landing must still be recorded, never silent");
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("skipped")
        );
        assert_eq!(
            landing.get("trunk_state").and_then(|v| v.as_str()),
            Some("left_in_working_tree")
        );
    }

    /// R1's second half: when landing genuinely cannot proceed (no
    /// `root_path` AND the worktree itself is gone/unresolvable — the reaper
    /// already swept it, or the path was never real), `land_approved_goal`
    /// must record an honest failure outcome, never the old silent
    /// `(None, None)`. Pre-fix, this test's `dispatch_evidence/landing`
    /// assertion fails outright (the key never exists) and the approval
    /// response carries no landing information at all.
    #[tokio::test]
    async fn a_project_with_no_root_path_records_an_honest_failure_when_it_genuinely_cannot_land() {
        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "No root_path, no resolvable worktree".to_string(),
                root_path: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Finished goal, unresolvable landing target".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        // Never a real git worktree — the reaper already
                        // swept it, or (as in every eval-seeded fixture)
                        // there never was a project root to branch it from.
                        "worktree_path": "/no/such/permagent-d1-worktree-run-1",
                        "head_commit": "deadbeefcafe",
                        "commits": ["abc123 the work"],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("NOT landed"),
            "an unresolvable landing target must be reported as a real failure, \
             not silently dropped: {effect}"
        );

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect(
                "a genuinely-unlandable goal must still get an honest \
                 dispatch_evidence.landing entry, never silence",
            );
        assert_ne!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("landed"),
            "must never read as landed when nothing was landed: {landing:?}"
        );
        assert!(
            landing
                .get("describe")
                .and_then(|v| v.as_str())
                .is_some_and(|d| d.contains("root_path")),
            "the recorded reason must name what actually went wrong: {landing:?}"
        );
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): a
        // Skipped outcome (nothing ever resolved a landing target) must
        // read as left_in_working_tree — never landed_on_trunk.
        assert_eq!(
            landing.get("trunk_state").and_then(|v| v.as_str()),
            Some("left_in_working_tree")
        );
    }

    /// A trunk on `main` that has moved (a second commit) since
    /// `goal/run-1` branched off it — `goal_landing::land` must report
    /// `NeedsMerge`, not fast-forward.
    fn repo_with_diverged_trunk() -> (tempfile::TempDir, String) {
        let (dir, head) = repo_with_finished_goal_branch();
        let root = dir.path();
        // `repo_with_finished_goal_branch` leaves the checkout on `main`.
        std::fs::write(root.join("trunk-moved.txt"), "someone else landed first\n").unwrap();
        git_in(root, &["add", "-A"]);
        git_in(root, &["commit", "-m", "trunk moved"]);
        (dir, head)
    }

    /// YF2 (G6 review, Lane Y-fix 2026-09-09, finding #2): approving a goal
    /// whose branch the trunk has since diverged from must persist the real
    /// `NeedsMerge` outcome onto the card — not just report it in the
    /// one-shot approval response and then discard it, which is exactly
    /// what let a later `Complete` card print a fabricated `landed` line
    /// for work that never reached the trunk.
    #[tokio::test]
    async fn needs_merge_outcome_is_persisted_on_the_card_and_never_reported_as_landed() {
        let (repo, head) = repo_with_diverged_trunk();
        let root = repo.path().to_string_lossy().into_owned();

        let pool = test_pool().await;
        let project = crate::projects::create_project(
            &pool,
            crate::projects::CreateProject {
                name: "Needs-merge landing test".to_string(),
                root_path: Some(root.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        crate::cards::seed_goal_columns(&pool, &project.id)
            .await
            .unwrap();
        let review = crate::cards::get_goal_column(&pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();
        let goal = crate::cards::create_card(
            &pool,
            crate::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Diverged goal".to_string(),
                description: Some("test".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({
                    "attempt_count": 0,
                    "goal_state": "review",
                    "dispatch_evidence": {
                        "worktree_path": "/tmp/.permagent-goal-worktrees/run-1",
                        "head_commit": head,
                        "commits": ["abc123 the work"],
                        "verdict": {"status": "pass"},
                    },
                })),
            },
        )
        .await
        .unwrap();

        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(project.id.clone()),
                headline: Some("Review the diverged work".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        decisions::answer_decision(
            &pool,
            &d.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let answered = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        let proof = DecisionProof::from_answered_row(&answered).unwrap();

        let (effect, _warning) = apply_decision_effect(&pool, &answered, proof, "approve_review")
            .await
            .unwrap();

        assert_ne!(
            git_in(repo.path(), &["rev-parse", "HEAD"]),
            head,
            "a diverged trunk must never be fast-forwarded"
        );
        let effect = effect.unwrap_or_default();
        assert!(
            effect.contains("NOT landed"),
            "a diverged trunk must be reported honestly, not as landed: {effect}"
        );

        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        let landing = card
            .metadata_json
            .pointer("/dispatch_evidence/landing")
            .expect("land_approved_goal must persist the outcome onto dispatch_evidence.landing");
        assert_eq!(
            landing.get("outcome").and_then(|v| v.as_str()),
            Some("needs_merge"),
            "pre-fix this field never existed at all"
        );
        assert!(
            landing
                .get("landed_head")
                .map(|v| v.is_null())
                .unwrap_or(true),
            "a NeedsMerge outcome must never carry a landed_head"
        );
        // E1 (harness loop, Round E, from Lane F9 forensics 2026-09-10): a
        // NeedsMerge outcome is real, committed work that is NOT on trunk —
        // must read as left_in_working_tree, never landed_on_trunk. Pre-fix:
        // the `trunk_state` key does not exist at all.
        assert_eq!(
            landing.get("trunk_state").and_then(|v| v.as_str()),
            Some("left_in_working_tree"),
            "pre-fix this field never existed at all"
        );
    }

    // ── manual-merge recovery (moved with `ensure_needs_merge_decision` from
    // routes/decisions.rs, whose copy of it was unreachable) ───────────────

    async fn needs_merge_test_goal(pool: &Pool<Sqlite>) -> crate::cards::Card {
        crate::cards::seed_goal_columns(pool, PERSONAL_PROJECT_ID)
            .await
            .unwrap();
        let review_col = crate::cards::get_goal_column(pool, PERSONAL_PROJECT_ID, "review")
            .await
            .unwrap()
            .unwrap();
        crate::cards::create_card(
            pool,
            crate::cards::CreateCard {
                project_id: PERSONAL_PROJECT_ID.to_string(),
                title: "Ship durable landing recovery".to_string(),
                description: None,
                card_type: Some("goal".to_string()),
                column_id: Some(review_col.id),
                created_by: None,
                metadata_json: Some(serde_json::json!({})),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_second_needs_merge_landing_does_not_create_another_open_decision() {
        let pool = test_pool().await;
        let goal = needs_merge_test_goal(&pool).await;
        for _ in 0..2 {
            ensure_needs_merge_decision(
                &pool,
                &goal.id,
                &goal.title,
                PERSONAL_PROJECT_ID,
                "goal/run-123",
                "main",
            )
            .await
            .unwrap();
        }

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM decisions \
             WHERE goal_id = ? AND kind = 'unblock' AND status = 'open'",
        )
        .bind(&goal.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn needs_merge_decision_names_refs_and_copy_pasteable_git_commands() {
        let pool = test_pool().await;
        let goal = needs_merge_test_goal(&pool).await;
        ensure_needs_merge_decision(
            &pool,
            &goal.id,
            &goal.title,
            PERSONAL_PROJECT_ID,
            "goal/run-123",
            "main",
        )
        .await
        .unwrap();

        let decision = decisions::find_open_decision_for_goal(&pool, &goal.id, "unblock")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decision.kind, "unblock");
        assert_eq!(decision.goal_id.as_deref(), Some(goal.id.as_str()));
        assert_eq!(decision.project_id.as_deref(), Some(PERSONAL_PROJECT_ID));
        assert!(decision.headline.contains("could not land"));
        for expected in [
            goal.id.as_str(),
            goal.title.as_str(),
            "goal/run-123",
            "main",
            "git switch main\ngit merge goal/run-123",
            "git switch goal/run-123\ngit rebase main\ngit switch main\ngit merge --ff-only goal/run-123",
        ] {
            assert!(
                decision.detail.contains(expected),
                "decision detail must contain {expected:?}: {}",
                decision.detail
            );
        }
    }

    #[tokio::test]
    async fn debug_dispatch_choice_sets_role_and_rejects_the_open_review() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        let review = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("Review the finished work on the test goal".to_string()),
                detail: Some("evidence".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let choice = answered_debug_choice(&pool, &goal.id, "dispatch-debugger").await;
        let proof = DecisionProof::from_answered_row(&choice).unwrap();
        let (effect, warning) = apply_decision_effect(&pool, &choice, proof, "choice")
            .await
            .unwrap();
        assert!(effect.unwrap().contains("debugger mandate set"));
        assert!(warning.is_none());

        // Sticky mandate persisted for the next dispatch.
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("dispatch_role")
                .and_then(|v| v.as_str()),
            Some("debugger")
        );
        // The open review was answered 'reject' under the same actor…
        let review_after = decisions::get_decision(&pool, &review.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(review_after.answer.as_deref(), Some("reject"));
        assert_eq!(review_after.acted_by.as_deref(), Some(ACTOR_JESSE));
        // …and draining the outbox applies the standard rework transition.
        drain_effect_outbox(&pool).await.unwrap();
        assert_eq!(goal_state_of(&pool, &goal.id).await, "in_progress");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            card.metadata_json
                .get("attempt_count")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            card.metadata_json
                .get("dispatch_role")
                .and_then(|v| v.as_str()),
            Some("debugger"),
            "the mandate survives the rework transition"
        );
    }

    #[tokio::test]
    async fn debug_dispatch_without_open_review_records_role_but_moves_nothing() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        let choice = answered_debug_choice(&pool, &goal.id, "dispatch-debugger").await;
        let proof = DecisionProof::from_answered_row(&choice).unwrap();
        let (effect, _) = apply_decision_effect(&pool, &choice, proof, "choice")
            .await
            .unwrap();
        assert!(effect.unwrap().contains("no open approve_review"));
        assert_eq!(goal_state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn leave_for_review_choice_changes_nothing() {
        let pool = test_pool().await;
        let goal = goal_in_review(&pool).await;
        let choice = answered_debug_choice(&pool, &goal.id, "leave-for-review").await;
        let proof = DecisionProof::from_answered_row(&choice).unwrap();
        let (effect, _) = apply_decision_effect(&pool, &choice, proof, "choice")
            .await
            .unwrap();
        assert!(effect.unwrap().contains("stays in Review"));
        assert_eq!(goal_state_of(&pool, &goal.id).await, "review");
        let card = crate::cards::get_card(&pool, &goal.id)
            .await
            .unwrap()
            .unwrap();
        assert!(card.metadata_json.get("dispatch_role").is_none());
    }

    async fn answered_file_decision(pool: &Pool<Sqlite>) -> Decision {
        let decision = decisions::create_decision(
            pool,
            NewDecision {
                kind: "file_to_project".to_string(),
                project_id: Some(PERSONAL_PROJECT_ID.to_string()),
                headline: Some("File these notes to Personal".to_string()),
                detail: Some("One reviewed note".to_string()),
                payload: serde_json::json!({
                    "project_id": PERSONAL_PROJECT_ID,
                    "project_name": "Personal",
                    "title": "Retry-safe note",
                    "body": "This body must be filed exactly once.",
                    "content_origin": "test fixture",
                    "people": []
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(decision.kind, "file_to_project");
        decisions::answer_decision(
            pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap()
        .0
    }

    async fn briefing_count(pool: &Pool<Sqlite>, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_briefings WHERE kind = ?")
            .bind(kind)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// D32: a dead effect is an approved, audited, user-authorised action that
    /// never happened, and it had zero surface — no briefing, no card, no
    /// doctor check, only a `tracing::warn`. It must reach a surface, ONCE, on
    /// the 0→nonzero transition.
    #[tokio::test]
    async fn a_dead_effect_briefs_once_on_the_transition_not_every_drain() {
        let pool = test_pool().await;
        crate::session::spectral_schema::apply_briefings_schema(&pool)
            .await
            .unwrap();
        let decision = answered_file_decision(&pool).await;
        // Bury the effect: one more failure than it has attempts left.
        sqlx::query(
            "UPDATE effect_outbox SET status = 'pending', attempts = max_attempts,
                 last_error = 'filesystem is read-only',
                 next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now','-1 hour')
             WHERE decision_id = ?",
        )
        .bind(&decision.id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM decisions WHERE id = ?")
            .bind(&decision.id)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(briefing_count(&pool, "effect_dead_letter").await, 0);
        drain_effect_outbox(&pool).await.unwrap();

        let dead = dead_effects(&pool).await.unwrap();
        assert_eq!(dead.len(), 1, "the effect must be retired, not retried");
        assert_eq!(dead[0].kind, "file_to_project");
        assert!(
            dead[0].last_error.contains("deleted"),
            "a dangling row must say the decision is gone, not a generic retry error: {}",
            dead[0].last_error
        );
        assert_eq!(
            briefing_count(&pool, "effect_dead_letter").await,
            1,
            "the transition into a non-empty dead-letter queue must be briefed"
        );

        // Depth is already non-zero: further drains must go quiet. A push per
        // tick is what teaches the reader to ignore the channel.
        for _ in 0..3 {
            drain_effect_outbox(&pool).await.unwrap();
        }
        assert_eq!(
            briefing_count(&pool, "effect_dead_letter").await,
            1,
            "a standing dead-letter depth belongs in the digest, not in a push per minute"
        );
    }

    /// D31: `effect_outbox.decision_id` has no foreign key, so removing a
    /// project cascades the decision away and strands the row. A PENDING
    /// stranded row must not burn five retries on "no longer exists".
    #[tokio::test]
    async fn a_dangling_effect_is_retired_once_not_retried_five_times() {
        let pool = test_pool().await;
        let decision = answered_file_decision(&pool).await;
        sqlx::query(
            "UPDATE effect_outbox SET status = 'pending', attempts = 0,
                 next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now','-1 hour')
             WHERE decision_id = ?",
        )
        .bind(&decision.id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM decisions WHERE id = ?")
            .bind(&decision.id)
            .execute(&pool)
            .await
            .unwrap();

        drain_effect_outbox(&pool).await.unwrap();
        let (status, attempts): (String, i64) =
            sqlx::query_as("SELECT status, attempts FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "dead");
        assert_eq!(
            attempts, 5,
            "retirement should consume the budget at once, not one tick at a time"
        );
    }

    #[tokio::test]
    async fn file_to_project_replay_creates_exactly_one_note() {
        let pool = test_pool().await;
        let decision = answered_file_decision(&pool).await;
        for _ in 0..2 {
            let proof = DecisionProof::from_answered_row(&decision).unwrap();
            apply_decision_effect(&pool, &decision, proof, &decision.kind)
                .await
                .unwrap();
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_notes WHERE project_id = ?")
                .bind(PERSONAL_PROJECT_ID)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn worker_claims_and_applies_a_pending_effect() {
        let pool = test_pool().await;
        let decision = answered_file_decision(&pool).await;
        drain_effect_outbox(&pool).await.unwrap();
        let row: (String, i64) =
            sqlx::query_as("SELECT status, attempts FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row, ("applied".to_string(), 1));
        assert_eq!(
            crate::project_notes::list_notes(&pool, PERSONAL_PROJECT_ID)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn transient_failure_is_retried_and_effect_applies() {
        let pool = test_pool().await;
        let missing_project = "retry-project";
        // The `project_id` *column* FK-references projects(id) ON DELETE CASCADE,
        // so it must stay NULL here: the target project does not exist yet. The
        // effect's target lives in the payload, which is exactly what
        // apply_file_to_project reads — so the first drain fails NotFound
        // (retriable → pending) and a later drain applies it once the project
        // appears. This exercises the retry-then-apply loop without the column FK
        // rejecting a decision that references a not-yet-created project.
        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "file_to_project".to_string(),
                headline: Some("File a retryable note".to_string()),
                detail: Some("The project will appear before retry".to_string()),
                payload: serde_json::json!({
                    "project_id": missing_project,
                    "project_name": "Retry Project",
                    "body": "Persist after the transient failure.",
                    "content_origin": "test fixture",
                    "people": []
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (decision, _) = decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();

        drain_effect_outbox(&pool).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "pending");

        sqlx::query(
            "INSERT INTO projects (id, user_id, slug, name, description, status)
             VALUES (?, 'default', 'retry-project', 'Retry Project', '', 'active')",
        )
        .bind(missing_project)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE effect_outbox
             SET next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE decision_id = ?",
        )
        .bind(&decision.id)
        .execute(&pool)
        .await
        .unwrap();

        drain_effect_outbox(&pool).await.unwrap();
        let status: String =
            sqlx::query_scalar("SELECT status FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "applied");
        assert_eq!(
            crate::project_notes::list_notes(&pool, missing_project)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// CASE A fix #4: before this fix, an effect driven all the way to `dead`
    /// through repeated `drain_effect_outbox` calls left ZERO decision_audit
    /// rows beyond the original answer — no per-attempt trail, and the
    /// decisions row stayed indistinguishable from one whose effect actually
    /// applied. Every failed attempt (retried or terminal) must now leave its
    /// own audit row, and the terminal one must be nameable as a dead-letter.
    #[tokio::test]
    async fn dead_lettered_effect_leaves_an_audit_row_per_failed_attempt() {
        let pool = test_pool().await;
        // A `file_to_project` whose target project never appears: every
        // attempt fails NotFound (retriable), so five drains exhaust
        // max_attempts=5 and land the row on 'dead'.
        let never_appears = "never-appears-project";
        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "file_to_project".to_string(),
                headline: Some("File a note that can never land".to_string()),
                detail: Some("The target project never appears".to_string()),
                payload: serde_json::json!({
                    "project_id": never_appears,
                    "project_name": "Never",
                    "body": "This never lands.",
                    "content_origin": "test fixture",
                    "people": []
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (decision, _) = decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();

        let audit_before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM decision_audit WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        // `create_decision` writes a 'created' row, `answer_decision` writes
        // the 'approve' row — nothing about the effect itself yet.
        assert_eq!(audit_before, 2, "just creation + the answer so far");

        // Drive every attempt to failure: 5 drains exhausts max_attempts=5.
        for _ in 0..5 {
            drain_effect_outbox(&pool).await.unwrap();
            sqlx::query(
                "UPDATE effect_outbox
                 SET next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE decision_id = ?",
            )
            .bind(&decision.id)
            .execute(&pool)
            .await
            .unwrap();
        }

        let (status, attempts): (String, i64) =
            sqlx::query_as("SELECT status, attempts FROM effect_outbox WHERE decision_id = ?")
                .bind(&decision.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "dead");
        assert_eq!(attempts, 5);

        let audit_outcomes: Vec<String> = sqlx::query_scalar(
            "SELECT outcome FROM decision_audit WHERE decision_id = ? ORDER BY seq",
        )
        .bind(&decision.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        // 2 (created + approved) + 5 (one per failed attempt) = 7. Today: 2.
        assert_eq!(
            audit_outcomes.len(),
            7,
            "expected an audit row per failed attempt on top of the created+answer rows, got: \
             {audit_outcomes:?}"
        );
        assert!(
            audit_outcomes[2..6]
                .iter()
                .all(|o| o.starts_with("effect_retry:")),
            "{audit_outcomes:?}"
        );
        assert!(
            audit_outcomes[6].starts_with("effect_dead:"),
            "the terminal attempt must be nameable as a dead-letter — distinguishable from a \
             successful decision's audit trail: {audit_outcomes:?}"
        );

        // decision_history (the read path decisions/history rows come from)
        // must actually carry that dead-letter marker — a UI-readable field
        // distinguishing this decision from one whose effect succeeded.
        let history = decisions::decision_history(&pool, 50, None).await.unwrap();
        assert!(
            history
                .iter()
                .any(|h| h.decision.id == decision.id && h.outcome.starts_with("effect_dead:")),
            "decision_history must surface the dead-letter outcome"
        );
    }

    // ── Steward git-health arm (repo_worktree_reap / repo_branch_delete) ──

    fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
    }

    /// Real repo at `<tmp>/proj` with a file:// origin, one pushed baseline
    /// commit, and one clean detached worktree registered at `<tmp>/wt-reap`.
    fn repo_with_worktree(tmp: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let origin = tmp.join("origin.git");
        std::process::Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&origin)
            .output()
            .unwrap();
        let repo = tmp.join("proj");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t.t"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&repo, &["branch", "-M", "main"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        git(&repo, &["fetch", "-q", "origin"]);
        let wt = tmp.join("wt-reap");
        git(
            &repo,
            &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
        );
        assert!(wt.is_dir());
        (repo, wt)
    }

    /// File the reap through the REAL proposer, then answer as the user.
    async fn answered_reap_decision(
        pool: &Pool<Sqlite>,
        repo: &std::path::Path,
        wt: &std::path::Path,
    ) -> Decision {
        let id = crate::steward::hygiene::propose_repo_hygiene(
            pool,
            crate::steward::hygiene::RepoHygieneProposal {
                action_class: crate::steward::hygiene::ACTION_REPO_WORKTREE_REAP.to_string(),
                repo_path: repo.to_string_lossy().to_string(),
                worktree_path: Some(wt.to_string_lossy().to_string()),
                branch: None,
                evidence: vec!["clean, fully pushed, detached at baseline".to_string()],
                headline: "Tidy up: remove a finished worktree?".to_string(),
                project_id: None,
            },
        )
        .await
        .unwrap()
        .expect("proposal files");
        let d = decisions::get_decision(pool, &id).await.unwrap().unwrap();
        assert_eq!(d.tier, 2, "repo hygiene must resolve to the user-only tier");
        decisions::answer_decision(
            pool,
            &id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap()
        .0
    }

    async fn outbox_status(pool: &Pool<Sqlite>, decision_id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM effect_outbox WHERE decision_id = ?")
            .bind(decision_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn approved_worktree_reap_removes_the_worktree_via_the_outbox() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt) = repo_with_worktree(tmp.path());
        let pool = test_pool().await;
        let decision = answered_reap_decision(&pool, &repo, &wt).await;

        drain_effect_outbox(&pool).await.unwrap();

        assert_eq!(outbox_status(&pool, &decision.id).await, "applied");
        assert!(!wt.exists(), "the approved worktree must be gone");
    }

    #[tokio::test]
    async fn world_changed_dirty_worktree_is_refused_but_outbox_settles_applied() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (repo, wt) = repo_with_worktree(tmp.path());
        let pool = test_pool().await;
        let decision = answered_reap_decision(&pool, &repo, &wt).await;

        // The world changes between approval and application.
        std::fs::write(wt.join("scratch.txt"), "uncommitted work").unwrap();

        drain_effect_outbox(&pool).await.unwrap();

        assert!(
            wt.exists(),
            "a now-dirty worktree must be refused, not removed"
        );
        assert_eq!(
            outbox_status(&pool, &decision.id).await,
            "applied",
            "a world-changed refusal is terminal — the outbox must NOT retry it"
        );
        let _ = repo;
    }

    #[tokio::test]
    async fn repo_hygiene_decision_without_repo_target_errs_not_applies() {
        let pool = test_pool().await;
        let decision = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "risk_gate".to_string(),
                headline: Some("Remove a worktree?".to_string()),
                detail: Some("no target".to_string()),
                payload: serde_json::json!({
                    "action_class": crate::steward::hygiene::ACTION_REPO_WORKTREE_REAP,
                    "description": "?",
                    "requested_by": "steward"
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (decision, proof) = decisions::answer_decision(
            &pool,
            &decision.id,
            &DecisionAnswer {
                answer: "approve".to_string(),
                ..Default::default()
            },
            ACTOR_JESSE,
        )
        .await
        .unwrap();
        let err = apply_decision_effect(&pool, &decision, proof, "risk_gate")
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("repo_target"));
    }

    fn project_intel_decision(items: serde_json::Value) -> NewDecision {
        NewDecision {
            kind: "project_intel_proposal".to_string(),
            project_id: Some(PERSONAL_PROJECT_ID.to_string()),
            headline: Some("Project intelligence found".to_string()),
            detail: Some("intel".to_string()),
            payload: serde_json::json!({
                "project_id": PERSONAL_PROJECT_ID,
                "project_name": "Personal",
                "items": items,
            }),
            ..Default::default()
        }
    }

    /// Fix-as-found (REVIEW_CONFIRMED #9): `project_intel` accumulates rows
    /// across every approved decision, but the Brain feed for it used to be
    /// built ONLY from the current call's `payload.items` and written under a
    /// static per-project key — so a second applied batch replaced (in
    /// recall) everything an earlier batch had fed. The feed must reflect the
    /// full, accumulated `project_intel` table after every apply.
    #[tokio::test]
    async fn project_intel_feed_accumulates_across_applied_batches() {
        let pool = test_pool().await;

        let d1 = decisions::create_decision(
            &pool,
            project_intel_decision(serde_json::json!([
                {"kind": "competitor", "name": "Acme", "note": null, "source_url": "https://acme.example"}
            ])),
        )
        .await
        .unwrap();
        apply_project_intel(&pool, &d1).await.unwrap();

        let d2 = decisions::create_decision(
            &pool,
            project_intel_decision(serde_json::json!([
                {"kind": "partner", "name": "Globex", "note": null, "source_url": "https://globex.example"}
            ])),
        )
        .await
        .unwrap();
        apply_project_intel(&pool, &d2).await.unwrap();

        let fed: String =
            sqlx::query_scalar("SELECT content FROM brain_feed_outbox WHERE memory_key = ?")
                .bind(crate::brain_feed::feed_key(
                    "project",
                    PERSONAL_PROJECT_ID,
                    "intel",
                ))
                .fetch_one(&pool)
                .await
                .unwrap();

        assert!(
            fed.contains("Acme"),
            "the first batch's item must still be recallable: {fed}"
        );
        assert!(fed.contains("Globex"), "{fed}");
    }

    fn regression_payload(task_id: &str) -> crate::decisions::RegressionProposalPayload {
        serde_json::from_value(serde_json::json!({
            "task_id": task_id,
            "title": "Parked goal summary stays honest",
            "prompt": "Make the summary honest.",
            "test": ["python3", "oracle/check.py"],
            "oracle_filename": "check.py",
            "oracle_source": "import sys\nsys.exit(0)\n",
            "incident_id": "incident-1",
        }))
        .unwrap()
    }

    /// Inbox fix group: the approve arm wrote to a path relative to the
    /// process's working directory — `/` under launchd, the read-only system
    /// volume — so every approval failed with os error 30. The tasks directory
    /// is absolute and never depends on the cwd.
    #[test]
    fn approved_regressions_never_resolve_against_the_working_directory() {
        let dir = eval_tasks_dir();
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(
            dir.ends_with("permagent-eval/tasks") || dir.ends_with("eval-tasks"),
            "{}",
            dir.display()
        );
    }

    /// Inbox fix group: the outbox retries a failed effect, so the write must
    /// be safe to repeat: a retry that finds exactly its own files succeeds,
    /// a half-written earlier attempt never blocks it, and a different task
    /// under the same id is still never overwritten.
    #[test]
    fn a_regression_task_write_is_safe_to_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks = tmp.path().join("tasks");
        let payload = regression_payload("honest-park-summary");

        // A crashed earlier attempt left a partial write behind.
        std::fs::create_dir_all(tasks.join(".honest-park-summary.partial/oracle")).unwrap();
        let path = write_regression_task_in(&tasks, &payload).unwrap();
        let root = tasks.join("honest-park-summary");
        assert_eq!(path, root.display().to_string());
        assert!(root.join("task.yaml").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join("oracle/check.py")).unwrap(),
            payload.oracle_source
        );
        assert!(!tasks.join(".honest-park-summary.partial").exists());

        // The outbox retries after the write landed: the same files, success.
        assert_eq!(write_regression_task_in(&tasks, &payload).unwrap(), path);

        // Different content under the same id is existing coverage: refused.
        let mut other = regression_payload("honest-park-summary");
        other.oracle_source = "import sys\nsys.exit(1)\n".to_string();
        let err = write_regression_task_in(&tasks, &other).unwrap_err();
        assert!(format!("{err:?}").contains("already exists"), "{err:?}");
    }
}
