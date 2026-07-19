//! The playbook synthesis worker — a periodic, project-scoped, local-first pass
//! that distills recent answered decisions + corrections into playbook hints.
//!
//! It mirrors the initiative driver ([`crate::initiative::driver`]): a plain
//! Rust interval loop (NOT a scheduler recipe — that would spend agent tokens
//! per run), a cheap LOCAL distiller with graceful degrade, and an in-memory
//! per-project signature cache so an unchanged decision set is not re-distilled
//! every pass (cold start re-accumulates; idempotent keys make that harmless).
//! Like the Librarian it runs in the background, non-blocking, and never
//! crashes the daemon: every failure is logged and swallowed.
//!
//! GATING — [`super::is_enabled`] (`PERMAGENT_PLAYBOOK_ENABLED`), default OFF.
//! [`spawn`] logs the on/off state and returns without starting a task when the
//! flag is unset, so an off daemon does exactly nothing here.
//!
//! LOCAL-FIRST / DEGRADE GRACEFULLY — distillation uses a cheap local model
//! (default `ollama`/`qwen2.5:7b`, mirroring the initiative + decomposition
//! routing). When no distiller is configured or reachable the pass is SKIPPED,
//! never fabricated — a template can't honestly generalize a preference, and
//! hints must never be invented.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use sqlx::{Pool, Sqlite};

use crate::brain_handle::SafeBrain;
use crate::config::Config;
use crate::decisions::Decision;
use crate::providers::base::Provider;

/// Config: seconds between synthesis passes. Long by default — distillation is a
/// background nicety, not a hot path, and each pass may spend a cheap local
/// model call per project that has fresh decisions.
pub const PLAYBOOK_TICK_SECS_KEY: &str = "playbook_tick_secs";
const DEFAULT_TICK_SECS: u64 = 6 * 60 * 60; // 6h

/// Cheap local distiller routing (mirrors `ORCHESTRATOR_DECOMPOSITION_*` /
/// `INITIATIVE_DRAFT_*`). Empty string = explicit opt-out → synthesis is
/// skipped (never fabricated).
pub const PLAYBOOK_DISTILL_PROVIDER_KEY: &str = "PLAYBOOK_DISTILL_PROVIDER";
pub const PLAYBOOK_DISTILL_MODEL_KEY: &str = "PLAYBOOK_DISTILL_MODEL";
const DEFAULT_DISTILL_PROVIDER: &str = "ollama";
const DEFAULT_DISTILL_MODEL: &str = "qwen2.5:7b";

/// Recent resolved decisions to scan per pass (across all projects, newest
/// first). Bounds the query; older signal ages out of the window.
const DECISION_SCAN_LIMIT: i64 = 200;
/// A project needs at least this many jesse-answered decisions before a
/// "tendency" is worth distilling — one decision is not a pattern.
const MIN_DECISIONS_FOR_SYNTHESIS: usize = 2;
/// Cap decisions fed to the distiller per project (bounds the prompt).
const MAX_DECISIONS_PER_PROJECT: usize = 20;
/// Cap hints stored per project per pass.
const MAX_HINTS_PER_PROJECT: usize = 4;
/// Drop an over-long "hint" — a distiller that rambled is not producing a hint.
const MAX_HINT_CHARS: usize = 400;

/// The project-key sentinel for decisions with no project (mirrors
/// [`crate::decision_inbox::learn`]'s `"personal"` slug fallback).
const PERSONAL_KEY: &str = "personal";

/// A synthetic session id for the distiller's provider call.
const DISTILL_SESSION_ID: &str = "playbook-synthesis";

const DISTILL_SYSTEM: &str = "You distill a user's (Jesse's) past product and engineering decisions \
    into a few short, reusable \"playbook\" hints about how he tends to decide, so an assistant can \
    plan more like he would.\n\n\
    You are given numbered items — each a past decision he answered, or a draft of yours he revised \
    before accepting. Output ONLY a JSON array of 0 to 4 objects, no prose and no markdown fences:\n\
    [{\"hint\": \"<one sentence>\", \"from\": [<item numbers>]}]\n\n\
    Rules:\n\
    - Each hint is ONE sentence describing a TENDENCY or PREFERENCE, phrased as a soft \
    generalization (\"tends to\", \"prefers\", \"usually\"), naming him as Jesse.\n\
    - \"from\" lists the item numbers the hint is grounded in; ground each hint in at least two items.\n\
    - Only emit a hint that genuinely generalizes across the items. If nothing does, output [].\n\
    - Never invent a preference the items do not support. These are HINTS, not rules.";

