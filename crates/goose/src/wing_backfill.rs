//! One-shot, dry-run-first survey of chat memories that could be winged in
//! hindsight — and are only assigned where two independent signals agree.
//!
//! # What this is for
//!
//! [`crate::session_wing`] fixes the chat writer going FORWARD: from now on a
//! session records the project the UI had open, and each turn earns that wing
//! by corroborating it. This module asks the same question of the turns already
//! written, where the session hint was never captured.
//!
//! # Why it is deliberately stingy
//!
//! For a past turn there is no captured hint, so it has to be reconstructed
//! from `project_selected` activity records — "a project was selected within
//! two hours before this session's first turn". Spectral measured that
//! reconstruction on the live brain and it is **not** trustworthy on its own:
//! of the 334 turns it would assign, the turn's own content names the SAME
//! project in 31, a DIFFERENT project in 114, and no project at all in 189.
//! 21% verified precision.
//!
//! So the two signals are required to AGREE:
//!
//! 1. a reconstructable project hint for the turn's session (the captured
//!    `sessions.project_hint_wing` when the session has one, else the ≤2h
//!    `project_selected` reconstruction), and
//! 2. corroboration from the memory's own content, by the same
//!    [`crate::session_wing::WingCorroborator`] the write path uses.
//!
//! Using one predicate for both paths is the point: a repair that classifies
//! turns differently from the writer makes the corpus measurement meaningless.
//!
//! Expect roughly 31 rows on the live brain. That is a small number on purpose.
//! The alternative — assigning all 334 — would put 114 memories in a wing their
//! own content contradicts, and a wrong wing is invisible while an empty one is
//! not.
//!
//! # Buckets
//!
//! Every scanned turn lands in exactly one bucket, and all four are reported:
//!
//! | bucket | meaning | written |
//! |---|---|---|
//! | `corroborated` | hint reconstructed AND content agrees | yes, when `apply` |
//! | `conflicting` | content names a DIFFERENT project than the hint | never |
//! | `unverifiable` | hint reconstructed, content names no project | never |
//! | `no_hint` | no project could be reconstructed for the session at all | never |
//!
//! # Safety
//!
//! `apply: false` is the default and changes nothing. It is a read-only survey
//! and is the shape you should run first, every time.

use crate::session_wing::{ProjectHint, WingCorroborator, WingVerdict};
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use std::collections::BTreeMap;
use std::path::Path;

/// How stale a `project_selected` record may be and still be taken as the
/// session's project. Spectral's bound, and the reason it is only ONE of two
/// required signals: even at two hours the UI context and the conversation
/// subject disagree 79% of the time.
pub const HINT_WINDOW_SECONDS: i64 = 2 * 60 * 60;

/// Cap on the corroborated examples carried back in the report. A survey is for
/// judging the decision rule, not for dumping the corpus.
const MAX_SAMPLES: usize = 25;

/// The wing a memory sits in when nothing classified it.
const CATCH_ALL_WING: &str = "general";

/// Why `apply: true` cannot currently write.
///
/// Spectral's public API exposes [`spectral::Brain::set_hall`] for moving one
/// memory's hall — which re-hashes the constellation fingerprints the memory
/// participates in — but there is no `set_wing` counterpart at the pinned rev,
/// and `reclassify_wings_in` re-runs the CLASSIFIER over a whole wing rather
/// than writing a decision the caller has already made per row.
///
/// The two ways to write a wing without that API are both wrong here: a raw
/// `UPDATE memories SET wing` moves the column and leaves the routing index
/// behind (the wing is part of the TACT fingerprint), and forget-then-remember
/// destroys the memory's signal and associations to change one field.
///
/// So this op surveys honestly and refuses to write, rather than writing badly.
/// The ask upstream is small and specific: `Brain::set_wing(id, wing)`,
/// mirroring `set_hall`.
pub const APPLY_UNSUPPORTED: &str =
    "apply is not supported at the pinned Spectral rev: there is no per-memory \
     `Brain::set_wing` (only `set_hall`), and writing the column directly would \
     leave the TACT constellation fingerprints stale. The survey is complete and \
     accurate; the write needs `Brain::set_wing(id, wing)` upstream.";

