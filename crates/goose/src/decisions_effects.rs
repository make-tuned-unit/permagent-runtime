//! Durable, retryable application of answered Decision Inbox effects.

use crate::decisions::{self, Decision, DecisionProof};
use crate::goal_state::GoalAction;
use crate::goal_transition::{self, GuardError, TransitionEffects};
use sqlx::{Pool, Row, Sqlite};

const OUTBOX_BATCH_SIZE: i64 = 32;
const RUNNING_LEASE_SECONDS: i64 = 300;

/// The result displayed by the inline answer path.
pub type EffectResult = (Option<String>, Option<String>);

/// Where approved regressions are materialised, relative to the repo root.
const EVAL_TASKS_DIR: &str = "crates/permagent-eval/tasks";

/// Write an approved regression as a `permagent-eval` task directory.
///
/// Re-validates the slug and filename rather than trusting the payload: these
/// values reach the filesystem, and the approval gate authorises WHAT is
/// written, not WHERE. Refuses to overwrite an existing task — a proposal that
/// silently replaced a passing regression would be a way to delete coverage
/// through the approval path.
fn write_regression_task(
    payload: &crate::decisions::RegressionProposalPayload,
) -> Result<String, GuardError> {
    use std::io::Write;

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

    let root = std::path::Path::new(EVAL_TASKS_DIR).join(&payload.task_id);
    if root.exists() {
        return Err(GuardError::Invalid(format!(
            "regression task '{}' already exists — refusing to overwrite existing coverage",
            payload.task_id
        )));
    }
    let oracle_dir = root.join("oracle");
    std::fs::create_dir_all(&oracle_dir)
        .map_err(|e| GuardError::Invalid(format!("could not create task dir: {e}")))?;

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

    let mut f = std::fs::File::create(root.join("task.yaml"))
        .map_err(|e| GuardError::Invalid(format!("could not write task.yaml: {e}")))?;
    f.write_all(spec.as_bytes())
        .map_err(|e| GuardError::Invalid(format!("could not write task.yaml: {e}")))?;

    let mut o = std::fs::File::create(oracle_dir.join(&payload.oracle_filename))
        .map_err(|e| GuardError::Invalid(format!("could not write oracle: {e}")))?;
    o.write_all(payload.oracle_source.as_bytes())
        .map_err(|e| GuardError::Invalid(format!("could not write oracle: {e}")))?;

    Ok(root.display().to_string())
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

/// Apply an outbox-eligible decision effect without daemon `AppState`.
///
/// `tool_approval` and `session_gate` deliberately are not handled here.
pub async fn apply_decision_effect(
    pool: &Pool<Sqlite>,
    decision: &Decision,
    proof: DecisionProof,
    kind: &str,
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
            match write_regression_task(&payload) {
                Ok(path) => Ok((None, Some(format!("Regression written to {path}")))),
                Err(e) => Err(e),
            }
        }
        ("approve_review", Some("approve")) => {
            let Some(goal_id) = decision.goal_id.as_deref() else {
                return Ok((None, None));
            };
            match goal_state(pool, goal_id).await?.as_deref() {
                Some("complete") => return already_applied("goal was already Complete"),
                Some("review") => {}
                None => return already_applied("goal was already gone"),
                Some(state) => {
                    return already_applied(format!(
                        "goal already advanced out of Review (current state: {state})"
                    ))
                }
            }
            goal_transition::advance_goal_checked(
                pool,
                goal_id,
                GoalAction::Approve,
                &acted_by,
                Some(proof),
                TransitionEffects {
                    review_notes: decision.answer_note.clone(),
                    ..Default::default()
                },
            )
            .await?;
            let warning = match decision.project_id.as_deref() {
                Some(project_id) => {
                    goal_transition::promote_eligible_dependents_or_warn(pool, project_id, goal_id)
                        .await
                }
                None => None,
            };
            crate::recognition::write_back_decision_outcome(pool, goal_id, true).await;
            Ok((
                Some("goal approved: Review → Complete".to_string()),
                warning,
            ))
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
                let unblock_id = goal_transition::exhaust_and_park(
                    pool,
                    goal_id,
                    &card.title,
                    &card.project_id,
                    goal_transition::BudgetExhaustion::AttemptCap {
                        spent,
                        cap: budget.attempt_cap,
                    },
                    Some(&reason),
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
                "goal unparked to Ready with attempt_cap raised to {attempt_cap}"
            ))
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

/// Claim and apply a bounded batch of due effects.
pub async fn drain_effect_outbox(pool: &Pool<Sqlite>) -> Result<(), String> {
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
        let result = async {
            let decision_id =
                decision_id.ok_or_else(|| "effect outbox row has no decision_id".to_string())?;
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
            }
        }
    }
    Ok(())
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
}