/// Spawn the synthesis worker if enabled. Called once at daemon startup with the
/// app DB pool. Never silent: logs the on/off state either way (the #360
/// "never silent" contract).
pub fn spawn(pool: Pool<Sqlite>) {
    if !super::is_enabled() {
        tracing::info!(
            target: "playbook",
            "playbook synthesis worker OFF ({}=unset) — set it to enable decision-playbook distillation",
            super::PLAYBOOK_ENABLED_ENV
        );
        return;
    }
    let tick_secs = Config::global()
        .get_param::<u64>(PLAYBOOK_TICK_SECS_KEY)
        .unwrap_or(DEFAULT_TICK_SECS)
        .max(60);
    tracing::info!(
        target: "playbook",
        tick_secs,
        "playbook synthesis worker ON — distilling answered decisions into hints"
    );
    tokio::spawn(run_worker(pool, tick_secs));
}

async fn run_worker(pool: Pool<Sqlite>, tick_secs: u64) {
    let mut interval = tokio::time::interval(Duration::from_secs(tick_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Per-project signature of the last-synthesized decision set: skip a project
    // whose decisions are unchanged since the previous pass. In-memory only — a
    // cold start re-accumulates and re-distills once (idempotent keys make that
    // a no-op write).
    let mut last_sig: HashMap<String, u64> = HashMap::new();

    loop {
        interval.tick().await;
        let Some(brain) = crate::agents::platform_extensions::get_global_brain() else {
            tracing::debug!(target: "playbook", "no Brain available — skipping synthesis pass");
            continue;
        };
        if let Err(e) = run_pass(&pool, &brain, &mut last_sig).await {
            tracing::warn!(target: "playbook", "synthesis pass failed: {e}");
        }
    }
}

/// One synthesis pass: scan recent answered decisions, group by project, and
/// (for each project with fresh signal) distill + store hints. Resolves the
/// cheap distiller once; when none is available the whole pass is skipped.
async fn run_pass(
    pool: &Pool<Sqlite>,
    brain: &SafeBrain,
    last_sig: &mut HashMap<String, u64>,
) -> anyhow::Result<()> {
    let history = crate::decisions::decision_history(pool, DECISION_SCAN_LIMIT, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let groups = group_answered_by_project(history.into_iter().map(|h| h.decision).collect());
    if groups.is_empty() {
        return Ok(());
    }

    // Any project with enough decisions AND fresh signal is a candidate; only
    // pay for a distiller if at least one candidate exists.
    let has_candidate = groups.iter().any(|(key, decisions)| {
        decisions.len() >= MIN_DECISIONS_FOR_SYNTHESIS
            && last_sig.get(key) != Some(&project_signature(decisions))
    });
    if !has_candidate {
        return Ok(());
    }

    let provider = match resolve_distill_provider().await {
        Some(p) => p,
        None => {
            tracing::info!(
                target: "playbook",
                "no local distiller configured/reachable — synthesis skipped this pass (never fabricated)"
            );
            return Ok(());
        }
    };

    for (project_key, decisions) in groups {
        if decisions.len() < MIN_DECISIONS_FOR_SYNTHESIS {
            continue;
        }
        let sig = project_signature(&decisions);
        if last_sig.get(&project_key) == Some(&sig) {
            continue; // unchanged since last pass
        }
        let slug = resolve_slug(pool, &project_key).await;
        match synthesize_project(brain, &provider, &slug, &decisions).await {
            Ok(n) => {
                tracing::info!(
                    target: "playbook",
                    project = %slug,
                    hints = n,
                    "distilled playbook hints from answered decisions"
                );
                // Only mark clean once the pass for this project completed, so a
                // failed pass retries next tick.
                last_sig.insert(project_key, sig);
            }
            Err(e) => {
                tracing::warn!(target: "playbook", project = %slug, "synthesis failed: {e}");
            }
        }
    }
    Ok(())
}

/// Distill + store hints for one project. Builds numbered refs from the
/// decisions, asks the cheap distiller for grounded hints, and stores each with
/// its provenance. Returns how many hints were stored.
async fn synthesize_project(
    brain: &SafeBrain,
    provider: &Arc<dyn Provider>,
    slug: &str,
    decisions: &[Decision],
) -> anyhow::Result<usize> {
    let refs = build_refs(decisions);
    let user_text = build_distill_input(slug, &refs);
    let user_msg = crate::conversation::message::Message::user().with_text(user_text);

    let (response, _usage) = provider
        .complete_fast(
            DISTILL_SESSION_ID,
            DISTILL_SYSTEM,
            std::slice::from_ref(&user_msg),
            &[],
        )
        .await?;
    let hints = parse_hints(&response.as_concat_text(), &refs);

    let mut stored = 0usize;
    for h in hints.into_iter().take(MAX_HINTS_PER_PROJECT) {
        let entry = super::PlaybookHint {
            project_slug: slug,
            wing: slug,
            hint: &h.hint,
            provenance: &h.provenance,
        };
        match super::ingest_playbook_hint(brain, &entry).await {
            Ok(_) => stored += 1,
            Err(e) => tracing::warn!(target: "playbook", "failed to store hint: {e}"),
        }
    }
    Ok(stored)
}

/// Resolve the cheap local distiller (mirrors the initiative/decomposition
/// routing). Empty config = explicit opt-out; an unavailable provider = skip.
/// Either way returns None and the caller skips synthesis — never fabricates.
async fn resolve_distill_provider() -> Option<Arc<dyn Provider>> {
    let config = Config::global();
    let provider_name = config
        .get_param::<String>(PLAYBOOK_DISTILL_PROVIDER_KEY)
        .unwrap_or_else(|_| DEFAULT_DISTILL_PROVIDER.to_string());
    let model_name = config
        .get_param::<String>(PLAYBOOK_DISTILL_MODEL_KEY)
        .unwrap_or_else(|_| DEFAULT_DISTILL_MODEL.to_string());

    if provider_name.trim().is_empty() || model_name.trim().is_empty() {
        return None;
    }
    match crate::providers::create_with_named_model(&provider_name, &model_name, Vec::new()).await {
        Ok(provider) => Some(provider),
        Err(e) => {
            tracing::debug!(
                target: "playbook",
                "distiller {provider_name}/{model_name} unavailable ({e}); synthesis skipped"
            );
            None
        }
    }
}

/// Resolve a project key to its wing slug. The personal sentinel and any
/// unresolvable id fall back to `"personal"` (mirrors the learn module).
async fn resolve_slug(pool: &Pool<Sqlite>, project_key: &str) -> String {
    if project_key == PERSONAL_KEY {
        return "personal".to_string();
    }
    match crate::projects::get_project_by_id_or_slug(pool, project_key).await {
        Ok(Some(p)) => p.slug,
        _ => "personal".to_string(),
    }
}

// ── Pure helpers (unit-testable without a runtime) ───────────────────────────

/// A numbered reference fed to the distiller: its item number, the source
/// decision id (for provenance), and the retrieval-shaped prose (reusing the
/// learn module's decision/correction prose).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ref {
    num: usize,
    decision_id: String,
    prose: String,
}

/// Build numbered refs from a project's decisions. A jesse edit is rendered as
/// its correction (draft → revision — the sharper "how he changes things"
/// signal); every other answered decision as its question/answer prose.
fn build_refs(decisions: &[Decision]) -> Vec<Ref> {
    decisions
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let prose = match crate::decision_inbox::learn::correction_delta(d) {
                Some((original, edited)) => {
                    crate::decision_inbox::learn::correction_memory_content(&original, &edited)
                }
                None => crate::decision_inbox::learn::decision_memory_content(
                    &d.headline,
                    &crate::decision_inbox::learn::answered_decision_answer_text(d),
                    d.answer_note.as_deref(),
                ),
            };
            Ref {
                num: i + 1,
                decision_id: d.id.clone(),
                prose,
            }
        })
        .collect()
}