/// One corroborated turn, carried back so a human can check the rule rather
/// than trust the count.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillSample {
    pub memory_key: String,
    pub session_id: String,
    pub wing: String,
    /// `content-name` | `alias` | `tool-path`.
    pub corroborated_by: String,
    /// First 160 characters of the memory, for eyeballing the decision.
    pub excerpt: String,
}

/// What the survey found, and (when applied) what it changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WingBackfillReport {
    /// Whether a write was requested. `false` is a pure dry run.
    pub apply: bool,
    /// Chat memories sitting in the catch-all wing that were examined.
    pub scanned: usize,
    /// Sessions whose project came from the captured `sessions.project_hint_*`
    /// columns — no reconstruction needed.
    pub hints_captured: usize,
    /// Sessions whose project was reconstructed from a `project_selected`
    /// record inside [`HINT_WINDOW_SECONDS`].
    pub hints_reconstructed: usize,
    /// Hint reconstructed AND the memory's own content agrees. The only bucket
    /// a write would touch.
    pub corroborated: usize,
    /// The memory names a different known project than the hint. Never written.
    /// A large number here falsifies the hint mechanism, which is exactly why
    /// it is counted rather than folded into "not assigned".
    pub conflicting: usize,
    /// Hint available, memory names no known project. Never written.
    pub unverifiable: usize,
    /// No project could be reconstructed for the session at all.
    pub no_hint: usize,
    /// Corroborated counts by signal (`content-name` / `alias` / `tool-path`),
    /// so the yield of each is measured rather than assumed.
    pub by_source: BTreeMap<String, usize>,
    /// Corroborated counts by wing.
    pub by_wing: BTreeMap<String, usize>,
    /// Up to [`MAX_SAMPLES`] corroborated rows, verbatim.
    pub samples: Vec<BackfillSample>,
    /// Rows actually written. Zero for a dry run, and zero while
    /// [`WingBackfillReport::apply_blocked`] is set.
    pub applied: usize,
    /// Set when `apply: true` was requested but nothing was written, with the
    /// reason verbatim. `None` on a dry run — a dry run is not blocked, it is
    /// doing what it was asked.
    pub apply_blocked: Option<String>,
}

impl WingBackfillReport {
    /// Every scanned row is in exactly one bucket. Asserted rather than assumed:
    /// a survey whose buckets do not sum to the scan has lost rows somewhere.
    pub fn buckets_account_for_every_row(&self) -> bool {
        self.corroborated + self.conflicting + self.unverifiable + self.no_hint == self.scanned
    }

    /// One line an agent can narrate or a log line can carry.
    pub fn summary(&self) -> String {
        let mode = if self.apply_blocked.is_some() {
            "apply REFUSED"
        } else if self.apply {
            "applied"
        } else {
            "dry run"
        };
        format!(
            "Wing backfill ({mode}): {} chat memories in `{CATCH_ALL_WING}` scanned — \
             {} corroborated, {} conflicting, {} unverifiable, {} with no reconstructable project. \
             {} written.",
            self.scanned,
            self.corroborated,
            self.conflicting,
            self.unverifiable,
            self.no_hint,
            self.applied,
        )
    }
}

/// One chat memory in the catch-all wing.
struct CatchAllTurn {
    key: String,
    content: String,
    created_at: String,
}

impl CatchAllTurn {
    /// The session id embedded in a `chat-<session>-<idx>` key.
    ///
    /// Parsed from the key rather than read from a column because the
    /// memory→session association lives in Spectral's own table and the key is
    /// the identity this writer minted. `None` for anything not shaped like a
    /// chat turn — those are skipped, not guessed at.
    fn session_id(&self) -> Option<&str> {
        let rest = self.key.strip_prefix("chat-")?;
        let cut = rest.rfind('-')?;
        (cut > 0).then(|| &rest[..cut])
    }
}

fn open_memory_db_read_only(brain_dir: &Path) -> Result<rusqlite::Connection, String> {
    let db_path = brain_dir.join("memory.db");
    rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open {}: {e}", db_path.display()))
}

