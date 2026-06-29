//! Verification pipeline for completed goals (Decision Inbox, Lane L2).
//!
//! After a goal worker finishes and L1's `handle_goal_completion` moves the
//! card to Review, `run_for_goal` executes the goal's declared completion
//! checks, analyzes the diff over the worker's own commits (`work_base..head`,
//! #523), grades the work with a local Ollama model, and writes the result to
//! `metadata_json.verification` via L1's narrow allowlist API
//! `cards::set_goal_verification` (the sanctioned L2 write path).
//!
//! L1 contract (PHASE0-L2.md §4):
//! - This module writes ONLY the `verification` key in metadata_json.
//! - It NEVER calls `move_card` and never touches `goal_state`,
//!   `attempt_count`, or `review_notes`.
//! - The diff anchor is the worker's `work_base_commit` (the parent of its first
//!   commit, captured emit-side by an in-worktree hook, #523), NOT the stale
//!   dispatch `baseline_commit`. A head-recorded goal missing its work_base / git
//!   failure / no declared_paths ⇒ uncertain, never a silent pass or false fail.
//! - Zero cloud tokens at runtime: the only network call is to the hardcoded
//!   loopback Ollama base URL.

pub mod checks;
pub mod digest;
pub mod verifier;

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::path::{Path, PathBuf};

use checks::{CheckResult, CheckStatus, CompletionCheck};
use digest::{DiffSummary, EvidenceDigest, PerFileDiff};
use verifier::{Grade, VerdictStatus, VerifierRun};

/// The single metadata key this module owns.
pub const VERIFICATION_KEY: &str = "verification";

// ── Record schema (metadata_json.verification) ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rubric {
    pub intent_match: Grade,
    pub evidence_attached: Grade,
    pub checks_support: Grade,
    pub path_discipline: Grade,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub version: u32,
    pub status: VerdictStatus,
    pub rubric: Rubric,
    pub rationale: String,
    pub check_results: Vec<CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_commit: Option<String>,
    pub diff_stat: String,
    pub out_of_path_files: Vec<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub evidence_digest: EvidenceDigest,
}

// ── Entry points ────────────────────────────────────────────────────────────

/// Verify a completed goal and persist the result. This is the call L1's
/// Review-transition will invoke (call-site insertion happens at coordinator
/// merge). Standalone and side-effect-scoped: one metadata write, no moves.
pub async fn run_for_goal(
    pool: &Pool<Sqlite>,
    goal_id: &str,
) -> Result<VerificationRecord, String> {
    run_for_goal_with(pool, goal_id, verifier::OLLAMA_BASE_URL).await
}

/// Install the post-Review verification hook on L1's orchestrator extension
/// point (`GOAL_REVIEW_HOOK`): after `handle_goal_completion` moves a goal to
/// Review, `run_for_goal` runs as a spawned, failure-tolerant task — a
/// verification failure never breaks the completion path. Idempotent; call
/// once at daemon startup (the startup call belongs to the coordinator's
/// state.rs wiring).
pub fn install_review_hook() {
    let _ = permagent::agents::platform_extensions::orchestrator::GOAL_REVIEW_HOOK.set(Box::new(
        |pool, goal_id| {
            tokio::spawn(async move {
                if let Err(e) = run_for_goal(&pool, &goal_id).await {
                    tracing::warn!(
                        target: "permagentd::verification",
                        goal_id = %goal_id,
                        "Post-Review verification failed (non-fatal): {}",
                        e
                    );
                }
            });
        },
    ));
}

/// Same as `run_for_goal` but with an injectable Ollama base URL (tests).
pub async fn run_for_goal_with(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    ollama_base_url: &str,
) -> Result<VerificationRecord, String> {
    run_for_goal_with_cfg(pool, goal_id, ollama_base_url, verifier::load_config()).await
}

/// As [`run_for_goal_with`] but with an injectable [`verifier::VerifierConfig`]
/// (tests exercise the auto-approve allow-list without writing `verifier.json`).
pub async fn run_for_goal_with_cfg(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    ollama_base_url: &str,
    cfg: verifier::VerifierConfig,
) -> Result<VerificationRecord, String> {
    let started_at = chrono::Utc::now().to_rfc3339();

    let card = permagent::cards::get_card(pool, goal_id)
        .await?
        .ok_or_else(|| format!("Card '{}' not found", goal_id))?;
    if card.card_type != "goal" {
        return Err(format!(
            "Card '{}' is a '{}' card, not a goal",
            goal_id, card.card_type
        ));
    }

    let meta = card.metadata_json.as_object().cloned().unwrap_or_default();

    // Verify dir: the goal's isolated WORKTREE when an external-CLI worker
    // committed there (captured by Layer-1 dispatch evidence), else the project
    // root. Diffing project.root_path is WRONG under the worktree-and-push
    // model — the root is local `main`, which the worker never touches (it
    // commits in the worktree and pushes to origin), so root yields a
    // false-empty diff that FALSE-FAILS correct work. Preferring the worktree
    // makes the verifier diff the SAME baseline..HEAD the Evidence panel shows.
    let project = permagent::projects::get_project(pool, &card.project_id).await?;
    let root_dir: Option<PathBuf> = project
        .as_ref()
        .and_then(|p| p.root_path.as_ref())
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let worktree_recorded: Option<String> = meta
        .get("dispatch_evidence")
        .and_then(|e| e.get("worktree_path"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let worktree_dir: Option<PathBuf> = worktree_recorded
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    // The worker's committed HEAD, durably recorded by Layer-1 dispatch evidence
    // at completion. Paired with `work_base_commit` (below), the durable
    // `work_base..head` range resolves from the project root's SHARED git object
    // store even after the worktree is reaped (#511 reaps fully-pushed Complete
    // worktrees, see goal_engine::reap_goal_worktree), so verification never
    // depends on the transient worktree. Unpushed goals keep their worktree
    // (reaper returns SkippedUnpushed), so the worktree branch still works too.
    let head_commit: Option<String> = meta
        .get("dispatch_evidence")
        .and_then(|e| e.get("head_commit"))
        .and_then(|v| v.as_str())
        .map(String::from);
    // Fix (#523): the worker's TRUE base — the parent of its FIRST commit,
    // captured emit-side by an in-worktree git hook (re-captured on rebase). This
    // is the only anchor correct for N commits AND robust to fast-forward/rebase
    // after dispatch. The dispatch `baseline_commit` is a stale project-root
    // snapshot and is NEVER used to anchor a head-recorded (external-CLI) goal.
    let work_base_commit: Option<String> = meta
        .get("dispatch_evidence")
        .and_then(|e| e.get("work_base_commit"))
        .and_then(|v| v.as_str())
        .map(String::from);
    // Fix A (#505): a worktree that was RECORDED at dispatch but has since
    // vanished (reaped) must be LOUD, not a silent fall-through to the clean
    // project root. With a durable work_base..head the root diff is correct;
    // without it the diff is unprovable (recorded Uncertain) — either way the
    // operator needs to see it.
    if worktree_dir.is_none() {
        if let Some(p) = worktree_recorded.as_deref() {
            tracing::warn!(
                target: "permagentd::verification",
                goal_id = %goal_id,
                worktree = %p,
                has_durable_head = head_commit.is_some(),
                "dispatch worktree is gone (reaped) — falling back to project root; \
                 verification diffs the durable work_base..head range when both were \
                 recorded, else the diff is unprovable (recorded Uncertain)"
            );
        }
    }
    let working_dir: Option<PathBuf> = worktree_dir.or(root_dir);

    // ── 1. Completion checks ──
    let declared_checks: Vec<CompletionCheck> = Vec::new();
    let (declared_checks, check_results) = match meta.get("completion_checks") {
        None => (declared_checks, Vec::new()),
        Some(raw) => match serde_json::from_value::<Vec<CompletionCheck>>(raw.clone()) {
            Ok(parsed) => {
                let results = match working_dir.as_deref() {
                    Some(wd) => checks::run_checks(&parsed, wd).await,
                    None => parsed
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            error_result(
                                i,
                                c.type_name(),
                                "working_dir unavailable — project has no resolvable root_path",
                            )
                        })
                        .collect(),
                };
                (parsed, results)
            }
            Err(e) => (
                Vec::new(),
                vec![error_result(
                    0,
                    "completion_checks",
                    &format!("completion_checks failed to parse: {}", e),
                )],
            ),
        },
    };

    // ── 2. Diff vs baseline + declared-paths discipline ──
    let baseline_commit = meta
        .get("baseline_commit")
        .and_then(|v| v.as_str())
        .map(String::from);
    let declared_paths: Vec<String> = meta
        .get("declared_paths")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let git_analysis = analyze_diff(
        working_dir.as_deref(),
        baseline_commit.as_deref(),
        head_commit.as_deref(),
        work_base_commit.as_deref(),
        &declared_paths,
    )
    .await;

    // ── 3. Verifier (local Ollama, deterministic aggregation in Rust) ──
    let acceptance_criteria: Vec<String> = meta
        .get("acceptance_criteria")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let claimed_evidence = meta
        .get("claimed_evidence")
        .and_then(|v| v.as_str())
        .unwrap_or("(none provided)")
        .to_string();
    let check_summaries: Vec<String> = declared_checks.iter().map(|c| c.summary()).collect();

    let user_prompt = verifier::build_user_prompt(
        &card.title,
        &card.description,
        &acceptance_criteria,
        &declared_paths,
        &git_analysis.diff_stat,
        &check_results,
        &check_summaries,
        &claimed_evidence,
    );

    let vr = verifier::run_verifier(ollama_base_url, &cfg.model, &user_prompt).await;

    // ── 4. Deterministic aggregation + machine-check clamps ──
    let record = aggregate_record(
        goal_id,
        &card.title,
        &meta,
        &declared_checks,
        check_results,
        &git_analysis,
        baseline_commit,
        &vr,
        &cfg,
        worker_tokens(pool, &meta).await,
        &started_at,
    );

    // ── 5. ONE atomic metadata write: only the `verification` key ──
    write_verification(pool, goal_id, &record).await?;

    tracing::info!(
        target: "permagentd::verification",
        goal_id = %goal_id,
        status = ?record.status,
        model = %record.model,
        "Goal verification complete"
    );

    // ── 6. Auto-approval — OPT-IN, DEFAULT-OFF (L3 policy × this verdict) ──
    // A corrected verifier PASS is ADVISORY: it is recorded as evidence
    // (status + rationale in the digest) and still requires manual approval,
    // UNLESS the goal's type is explicitly allow-listed in verifier.json
    // (`auto_approve_goal_types`). With no types designated — the default —
    // nothing auto-approves, so fixing the diff directory cannot silently turn
    // on blanket auto-approval. When enabled, Henry answers the goal's open
    // approve_review decision as 'henry-policy' through L1's tier-gated answer
    // path. Failure-tolerant: the verification record stands regardless.
    if record.status == VerdictStatus::Pass {
        if auto_approve_allowed(&cfg, &meta) {
            henry_approve_after_pass(pool, goal_id, &record).await;
        } else {
            tracing::info!(
                target: "permagentd::verification",
                goal_id = %goal_id,
                "Verifier PASS recorded as advisory evidence — auto-approve not \
                 enabled for this goal type; manual approval required"
            );
        }
    }

    Ok(record)
}