/// Render the numbered distillation prompt for one project.
fn build_distill_input(project_slug: &str, refs: &[Ref]) -> String {
    let mut s = format!(
        "Past decisions and draft-revisions by Jesse for project \"{project_slug}\", each numbered:\n"
    );
    for r in refs {
        s.push_str(&format!("[{}] {}\n", r.num, r.prose));
    }
    s.push_str(
        "\nDistill 0 to 4 short hints about how Jesse tends to decide or what he prefers on this \
         project, each grounded in at least two of the numbered items. Respond with ONLY the JSON \
         array described in your instructions.",
    );
    s
}

/// A parsed, grounded hint ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedHint {
    hint: String,
    provenance: Vec<String>,
}

/// Parse the distiller's output into grounded hints. Resilient: tolerates
/// markdown fences / surrounding prose (takes the outermost `[ … ]`), silently
/// yields nothing on unparseable output, drops empty/over-long hints, and — the
/// grounding invariant — drops any hint whose `from` cites no real item (a hint
/// with no provenance is ungrounded and must never be stored).
fn parse_hints(model_output: &str, refs: &[Ref]) -> Vec<ParsedHint> {
    let Some(json) = extract_json_array(model_output) else {
        return Vec::new();
    };
    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in &items {
        let Some(hint) = item.get("hint").and_then(|v| v.as_str()) else {
            continue;
        };
        let hint = hint.trim();
        if hint.is_empty() || hint.chars().count() > MAX_HINT_CHARS {
            continue;
        }
        let from = item.get("from").and_then(|v| v.as_array());
        let mut provenance = Vec::new();
        if let Some(from) = from {
            let mut seen = HashSet::new();
            for f in from {
                let Some(n) = f.as_u64() else { continue };
                if let Some(r) = refs.iter().find(|r| r.num as u64 == n) {
                    if seen.insert(r.decision_id.clone()) {
                        provenance.push(format!("decision {}", r.decision_id));
                    }
                }
            }
        }
        // Grounding requirement: no valid provenance ⇒ drop.
        if provenance.is_empty() {
            continue;
        }
        out.push(ParsedHint {
            hint: hint.to_string(),
            provenance,
        });
    }
    out
}