/// Chat turns currently in the catch-all wing.
///
/// `wing IS NULL OR wing = 'general'` is the same predicate the corpus
/// measurement uses; a survey that counted one set and judged another would
/// report a clean number over rows it never looked at.
fn select_catch_all_chat_turns(conn: &rusqlite::Connection) -> Result<Vec<CatchAllTurn>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT key, content, created_at
             FROM memories
             WHERE key LIKE 'chat-%' AND (wing IS NULL OR wing = 'general')
             ORDER BY created_at",
        )
        .map_err(|e| format!("prepare catch-all chat select: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CatchAllTurn {
                key: row.get(0)?,
                content: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                // NULL is possible and must not abort the whole survey. A turn
                // with no timestamp cannot be time-matched to a project
                // selection, so it lands in `no_hint` — counted, not dropped.
                created_at: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        })
        .map_err(|e| format!("select catch-all chat turns: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read catch-all chat turns: {e}"))?;
    Ok(rows)
}

/// The `project_selected` timeline, as `(created_at, wing_slug)` sorted by time.
///
/// Both key shapes the ambient writer has used are read: the current
/// project-stable `activity:project_selected:project:<slug>` and the older
/// per-instant `activity:<ts>:project_selected:` fallback. The slug is taken
/// from the key where the key carries it, because the key is canonical and the
/// rendered content is prose.
///
/// This is an approximation and is treated as one — it is why corroboration is
/// required on top, not instead.
fn select_project_selection_timeline(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT key, created_at
             FROM memories
             WHERE key LIKE 'activity:%project_selected%'
             ORDER BY created_at",
        )
        .map_err(|e| format!("prepare project_selected select: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .map_err(|e| format!("select project_selected records: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read project_selected records: {e}"))?;

    let mut out = Vec::new();
    for (key, created_at) in rows {
        if let Some(slug) = key
            .split("project_selected:")
            .nth(1)
            .and_then(ProjectHint::slug_from_canonical)
        {
            out.push((created_at, slug.to_string()));
        }
    }
    Ok(out)
}

/// The most recent project selected at or before `at`, within
/// [`HINT_WINDOW_SECONDS`].
fn reconstruct_hint_slug(timeline: &[(String, String)], at: &str) -> Option<String> {
    let at_ts = parse_ts(at)?;
    timeline
        .iter()
        .filter_map(|(ts, slug)| {
            let ts = parse_ts(ts)?;
            let age = at_ts - ts;
            (age >= 0 && age <= HINT_WINDOW_SECONDS).then_some((ts, slug.clone()))
        })
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, slug)| slug)
}

/// Seconds since the epoch for an RFC3339-ish timestamp, or `None` when the
/// value cannot be parsed. A row with an unreadable timestamp is skipped, never
/// treated as "now".
fn parse_ts(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|dt| dt.and_utc().timestamp())
        })
}

/// Captured session hints: `session_id -> wing slug`, for sessions created
/// after the capture landed.
async fn load_captured_hints(pool: &Pool<Sqlite>) -> BTreeMap<String, String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, project_hint_wing FROM sessions
         WHERE project_hint_wing IS NOT NULL AND project_hint_wing <> ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().collect()
}

