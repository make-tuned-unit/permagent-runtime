//! Verification pipeline for completed goals (Decision Inbox, Lane L2).
//!
//! After a goal worker finishes and L1's `handle_goal_completion` moves the
//! card to Review, `run_for_goal` executes the goal's declared completion
//! checks, analyzes the diff against the dispatch-time baseline commit, grades
//! the work with a local Ollama model, and writes the result to
//! `metadata_json.verification` via L1's narrow allowlist API
//! `cards::set_goal_verification` (the sanctioned L2 write path).
//!
//! L1 contract (PHASE0-L2.md §4):
//! - This module writes ONLY the `verification` key in metadata_json.
//! - It NEVER calls `move_card` and never touches `goal_state`,
//!   `attempt_count`, or `review_notes`.
//! - `baseline_commit` is consumed from goal metadata (L1 writes it at
//!   dispatch); absent baseline / git failure / no declared_paths ⇒
//!   path_discipline = uncertain, never a silent pass.
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

    // Working dir: exactly as dispatch resolves it — project.root_path.
    let project = permagent::projects::get_project(pool, &card.project_id).await?;
    let working_dir: Option<PathBuf> = project
        .as_ref()
        .and_then(|p| p.root_path.as_ref())
        .map(PathBuf::from)
        .filter(|p| p.is_dir());

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
        &declared_paths,
    )
    .await;

    // ── 3. Verifier (local Ollama, deterministic aggregation in Rust) ──
    let cfg = verifier::load_config();
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

    // ── 6. Tier-1 auto-approval (L3 policy × this verdict) ──
    // On a PASS verdict, Henry answers the goal's open approve_review
    // decision as 'henry-policy' through L1's tier-gated answer path
    // (Tier-2 is Forbidden there and nothing moves). Failure-tolerant:
    // the verification record stands regardless.
    if record.status == VerdictStatus::Pass {
        henry_approve_after_pass(pool, goal_id, &record).await;
    }

    Ok(record)
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

/// Analyze changes since `baseline_commit` and grade declared-path discipline.
/// Out-of-path = union of `git diff --name-only <baseline>` and
/// `git status --porcelain` paths not matching any declared glob.
pub async fn analyze_diff(
    working_dir: Option<&Path>,
    baseline_commit: Option<&str>,
    declared_paths: &[String],
) -> GitAnalysis {
    let Some(wd) = working_dir else {
        return uncertain_analysis("no working_dir");
    };
    let Some(baseline) = baseline_commit else {
        return uncertain_analysis("no baseline_commit recorded at dispatch");
    };

    // Committed + uncommitted tracked changes since baseline.
    let names = match git_output(wd, &["diff", "--name-only", baseline]).await {
        Ok(o) => o,
        Err(e) => return uncertain_analysis(&format!("git diff failed: {}", e)),
    };
    let numstat = match git_output(wd, &["diff", "--numstat", baseline]).await {
        Ok(o) => o,
        Err(e) => return uncertain_analysis(&format!("git numstat failed: {}", e)),
    };
    let porcelain = match git_output(wd, &["status", "--porcelain"]).await {
        Ok(o) => o,
        Err(e) => return uncertain_analysis(&format!("git status failed: {}", e)),
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
    let diff_stat = format!(
        "{} file(s) changed since {}: +{} -{}\n{}",
        changed.len(),
        baseline,
        insertions,
        deletions,
        changed.join("\n")
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
        let record = run_for_goal_with(&pool, &goal.id, &base_url).await.unwrap();
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

        let analysis =
            analyze_diff(Some(repo.path()), Some(&baseline), &["src/**".to_string()]).await;
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
        let analysis = analyze_diff(Some(repo.path()), Some(&baseline), &[]).await;
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
        let analysis = analyze_diff(Some(dir.path()), Some("abc123"), &["**".to_string()]).await;
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
}

#[cfg(test)]
impl VerificationRecord {
    /// Test helper: degraded runs have an empty rationale.
    fn grades_present(&self) -> bool {
        !self.rationale.is_empty()
    }
}