/// Extract the outermost JSON array substring, tolerating fences and prose.
/// Uses `get` (not panicking index) so a malformed slice yields None, never a
/// panic. `[` and `]` are ASCII, so the byte range is always on char boundaries.
fn extract_json_array(s: &str) -> Option<String> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end <= start {
        return None;
    }
    s.get(start..=end).map(|slice| slice.to_string())
}

/// Dedup (by id, keeping the newest — history is newest-first), keep only
/// jesse-answered rows, group by project key (`project_id` or the personal
/// sentinel), preserving first-seen project order and capping decisions per
/// project. Pure so the whole selection contract is testable without a DB.
fn group_answered_by_project(decisions: Vec<Decision>) -> Vec<(String, Vec<Decision>)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<Decision>> = HashMap::new();

    for d in decisions {
        if d.status != "answered" || d.acted_by.as_deref() != Some(crate::decisions::ACTOR_JESSE) {
            continue;
        }
        if !seen.insert(d.id.clone()) {
            continue;
        }
        let key = d
            .project_id
            .clone()
            .unwrap_or_else(|| PERSONAL_KEY.to_string());
        let entry = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Vec::new()
        });
        if entry.len() < MAX_DECISIONS_PER_PROJECT {
            entry.push(d);
        }
    }

    order
        .into_iter()
        .map(|k| {
            let v = map.remove(&k).unwrap_or_default();
            (k, v)
        })
        .collect()
}