/// Survey (and, when Spectral supports it, repair) chat memories in the
/// catch-all wing.
///
/// `brain_dir` and `pool` are parameters rather than reads of
/// [`crate::config::paths::Paths`] so tests can run this against a fixture.
/// Production callers use [`run_on_default_paths`].
pub async fn run(
    brain_dir: &Path,
    pool: &Pool<Sqlite>,
    apply: bool,
) -> Result<WingBackfillReport, String> {
    let projects = crate::wing_rules::load_project_rows(pool).await;
    let project_by_slug: BTreeMap<&str, &str> = projects
        .iter()
        .map(|(slug, name)| (slug.as_str(), name.as_str()))
        .collect();
    let roots = load_project_roots(pool).await;
    let captured = load_captured_hints(pool).await;

    let (turns, timeline) = {
        let conn = open_memory_db_read_only(brain_dir)?;
        (
            select_catch_all_chat_turns(&conn)?,
            select_project_selection_timeline(&conn)?,
        )
    };

    let mut report = WingBackfillReport {
        apply,
        scanned: turns.len(),
        hints_captured: 0,
        hints_reconstructed: 0,
        corroborated: 0,
        conflicting: 0,
        unverifiable: 0,
        no_hint: 0,
        by_source: BTreeMap::new(),
        by_wing: BTreeMap::new(),
        samples: Vec::new(),
        applied: 0,
        apply_blocked: apply.then(|| APPLY_UNSUPPORTED.to_string()),
    };

    for turn in &turns {
        let Some(session_id) = turn.session_id() else {
            // Not a `chat-<session>-<idx>` key after all. Counted honestly as
            // "no hint" rather than dropped from the scan total.
            report.no_hint += 1;
            continue;
        };

        let (slug, was_captured) = match captured.get(session_id) {
            Some(slug) => (Some(slug.clone()), true),
            None => (reconstruct_hint_slug(&timeline, &turn.created_at), false),
        };
        let Some(slug) = slug else {
            report.no_hint += 1;
            continue;
        };
        if was_captured {
            report.hints_captured += 1;
        } else {
            report.hints_reconstructed += 1;
        }

        let hint = ProjectHint {
            project_id: format!("project:{slug}"),
            name: project_by_slug
                .get(slug.as_str())
                .map(|n| n.to_string())
                .unwrap_or_else(|| slug.clone()),
            root_path: roots.get(slug.as_str()).cloned(),
            slug,
        };

        // The SAME corroborator the write path uses. `tool_text` is empty
        // because a stored memory holds only `User: …\nAssistant: …` — the tool
        // arguments were never persisted, which is precisely why the write-time
        // check exists and why a retrospective sweep recovers so few rows.
        let corroborator = WingCorroborator::new(hint, &projects);
        match corroborator.verdict(&turn.content, "") {
            WingVerdict::Corroborated { wing, source } => {
                report.corroborated += 1;
                *report
                    .by_source
                    .entry(source.as_str().to_string())
                    .or_insert(0) += 1;
                *report.by_wing.entry(wing.clone()).or_insert(0) += 1;
                if report.samples.len() < MAX_SAMPLES {
                    report.samples.push(BackfillSample {
                        memory_key: turn.key.clone(),
                        session_id: session_id.to_string(),
                        wing,
                        corroborated_by: source.as_str().to_string(),
                        excerpt: turn.content.chars().take(160).collect(),
                    });
                }
            }
            WingVerdict::Conflicting { .. } => report.conflicting += 1,
            WingVerdict::Unverifiable => report.unverifiable += 1,
        }
    }

    if report.apply_blocked.is_some() {
        tracing::warn!(
            target: "permagent::wing_backfill",
            summary = %report.summary(),
            reason = APPLY_UNSUPPORTED,
            "Wing backfill surveyed but refused to write"
        );
    } else {
        tracing::info!(
            target: "permagent::wing_backfill",
            summary = %report.summary(),
            "Wing backfill survey complete"
        );
    }

    Ok(report)
}

/// `slug -> root_path` for projects that have one.
async fn load_project_roots(pool: &Pool<Sqlite>) -> BTreeMap<String, String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, root_path FROM projects WHERE root_path IS NOT NULL AND root_path <> ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().collect()
}