/// The single auto-approval gate. A verified PASS may be auto-approved by
/// henry-policy ONLY when the goal's `goal_type` (card metadata) appears in the
/// verifier config's `auto_approve_goal_types` allow-list. An empty allow-list
/// (the default) or an untyped goal ⇒ never. This is the seam the future
/// low-risk-type taxonomy plugs into: it will set `goal_type` on the card and
/// list the earned types here. Until then, every verdict is advisory.
fn auto_approve_allowed(
    cfg: &verifier::VerifierConfig,
    meta: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    if cfg.auto_approve_goal_types.is_empty() {
        return false;
    }
    match meta.get("goal_type").and_then(|v| v.as_str()) {
        Some(goal_type) => cfg.auto_approve_goal_types.iter().any(|t| t == goal_type),
        None => false,
    }
}

/// Wire 4 (integration): verifier pass → henry-policy Tier-1 auto-approve.
/// Looks up the open approve_review decision `handle_goal_completion`
/// created when the goal moved to Review, then routes through L3's
/// [`permagent::decision_inbox::policy::henry_approve_on_verifier_pass`].
/// Every failure path is logged and swallowed — this never breaks the
/// verification flow, and the daemon still tier-validates inside
/// `decisions::answer_decision`.
async fn henry_approve_after_pass(pool: &Pool<Sqlite>, goal_id: &str, record: &VerificationRecord) {
    let decision =
        match permagent::decisions::find_open_decision_for_goal(pool, goal_id, "approve_review")
            .await
        {
            Ok(Some(d)) => d,
            Ok(None) => {
                tracing::info!(
                    target: "permagentd::verification",
                    goal_id = %goal_id,
                    "Verifier pass, but no open approve_review decision — nothing to auto-approve"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::verification",
                    goal_id = %goal_id,
                    "Verifier pass, but approve_review lookup failed (non-fatal): {}",
                    e
                );
                return;
            }
        };

    let rationale = format!("verifier pass: {}", record.rationale);
    match permagent::decision_inbox::policy::henry_approve_on_verifier_pass(
        pool,
        &decision.id,
        &rationale,
    )
    .await
    {
        Ok(approval) => match approval.effect_error {
            None => tracing::info!(
                target: "permagentd::verification",
                goal_id = %goal_id,
                decision_id = %decision.id,
                effect = ?approval.effect,
                "Verifier pass — approve_review auto-approved by henry-policy"
            ),
            Some(e) => tracing::warn!(
                target: "permagentd::verification",
                goal_id = %goal_id,
                decision_id = %decision.id,
                "henry-policy approval recorded but effect failed (non-fatal): {}",
                e
            ),
        },
        Err(e) => tracing::warn!(
            target: "permagentd::verification",
            goal_id = %goal_id,
            decision_id = %decision.id,
            "henry-policy auto-approval refused (non-fatal): {}",
            e
        ),
    }
}

fn error_result(index: usize, check_type: &str, message: &str) -> CheckResult {
    CheckResult {
        check_index: index,
        check_type: check_type.to_string(),
        status: CheckStatus::Error,
        started_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: 0,
        evidence: checks::CheckEvidence {
            message: Some(message.to_string()),
            ..Default::default()
        },
        truncated: false,
    }
}

// ── Aggregation ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn aggregate_record(
    goal_id: &str,
    goal_title: &str,
    meta: &serde_json::Map<String, serde_json::Value>,
    declared_checks: &[CompletionCheck],
    check_results: Vec<CheckResult>,
    git: &GitAnalysis,
    baseline_commit: Option<String>,
    vr: &VerifierRun,
    cfg: &verifier::VerifierConfig,
    tokens: Option<i64>,
    started_at: &str,
) -> VerificationRecord {
    let any_check_not_pass = check_results.iter().any(|r| r.status != CheckStatus::Pass);

    // Model grades; a degraded run grades everything uncertain.
    let (q1, q2, q3, q4, rationale) = match &vr.grades {
        Some(g) => (
            g.q1_intent,
            g.q2_evidence,
            g.q3_checks,
            g.q4_paths,
            g.rationale.clone(),
        ),
        None => (
            Grade::Uncertain,
            Grade::Uncertain,
            Grade::Uncertain,
            Grade::Uncertain,
            String::new(),
        ),
    };

    // Deterministic clamps: the model can never upgrade machine findings.
    let checks_clamp = if any_check_not_pass {
        Grade::Fail
    } else {
        Grade::Pass
    };
    let rubric = Rubric {
        intent_match: q1,
        evidence_attached: q2,
        checks_support: q3.worse(checks_clamp),
        path_discipline: q4.worse(git.path_discipline),
    };

    // Final verdict: aggregate the clamped rubric, then apply the
    // machine-check clamp (any check fail/error → at most fail).
    let aggregated = verifier::aggregate_grades(&verifier::RubricGrades {
        q1_intent: rubric.intent_match,
        q2_evidence: rubric.evidence_attached,
        q3_checks: rubric.checks_support,
        q4_paths: rubric.path_discipline,
        rationale: if rationale.is_empty() {
            "(degraded)".to_string()
        } else {
            rationale.clone()
        },
    });
    let status = verifier::clamp_with_check_results(aggregated, &check_results);

    // Deterministic no-op clamp: now that the diff is taken against the correct
    // worktree, a *successful* diff that found zero changed files is an
    // unambiguous true no-op — the worker produced no work product, so it can
    // never pass. Guards: skip when the git diff itself degraded (git failure /
    // missing baseline) and when the model didn't run (verifier degraded →
    // Uncertain stands, never downgraded to Fail) — only a clean, empty diff on
    // a completed verifier run is a real fail.
    let true_no_op =
        git.degraded_note.is_none() && git.diff_summary.files_changed == 0 && vr.grades.is_some();
    let status = if true_no_op {
        VerdictStatus::Fail
    } else {
        status
    };
    let rationale = if true_no_op {
        "No changes since baseline in the goal's worktree — the worker produced no \
         work product, so this cannot be approved as complete."
            .to_string()
    } else {
        rationale
    };

    let finished_at = chrono::Utc::now().to_rfc3339();

    let costs = digest::build_costs(
        tokens,
        cfg.usd_per_1k_tokens,
        meta.get("worker_session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        meta.get("attempt_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    );

    let persona_name = permagent::config::agent_identity::load_agent_config()
        .primary
        .display_name();
    let evidence_digest = digest::assemble_digest(
        &persona_name,
        goal_id,
        goal_title,
        declared_checks,
        &check_results,
        git.diff_summary.clone(),
        git.out_of_path_files.clone(),
        status,
        vr,
        costs,
        started_at,
        &finished_at,
    );

    // Surface the path-analysis degradation when the verifier itself is fine.
    let degraded_reason = vr
        .degraded_reason
        .clone()
        .or_else(|| git.degraded_note.clone());

    VerificationRecord {
        version: 1,
        status,
        rubric,
        rationale,
        check_results,
        baseline_commit,
        diff_stat: git.diff_stat.clone(),
        out_of_path_files: git.out_of_path_files.clone(),
        model: vr.model.clone(),
        degraded_reason,
        started_at: started_at.to_string(),
        finished_at,
        evidence_digest,
    }
}

// ── Git analysis ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GitAnalysis {
    pub diff_summary: DiffSummary,
    pub diff_stat: String,
    pub out_of_path_files: Vec<String>,
    /// Deterministic path-discipline grade: uncertain when baseline/git/globs
    /// are unavailable; fail when out-of-path changes exist; else pass.
    pub path_discipline: Grade,
    pub degraded_note: Option<String>,
}