/// Stable FNV-1a signature of a project's decision set (id + resolved_at, so an
/// edited answer changes it), order-independent. Lets a pass skip a project
/// whose decisions are unchanged since the previous pass.
fn project_signature(decisions: &[Decision]) -> u64 {
    let mut parts: Vec<String> = decisions
        .iter()
        .map(|d| format!("{}={}", d.id, d.resolved_at.clone().unwrap_or_default()))
        .collect();
    parts.sort();
    let joined = parts.join("|");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered_row(id: &str, project: Option<&str>, actor: &str, status: &str) -> Decision {
        Decision {
            id: id.to_string(),
            kind: "choice".to_string(),
            goal_id: None,
            project_id: project.map(String::from),
            tier: 1,
            headline: format!("Question {id}"),
            detail: "detail".to_string(),
            payload: serde_json::json!({}),
            rank: None,
            status: status.to_string(),
            answer: Some("input".to_string()),
            answer_note: None,
            answer_choice_id: None,
            answer_input: Some(format!("answer {id}")),
            acted_by: Some(actor.to_string()),
            created_at: "2026-07-16T00:00:00.000Z".to_string(),
            resolved_at: Some("2026-07-16T00:00:01.000Z".to_string()),
        }
    }

    fn edit_row(id: &str, project: Option<&str>, draft: &str, edited: &str) -> Decision {
        let mut d = answered_row(id, project, crate::decisions::ACTOR_JESSE, "answered");
        d.answer = Some("edit".to_string());
        d.payload = serde_json::json!({ "draft": draft });
        d.answer_input = Some(edited.to_string());
        d
    }

    // ── grouping / selection ──

    #[test]
    fn group_dedups_filters_and_orders() {
        let input = vec![
            answered_row(
                "d1",
                Some("proj-a"),
                crate::decisions::ACTOR_JESSE,
                "answered",
            ),
            // duplicate id (a second audit row for d1) — collapsed.
            answered_row(
                "d1",
                Some("proj-a"),
                crate::decisions::ACTOR_JESSE,
                "answered",
            ),
            answered_row(
                "d2",
                Some("proj-b"),
                crate::decisions::ACTOR_JESSE,
                "answered",
            ),
            // henry-answered → excluded (only Jesse's decisions train the playbook).
            answered_row("d3", Some("proj-a"), "henry-policy", "answered"),
            // still open → excluded.
            answered_row("d4", Some("proj-a"), crate::decisions::ACTOR_JESSE, "open"),
            answered_row(
                "d5",
                Some("proj-a"),
                crate::decisions::ACTOR_JESSE,
                "answered",
            ),
            // no project → personal sentinel.
            answered_row("d6", None, crate::decisions::ACTOR_JESSE, "answered"),
        ];
        let groups = group_answered_by_project(input);
        let keys: Vec<&str> = groups.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec!["proj-a", "proj-b", PERSONAL_KEY],
            "first-seen order"
        );

        let proj_a = &groups[0].1;
        let ids: Vec<&str> = proj_a.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["d1", "d5"],
            "dedup d1, drop henry (d3) and open (d4)"
        );
        assert_eq!(
            groups[2].1.len(),
            1,
            "personal group has the project-less d6"
        );
    }

    #[test]
    fn group_caps_decisions_per_project() {
        let input: Vec<Decision> = (0..(MAX_DECISIONS_PER_PROJECT + 5))
            .map(|i| {
                answered_row(
                    &format!("d{i}"),
                    Some("proj-a"),
                    crate::decisions::ACTOR_JESSE,
                    "answered",
                )
            })
            .collect();
        let groups = group_answered_by_project(input);
        assert_eq!(groups[0].1.len(), MAX_DECISIONS_PER_PROJECT);
    }

    // ── refs / prompt ──

    #[test]
    fn build_refs_uses_correction_prose_for_edits() {
        let decisions = vec![
            answered_row("d1", Some("p"), crate::decisions::ACTOR_JESSE, "answered"),
            edit_row("d2", Some("p"), "send at 9am", "send at 8am sharp"),
        ];
        let refs = build_refs(&decisions);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].num, 1);
        assert_eq!(refs[0].decision_id, "d1");
        assert!(refs[0].prose.starts_with("Jesse was asked:"));
        // The edit renders as its draft→revision correction (the sharper signal).
        assert_eq!(refs[1].decision_id, "d2");
        assert!(refs[1].prose.starts_with("The agent drafted:"));
        assert!(refs[1].prose.contains("send at 8am sharp"));
    }

    #[test]
    fn distill_input_numbers_every_ref() {
        let refs = vec![
            Ref {
                num: 1,
                decision_id: "d1".into(),
                prose: "alpha".into(),
            },
            Ref {
                num: 2,
                decision_id: "d2".into(),
                prose: "beta".into(),
            },
        ];
        let input = build_distill_input("permagent", &refs);
        assert!(input.contains("project \"permagent\""));
        assert!(input.contains("[1] alpha"));
        assert!(input.contains("[2] beta"));
        assert!(input.contains("ONLY the JSON array"));
    }

    // ── hint parsing (the risky, model-facing seam) ──

    fn refs3() -> Vec<Ref> {
        (1..=3)
            .map(|n| Ref {
                num: n,
                decision_id: format!("d{n}"),
                prose: format!("item {n}"),
            })
            .collect()
    }

    #[test]
    fn parse_hints_maps_refs_to_provenance() {
        let out = r#"[{"hint":"Jesse prefers Postgres","from":[1,3]}]"#;
        let hints = parse_hints(out, &refs3());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].hint, "Jesse prefers Postgres");
        assert_eq!(
            hints[0].provenance,
            vec!["decision d1".to_string(), "decision d3".to_string()]
        );
    }

    #[test]
    fn parse_hints_tolerates_fences_and_prose() {
        let out = "Sure! Here you go:\n```json\n[{\"hint\":\"Jesse ships small\",\"from\":[2]}]\n```\nHope that helps.";
        let hints = parse_hints(out, &refs3());
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].hint, "Jesse ships small");
        assert_eq!(hints[0].provenance, vec!["decision d2".to_string()]);
    }

    #[test]
    fn parse_hints_drops_ungrounded_and_bad_refs() {
        // No "from" → ungrounded → dropped. Out-of-range ref → no provenance → dropped.
        let out = r#"[
            {"hint":"ungrounded claim"},
            {"hint":"cites nothing real","from":[99]},
            {"hint":"","from":[1]},
            {"hint":"Jesse prefers explicit config","from":[1,2]}
        ]"#;
        let hints = parse_hints(out, &refs3());
        assert_eq!(hints.len(), 1, "only the grounded, non-empty hint survives");
        assert_eq!(hints[0].hint, "Jesse prefers explicit config");
    }

    #[test]
    fn parse_hints_empty_on_garbage_or_empty_array() {
        assert!(parse_hints("not json at all", &refs3()).is_empty());
        assert!(parse_hints("[]", &refs3()).is_empty());
        assert!(parse_hints("", &refs3()).is_empty());
    }

    #[test]
    fn parse_hints_drops_overlong_hint() {
        let long = "x".repeat(MAX_HINT_CHARS + 1);
        let out = format!(r#"[{{"hint":"{long}","from":[1]}}]"#);
        assert!(parse_hints(&out, &refs3()).is_empty());
    }

    // ── signature ──

    #[test]
    fn signature_is_order_independent_and_change_sensitive() {
        let a = answered_row("d1", Some("p"), crate::decisions::ACTOR_JESSE, "answered");
        let b = answered_row("d2", Some("p"), crate::decisions::ACTOR_JESSE, "answered");
        let s1 = project_signature(&[a.clone(), b.clone()]);
        let s2 = project_signature(&[b.clone(), a.clone()]);
        assert_eq!(s1, s2, "order-independent");

        // A new decision changes the signature (triggers re-synthesis).
        let c = answered_row("d3", Some("p"), crate::decisions::ACTOR_JESSE, "answered");
        assert_ne!(s1, project_signature(&[a.clone(), b.clone(), c]));

        // An edited answer (new resolved_at) changes it too.
        let mut a_edited = a.clone();
        a_edited.resolved_at = Some("2026-07-17T00:00:00.000Z".to_string());
        assert_ne!(s1, project_signature(&[a_edited, b]));
    }
}