/// [`run`] against the live brain directory.
pub async fn run_on_default_paths(
    pool: &Pool<Sqlite>,
    apply: bool,
) -> Result<WingBackfillReport, String> {
    let brain_dir = crate::config::paths::Paths::brain_dir();
    run(&brain_dir, pool, apply).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(key: &str, content: &str, created_at: &str) -> CatchAllTurn {
        CatchAllTurn {
            key: key.to_string(),
            content: content.to_string(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn a_chat_key_yields_its_session_id() {
        let t = turn("chat-01931f7a-b2c3-7000-8000-000000000001-42", "", "");
        assert_eq!(
            t.session_id(),
            Some("01931f7a-b2c3-7000-8000-000000000001"),
            "the turn index is the LAST hyphen-separated field; a session id \
             contains hyphens of its own"
        );
    }

    #[test]
    fn a_non_chat_key_yields_no_session() {
        assert_eq!(turn("note:acme:note-1", "", "").session_id(), None);
        assert_eq!(turn("chat-", "", "").session_id(), None);
    }

    #[test]
    fn the_most_recent_selection_inside_the_window_wins() {
        let timeline = vec![
            ("2026-08-20T09:00:00Z".to_string(), "plekk".to_string()),
            ("2026-08-20T10:30:00Z".to_string(), "permagent".to_string()),
        ];
        assert_eq!(
            reconstruct_hint_slug(&timeline, "2026-08-20T11:00:00Z"),
            Some("permagent".to_string())
        );
    }

    #[test]
    fn a_selection_older_than_the_window_is_not_a_hint() {
        let timeline = vec![("2026-08-20T06:00:00Z".to_string(), "plekk".to_string())];
        assert_eq!(
            reconstruct_hint_slug(&timeline, "2026-08-20T11:00:00Z"),
            None,
            "five hours stale is exactly the leakage this bound exists to stop"
        );
    }

    #[test]
    fn a_selection_after_the_turn_is_never_a_hint() {
        let timeline = vec![("2026-08-20T12:00:00Z".to_string(), "plekk".to_string())];
        assert_eq!(
            reconstruct_hint_slug(&timeline, "2026-08-20T11:00:00Z"),
            None
        );
    }

    #[test]
    fn an_unparseable_timestamp_is_skipped_not_treated_as_now() {
        let timeline = vec![("not-a-timestamp".to_string(), "plekk".to_string())];
        assert_eq!(
            reconstruct_hint_slug(&timeline, "2026-08-20T11:00:00Z"),
            None
        );
        assert_eq!(
            reconstruct_hint_slug(&timeline, "also-not-a-timestamp"),
            None
        );
    }

    #[test]
    fn a_dry_run_is_never_blocked_and_never_writes() {
        let report = WingBackfillReport {
            apply: false,
            scanned: 10,
            hints_captured: 0,
            hints_reconstructed: 4,
            corroborated: 1,
            conflicting: 2,
            unverifiable: 1,
            no_hint: 6,
            by_source: BTreeMap::new(),
            by_wing: BTreeMap::new(),
            samples: Vec::new(),
            applied: 0,
            apply_blocked: None,
        };
        assert_eq!(report.applied, 0);
        assert!(report.apply_blocked.is_none());
        assert!(report.buckets_account_for_every_row());
        assert!(report.summary().contains("dry run"));
    }

    #[test]
    fn requesting_apply_reports_why_nothing_was_written() {
        let report = WingBackfillReport {
            apply: true,
            scanned: 1,
            hints_captured: 0,
            hints_reconstructed: 1,
            corroborated: 1,
            conflicting: 0,
            unverifiable: 0,
            no_hint: 0,
            by_source: BTreeMap::new(),
            by_wing: BTreeMap::new(),
            samples: Vec::new(),
            applied: 0,
            apply_blocked: Some(APPLY_UNSUPPORTED.to_string()),
        };
        assert_eq!(
            report.applied, 0,
            "a refused apply must not report rows it did not write"
        );
        assert!(report.summary().contains("apply REFUSED"));
    }

    #[test]
    fn buckets_that_do_not_sum_to_the_scan_are_detectable() {
        let report = WingBackfillReport {
            apply: false,
            scanned: 10,
            hints_captured: 0,
            hints_reconstructed: 0,
            corroborated: 1,
            conflicting: 1,
            unverifiable: 1,
            no_hint: 1,
            by_source: BTreeMap::new(),
            by_wing: BTreeMap::new(),
            samples: Vec::new(),
            applied: 0,
            apply_blocked: None,
        };
        assert!(
            !report.buckets_account_for_every_row(),
            "a survey that loses rows must be able to say so"
        );
    }
}