fn uncertain_analysis(note: &str) -> GitAnalysis {
    GitAnalysis {
        diff_summary: DiffSummary::default(),
        diff_stat: format!("(diff unavailable: {})", note),
        out_of_path_files: Vec::new(),
        path_discipline: Grade::Uncertain,
        degraded_note: Some(note.to_string()),
    }
}

async fn git_output(working_dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("failed to run git: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `git merge-base --is-ancestor <ancestor> <descendant>`: `Ok(true)` when
/// `ancestor` is an ancestor of (or equal to) `descendant`, `Ok(false)` when it
/// is not, `Err` only on a real git error (e.g. an unknown object). Exit 0 = is
/// ancestor, exit 1 = is not; any other exit is an error. Used to detect an
/// inverted diff range before diffing (#531).
async fn git_is_ancestor(
    working_dir: &Path,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, String> {
    let output = tokio::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("failed to run git: {}", e))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        other => Err(format!(
            "git merge-base --is-ancestor exited {:?}: {}",
            other,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// Analyze the worker's changes and grade declared-path discipline.
///
/// For a head-recorded (external-CLI) goal the diff is anchored to the worker's
/// TRUE base (`work_base`), the parent of its first commit captured emit-side by
/// an in-worktree git hook (#523). `work_base..head` is exactly the worker's own
/// work — correct for N commits and robust to fast-forward/rebase after dispatch.
/// If a head is recorded but `work_base` is absent (the hook did not fire), the
/// analysis is Uncertain — we never silently fall back to the stale dispatch
/// baseline. The legacy working-tree path (diff vs `baseline_commit`) is used
/// ONLY for evidence with no recorded head (in-process subagent).
pub async fn analyze_diff(
    working_dir: Option<&Path>,
    baseline_commit: Option<&str>,
    head_commit: Option<&str>,
    work_base: Option<&str>,
    declared_paths: &[String],
) -> GitAnalysis {
    let Some(wd) = working_dir else {
        return uncertain_analysis("no working_dir");
    };
    let Some(baseline) = baseline_commit else {
        return uncertain_analysis("no baseline_commit recorded at dispatch");
    };

    // Fix A (#505): every git command here is failure-LOUD. A non-zero/failed
    // git invocation is an ERRORED diff (Uncertain), surfaced via tracing::error!
    // — NEVER a false zero-diff that the no-op clamp would turn into a wrongful
    // Fail of correct, pushed work.
    async fn run_or_error(wd: &Path, args: &[&str]) -> Result<String, GitAnalysis> {
        git_output(wd, args).await.map_err(|e| {
            tracing::error!(
                target: "permagentd::verification",
                dir = %wd.display(),
                args = ?args,
                "git command failed during diff analysis — recording ERRORED \
                 (Uncertain) evidence, NOT a false zero-diff: {}",
                e
            );
            uncertain_analysis(&format!(
                "git {} failed: {}",
                args.first().unwrap_or(&""),
                e
            ))
        })
    }

    // Fix (#523): anchor the diff to the worker's TRUE base (`work_base`), NOT the
    // dispatch-time baseline. Between dispatch and commit the worker routinely
    // fast-forwards/pulls (and may rebase) `main` past the dispatch baseline, so
    // `dispatch_baseline..head` over-counts the intervening commits' files (false
    // path-discipline Fail) or — when the local root never advanced — reads EMPTY
    // (the #505 false zero-diff Fail). `work_base` is the parent of the worker's
    // first commit, captured emit-side by the in-worktree hook and re-captured on
    // rebase; `work_base..head` is exactly the worker's own commits (1 or N) and
    // is read from the DURABLE shared object store (survives the #511 reaper).
    //
    // A head WITHOUT a work_base means the capture hook did not fire. We do NOT
    // silently fall back to the stale dispatch baseline (the silent-wrongness that
    // is this whole bug class) — we record Uncertain and surface it loudly.
    let (names, numstat, porcelain, effective_baseline) = match head_commit {
        Some(head) => {
            let Some(base) = work_base else {
                tracing::error!(
                    target: "permagentd::verification",
                    dispatch_baseline = %baseline,
                    head_commit = %head,
                    "work_base_commit absent for a head-recorded goal — the base-capture \
                     hook did not fire; recording Uncertain rather than diffing against a \
                     possibly-stale baseline"
                );
                return uncertain_analysis(
                    "work-base capture hook did not fire — the worker's true base is \
                     unprovable; refusing to diff against the stale dispatch baseline",
                );
            };
            // Defensive (#531): never diff an INVERTED range. `work_base..head` is
            // valid only when base is an ancestor of head (base = parent of the
            // worker's first commit; head = its tip). If instead head is an
            // ancestor of base, the recorded head is wrong (e.g. a stale
            // completion-time read that landed on an ancestor of the true tip) and
            // `git diff base head` computes a REVERSE diff — silently turning the
            // worker's additions into deletions and false-failing correct work.
            // Record Uncertain LOUDLY rather than emit a confident wrong grade.
            // (head == base is the genuine empty range, handled by the no-op clamp.)
            if base != head {
                match git_is_ancestor(wd, head, base).await {
                    Ok(true) => {
                        tracing::error!(
                            target: "permagentd::verification",
                            work_base = %base,
                            head_commit = %head,
                            "INVERTED diff range: head is an ancestor of work_base — \
                             the recorded head commit is wrong; recording Uncertain \
                             instead of diffing a reverse range (#531)"
                        );
                        return uncertain_analysis(&format!(
                            "inverted diff range: head {head} is an ancestor of base \
                             {base} — the recorded head commit is wrong; refusing to \
                             compute a reverse diff that would false-fail correct work"
                        ));
                    }
                    Ok(false) => {}
                    Err(e) => {
                        tracing::error!(
                            target: "permagentd::verification",
                            work_base = %base,
                            head_commit = %head,
                            "could not verify diff-range ancestry: {e} — recording \
                             Uncertain rather than risk a reverse diff (#531)"
                        );
                        return uncertain_analysis(&format!(
                            "could not verify diff-range ancestry ({base}..{head}): {e}"
                        ));
                    }
                }
            }
            let names = match run_or_error(wd, &["diff", "--name-only", base, head]).await {
                Ok(o) => o,
                Err(a) => return a,
            };
            let numstat = match run_or_error(wd, &["diff", "--numstat", base, head]).await {
                Ok(o) => o,
                Err(a) => return a,
            };
            // Fix A for real (#523): log the resolved refs on the SUCCESS path, not
            // only on git failure (the prior fix logged nothing here, leaving the
            // diff undebuggable). Surfacing dispatch_baseline vs work_base makes a
            // stale anchor obvious at a glance, and names the mechanism (hook).
            tracing::info!(
                target: "permagentd::verification",
                dispatch_baseline = %baseline,
                work_base = %base,
                head_commit = %head,
                range = %format!("{base}..{head}"),
                mechanism = "hook",
                "verifier diff anchored to work_base..head (durable range)"
            );
            (names, numstat, String::new(), base.to_string())
        }
        None => {
            // Committed + uncommitted tracked changes since the dispatch baseline
            // (live worktree). No durable head ⇒ the dispatch baseline is the only
            // anchor available.
            let names = match run_or_error(wd, &["diff", "--name-only", baseline]).await {
                Ok(o) => o,
                Err(a) => return a,
            };
            let numstat = match run_or_error(wd, &["diff", "--numstat", baseline]).await {
                Ok(o) => o,
                Err(a) => return a,
            };
            let porcelain = match run_or_error(wd, &["status", "--porcelain"]).await {
                Ok(o) => o,
                Err(a) => return a,
            };
            tracing::info!(
                target: "permagentd::verification",
                dispatch_baseline = %baseline,
                "verifier diff anchored to dispatch baseline (no durable head recorded \
                 — legacy working-tree diff)"
            );
            (names, numstat, porcelain, baseline.to_string())
        }
    };

    let mut changed: Vec<String> = Vec::new();
    for line in names.lines() {
        let f = line.trim();
        if !f.is_empty() && !changed.iter().any(|c| c == f) {
            changed.push(f.to_string());
        }
    }
    for line in porcelain.lines() {
        // Porcelain v1: "XY path" (rename: "XY old -> new").
        let path_part = line.get(3..).unwrap_or("").trim();
        let f = path_part
            .rsplit(" -> ")
            .next()
            .unwrap_or(path_part)
            .trim_matches('"');
        if !f.is_empty() && !changed.iter().any(|c| c == f) {
            changed.push(f.to_string());
        }
    }

    let mut insertions = 0u64;
    let mut deletions = 0u64;
    let mut per_file: Vec<PerFileDiff> = Vec::new();
    for line in numstat.lines() {
        let mut parts = line.split('\t');
        let ins = parts.next().unwrap_or("0").trim();
        let del = parts.next().unwrap_or("0").trim();
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let ins_n = ins.parse::<u64>().unwrap_or(0); // "-" for binary
        let del_n = del.parse::<u64>().unwrap_or(0);
        insertions += ins_n;
        deletions += del_n;
        per_file.push(PerFileDiff {
            path: path.to_string(),
            insertions: ins_n,
            deletions: del_n,
        });
    }

    let diff_summary = DiffSummary {
        files_changed: changed.len(),
        insertions,
        deletions,
        per_file,
    };
    // Fix A for real (#523): a genuinely empty range is LOUD too, so the
    // false-empty (a git failure — already ERRORED above) vs true-empty (the
    // worker committed nothing in range) distinction is visible in the logs and
    // not silently inferred from a bare Fail.
    if diff_summary.files_changed == 0 {
        tracing::warn!(
            target: "permagentd::verification",
            baseline = %effective_baseline,
            "verifier diff is genuinely EMPTY over the resolved range — the worker \
             committed no changes here (a real zero-diff, NOT a git failure)"
        );
    }
    // Annotate each file with its own +ins/-del so the verifier model can see
    // which files actually changed (and by how much), not just a bare list of
    // names under an aggregate total. Files without a numstat entry (untracked,
    // binary "-") are listed without counts. This matches the per-file shape the
    // system-prompt examples advertise (verifier.rs EXAMPLES).
    let file_lines: Vec<String> = changed
        .iter()
        .map(
            |f| match diff_summary.per_file.iter().find(|p| &p.path == f) {
                Some(p) => format!("{} (+{} -{})", f, p.insertions, p.deletions),
                None => f.clone(),
            },
        )
        .collect();
    let diff_stat = format!(
        "{} file(s) changed since {}: +{} -{}\n{}",
        changed.len(),
        effective_baseline,
        insertions,
        deletions,
        file_lines.join("\n")
    );

    // Path discipline.
    if declared_paths.is_empty() {
        return GitAnalysis {
            diff_summary,
            diff_stat,
            out_of_path_files: Vec::new(),
            path_discipline: Grade::Uncertain,
            degraded_note: Some(
                "no declared_paths on goal — path discipline unverifiable".to_string(),
            ),
        };
    }

    let mut builder = globset::GlobSetBuilder::new();
    for g in declared_paths {
        match globset::Glob::new(g) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(e) => {
                return uncertain_analysis(&format!("invalid declared_paths glob '{}': {}", g, e))
            }
        }
    }
    let set = match builder.build() {
        Ok(s) => s,
        Err(e) => return uncertain_analysis(&format!("declared_paths glob set failed: {}", e)),
    };

    let out_of_path: Vec<String> = changed
        .iter()
        .filter(|f| !set.is_match(f.as_str()))
        .cloned()
        .collect();

    let path_discipline = if out_of_path.is_empty() {
        Grade::Pass
    } else {
        Grade::Fail
    };

    GitAnalysis {
        diff_summary,
        diff_stat,
        out_of_path_files: out_of_path,
        path_discipline,
        degraded_note: None,
    }
}

// ── Costs ───────────────────────────────────────────────────────────────────

/// Worker token usage from the sessions table (existing accounting).
async fn worker_tokens(
    pool: &Pool<Sqlite>,
    meta: &serde_json::Map<String, serde_json::Value>,
) -> Option<i64> {
    let session_id = meta.get("worker_session_id")?.as_str()?;
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT accumulated_total_tokens FROM sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

// ── Persistence (L1 contract: one atomic update, verification key only) ─────

async fn write_verification(
    pool: &Pool<Sqlite>,
    goal_id: &str,
    record: &VerificationRecord,
) -> Result<(), String> {
    // L1's sanctioned narrow API (allowlist): writes ONLY the `verification`
    // key, re-reading the card internally so concurrent L1 metadata writes
    // are preserved. Never moves cards, never touches protected keys.
    permagent::cards::set_goal_verification(
        pool,
        goal_id,
        serde_json::to_value(record).map_err(|e| e.to_string())?,
    )
    .await
}

// ── Test support: mock Ollama server ────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    pub enum MockMode {
        /// Stream the given text then a done line.
        Respond(String),
        /// Always return HTTP 500.
        Status500,
        /// Stream text but never send done.
        NoDone(String),
        /// Send a non-JSON line.
        MalformedNdjson,
        /// Send an inline {"error": ...} line.
        StreamError,
        /// First streamed call returns `bad`, subsequent return `good`.
        BadThenGood { bad: String, good: String },
    }

    struct MockState {
        mode: MockMode,
        stream_calls: AtomicUsize,
    }

    fn ndjson_respond(text: &str) -> String {
        format!(
            "{}\n{}\n",
            serde_json::json!({"response": text, "done": false}),
            serde_json::json!({"response": "", "done": true}),
        )
    }

    async fn generate_handler(
        state: axum::extract::State<Arc<MockState>>,
        body: axum::extract::Json<serde_json::Value>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;

        let streaming = body
            .0
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Warm-load probe (stream:false): succeed unless the mode is Status500.
        if !streaming {
            return match state.mode {
                MockMode::Status500 => (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "mock failure",
                )
                    .into_response(),
                _ => (
                    axum::http::StatusCode::OK,
                    serde_json::json!({"response": "ok", "done": true}).to_string(),
                )
                    .into_response(),
            };
        }

        let n = state.stream_calls.fetch_add(1, Ordering::SeqCst);
        let body = match &state.mode {
            MockMode::Respond(text) => ndjson_respond(text),
            MockMode::Status500 => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "mock failure",
                )
                    .into_response()
            }
            MockMode::NoDone(text) => {
                format!("{}\n", serde_json::json!({"response": text, "done": false}))
            }
            MockMode::MalformedNdjson => "this is not json\n".to_string(),
            MockMode::StreamError => format!("{}\n", serde_json::json!({"error": "boom"})),
            MockMode::BadThenGood { bad, good } => {
                if n == 0 {
                    ndjson_respond(bad)
                } else {
                    ndjson_respond(good)
                }
            }
        };
        (axum::http::StatusCode::OK, body).into_response()
    }

    /// Spawn a mock Ollama on an ephemeral loopback port.
    /// Returns (base_url, server task handle).
    pub async fn spawn_mock_ollama(mode: MockMode) -> (String, tokio::task::JoinHandle<()>) {
        let state = Arc::new(MockState {
            mode,
            stream_calls: AtomicUsize::new(0),
        });
        let app = axum::Router::new()
            .route("/api/generate", axum::routing::post(generate_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://127.0.0.1:{}", addr.port()), handle)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::test_support::{spawn_mock_ollama, MockMode};
    use super::*;

    const GOOD_PASS: &str = "Q1_INTENT: PASS\nQ2_EVIDENCE: PASS\nQ3_CHECKS: PASS\nQ4_PATHS: PASS\nRATIONALE: Everything checks out.";

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        permagent::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    fn sh(dir: &Path, cmd: &str) -> String {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "command failed: {}\nstderr: {}",
            cmd,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Init a git repo with one baseline commit; returns the baseline sha.
    fn init_repo(dir: &Path) -> String {
        sh(dir, "git init -q -b main");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(dir.join("README.md"), "readme\n").unwrap();
        sh(dir, "git add -A");
        sh(
            dir,
            "git -c user.email=t@t -c user.name=t commit -q -m baseline",
        );
        sh(dir, "git rev-parse HEAD")
    }

    async fn make_goal(
        pool: &Pool<Sqlite>,
        root_path: &str,
        extra_meta: serde_json::Value,
    ) -> permagent::cards::Card {
        let project = permagent::projects::create_project(
            pool,
            permagent::projects::CreateProject {
                name: "Verify Test".to_string(),
                root_path: Some(root_path.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mut meta = serde_json::Map::new();
        meta.insert("goal_state".to_string(), serde_json::json!("review"));
        meta.insert("attempt_count".to_string(), serde_json::json!(1));
        if let Some(obj) = extra_meta.as_object() {
            for (k, v) in obj {
                meta.insert(k.clone(), v.clone());
            }
        }

        permagent::cards::create_card(
            pool,
            permagent::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Toy goal".to_string(),
                description: Some("Make src/lib.rs have function b".to_string()),
                card_type: Some("goal".to_string()),
                column_id: None,
                created_by: Some("user".to_string()),
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn e2e_pass_writes_only_verification_key_and_never_moves_card() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        // In-path change.
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn a() {}\npub fn b() {}\n",
        )
        .unwrap();

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "true", "timeout_secs": 30},
                    {"type": "file_exists", "path": "src/lib.rs"},
                    {"type": "grep_absent", "pattern": "FIXME", "paths": ["src/lib.rs"]}
                ],
                "worker_session_id": "sess-none"
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &card.id, &base_url).await.unwrap();

        assert_eq!(record.status, VerdictStatus::Pass);
        assert_eq!(record.version, 1);
        assert_eq!(record.check_results.len(), 3);
        assert!(record
            .check_results
            .iter()
            .all(|r| r.status == CheckStatus::Pass));
        assert_eq!(record.rubric.path_discipline, Grade::Pass);
        assert!(record.out_of_path_files.is_empty());
        assert_eq!(record.rationale, "Everything checks out.");

        // Card not moved, other metadata untouched, verification key written.
        let after = permagent::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.column_id, card.column_id, "L2 must never move cards");
        let meta = after.metadata_json.as_object().unwrap();
        assert_eq!(meta.get("goal_state").unwrap(), "review");
        assert_eq!(meta.get("attempt_count").unwrap(), 1);
        let v = meta.get("verification").expect("verification key written");
        let parsed: VerificationRecord = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(parsed.status, VerdictStatus::Pass);
        // Digest summary layer present.
        assert!(parsed
            .evidence_digest
            .checks_summary
            .one_line
            .contains("All 3 automated checks passed"));
        assert!(parsed
            .evidence_digest
            .verifier_summary
            .contains("confirmed"));
        // No rate configured → cost_usd null + note.
        assert_eq!(parsed.evidence_digest.costs.cost_usd, None);
    }

    /// CONTRACT (L1 hardening × L2 allowlist, part 1): the module's REAL
    /// `metadata_json.verification` write — run_for_goal persisting via L1's
    /// narrow API `cards::set_goal_verification` — succeeds against a goal
    /// card, never moves it, and never disturbs protected keys.
    #[tokio::test]
    async fn contract_l1_allowlist_permits_verification_write_on_goal_card() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        run_for_goal_with(&pool, &card.id, &base_url)
            .await
            .expect("L1's hardened layer must permit the verification write on a goal card");

        let after = permagent::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        let meta = after.metadata_json.as_object().unwrap();
        assert!(
            meta.contains_key(VERIFICATION_KEY),
            "verification key written through the allowlist"
        );
        assert_eq!(after.column_id, card.column_id, "card never moved");
        assert_eq!(meta.get("goal_state").unwrap(), "review");
        assert_eq!(meta.get("attempt_count").unwrap(), 1);
    }

    /// CONTRACT (part 2): from this module's exact call path (goose-server →
    /// permagent::cards), a metadata write touching a PROTECTED key
    /// (attempt_count) is refused by L1's guard, while the same write shape
    /// carrying only a `verification` change passes — the allowlist is
    /// exactly as narrow as designed.
    #[tokio::test]
    async fn contract_l1_guard_rejects_protected_key_write_from_module_path() {
        let pool = test_pool().await;
        let card = make_goal(&pool, "/nonexistent", serde_json::json!({})).await;

        // Allowed: general update carrying only a `verification` change.
        let mut meta = card.metadata_json.as_object().cloned().unwrap();
        meta.insert(
            VERIFICATION_KEY.to_string(),
            serde_json::json!({"status": "uncertain"}),
        );
        permagent::cards::update_card(
            &pool,
            &card.id,
            permagent::cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta.clone())),
                ..Default::default()
            },
        )
        .await
        .expect("`verification` is deliberately NOT protected (L2 allowlist)");

        // Refused: identical path, but mutating the protected attempt_count.
        meta.insert("attempt_count".to_string(), serde_json::json!(99));
        let err = permagent::cards::update_card(
            &pool,
            &card.id,
            permagent::cards::UpdateCard {
                metadata_json: Some(serde_json::Value::Object(meta)),
                ..Default::default()
            },
        )
        .await;
        let msg = err.expect_err("protected key write must be refused");
        assert!(
            msg.contains("protected goal metadata key 'attempt_count'"),
            "unexpected refusal message: {}",
            msg
        );

        // Protected key unchanged on disk.
        let after = permagent::cards::get_card(&pool, &card.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.metadata_json.get("attempt_count").unwrap(), 1);
    }

    /// The hook installer registers on L1's orchestrator extension point, is
    /// idempotent, and the installed closure is failure-tolerant (unknown
    /// goal id → logged, no panic, completion path unaffected).
    #[tokio::test]
    async fn install_review_hook_registers_and_tolerates_failure() {
        install_review_hook();
        install_review_hook(); // idempotent: second set is a no-op
        let hook = permagent::agents::platform_extensions::orchestrator::GOAL_REVIEW_HOOK
            .get()
            .expect("hook installed");

        let pool = test_pool().await;
        hook(pool, "no-such-goal".to_string()); // must not panic or block
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    /// Build a goal that sits in its project's REAL Review column (so the
    /// henry-policy approve effect can move it through the guard), backed by
    /// a git repo at `root_path` and carrying `extra_meta`.
    async fn make_review_goal_in_columns(
        pool: &Pool<Sqlite>,
        root_path: &str,
        extra_meta: serde_json::Value,
    ) -> permagent::cards::Card {
        let project = permagent::projects::create_project(
            pool,
            permagent::projects::CreateProject {
                name: "Wire4 Test".to_string(),
                root_path: Some(root_path.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        permagent::cards::seed_goal_columns(pool, &project.id)
            .await
            .unwrap();
        let review_col = permagent::cards::get_goal_column(pool, &project.id, "review")
            .await
            .unwrap()
            .unwrap();

        let mut meta = serde_json::Map::new();
        meta.insert("goal_state".to_string(), serde_json::json!("review"));
        meta.insert("attempt_count".to_string(), serde_json::json!(1));
        if let Some(obj) = extra_meta.as_object() {
            for (k, v) in obj {
                meta.insert(k.clone(), v.clone());
            }
        }
        permagent::cards::create_card(
            pool,
            permagent::cards::CreateCard {
                project_id: project.id.clone(),
                title: "Wire4 goal".to_string(),
                description: Some("Make src/lib.rs have function b".to_string()),
                card_type: Some("goal".to_string()),
                column_id: Some(review_col.id.clone()),
                created_by: Some("user".to_string()),
                metadata_json: Some(serde_json::Value::Object(meta)),
            },
        )
        .await
        .unwrap()
    }

    async fn state_of(pool: &Pool<Sqlite>, card_id: &str) -> String {
        let card = permagent::cards::get_card(pool, card_id)
            .await
            .unwrap()
            .unwrap();
        let col = permagent::cards::get_column(pool, &card.column_id)
            .await
            .unwrap()
            .unwrap();
        col.state_binding.unwrap_or_default()
    }

    /// Wire 4: verifier PASS → the goal's open Tier-1 approve_review decision
    /// is answered by henry-policy with the verifier rationale and the goal
    /// completes; a Tier-2 decision is untouched AND not auto-approvable by
    /// henry-policy (tier gate in L1's answer path).
    #[tokio::test]
    async fn verifier_pass_auto_approves_tier1_review_via_henry_policy() {
        use permagent::decisions::{self, NewDecision};

        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn a() {}\npub fn b() {}\n",
        )
        .unwrap();

        let pool = test_pool().await;
        let goal = make_review_goal_in_columns(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "goal_type": "chore",
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "true", "timeout_secs": 30}
                ],
            }),
        )
        .await;

        // The approval need handle_goal_completion creates at Review time.
        let d_review = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(goal.project_id.clone()),
                headline: Some("Review the finished work on the wire4 goal".to_string()),
                detail: Some("worker reported success".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d_review.tier, 1);

        // A Tier-2 decision on the same goal: must never be auto-approved.
        let d_risk = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "risk_gate".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(goal.project_id.clone()),
                headline: Some("Permission to push the release".to_string()),
                detail: Some("merge_to_main risk gate".to_string()),
                payload: serde_json::json!({
                    "action_class": "merge_to_main",
                    "description": "publish",
                    "requested_by": "test"
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(d_risk.tier, 2);

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        // Auto-approve is opt-in: this goal's type is allow-listed, so a PASS
        // auto-approves. (The default empty allow-list is covered by
        // `verifier_pass_is_advisory_by_default_no_auto_approve`.)
        let cfg = verifier::VerifierConfig {
            auto_approve_goal_types: vec!["chore".to_string()],
            ..Default::default()
        };
        let record = run_for_goal_with_cfg(&pool, &goal.id, &base_url, cfg)
            .await
            .unwrap();
        assert_eq!(record.status, VerdictStatus::Pass);

        // Tier-1 approve_review answered by henry-policy, rationale recorded,
        // goal completed through the guard.
        let d_review = decisions::get_decision(&pool, &d_review.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d_review.status, "answered");
        assert_eq!(d_review.acted_by.as_deref(), Some(decisions::ACTOR_HENRY));
        assert_eq!(d_review.answer.as_deref(), Some("approve"));
        let note = d_review.answer_note.as_deref().unwrap();
        assert!(
            note.contains("verifier pass"),
            "rationale missing: {}",
            note
        );
        assert!(
            note.contains("Everything checks out."),
            "verifier rationale must be carried into the answer note: {}",
            note
        );
        assert_eq!(state_of(&pool, &goal.id).await, "complete");

        // Tier-2 untouched by the pass…
        let d_risk_after = decisions::get_decision(&pool, &d_risk.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d_risk_after.status, "open");
        assert!(d_risk_after.acted_by.is_none());

        // …and NOT auto-approvable by henry-policy: the daemon tier-validates.
        let err = permagent::decision_inbox::policy::henry_approve_on_verifier_pass(
            &pool,
            &d_risk.id,
            "verifier pass: should be refused",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, decisions::AnswerError::Forbidden(_)),
            "Tier-2 via henry-policy must be Forbidden: {:?}",
            err
        );
        let d_risk_final = decisions::get_decision(&pool, &d_risk.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d_risk_final.status, "open");
    }

    /// Redirect (default-off gate): with NO goal types designated — the
    /// default — a verifier PASS is ADVISORY. The verdict is recorded as
    /// evidence but the approve_review decision stays open and the goal stays in
    /// Review; nothing auto-approves. Same setup as the auto-approve test, only
    /// the empty allow-list differs.
    #[tokio::test]
    async fn verifier_pass_is_advisory_by_default_no_auto_approve() {
        use permagent::decisions::{self, NewDecision};

        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn a() {}\npub fn b() {}\n",
        )
        .unwrap();

        let pool = test_pool().await;
        let goal = make_review_goal_in_columns(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "goal_type": "chore",
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "true", "timeout_secs": 30}
                ],
            }),
        )
        .await;
        let d_review = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(goal.project_id.clone()),
                headline: Some("Review the finished work".to_string()),
                detail: Some("worker reported success".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        // Default config: empty auto_approve_goal_types — nothing auto-approves
        // even though the verdict is PASS and the goal carries a goal_type.
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();
        assert_eq!(record.status, VerdictStatus::Pass);

        let d_after = decisions::get_decision(&pool, &d_review.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d_after.status, "open", "advisory PASS must not auto-answer");
        assert!(d_after.acted_by.is_none());
        assert_eq!(
            state_of(&pool, &goal.id).await,
            "review",
            "goal must stay in Review for manual approval"
        );
    }

    /// Redirect (core fix): the verifier diffs the goal's WORKTREE, not the
    /// project root. The root stays clean at baseline (as under the
    /// worktree-and-push model); the real change is committed only in the
    /// worktree. Diffing the root would yield a false-empty (true-no-op → Fail);
    /// diffing the worktree sees the change → PASS with files_changed > 0.
    #[tokio::test]
    async fn verifier_diffs_worktree_not_project_root() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        // Root stays at baseline, clean — the worker never touched it.
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        // The worker's real, committed work lives in the worktree.
        std::fs::write(wt.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        sh(&wt, "git add -A");
        sh(
            &wt,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "dispatch_evidence": { "worktree_path": wt.to_str().unwrap() },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.status,
            VerdictStatus::Pass,
            "real worktree change must be seen (root would be false-empty → Fail)"
        );
        assert!(
            record.evidence_digest.diff.files_changed >= 1,
            "diff must reflect the worktree change, got {:?}",
            record.diff_stat
        );
    }

    /// Finding 4: the diff_stat handed to the verifier model must annotate EACH
    /// changed file with its own +ins/-del — not list bare names under one
    /// aggregate total. The live dogfood showed qwen2.5:7b false-claiming
    /// "README.md changed but not TESTFILE.md" from a bare list; per-file counts
    /// give it the signal to see both. Asserts both files carry counts.
    #[tokio::test]
    async fn diff_stat_annotates_each_changed_file_with_per_file_counts() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        // Two distinct files change: README.md gains a line, TESTFILE.md is new.
        std::fs::write(wt.join("README.md"), "readme\nmore\n").unwrap();
        std::fs::write(wt.join("TESTFILE.md"), "hello\n").unwrap();
        sh(&wt, "git add -A");
        sh(
            &wt,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["**"],
                "dispatch_evidence": { "worktree_path": wt.to_str().unwrap() },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        let ds = &record.diff_stat;
        assert!(
            ds.contains("README.md (+"),
            "diff_stat must annotate README.md with per-file counts: {ds}"
        );
        assert!(
            ds.contains("TESTFILE.md (+"),
            "diff_stat must annotate TESTFILE.md with per-file counts: {ds}"
        );
    }

    /// Redirect: a genuine no-op — the worktree exists but has no commits since
    /// baseline — is a REAL fail (deterministic clamp), even if the model would
    /// grade PASS. Distinct from the old false-empty caused by the wrong dir.
    #[tokio::test]
    async fn verifier_true_no_op_in_worktree_fails() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        // No changes in the worktree — the worker produced nothing.

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "dispatch_evidence": { "worktree_path": wt.to_str().unwrap() },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.status,
            VerdictStatus::Fail,
            "an empty worktree diff is a true no-op — must fail even on a model PASS"
        );
        assert!(
            record.rationale.contains("no work product"),
            "rationale must explain the no-op: {}",
            record.rationale
        );
    }

    /// Fix B2 (#505): a PUSHED goal whose worktree was REAPED (#511 removes
    /// fully-pushed Complete worktrees) still verifies correctly. The worker's
    /// committed HEAD is durably recorded; the commit object persists in the
    /// project root's shared object store, so the verifier diffs the durable
    /// `baseline..head` range from the root — NOT the vanished worktree — and
    /// sees the real change instead of a false-empty (which used to → Fail).
    #[tokio::test]
    async fn verifier_uses_durable_range_when_worktree_reaped() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        // The worker commits its work in the worktree (shared object store).
        std::fs::write(wt.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        sh(&wt, "git add -A");
        sh(
            &wt,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );
        let head = sh(&wt, "git rev-parse --short HEAD");
        // #511 reaps the (pushed) worktree — the dir is now GONE.
        sh(
            repo.path(),
            &format!("git worktree remove --force {}", wt.display()),
        );
        assert!(!wt.is_dir(), "worktree must be reaped for this test");

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "dispatch_evidence": {
                    "worktree_path": wt.to_str().unwrap(),
                    "head_commit": head,
                    // Single commit off baseline ⇒ the worker's true base IS baseline.
                    "work_base_commit": baseline,
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.status,
            VerdictStatus::Pass,
            "reaped-but-pushed work must verify from the durable range, not false-fail"
        );
        assert!(
            record.evidence_digest.diff.files_changed >= 1,
            "durable range must reflect the committed change, got {:?}",
            record.diff_stat
        );
    }

    /// Fix B2 (#505): an UNPUSHED goal keeps its worktree (the reaper returns
    /// SkippedUnpushed), and verification still reads the durable
    /// `baseline..head` range from that preserved worktree — proving B2 does not
    /// regress worktree-only goals.
    #[tokio::test]
    async fn verifier_uses_durable_range_in_preserved_worktree() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        std::fs::write(wt.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        sh(&wt, "git add -A");
        sh(
            &wt,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );
        let head = sh(&wt, "git rev-parse --short HEAD");
        // Worktree is PRESERVED (unpushed goal).
        assert!(wt.is_dir());

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "dispatch_evidence": {
                    "worktree_path": wt.to_str().unwrap(),
                    "head_commit": head,
                    "work_base_commit": baseline,
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(record.status, VerdictStatus::Pass);
        assert!(record.evidence_digest.diff.files_changed >= 1);
    }

    /// Fix (#523), the ACCEPTANCE scenario: a worker that FAST-FORWARDS main past
    /// N intervening commits (other goals pushed in the interim) before committing
    /// its own work. The diff must anchor to the worker's TRUE base (`work_base`,
    /// the parent of its first commit), NOT the stale dispatch baseline — so it
    /// sees ONLY the worker's file, never the intervening commits' files, and
    /// PASSES. Anchoring to the dispatch baseline would either over-count (false
    /// path-discipline Fail) or, when the local root never advanced, read empty
    /// (false zero-diff Fail — the #505 symptom that #520 did not fix).
    #[tokio::test]
    async fn verifier_anchors_to_worker_parent_after_fast_forward() {
        let repo = tempfile::tempdir().unwrap();
        let dispatch_baseline = init_repo(repo.path());

        // Two intervening "other goal" commits land on main AFTER dispatch — the
        // worker fast-forwards past them before doing its own work.
        std::fs::write(repo.path().join("OTHER_A.md"), "other goal a\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m other-a",
        );
        std::fs::write(repo.path().join("OTHER_B.md"), "other goal b\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m other-b",
        );

        // The worker's TRUE base = the tip it forked onto after fast-forwarding =
        // the parent of its first commit. This is what the emit-side hook records.
        let work_base = sh(repo.path(), "git rev-parse HEAD");

        // The worker commits ONLY its own declared file on top.
        std::fs::write(repo.path().join("WORKER.md"), "the worker's work\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m worker-work",
        );
        let head = sh(repo.path(), "git rev-parse --short HEAD");

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                // STALE dispatch baseline — recorded before the fast-forward.
                "baseline_commit": dispatch_baseline,
                "declared_paths": ["WORKER.md"],
                "dispatch_evidence": {
                    // Worktree reaped (#511) — verification runs from the root.
                    "worktree_path": "/nonexistent/reaped/wt",
                    "head_commit": head,
                    "work_base_commit": work_base,
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.status,
            VerdictStatus::Pass,
            "fast-forward-then-commit must verify from work_base..head, not the \
             stale dispatch baseline; diff_stat={:?}",
            record.diff_stat
        );
        assert_eq!(
            record.evidence_digest.diff.files_changed, 1,
            "only the worker's own commit must be in range, not the intervening \
             fast-forwarded commits; diff_stat={:?}",
            record.diff_stat
        );
        assert!(
            record.diff_stat.contains("WORKER.md"),
            "diff must show the worker's file: {:?}",
            record.diff_stat
        );
        assert!(
            !record.diff_stat.contains("OTHER_A.md") && !record.diff_stat.contains("OTHER_B.md"),
            "diff must NOT include the fast-forwarded intervening commits' files: {:?}",
            record.diff_stat
        );
    }

    /// Fix (#523), option 3: a head-recorded (external-CLI) goal whose work-base
    /// hook did NOT fire is recorded Uncertain — we never silently fall back to
    /// the stale dispatch baseline (the silent-wrongness that is this whole bug
    /// class). Honest-Uncertain beats silently-maybe-wrong.
    #[tokio::test]
    async fn verifier_missing_work_base_is_uncertain_never_fail() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        // A foreign commit advances the root past the dispatch baseline, so a diff
        // against that baseline would NOT be empty — proving the Uncertain verdict
        // comes from the missing base, not from an incidental zero-diff.
        std::fs::write(repo.path().join("OTHER.md"), "other\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m other",
        );
        std::fs::write(repo.path().join("WORKER.md"), "work\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );
        let head = sh(repo.path(), "git rev-parse --short HEAD");

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["WORKER.md"],
                "dispatch_evidence": {
                    "worktree_path": "/nonexistent/reaped/wt",
                    "head_commit": head,
                    // NO work_base_commit — the capture hook did not fire.
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.status,
            VerdictStatus::Uncertain,
            "a missing work_base must be Uncertain, never a silent diff against the \
             stale dispatch baseline"
        );
        assert!(record.degraded_reason.is_some());
    }

    /// Fix (#523), acceptance (2): a goal that lands TWO commits has BOTH files in
    /// the diff — `work_base..head` spans the whole chain. This is the case the
    /// rejected `head^..head` shortcut would undercount to only the last commit.
    #[tokio::test]
    async fn verifier_two_commit_goal_shows_both_files() {
        let repo = tempfile::tempdir().unwrap();
        let dispatch_baseline = init_repo(repo.path());
        // The foreign commit the worker fast-forwards onto = its true base.
        std::fs::write(repo.path().join("OTHER.md"), "other\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m other",
        );
        let work_base = sh(repo.path(), "git rev-parse HEAD");
        // The worker makes TWO commits.
        std::fs::write(repo.path().join("FIRST.md"), "first\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m first",
        );
        std::fs::write(repo.path().join("SECOND.md"), "second\n").unwrap();
        sh(repo.path(), "git add -A");
        sh(
            repo.path(),
            "git -c user.email=t@t -c user.name=t commit -q -m second",
        );
        let head = sh(repo.path(), "git rev-parse --short HEAD");

        let pool = test_pool().await;
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": dispatch_baseline,
                "declared_paths": ["*.md"],
                "dispatch_evidence": {
                    "worktree_path": "/nonexistent/reaped/wt",
                    "head_commit": head,
                    "work_base_commit": work_base,
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_eq!(
            record.evidence_digest.diff.files_changed, 2,
            "both worker commits must be in range; diff_stat={:?}",
            record.diff_stat
        );
        assert!(
            record.diff_stat.contains("FIRST.md") && record.diff_stat.contains("SECOND.md"),
            "both worker files must appear: {:?}",
            record.diff_stat
        );
        assert!(
            !record.diff_stat.contains("OTHER.md"),
            "the foreign base commit's file must NOT appear: {:?}",
            record.diff_stat
        );
    }

    /// Fix A (#505): when the diff git command itself FAILS (here: an unreachable
    /// baseline), the verdict is ERRORED (Uncertain) with a degraded reason —
    /// NEVER a false zero-diff that the no-op clamp would turn into a wrongful
    /// Fail of correct work.
    #[tokio::test]
    async fn verifier_git_failure_is_uncertain_not_false_fail() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let wt_parent = tempfile::tempdir().unwrap();
        let wt = wt_parent.path().join("wt");
        sh(
            repo.path(),
            &format!("git worktree add --detach {} {}", wt.display(), baseline),
        );
        std::fs::write(wt.join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        sh(&wt, "git add -A");
        sh(
            &wt,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );
        let _head = sh(&wt, "git rev-parse --short HEAD");

        let pool = test_pool().await;
        // Fix (#523): with a real work_base and a bogus HEAD, `git diff <base>
        // <bogus>` ERRORS — the failure path we assert must surface as ERRORED
        // (Uncertain), never a false Fail. (work_base present, so this exercises
        // the git-failure path, not the missing-hook path.)
        let goal = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "dispatch_evidence": {
                    "worktree_path": wt.to_str().unwrap(),
                    "head_commit": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                    "work_base_commit": baseline,
                },
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();

        assert_ne!(
            record.status,
            VerdictStatus::Fail,
            "a git failure must never produce a (false) Fail"
        );
        assert_eq!(record.status, VerdictStatus::Uncertain);
        assert!(
            record.degraded_reason.is_some(),
            "the git failure must be surfaced as a degraded reason"
        );
    }

    /// Wire 4 negative: a FAIL verdict leaves the approve_review decision
    /// open and the goal in Review — auto-approval only fires on pass.
    #[tokio::test]
    async fn verifier_fail_leaves_review_decision_open() {
        use permagent::decisions::{self, NewDecision};

        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());

        let pool = test_pool().await;
        let goal = make_review_goal_in_columns(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "exit 1", "timeout_secs": 30}
                ],
            }),
        )
        .await;
        let d = decisions::create_decision(
            &pool,
            NewDecision {
                kind: "approve_review".to_string(),
                goal_id: Some(goal.id.clone()),
                project_id: Some(goal.project_id.clone()),
                headline: Some("Review the finished work on the failing goal".to_string()),
                detail: Some("worker reported success".to_string()),
                payload: serde_json::json!({}),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();
        assert_eq!(record.status, VerdictStatus::Fail);

        let d = decisions::get_decision(&pool, &d.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.status, "open");
        assert!(d.acted_by.is_none());
        assert_eq!(state_of(&pool, &goal.id).await, "review");
    }

    #[tokio::test]
    async fn model_pass_but_failed_check_clamps_to_fail() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "exit 1", "timeout_secs": 30}
                ]
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &card.id, &base_url).await.unwrap();

        // Model said PASS on everything; machine check failed → fail.
        assert_eq!(record.status, VerdictStatus::Fail);
        assert_eq!(record.rubric.checks_support, Grade::Fail);
    }

    #[tokio::test]
    async fn ollama_down_degrades_to_uncertain() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "true", "timeout_secs": 30}
                ]
            }),
        )
        .await;

        // Nothing listens on port 1.
        let record = run_for_goal_with(&pool, &card.id, "http://127.0.0.1:1")
            .await
            .unwrap();
        assert_eq!(record.status, VerdictStatus::Uncertain);
        assert!(record.degraded_reason.is_some());
        assert!(record
            .evidence_digest
            .verifier_summary
            .contains("could not complete"));
    }

    #[tokio::test]
    async fn missing_baseline_makes_path_discipline_uncertain_never_pass() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "true", "timeout_secs": 30}
                ]
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &card.id, &base_url).await.unwrap();

        assert_eq!(record.rubric.path_discipline, Grade::Uncertain);
        assert_eq!(record.status, VerdictStatus::Uncertain);
        assert!(record
            .degraded_reason
            .as_deref()
            .unwrap()
            .contains("baseline"));
    }

    #[tokio::test]
    async fn out_of_path_change_fails_path_discipline() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        // Out-of-path change (README.md not under src/**).
        std::fs::write(repo.path().join("README.md"), "modified\n").unwrap();

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &card.id, &base_url).await.unwrap();

        assert_eq!(record.out_of_path_files, vec!["README.md".to_string()]);
        assert_eq!(record.rubric.path_discipline, Grade::Fail);
        assert_eq!(record.status, VerdictStatus::Fail);
    }

    #[tokio::test]
    async fn untracked_files_count_as_changes() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        std::fs::write(repo.path().join("brand_new.txt"), "new\n").unwrap();

        let analysis = analyze_diff(
            Some(repo.path()),
            Some(&baseline),
            None,
            None,
            &["src/**".to_string()],
        )
        .await;
        assert_eq!(
            analysis.out_of_path_files,
            vec!["brand_new.txt".to_string()]
        );
        assert_eq!(analysis.path_discipline, Grade::Fail);
    }

    #[tokio::test]
    async fn no_declared_paths_is_uncertain() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        let analysis = analyze_diff(Some(repo.path()), Some(&baseline), None, None, &[]).await;
        assert_eq!(analysis.path_discipline, Grade::Uncertain);
        assert!(analysis
            .degraded_note
            .as_deref()
            .unwrap()
            .contains("no declared_paths"));
    }

    #[tokio::test]
    async fn non_git_working_dir_is_uncertain() {
        let dir = tempfile::tempdir().unwrap();
        let analysis = analyze_diff(
            Some(dir.path()),
            Some("abc123"),
            None,
            None,
            &["**".to_string()],
        )
        .await;
        assert_eq!(analysis.path_discipline, Grade::Uncertain);
    }

    #[tokio::test]
    async fn malformed_completion_checks_recorded_as_error_and_clamps() {
        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["**"],
                "completion_checks": [{"type": "not_a_real_check"}]
            }),
        )
        .await;

        let (base_url, _h) = spawn_mock_ollama(MockMode::Respond(GOOD_PASS.to_string())).await;
        let record = run_for_goal_with(&pool, &card.id, &base_url).await.unwrap();

        assert_eq!(record.check_results.len(), 1);
        assert_eq!(record.check_results[0].status, CheckStatus::Error);
        // Error never counts as pass; clamps the verdict.
        assert_eq!(record.status, VerdictStatus::Fail);
    }

    #[tokio::test]
    async fn non_goal_card_is_rejected() {
        let pool = test_pool().await;
        let card = permagent::cards::create_card(
            &pool,
            permagent::cards::CreateCard {
                project_id: permagent::projects::PERSONAL_PROJECT_ID.to_string(),
                title: "Standard".to_string(),
                description: None,
                card_type: Some("standard".to_string()),
                column_id: None,
                created_by: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let err = run_for_goal_with(&pool, &card.id, "http://127.0.0.1:1").await;
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("not a goal"));
    }

    /// REAL RUN against local Ollama (qwen2.5:7b). Ignored by default.
    /// Run with:
    ///   cargo test -p permagent-daemon --lib verification::tests::real_run \
    ///     -- --ignored --nocapture
    /// Self-isolating: sets PERMAGENT_PATH_ROOT to a throwaway tempdir so the
    /// live data root is never read or written. Uses a throwaway git repo +
    /// in-memory DB; only network call is http://localhost:11434 (zero cloud
    /// tokens).
    #[tokio::test]
    #[ignore = "requires local Ollama with qwen2.5:7b pulled"]
    async fn real_run_local_ollama_toy_goal() {
        // Isolate config/data reads from the live data root.
        let throwaway_root = tempfile::tempdir().unwrap();
        std::env::set_var("PERMAGENT_PATH_ROOT", throwaway_root.path());

        let repo = tempfile::tempdir().unwrap();
        let baseline = init_repo(repo.path());
        // Do the "work": add function b, in-path.
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn a() {}\npub fn b() {}\n",
        )
        .unwrap();

        let pool = test_pool().await;
        let card = make_goal(
            &pool,
            repo.path().to_str().unwrap(),
            serde_json::json!({
                "baseline_commit": baseline,
                "declared_paths": ["src/**"],
                "completion_checks": [
                    {"type": "command_exit_zero", "cmd": "grep -q 'pub fn b' src/lib.rs", "timeout_secs": 30},
                    {"type": "file_exists", "path": "src/lib.rs"}
                ],
                "claimed_evidence": "Added pub fn b() to src/lib.rs as requested; grep check passes."
            }),
        )
        .await;

        let cfg = verifier::load_config();
        println!("VERIFIER MODEL: {}", cfg.model);
        println!("OLLAMA BASE: {}", verifier::OLLAMA_BASE_URL);

        let record = run_for_goal(&pool, &card.id).await.unwrap();
        println!(
            "VERIFICATION RECORD:\n{}",
            serde_json::to_string_pretty(&record).unwrap()
        );

        assert!(record
            .check_results
            .iter()
            .all(|r| r.status == CheckStatus::Pass));
        assert!(
            record.degraded_reason.is_none(),
            "verifier degraded: {:?}",
            record.degraded_reason
        );
        assert!(record.grades_present(), "model grades should be present");
    }

    /// Defensive (#531): an inverted `work_base..head` range — where the recorded
    /// head is an ANCESTOR of the base (the exact multi-commit false-fail: base
    /// captured at VERIFY_E, head mis-captured as an older ancestor) — must record
    /// Uncertain, NOT compute a confident reverse diff. The reverse diff would show
    /// the worker's additions as deletions and false-fail correct work.
    #[tokio::test]
    async fn analyze_diff_rejects_inverted_range_as_uncertain() {
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        sh(dir, "git init -q -b main");
        std::fs::write(dir.join("a.txt"), "c\n").unwrap();
        sh(dir, "git add -A");
        sh(dir, "git -c user.email=t@t -c user.name=t commit -q -m c");
        let ancestor = sh(dir, "git rev-parse HEAD"); // older commit (the mis-captured head)
        std::fs::write(dir.join("a.txt"), "c\nd\n").unwrap();
        sh(dir, "git add -A");
        sh(dir, "git -c user.email=t@t -c user.name=t commit -q -m d");
        std::fs::write(dir.join("a.txt"), "c\nd\ne\n").unwrap();
        sh(dir, "git add -A");
        sh(dir, "git -c user.email=t@t -c user.name=t commit -q -m e");
        let base = sh(dir, "git rev-parse HEAD"); // the correct work_base (VERIFY_E shape)

        // head=ancestor is an ancestor of base → inverted range.
        let analysis = analyze_diff(
            Some(dir),
            Some(&base), // dispatch baseline (unused on the head path)
            Some(&ancestor),
            Some(&base),
            &["**".to_string()],
        )
        .await;

        assert_eq!(
            analysis.path_discipline,
            Grade::Uncertain,
            "an inverted range must never produce a confident grade"
        );
        assert_eq!(
            analysis.diff_summary.files_changed, 0,
            "no reverse diff should be computed"
        );
        assert!(
            analysis
                .degraded_note
                .as_deref()
                .unwrap()
                .contains("inverted diff range"),
            "the Uncertain reason must name the inverted range, got: {:?}",
            analysis.degraded_note
        );
    }

    /// The positive control for the #531 guard: a NORMAL range (base is the parent
    /// of head) diffs cleanly and surfaces the worker's added file — the guard only
    /// trips on inversion, never on a valid forward range.
    #[tokio::test]
    async fn analyze_diff_accepts_normal_forward_range() {
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        sh(dir, "git init -q -b main");
        std::fs::write(dir.join("a.txt"), "base\n").unwrap();
        sh(dir, "git add -A");
        sh(
            dir,
            "git -c user.email=t@t -c user.name=t commit -q -m base",
        );
        let base = sh(dir, "git rev-parse HEAD");
        std::fs::write(dir.join("b.txt"), "work\n").unwrap();
        sh(dir, "git add -A");
        sh(
            dir,
            "git -c user.email=t@t -c user.name=t commit -q -m work",
        );
        let head = sh(dir, "git rev-parse HEAD");

        let analysis = analyze_diff(
            Some(dir),
            Some(&base),
            Some(&head),
            Some(&base),
            &["**".to_string()],
        )
        .await;

        assert_eq!(
            analysis.diff_summary.files_changed, 1,
            "the forward range must surface the worker's one added file"
        );
        assert!(
            analysis.degraded_note.is_none(),
            "a valid forward range is not degraded, got: {:?}",
            analysis.degraded_note
        );
    }
}

#[cfg(test)]
impl VerificationRecord {
    /// Test helper: degraded runs have an empty rationale.
    fn grades_present(&self) -> bool {
        !self.rationale.is_empty()
    }
}
