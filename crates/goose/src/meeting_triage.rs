//! Durable Orchestrator meeting triage — idempotent note save, inspectable
//! hidden session per recording, privacy-aware tool-free extraction, stable
//! card materialization. Does not dispatch goals or inject into user chat.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite, Transaction};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::config::agent_identity::{load_agent_config, PrimaryPersona};
use crate::conversation::message::Message;
use crate::meeting_writeup::{self, MeetingWriteupPlan, ORCHESTRATOR_TRIAGE_SESSION_PREFIX};
use crate::project_notes::{self, ProjectNote};
use crate::projects::Project;
use crate::session::extension_data::ExtensionData;

pub const MAX_MEETING_TODOS: usize = 20;
const MAX_TRIAGE_ATTEMPTS: i64 = 5;
const MEETING_TRIAGE_EXTENSION: &str = "meeting_triage";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl TriageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTriageReceipt {
    pub id: String,
    pub project_id: String,
    pub recording_id: String,
    pub note_id: String,
    pub status: TriageStatus,
    pub attempt_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_route: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestrator_name: Option<String>,
    /// `true` when the client supplied a stable `recordingId`; legacy saves
    /// without one get a server UUID per request (non-idempotent by design).
    pub idempotent_recording: bool,
    pub card_ids: Vec<String>,
    pub truncated: bool,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTriageSessionView {
    pub session_id: String,
    pub orchestrator_name: String,
    pub project_id: String,
    pub recording_id: String,
    pub note_id: String,
    pub idempotent_recording: bool,
    pub message_count: usize,
    pub context: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeetingTriageSessionState {
    project_id: String,
    recording_id: String,
    note_id: String,
    orchestrator_name: String,
    idempotent_recording: bool,
}

#[derive(Debug)]
pub enum MeetingSaveError {
    EmptyBody,
    BodyConflict { note_id: String },
    Database(String),
}

impl MeetingSaveError {
    pub fn message(&self) -> String {
        match self {
            Self::EmptyBody => "note body is empty".to_string(),
            Self::BodyConflict { note_id } => {
                format!("recording body conflict (existing note {note_id})")
            }
            Self::Database(e) => e.clone(),
        }
    }
}

#[derive(Debug)]
pub struct MeetingSaveResult {
    pub note: ProjectNote,
    pub triage: MeetingTriageReceipt,
}

struct TriageRow {
    id: String,
    project_id: String,
    recording_id: String,
    note_id: String,
    body_hash: String,
    status: TriageStatus,
    attempt_count: i64,
    last_error: Option<String>,
    privacy_route: Option<String>,
    session_id: Option<String>,
    result_json: Option<String>,
    writeup_provider: Option<String>,
    writeup_model: Option<String>,
    writeup_privacy: Option<String>,
    card_ids_json: String,
    truncated: bool,
    idempotent_recording: bool,
    orchestrator_name: Option<String>,
}

pub fn body_hash(body: &str) -> String {
    hex::encode(Sha256::digest(body.trim().as_bytes()))
}

pub fn triage_session_name(project_id: &str, recording_id: &str) -> String {
    format!("{ORCHESTRATOR_TRIAGE_SESSION_PREFIX}:{project_id}:{recording_id}")
}

/// Deterministic triage + note ids for idempotent client recording ids.
#[allow(clippy::string_slice)] // slices an ASCII hex digest
pub fn stable_triage_ids(project_id: &str, recording_id: &str) -> (String, String) {
    let digest = hex::encode(Sha256::digest(
        format!("{project_id}\0{recording_id}").as_bytes(),
    ));
    (
        format!("mt-{}", &digest[..32]),
        format!("mn-{}", &digest[32..64]),
    )
}

#[allow(clippy::string_slice)] // slices an ASCII hex digest
fn stable_card_id(triage_id: &str, item_key: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("card:{triage_id}:{item_key}").as_bytes(),
    ));
    format!("mc-{}", &digest[..32])
}

/// Resolve recording id from the HTTP request. Client-supplied ids are
/// idempotent; absent ids get a fresh server UUID (legacy, non-idempotent).
pub fn resolve_recording_id(client: Option<&str>) -> (String, bool) {
    match client.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => (id.to_string(), true),
        None => (uuid::Uuid::now_v7().to_string(), false),
    }
}

/// Stable card identity within one triage — not global title matching.
pub fn item_key(index: usize, title: &str) -> String {
    let normalized: String = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    hex::encode(Sha256::digest(format!("{index}:{normalized}").as_bytes()))
}

/// Split a saved meeting body into (user notes, transcript).
/// Cross-language contract with `composeMeetingBody` in useMeetingDictation.ts.
pub fn split_meeting_body(body: &str) -> (Option<String>, String) {
    const MARKER: &str = "## Your notes";
    const TRANSCRIPT: &str = "## Transcript";
    let Some(start) = body.find(MARKER) else {
        return (None, body.to_string());
    };
    let Some(after) = body.get(start + MARKER.len()..) else {
        return (None, body.to_string());
    };
    match after.find(TRANSCRIPT) {
        Some(end) => {
            let notes = after.get(..end).unwrap_or_default().trim();
            let rest = after
                .get(end + TRANSCRIPT.len()..)
                .unwrap_or_default()
                .trim();
            (
                (!notes.is_empty()).then(|| notes.to_string()),
                rest.to_string(),
            )
        }
        None => {
            let notes = after.trim();
            (
                (!notes.is_empty()).then(|| notes.to_string()),
                String::new(),
            )
        }
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db) if db.code().as_deref() == Some("2067")
            || db.message().contains("UNIQUE constraint failed")
    )
}

fn load_orchestrator_persona() -> PrimaryPersona {
    load_agent_config().primary
}

pub async fn apply_schema(pool: &Pool<Sqlite>) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS meeting_triage (
            id              TEXT PRIMARY KEY,
            project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            recording_id    TEXT NOT NULL,
            note_id         TEXT NOT NULL,
            body_hash       TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','running','completed','failed','skipped')),
            attempt_count   INTEGER NOT NULL DEFAULT 0,
            last_error      TEXT,
            privacy_route   TEXT,
            session_id      TEXT,
            result_json     TEXT,
            card_ids_json   TEXT NOT NULL DEFAULT '[]',
            truncated       INTEGER NOT NULL DEFAULT 0,
            idempotent_recording INTEGER NOT NULL DEFAULT 1,
            orchestrator_name TEXT,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            started_at      TEXT,
            completed_at    TEXT,
            UNIQUE(project_id, recording_id)
        )",
    )
    .execute(pool)
    .await?;
    for stmt in [
        "ALTER TABLE meeting_triage ADD COLUMN idempotent_recording INTEGER NOT NULL DEFAULT 1",
        "ALTER TABLE meeting_triage ADD COLUMN orchestrator_name TEXT",
        "ALTER TABLE meeting_triage ADD COLUMN writeup_provider TEXT",
        "ALTER TABLE meeting_triage ADD COLUMN writeup_model TEXT",
        "ALTER TABLE meeting_triage ADD COLUMN writeup_privacy TEXT",
    ] {
        let _ = sqlx::query(stmt).execute(pool).await;
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_meeting_triage_drain
         ON meeting_triage(status, updated_at, id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS meeting_triage_items (
            triage_id   TEXT NOT NULL REFERENCES meeting_triage(id) ON DELETE CASCADE,
            item_key    TEXT NOT NULL,
            title       TEXT NOT NULL,
            context     TEXT NOT NULL DEFAULT '',
            card_id     TEXT,
            PRIMARY KEY (triage_id, item_key)
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_receipt(row: &TriageRow, created: bool) -> MeetingTriageReceipt {
    let card_ids: Vec<String> = serde_json::from_str(&row.card_ids_json).unwrap_or_default();
    MeetingTriageReceipt {
        id: row.id.clone(),
        project_id: row.project_id.clone(),
        recording_id: row.recording_id.clone(),
        note_id: row.note_id.clone(),
        status: row.status,
        attempt_count: row.attempt_count,
        last_error: row.last_error.clone(),
        privacy_route: row.privacy_route.clone(),
        session_id: row.session_id.clone(),
        orchestrator_name: row.orchestrator_name.clone(),
        idempotent_recording: row.idempotent_recording,
        card_ids,
        truncated: row.truncated,
        created,
    }
}

fn parse_row(r: &sqlx::sqlite::SqliteRow) -> Option<TriageRow> {
    let status = TriageStatus::parse(r.get::<String, _>("status").as_str())?;
    Some(TriageRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        recording_id: r.get("recording_id"),
        note_id: r.get("note_id"),
        body_hash: r.get("body_hash"),
        status,
        attempt_count: r.get("attempt_count"),
        last_error: r.get("last_error"),
        privacy_route: r.get("privacy_route"),
        session_id: r.get("session_id"),
        result_json: r.get("result_json"),
        writeup_provider: r
            .try_get::<Option<String>, _>("writeup_provider")
            .ok()
            .flatten(),
        writeup_model: r
            .try_get::<Option<String>, _>("writeup_model")
            .ok()
            .flatten(),
        writeup_privacy: r
            .try_get::<Option<String>, _>("writeup_privacy")
            .ok()
            .flatten(),
        card_ids_json: r.get("card_ids_json"),
        truncated: r.get::<i64, _>("truncated") != 0,
        idempotent_recording: r.try_get::<i64, _>("idempotent_recording").unwrap_or(1) != 0,
        orchestrator_name: r
            .try_get::<Option<String>, _>("orchestrator_name")
            .ok()
            .flatten(),
    })
}

fn provenance_from_row(row: &TriageRow) -> meeting_writeup::WriteupProvenance {
    match (
        row.writeup_provider.as_deref(),
        row.writeup_model.as_deref(),
        row.writeup_privacy.as_deref(),
    ) {
        (Some(provider), Some(model), Some(privacy_tag)) => meeting_writeup::WriteupProvenance {
            provider: provider.to_string(),
            model: model.to_string(),
            privacy: meeting_writeup::writeup_privacy_from_tag(privacy_tag),
        },
        _ => meeting_writeup::provenance_unknown(),
    }
}

async fn fetch_by_recording(
    pool: &Pool<Sqlite>,
    project_id: &str,
    recording_id: &str,
) -> Result<Option<TriageRow>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, recording_id, note_id, body_hash, status, attempt_count,
                last_error, privacy_route, session_id, result_json, writeup_provider,
                writeup_model, writeup_privacy, card_ids_json, truncated,
                idempotent_recording, orchestrator_name
         FROM meeting_triage WHERE project_id = ? AND recording_id = ?",
    )
    .bind(project_id)
    .bind(recording_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().and_then(parse_row))
}

async fn fetch_by_id(pool: &Pool<Sqlite>, triage_id: &str) -> Result<Option<TriageRow>, String> {
    let row = sqlx::query(
        "SELECT id, project_id, recording_id, note_id, body_hash, status, attempt_count,
                last_error, privacy_route, session_id, result_json, writeup_provider,
                writeup_model, writeup_privacy, card_ids_json, truncated,
                idempotent_recording, orchestrator_name
         FROM meeting_triage WHERE id = ?",
    )
    .bind(triage_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().and_then(parse_row))
}

pub async fn get_triage_receipt(
    pool: &Pool<Sqlite>,
    project_id: &str,
    recording_id: &str,
) -> Result<Option<MeetingTriageReceipt>, String> {
    let row = fetch_by_recording(pool, project_id, recording_id).await?;
    Ok(row.map(|r| row_to_receipt(&r, false)))
}

pub async fn get_triage_session_view(
    pool: &Pool<Sqlite>,
    project_id: &str,
    recording_id: &str,
) -> Result<Option<MeetingTriageSessionView>, String> {
    let Some(row) = fetch_by_recording(pool, project_id, recording_id).await? else {
        return Ok(None);
    };
    let Some(session_id) = row.session_id.clone() else {
        return Ok(None);
    };
    let manager = crate::session::SessionManager::instance();
    let session = manager
        .get_session(&session_id, false)
        .await
        .map_err(|e| e.to_string())?;
    let state = MeetingTriageSessionState::from_extension_data(&session.extension_data).unwrap_or(
        MeetingTriageSessionState {
            project_id: row.project_id.clone(),
            recording_id: row.recording_id.clone(),
            note_id: row.note_id.clone(),
            orchestrator_name: row
                .orchestrator_name
                .clone()
                .unwrap_or_else(|| load_orchestrator_persona().display_name()),
            idempotent_recording: row.idempotent_recording,
        },
    );
    let message_count = session.message_count;
    let context = serde_json::to_value(&state).unwrap_or(serde_json::Value::Null);
    Ok(Some(MeetingTriageSessionView {
        session_id,
        orchestrator_name: state.orchestrator_name,
        project_id: state.project_id,
        recording_id: state.recording_id,
        note_id: state.note_id,
        idempotent_recording: state.idempotent_recording,
        message_count,
        context,
    }))
}

impl MeetingTriageSessionState {
    fn from_extension_data(data: &ExtensionData) -> Option<Self> {
        data.get_extension_state(MEETING_TRIAGE_EXTENSION, "v1")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

async fn repair_note_catalogue(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    note_id: &str,
) -> Result<ProjectNote, MeetingSaveError> {
    // Read and enqueue under the same writer reservation: a retry must not
    // publish a raw snapshot after triage has committed an enhanced note.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| MeetingSaveError::Database(e.to_string()))?;
    let note = project_notes::get_note_in_tx(&mut tx, project_id, note_id)
        .await
        .map_err(MeetingSaveError::Database)?
        .ok_or_else(|| MeetingSaveError::Database("triage note row missing".to_string()))?;
    project_notes::enqueue_note_catalogue_in_tx(
        &mut tx,
        project_id,
        &note.id,
        note.title.as_deref(),
        &note.body,
    )
    .await
    .map_err(MeetingSaveError::Database)?;
    tx.commit()
        .await
        .map_err(|e| MeetingSaveError::Database(e.to_string()))?;
    if let Some(brain) = brain {
        crate::brain_feed::spawn_drain(brain.clone(), pool.clone());
    }
    Ok(note)
}

async fn fetch_existing_save(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    recording_id: &str,
    hash: &str,
) -> Result<MeetingSaveResult, MeetingSaveError> {
    let existing = fetch_by_recording(pool, project_id, recording_id)
        .await
        .map_err(MeetingSaveError::Database)?
        .ok_or_else(|| MeetingSaveError::Database("triage row missing after conflict".into()))?;
    if existing.body_hash != hash {
        return Err(MeetingSaveError::BodyConflict {
            note_id: existing.note_id.clone(),
        });
    }
    let note = repair_note_catalogue(pool, brain, project_id, &existing.note_id).await?;
    Ok(MeetingSaveResult {
        note,
        triage: row_to_receipt(&existing, false),
    })
}

/// Idempotent meeting note + triage row in one transaction; catalogue enqueued in-txn.
pub async fn save_meeting_note(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    project_id: &str,
    recording_id: &str,
    idempotent_recording: bool,
    title: Option<&str>,
    body: &str,
) -> Result<MeetingSaveResult, MeetingSaveError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(MeetingSaveError::EmptyBody);
    }
    let hash = body_hash(body);
    if let Some(existing) = fetch_by_recording(pool, project_id, recording_id)
        .await
        .map_err(MeetingSaveError::Database)?
    {
        if existing.body_hash != hash {
            return Err(MeetingSaveError::BodyConflict {
                note_id: existing.note_id.clone(),
            });
        }
        let note = repair_note_catalogue(pool, brain, project_id, &existing.note_id).await?;
        return Ok(MeetingSaveResult {
            note,
            triage: row_to_receipt(&existing, false),
        });
    }

    let (triage_id, note_id) = if idempotent_recording {
        stable_triage_ids(project_id, recording_id)
    } else {
        (
            uuid::Uuid::now_v7().to_string(),
            uuid::Uuid::now_v7().to_string(),
        )
    };
    let memory_key = format!("note:{}:{}", project_id, note_id);
    let orchestrator_name = load_orchestrator_persona().display_name();
    let title = title.map(str::trim).filter(|t| !t.is_empty());

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| MeetingSaveError::Database(e.to_string()))?;

    if let Err(e) = project_notes::insert_note_tx(
        &mut tx,
        &note_id,
        project_id,
        title,
        body,
        Some(&memory_key),
    )
    .await
    {
        let _ = tx.rollback().await;
        if e.contains("UNIQUE constraint failed") {
            return fetch_existing_save(pool, brain, project_id, recording_id, &hash).await;
        }
        return Err(MeetingSaveError::Database(e));
    }

    project_notes::enqueue_note_catalogue_in_tx(&mut tx, project_id, &note_id, title, body)
        .await
        .map_err(MeetingSaveError::Database)?;

    let triage_insert = sqlx::query(
        "INSERT INTO meeting_triage
         (id, project_id, recording_id, note_id, body_hash, status, idempotent_recording, orchestrator_name)
         VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
    )
    .bind(&triage_id)
    .bind(project_id)
    .bind(recording_id)
    .bind(&note_id)
    .bind(&hash)
    .bind(idempotent_recording as i64)
    .bind(&orchestrator_name)
    .execute(&mut *tx)
    .await;

    match triage_insert {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            let _ = tx.rollback().await;
            return fetch_existing_save(pool, brain, project_id, recording_id, &hash).await;
        }
        Err(e) => {
            let _ = tx.rollback().await;
            return Err(MeetingSaveError::Database(e.to_string()));
        }
    }

    tx.commit()
        .await
        .map_err(|e| MeetingSaveError::Database(e.to_string()))?;

    if let Some(brain) = brain {
        crate::brain_feed::spawn_drain(brain.clone(), pool.clone());
    }

    let note = project_notes::get_note(pool, project_id, &note_id)
        .await
        .map_err(MeetingSaveError::Database)?
        .ok_or_else(|| MeetingSaveError::Database("note row missing after commit".into()))?;
    let row = fetch_by_id(pool, &triage_id)
        .await
        .map_err(MeetingSaveError::Database)?
        .ok_or_else(|| MeetingSaveError::Database("triage row missing after commit".into()))?;

    Ok(MeetingSaveResult {
        note,
        triage: row_to_receipt(&row, true),
    })
}

pub async fn retry_triage(
    pool: &Pool<Sqlite>,
    project_id: &str,
    recording_id: &str,
) -> Result<Option<MeetingTriageReceipt>, String> {
    let Some(row) = fetch_by_recording(pool, project_id, recording_id).await? else {
        return Ok(None);
    };
    if !matches!(row.status, TriageStatus::Failed | TriageStatus::Skipped) {
        return Ok(Some(row_to_receipt(&row, false)));
    }
    sqlx::query(
        "UPDATE meeting_triage SET status = 'pending', last_error = NULL, attempt_count = 0,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ? AND status IN ('failed','skipped')",
    )
    .bind(&row.id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    let updated = fetch_by_id(pool, &row.id).await?;
    Ok(updated.map(|r| row_to_receipt(&r, false)))
}

fn privacy_route_label(plan: &MeetingWriteupPlan) -> Option<String> {
    match plan {
        MeetingWriteupPlan::ViaMesh { route, .. } => Some(match route.approach {
            crate::mesh::PrivacyApproach::LocalOnly => "local_only".to_string(),
            crate::mesh::PrivacyApproach::TrustedPool => "trusted_pool".to_string(),
        }),
        MeetingWriteupPlan::ViaSession { .. } => Some("cloud_session".to_string()),
        MeetingWriteupPlan::Skip => Some("skipped".to_string()),
    }
}

async fn ensure_triage_session(
    project_id: &str,
    recording_id: &str,
    note_id: &str,
    project_name: &str,
    idempotent_recording: bool,
) -> Result<(Arc<crate::session::SessionManager>, crate::session::Session), String> {
    use crate::agents::reply_parts::AccountedFastCompletion;
    use crate::session::SessionType;

    let persona = load_orchestrator_persona();
    let orchestrator_name = persona.display_name();
    let manager = Arc::new(crate::session::SessionManager::instance());
    let name = triage_session_name(project_id, recording_id);
    if let Some(session) = manager
        .list_sessions_by_types(&[SessionType::Hidden])
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.name == name)
    {
        return Ok((manager, session));
    }
    let session = AccountedFastCompletion::ensure_background_session(manager.clone(), &name)
        .await
        .map_err(|e| e.to_string())?;

    let state = MeetingTriageSessionState {
        project_id: project_id.to_string(),
        recording_id: recording_id.to_string(),
        note_id: note_id.to_string(),
        orchestrator_name: orchestrator_name.clone(),
        idempotent_recording,
    };
    let mut extension_data = session.extension_data.clone();
    extension_data.set_extension_state(
        MEETING_TRIAGE_EXTENSION,
        "v1",
        serde_json::to_value(&state).map_err(|e| e.to_string())?,
    );
    manager
        .update(&session.id)
        .extension_data(extension_data)
        .apply()
        .await
        .map_err(|e| e.to_string())?;

    let context = format!(
        "You are {orchestrator_name}, the Orchestrator for this Permagent instance.\n\n\
         {}\n\n\
         Meeting triage for project `{project_name}` (recording `{recording_id}`, note `{note_id}`). \
         Tool-free catalogue pass — extract commitments only; do not execute tasks.",
        persona.system_prompt_block()
    );
    manager
        .add_message(&session.id, &Message::user().with_text(context))
        .await
        .map_err(|e| e.to_string())?;
    let session = manager
        .get_session(&session.id, false)
        .await
        .map_err(|e| e.to_string())?;
    Ok((manager, session))
}

async fn persist_session_turn(
    manager: &Arc<crate::session::SessionManager>,
    session_id: &str,
    prompt: &str,
    response: &str,
) -> Result<(), String> {
    manager
        .add_message(session_id, &Message::user().with_text(prompt))
        .await
        .map_err(|e| e.to_string())?;
    manager
        .add_message(session_id, &Message::assistant().with_text(response))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

static STARTUP_RECOVERY_DONE: AtomicBool = AtomicBool::new(false);

static DRAIN_COALESCE: OnceLock<Mutex<()>> = OnceLock::new();

fn drain_coalesce() -> &'static Mutex<()> {
    DRAIN_COALESCE.get_or_init(|| Mutex::new(()))
}

const EXHAUSTED_RUNNING_ERROR: &str =
    "triage abandoned at startup after exhausting max attempts; use manual retry";

/// Reset abandoned `running` rows at startup. Rows under the attempt cap become
/// `pending`; exhausted rows become `failed` so they are not forever unclaimable.
pub(crate) async fn reset_abandoned_running_rows(
    pool: &Pool<Sqlite>,
) -> Result<(u64, u64), String> {
    let pending = sqlx::query(
        "UPDATE meeting_triage SET status = 'pending', last_error = NULL,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status = 'running' AND attempt_count < ?",
    )
    .bind(MAX_TRIAGE_ATTEMPTS)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    let failed = sqlx::query(
        "UPDATE meeting_triage SET status = 'failed', last_error = ?,
         completed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status = 'running' AND attempt_count >= ?",
    )
    .bind(EXHAUSTED_RUNNING_ERROR)
    .bind(MAX_TRIAGE_ATTEMPTS)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok((pending.rows_affected(), failed.rows_affected()))
}

/// Reset `running` rows abandoned by a prior daemon crash. Call once at daemon
/// startup before the periodic drain worker — never from POST-triggered drains.
pub async fn recover_abandoned_at_startup(pool: &Pool<Sqlite>) -> Result<(), String> {
    if STARTUP_RECOVERY_DONE.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let (pending, failed) = reset_abandoned_running_rows(pool).await?;
    if pending > 0 || failed > 0 {
        tracing::info!(
            pending,
            failed,
            "meeting triage startup recovery reset abandoned running rows"
        );
    }
    Ok(())
}

async fn claim_next(pool: &Pool<Sqlite>) -> Result<Option<String>, String> {
    let candidate: Option<String> = sqlx::query_scalar(
        "SELECT id FROM meeting_triage
         WHERE status = 'pending'
           AND attempt_count < ?
         ORDER BY created_at ASC LIMIT 1",
    )
    .bind(MAX_TRIAGE_ATTEMPTS)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(id) = candidate else {
        return Ok(None);
    };
    let updated = sqlx::query(
        "UPDATE meeting_triage SET status = 'running',
         attempt_count = attempt_count + 1,
         started_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ? AND status = 'pending'",
    )
    .bind(&id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    Ok(Some(id))
}

#[allow(clippy::too_many_arguments)]
async fn mark_status(
    pool: &Pool<Sqlite>,
    triage_id: &str,
    status: TriageStatus,
    last_error: Option<&str>,
    privacy_route: Option<&str>,
    session_id: Option<&str>,
    result_json: Option<&str>,
    card_ids: Option<&[String]>,
    truncated: bool,
    orchestrator_name: Option<&str>,
) -> Result<(), String> {
    let cards = card_ids
        .map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());
    sqlx::query(
        "UPDATE meeting_triage SET status = ?, last_error = ?, privacy_route = ?,
         session_id = COALESCE(?, session_id), result_json = COALESCE(?, result_json),
         card_ids_json = CASE WHEN ? != '[]' THEN ? ELSE card_ids_json END,
         truncated = CASE WHEN ? = 1 THEN 1 ELSE truncated END,
         orchestrator_name = COALESCE(?, orchestrator_name),
         completed_at = CASE WHEN ? IN ('completed','failed','skipped')
           THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE completed_at END,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(last_error)
    .bind(privacy_route)
    .bind(session_id)
    .bind(result_json)
    .bind(&cards)
    .bind(&cards)
    .bind(truncated as i64)
    .bind(orchestrator_name)
    .bind(status.as_str())
    .bind(triage_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct ExtractedTodo {
    title: String,
    context: String,
}

fn parse_todos(value: &serde_json::Value) -> Vec<ExtractedTodo> {
    value
        .get("todos")
        .and_then(|t| t.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|t| {
                    let title = t.get("title")?.as_str()?.trim().to_string();
                    if title.is_empty() {
                        return None;
                    }
                    let context = t
                        .get("context")
                        .and_then(|c| c.as_str())
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    Some(ExtractedTodo { title, context })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn default_column_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<String, String> {
    let first: Option<String> = sqlx::query_scalar(
        "SELECT id FROM board_columns WHERE project_id = ? ORDER BY position ASC LIMIT 1",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    first.ok_or_else(|| "No columns exist for this project. Create columns first.".to_string())
}

struct NewCardFeed {
    card_id: String,
    title: String,
}

async fn materialize_cards_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    triage: &TriageRow,
    todos: &[ExtractedTodo],
    note_id: &str,
) -> Result<(Vec<String>, Vec<NewCardFeed>), String> {
    if todos.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let column_id = default_column_id_tx(tx, project_id).await?;
    let mut card_ids = Vec::new();
    let mut new_cards = Vec::new();
    let capped = todos.iter().take(MAX_MEETING_TODOS);
    for (index, todo) in capped.enumerate() {
        let key = item_key(index, &todo.title);
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT card_id FROM meeting_triage_items WHERE triage_id = ? AND item_key = ?",
        )
        .bind(&triage.id)
        .bind(&key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(card_id) = existing.filter(|id| !id.is_empty()) {
            card_ids.push(card_id);
            continue;
        }
        let card_id = stable_card_id(&triage.id, &key);
        let metadata = serde_json::json!({
            "created_by_agent": "henry",
            "source": "meeting_triage",
            "source_note_id": note_id,
            "source_triage_id": triage.id,
            "source_item_key": key,
            "source_recording_id": triage.recording_id,
        });
        let metadata_str = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;
        let max_pos: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(position) FROM cards WHERE column_id = ? AND archived_at IS NULL",
        )
        .bind(&column_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        let position = max_pos.unwrap_or(-1) + 1;
        let description = format!("{}\n\n— from meeting triage on this project", todo.context);
        sqlx::query(
            "INSERT INTO cards (id, project_id, card_type, title, description, column_id, position, created_by, metadata_json)
             VALUES (?, ?, 'standard', ?, ?, ?, ?, 'henry', ?)
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(&card_id)
        .bind(project_id)
        .bind(&todo.title)
        .bind(&description)
        .bind(&column_id)
        .bind(position)
        .bind(&metadata_str)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO meeting_triage_items (triage_id, item_key, title, context, card_id)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(triage_id, item_key) DO UPDATE SET
               card_id = excluded.card_id,
               title = excluded.title,
               context = excluded.context",
        )
        .bind(&triage.id)
        .bind(&key)
        .bind(&todo.title)
        .bind(&todo.context)
        .bind(&card_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        card_ids.push(card_id.clone());
        new_cards.push(NewCardFeed {
            card_id,
            title: todo.title.clone(),
        });
    }
    Ok((card_ids, new_cards))
}

struct ApplyInput<'a> {
    pool: &'a Pool<Sqlite>,
    brain: Option<&'a crate::brain_handle::SafeBrain>,
    project: &'a Project,
    row: &'a TriageRow,
    parsed: &'a serde_json::Value,
    provenance: &'a meeting_writeup::WriteupProvenance,
    user_notes: Option<&'a str>,
    transcript_only: &'a str,
    truncated: bool,
    session_id: Option<&'a str>,
    privacy_route: Option<&'a str>,
    orchestrator_name: Option<&'a str>,
}

fn post_commit_apply_side_effects(input: &ApplyInput<'_>, new_cards: &[NewCardFeed]) {
    if !new_cards.is_empty() {
        crate::events::emit(crate::events::project_changed(&input.project.id, "cards"));
    }
    if let Some(brain) = input.brain {
        crate::brain_feed::spawn_drain(brain.clone(), input.pool.clone());
    }
}

async fn apply_extraction_effects(input: ApplyInput<'_>) -> Result<Vec<String>, String> {
    if !meeting_writeup::valid_extraction(input.parsed) {
        return Err("invalid meeting extraction; refusing to complete triage".to_string());
    }
    let todos = parse_todos(input.parsed);
    let summary_body = input
        .parsed
        .get("summary_markdown")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|summary| {
            meeting_writeup::build_structured_body(
                summary,
                input.provenance,
                input.user_notes,
                input.transcript_only,
                input.truncated,
            )
        });

    let note_title = project_notes::get_note(input.pool, &input.project.id, &input.row.note_id)
        .await
        .ok()
        .flatten()
        .and_then(|n| n.title);

    let mut tx = input
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;

    if let Some(body) = &summary_body {
        project_notes::update_note_body_tx(&mut tx, &input.row.note_id, &input.project.id, body)
            .await?;
    }

    let (card_ids, new_cards) = materialize_cards_in_tx(
        &mut tx,
        &input.project.id,
        input.row,
        &todos,
        &input.row.note_id,
    )
    .await?;

    if let Some(body) = &summary_body {
        project_notes::enqueue_note_catalogue_in_tx(
            &mut tx,
            &input.project.id,
            &input.row.note_id,
            note_title.as_deref(),
            body,
        )
        .await?;
    }
    for card in &new_cards {
        crate::brain_feed::feed_for_project_in_tx(
            &mut tx,
            crate::brain_feed::FeedSource::Card,
            crate::brain_feed::feed_key("card", &card.card_id, "created"),
            &input.project.id,
            format!("Card '{}' (standard) created.", card.title),
        )
        .await?;
    }

    let result_json = serde_json::to_string(input.parsed).map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE meeting_triage SET status = 'completed', last_error = NULL,
         result_json = COALESCE(result_json, ?), card_ids_json = ?,
         truncated = CASE WHEN ? = 1 THEN 1 ELSE truncated END,
         session_id = COALESCE(?, session_id), privacy_route = COALESCE(?, privacy_route),
         orchestrator_name = COALESCE(?, orchestrator_name),
         completed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?",
    )
    .bind(&result_json)
    .bind(serde_json::to_string(&card_ids).unwrap_or_else(|_| "[]".into()))
    .bind(input.truncated as i64)
    .bind(input.session_id)
    .bind(input.privacy_route)
    .bind(input.orchestrator_name)
    .bind(&input.row.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    post_commit_apply_side_effects(&input, &new_cards);

    if summary_body.is_some() {
        if let Ok(Some(updated)) =
            project_notes::get_note(input.pool, &input.project.id, &input.row.note_id).await
        {
            project_notes::write_note_export(
                &input.project.slug,
                &input.project.name,
                &updated,
                Some("meeting"),
                Some(input.provenance),
            )
            .await;
            crate::events::emit(crate::events::project_changed(&input.project.id, "notes"));
        }
    }

    Ok(card_ids)
}

#[allow(clippy::too_many_arguments)]
async fn persist_canonical_result(
    pool: &Pool<Sqlite>,
    triage_id: &str,
    result_json: &str,
    session_id: Option<&str>,
    privacy_route: Option<&str>,
    truncated: bool,
    orchestrator_name: Option<&str>,
    provenance: &meeting_writeup::WriteupProvenance,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE meeting_triage SET result_json = ?,
         session_id = COALESCE(?, session_id),
         privacy_route = COALESCE(?, privacy_route),
         writeup_provider = ?, writeup_model = ?, writeup_privacy = ?,
         truncated = CASE WHEN ? = 1 THEN 1 ELSE truncated END,
         orchestrator_name = COALESCE(?, orchestrator_name),
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?",
    )
    .bind(result_json)
    .bind(session_id)
    .bind(privacy_route)
    .bind(&provenance.provider)
    .bind(&provenance.model)
    .bind(meeting_writeup::writeup_privacy_tag(provenance.privacy))
    .bind(truncated as i64)
    .bind(orchestrator_name)
    .bind(triage_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Old daemons may have cached syntactically valid but unusable JSON. Drop
/// that cache before the normal inference path so Retry can actually recover.
async fn validated_cached_result(
    pool: &Pool<Sqlite>,
    row: &TriageRow,
) -> Result<Option<serde_json::Value>, String> {
    let Some(text) = row.result_json.as_deref() else {
        return Ok(None);
    };
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if meeting_writeup::valid_extraction(&value) {
            return Ok(Some(value));
        }
    }
    sqlx::query(
        "UPDATE meeting_triage SET result_json = NULL,
         writeup_provider = NULL, writeup_model = NULL, writeup_privacy = NULL
         WHERE id = ?",
    )
    .bind(&row.id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    tracing::warn!(triage = %row.id, "discarded malformed cached meeting extraction");
    Ok(None)
}

pub async fn process_triage(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    triage_id: &str,
) -> Result<(), String> {
    process_triage_with_plan(pool, brain, triage_id, None).await
}

// Supplying a resolved plan keeps routing tests independent of mutable global
// config, mesh discovery and installed local models. Production resolves live.
async fn process_triage_with_plan(
    pool: &Pool<Sqlite>,
    brain: Option<&crate::brain_handle::SafeBrain>,
    triage_id: &str,
    resolved_plan: Option<MeetingWriteupPlan>,
) -> Result<(), String> {
    let Some(row) = fetch_by_id(pool, triage_id).await? else {
        return Ok(());
    };
    let project = crate::projects::get_project(pool, &row.project_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} missing for triage", row.project_id))?;
    let note = project_notes::get_note(pool, &project.id, &row.note_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("note {} missing for triage", row.note_id))?;
    let transcript_body = note.body.clone();
    let orchestrator_name = row
        .orchestrator_name
        .clone()
        .unwrap_or_else(|| load_orchestrator_persona().display_name());
    let (user_notes, transcript_only) = split_meeting_body(&transcript_body);

    if let Some(parsed) = validated_cached_result(pool, &row).await? {
        let (manager, session) = ensure_triage_session(
            &project.id,
            &row.recording_id,
            &row.note_id,
            &project.name,
            row.idempotent_recording,
        )
        .await?;
        let _ = manager;
        let provenance = provenance_from_row(&row);
        let card_ids = apply_extraction_effects(ApplyInput {
            pool,
            brain,
            project: &project,
            row: &row,
            parsed: &parsed,
            provenance: &provenance,
            user_notes: user_notes.as_deref(),
            transcript_only: &transcript_only,
            truncated: row.truncated,
            session_id: Some(&session.id),
            privacy_route: row.privacy_route.as_deref(),
            orchestrator_name: Some(&orchestrator_name),
        })
        .await?;
        tracing::info!(
            project = %project.id,
            triage = %triage_id,
            cards = card_ids.len(),
            "meeting triage completed from persisted result"
        );
        return Ok(());
    }

    let mut plan = resolved_plan.unwrap_or_else(|| {
        meeting_writeup::resolve_live_writeup_plan(crate::config::Config::global())
    });
    let route_label = privacy_route_label(&plan);
    if matches!(plan, MeetingWriteupPlan::Skip) {
        mark_status(
            pool,
            triage_id,
            TriageStatus::Skipped,
            Some("meeting_writeup_local_only and no local/mesh route"),
            route_label.as_deref(),
            row.session_id.as_deref(),
            None,
            None,
            false,
            Some(&orchestrator_name),
        )
        .await?;
        tracing::info!(project = %project.id, triage = %triage_id, "meeting triage skipped (privacy)");
        return Ok(());
    }

    let (manager, session) = ensure_triage_session(
        &project.id,
        &row.recording_id,
        &row.note_id,
        &project.name,
        row.idempotent_recording,
    )
    .await?;
    mark_status(
        pool,
        triage_id,
        TriageStatus::Running,
        None,
        route_label.as_deref(),
        Some(&session.id),
        None,
        None,
        row.truncated,
        Some(&orchestrator_name),
    )
    .await?;

    let context_limit = meeting_writeup::context_limit_for_plan(&plan);
    let (prompt, truncated) = meeting_writeup::build_extraction_prompt(
        &project.name,
        user_notes.as_deref(),
        &transcript_only,
        context_limit,
    );

    let session_name = triage_session_name(&project.id, &row.recording_id);
    let text =
        match meeting_writeup::run_writeup_plan_with_session(&plan, &prompt, &session_name).await {
            Ok(text) => text,
            Err(e) => {
                let config = crate::config::Config::global();
                let local_only = meeting_writeup::meeting_writeup_local_only(config);
                let session_fallback = match (local_only, &plan) {
                    (false, MeetingWriteupPlan::ViaMesh { .. }) => {
                        match (config.get_goose_provider(), config.get_goose_model()) {
                            (Ok(provider), Ok(model)) => {
                                Some(MeetingWriteupPlan::ViaSession { provider, model })
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let Some(fallback) = session_fallback else {
                    mark_status(
                        pool,
                        triage_id,
                        TriageStatus::Failed,
                        Some(&e),
                        route_label.as_deref(),
                        Some(&session.id),
                        None,
                        None,
                        truncated,
                        Some(&orchestrator_name),
                    )
                    .await?;
                    return Ok(());
                };
                plan = fallback;
                match meeting_writeup::run_writeup_plan_with_session(&plan, &prompt, &session_name)
                    .await
                {
                    Ok(text) => text,
                    Err(e) => {
                        mark_status(
                            pool,
                            triage_id,
                            TriageStatus::Failed,
                            Some(&e),
                            privacy_route_label(&plan).as_deref(),
                            Some(&session.id),
                            None,
                            None,
                            truncated,
                            Some(&orchestrator_name),
                        )
                        .await?;
                        return Ok(());
                    }
                }
            }
        };

    persist_session_turn(&manager, &session.id, &prompt, &text).await?;

    let parsed = meeting_writeup::parse_extraction_response(&text).ok_or_else(|| {
        "model response did not contain a valid meeting summary and todo list".to_string()
    })?;
    let result_json = serde_json::to_string(&parsed).map_err(|e| e.to_string())?;

    let provenance = match &plan {
        MeetingWriteupPlan::ViaMesh {
            route,
            provider,
            model,
        } => meeting_writeup::provenance_for_mesh_route(provider, model, route),
        MeetingWriteupPlan::ViaSession { provider, model } => {
            meeting_writeup::provenance_for_session(provider, model)
        }
        MeetingWriteupPlan::Skip => return Ok(()),
    };

    persist_canonical_result(
        pool,
        triage_id,
        &result_json,
        Some(&session.id),
        privacy_route_label(&plan).as_deref(),
        truncated,
        Some(&orchestrator_name),
        &provenance,
    )
    .await?;

    let card_ids = apply_extraction_effects(ApplyInput {
        pool,
        brain,
        project: &project,
        row: &row,
        parsed: &parsed,
        provenance: &provenance,
        user_notes: user_notes.as_deref(),
        transcript_only: &transcript_only,
        truncated,
        session_id: Some(&session.id),
        privacy_route: privacy_route_label(&plan).as_deref(),
        orchestrator_name: Some(&orchestrator_name),
    })
    .await?;

    tracing::info!(
        project = %project.id,
        triage = %triage_id,
        cards = card_ids.len(),
        "meeting triage completed"
    );
    Ok(())
}

pub async fn drain(pool: &Pool<Sqlite>, brain: Option<&crate::brain_handle::SafeBrain>) {
    // Coalesce overlapping spawn_drain / periodic passes — one drain at a time.
    let _guard = drain_coalesce().lock().await;
    while let Ok(Some(id)) = claim_next(pool).await {
        if let Err(e) = process_triage(pool, brain, &id).await {
            tracing::warn!(triage = %id, "meeting triage failed: {e}");
            let _ = mark_status(
                pool,
                &id,
                TriageStatus::Failed,
                Some(&e),
                None,
                None,
                None,
                None,
                false,
                None,
            )
            .await;
        }
    }
}

pub fn spawn_drain(pool: Pool<Sqlite>, brain: Option<crate::brain_handle::SafeBrain>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        drain(&pool, brain.as_ref()).await;
    });
}

/// Periodic drain for startup replay. Abandoned-row recovery is
/// [`recover_abandoned_at_startup`] at daemon boot, not here.
pub fn spawn_periodic_drain(pool: Pool<Sqlite>, brain: Option<crate::brain_handle::SafeBrain>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        loop {
            drain(&pool, brain.as_ref()).await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{CreateProject, PERSONAL_PROJECT_ID};
    use crate::session::spectral_schema::init_spectral_db;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        apply_schema(&pool).await.unwrap();
        pool
    }

    async fn test_project(pool: &Pool<Sqlite>, name: &str) -> String {
        if name == "Personal" {
            return PERSONAL_PROJECT_ID.to_string();
        }
        crate::projects::create_project(
            pool,
            CreateProject {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn idempotent_repair_catalogues_latest_committed_note() {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("repair.db"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(2));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .min_connections(2)
            .connect_with(options)
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        let project = test_project(&pool, "Repair ordering").await;
        let saved = save_meeting_note(
            &pool,
            None,
            &project,
            "repair",
            true,
            None,
            "raw transcript",
        )
        .await
        .unwrap();
        let mut writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        project_notes::update_note_body_tx(
            &mut writer,
            &saved.note.id,
            &project,
            "Enhanced summary plus transcript",
        )
        .await
        .unwrap();
        project_notes::enqueue_note_catalogue_in_tx(
            &mut writer,
            &project,
            &saved.note.id,
            None,
            "Enhanced summary plus transcript",
        )
        .await
        .unwrap();
        // Start the retry while the enhancement is still uncommitted on a
        // separate WAL connection. It must wait, then read the new body.
        let repair = repair_note_catalogue(&pool, None, &project, &saved.note.id);
        tokio::pin!(repair);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut repair)
                .await
                .is_err()
        );
        writer.commit().await.unwrap();
        let repaired = repair.await.unwrap();
        assert_eq!(repaired.body, "Enhanced summary plus transcript");
        let content: String =
            sqlx::query_scalar("SELECT content FROM brain_feed_outbox WHERE memory_key = ?")
                .bind(format!("note:{}:{}", project, saved.note.id))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(content.contains("Enhanced summary plus transcript"));
        let replay = save_meeting_note(
            &pool,
            None,
            &project,
            "repair",
            true,
            None,
            "raw transcript",
        )
        .await
        .unwrap();
        assert_eq!(replay.note.body, repaired.body);
        assert_eq!(replay.note.id, saved.note.id);
    }

    #[test]
    fn split_meeting_body_inverts_ui_compose() {
        let body = "## Your notes\n\npricing\n\n## Transcript\n\nsaid hello";
        let (notes, transcript) = split_meeting_body(body);
        assert_eq!(notes.as_deref(), Some("pricing"));
        assert_eq!(transcript, "said hello");
    }

    #[test]
    fn item_key_is_stable_and_index_sensitive() {
        let a = item_key(0, "  Ship   the Thing ");
        let b = item_key(0, "ship the thing");
        let c = item_key(1, "ship the thing");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn stable_triage_ids_are_deterministic() {
        let (t1, n1) = stable_triage_ids("proj", "rec");
        let (t2, n2) = stable_triage_ids("proj", "rec");
        assert_eq!(t1, t2);
        assert_eq!(n1, n2);
        assert_ne!(t1, n1);
    }

    #[test]
    fn resolve_recording_id_marks_legacy_non_idempotent() {
        let (id, idempotent) = resolve_recording_id(None);
        assert!(!id.is_empty());
        assert!(!idempotent);
        let (stable, idempotent) = resolve_recording_id(Some("client-rec"));
        assert_eq!(stable, "client-rec");
        assert!(idempotent);
    }

    #[tokio::test]
    async fn save_meeting_note_is_idempotent() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let body = "## Transcript\n\nhello";
        let first = save_meeting_note(&pool, None, &project, "rec-1", true, None, body)
            .await
            .unwrap();
        assert!(first.triage.created);
        let second = save_meeting_note(&pool, None, &project, "rec-1", true, None, body)
            .await
            .unwrap();
        assert!(!second.triage.created);
        assert_eq!(first.note.id, second.note.id);
        assert_eq!(first.triage.id, second.triage.id);
    }

    #[tokio::test]
    async fn concurrent_save_produces_one_note() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let body = "## Transcript\n\nconcurrent";
        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let pid = project.clone();
        let (a, b) = tokio::join!(
            save_meeting_note(&pool_a, None, &pid, "rec-concurrent", true, None, body),
            save_meeting_note(&pool_b, None, &pid, "rec-concurrent", true, None, body),
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.note.id, b.note.id);
        assert_eq!(a.triage.id, b.triage.id);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_notes WHERE project_id = ?")
                .bind(&project)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn save_meeting_note_rejects_body_conflict() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-2", true, None, "first body")
            .await
            .unwrap();
        let err = save_meeting_note(&pool, None, &project, "rec-2", true, None, "second body")
            .await
            .unwrap_err();
        assert!(matches!(err, MeetingSaveError::BodyConflict { .. }));
    }

    #[tokio::test]
    async fn legacy_recording_ids_are_not_idempotent() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let first = save_meeting_note(&pool, None, &project, "legacy-a", false, None, "one")
            .await
            .unwrap();
        let second = save_meeting_note(&pool, None, &project, "legacy-b", false, None, "two")
            .await
            .unwrap();
        assert!(!first.triage.idempotent_recording);
        assert_ne!(first.note.id, second.note.id);
    }

    #[tokio::test]
    async fn retry_triage_only_from_failed_or_skipped() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-3", true, None, "body")
            .await
            .unwrap();
        sqlx::query("UPDATE meeting_triage SET status = 'failed' WHERE recording_id = 'rec-3'")
            .execute(&pool)
            .await
            .unwrap();
        let retried = retry_triage(&pool, &project, "rec-3")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retried.status, TriageStatus::Pending);
        assert_eq!(retried.attempt_count, 0);
    }

    #[tokio::test]
    async fn claim_next_ignores_failed_without_explicit_retry() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-fail", true, None, "body")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE meeting_triage SET status = 'failed', attempt_count = 3 WHERE recording_id = 'rec-fail'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(claim_next(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn claim_next_ignores_exhausted_failed() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-exhaust", true, None, "body")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE meeting_triage SET status = 'failed', attempt_count = 5 WHERE recording_id = 'rec-exhaust'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(claim_next(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reset_abandoned_running_rows_reclaims_stale_running() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-run", true, None, "body")
            .await
            .unwrap();
        sqlx::query("UPDATE meeting_triage SET status = 'running' WHERE recording_id = 'rec-run'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(reset_abandoned_running_rows(&pool).await.unwrap(), (1, 0));
        let row = fetch_by_recording(&pool, &project, "rec-run")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, TriageStatus::Pending);
    }

    #[tokio::test]
    async fn reset_abandoned_exhausted_running_marks_failed() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-exhaust-run", true, None, "body")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE meeting_triage SET status = 'running', attempt_count = 5
             WHERE recording_id = 'rec-exhaust-run'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(reset_abandoned_running_rows(&pool).await.unwrap(), (0, 1));
        let row = fetch_by_recording(&pool, &project, "rec-exhaust-run")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, TriageStatus::Failed);
        assert_eq!(row.last_error.as_deref(), Some(EXHAUSTED_RUNNING_ERROR));
        assert!(claim_next(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn concurrent_drain_does_not_reset_active_work() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        save_meeting_note(&pool, None, &project, "rec-active", true, None, "body")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE meeting_triage SET status = 'running' WHERE recording_id = 'rec-active'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let ((), ()) = tokio::join!(drain(&pool_a, None), drain(&pool_b, None));
        let row = fetch_by_recording(&pool, &project, "rec-active")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.status,
            TriageStatus::Running,
            "POST/coalesced drains must not reclaim in-flight rows"
        );
    }

    #[tokio::test]
    async fn persist_canonical_result_survives_before_apply() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let saved = save_meeting_note(
            &pool,
            None,
            &project,
            "rec-cache",
            true,
            None,
            "## Transcript\n\nhi",
        )
        .await
        .unwrap();
        let result = serde_json::json!({
            "summary_markdown": "Summary here",
            "todos": [{"title": "Follow up", "context": "with team"}]
        })
        .to_string();
        let provenance = meeting_writeup::WriteupProvenance {
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            privacy: meeting_writeup::WriteupPrivacy::CloudSession,
        };
        persist_canonical_result(
            &pool,
            &saved.triage.id,
            &result,
            None,
            Some("cloud_session"),
            false,
            Some("Aria"),
            &provenance,
        )
        .await
        .unwrap();
        let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(row.result_json.as_deref(), Some(result.as_str()));
        assert_eq!(row.writeup_provider.as_deref(), Some("anthropic"));
        assert_eq!(row.writeup_model.as_deref(), Some("claude-sonnet"));
        assert_eq!(row.writeup_privacy.as_deref(), Some("cloud_session"));
        assert_eq!(row.status, TriageStatus::Pending);
    }

    #[tokio::test]
    async fn malformed_cached_extraction_is_discarded_and_valid_replacement_reused() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Cache validation").await;
        let saved = save_meeting_note(
            &pool,
            None,
            &project,
            "rec-invalid",
            true,
            None,
            "transcript",
        )
        .await
        .unwrap();
        for malformed in [
            "{}",
            "not JSON",
            r#"{"summary_markdown":"ok","todos":[{"title":" "}]}"#,
        ] {
            sqlx::query(
                "UPDATE meeting_triage SET result_json = ?, writeup_provider = 'old',
                 writeup_model = 'old-model', writeup_privacy = 'cloud_session' WHERE id = ?",
            )
            .bind(malformed)
            .bind(&saved.triage.id)
            .execute(&pool)
            .await
            .unwrap();
            let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
            assert!(validated_cached_result(&pool, &row)
                .await
                .unwrap()
                .is_none());
            let cleared = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
            assert!(cleared.result_json.is_none());
            assert!(cleared.writeup_provider.is_none());
            assert!(cleared.writeup_model.is_none());
            assert!(cleared.writeup_privacy.is_none());
            let null_columns: i64 = sqlx::query_scalar(
                "SELECT result_json IS NULL AND writeup_provider IS NULL AND
                 writeup_model IS NULL AND writeup_privacy IS NULL
                 FROM meeting_triage WHERE id = ?",
            )
            .bind(&saved.triage.id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(null_columns, 1, "cache and provenance must be SQL NULL");
            assert_ne!(cleared.status, TriageStatus::Completed);
            assert!(validated_cached_result(&pool, &cleared)
                .await
                .unwrap()
                .is_none());
        }
        let valid = serde_json::json!({"summary_markdown": "Summary", "todos": []});
        let provenance = meeting_writeup::provenance_unknown();
        persist_canonical_result(
            &pool,
            &saved.triage.id,
            &valid.to_string(),
            None,
            None,
            false,
            None,
            &provenance,
        )
        .await
        .unwrap();
        let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(
            validated_cached_result(&pool, &row).await.unwrap(),
            Some(valid)
        );
        assert_eq!(
            fetch_by_id(&pool, &row.id)
                .await
                .unwrap()
                .unwrap()
                .result_json,
            row.result_json
        );
    }

    #[test]
    fn provenance_from_row_restores_stored_writeup_fields() {
        let row = TriageRow {
            id: "t".into(),
            project_id: "p".into(),
            recording_id: "r".into(),
            note_id: "n".into(),
            body_hash: "h".into(),
            status: TriageStatus::Pending,
            attempt_count: 0,
            last_error: None,
            privacy_route: None,
            session_id: None,
            result_json: Some("{}".into()),
            writeup_provider: Some("anthropic".into()),
            writeup_model: Some("claude-sonnet".into()),
            writeup_privacy: Some("cloud_session".into()),
            card_ids_json: "[]".into(),
            truncated: false,
            idempotent_recording: true,
            orchestrator_name: None,
        };
        let p = provenance_from_row(&row);
        assert_eq!(p.provider, "anthropic");
        assert_eq!(p.model, "claude-sonnet");
        assert_eq!(p.privacy, meeting_writeup::WriteupPrivacy::CloudSession);
    }

    #[test]
    fn provenance_from_row_without_storage_is_unknown_not_local() {
        let row = TriageRow {
            id: "t".into(),
            project_id: "p".into(),
            recording_id: "r".into(),
            note_id: "n".into(),
            body_hash: "h".into(),
            status: TriageStatus::Pending,
            attempt_count: 0,
            last_error: None,
            privacy_route: None,
            session_id: None,
            result_json: Some("{}".into()),
            writeup_provider: None,
            writeup_model: None,
            writeup_privacy: None,
            card_ids_json: "[]".into(),
            truncated: false,
            idempotent_recording: true,
            orchestrator_name: None,
        };
        let p = provenance_from_row(&row);
        assert_eq!(p.privacy, meeting_writeup::WriteupPrivacy::Unknown);
        assert_eq!(p.provider, "unknown");
    }

    #[tokio::test]
    async fn apply_extraction_effects_empty_todos_completes_without_board_columns() {
        let pool = test_pool().await;
        let project_id = test_project(&pool, "No Board").await;
        sqlx::query("DELETE FROM board_columns WHERE project_id = ?")
            .bind(&project_id)
            .execute(&pool)
            .await
            .unwrap();
        let col_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM board_columns WHERE project_id = ?")
                .bind(&project_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(col_count, 0);

        let saved = save_meeting_note(
            &pool,
            None,
            &project_id,
            "rec-empty",
            true,
            None,
            "## Transcript\n\nnothing actionable",
        )
        .await
        .unwrap();
        let project = crate::projects::get_project(&pool, &project_id)
            .await
            .unwrap()
            .unwrap();
        let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        let parsed = serde_json::json!({
            "summary_markdown": "Summary only — no commitments",
            "todos": []
        });
        let provenance = meeting_writeup::WriteupProvenance {
            provider: "test".into(),
            model: "test".into(),
            privacy: meeting_writeup::WriteupPrivacy::LocalOnly,
        };
        let invalid = serde_json::json!({"error": "upstream failed"});
        let rejected = apply_extraction_effects(ApplyInput {
            pool: &pool,
            brain: None,
            project: &project,
            row: &row,
            parsed: &invalid,
            provenance: &provenance,
            user_notes: None,
            transcript_only: "nothing actionable",
            truncated: false,
            session_id: None,
            privacy_route: Some("local_only"),
            orchestrator_name: Some("Aria"),
        })
        .await;
        assert!(rejected.is_err());
        let unchanged = fetch_by_id(&pool, &row.id).await.unwrap().unwrap();
        assert_eq!(unchanged.status, TriageStatus::Pending);
        assert!(unchanged.result_json.is_none());
        assert_eq!(unchanged.card_ids_json, "[]");

        let card_ids = apply_extraction_effects(ApplyInput {
            pool: &pool,
            brain: None,
            project: &project,
            row: &row,
            parsed: &parsed,
            provenance: &provenance,
            user_notes: None,
            transcript_only: "nothing actionable",
            truncated: false,
            session_id: None,
            privacy_route: Some("local_only"),
            orchestrator_name: Some("Aria"),
        })
        .await
        .unwrap();
        assert!(card_ids.is_empty());
        let updated = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TriageStatus::Completed);
        assert_eq!(updated.card_ids_json, "[]");
    }

    #[tokio::test]
    async fn apply_extraction_effects_materializes_cards_in_transaction() {
        let pool = test_pool().await;
        let project_id = test_project(&pool, "Record QA").await;
        let saved = save_meeting_note(
            &pool,
            None,
            &project_id,
            "rec-apply",
            true,
            None,
            "## Transcript\n\nhi",
        )
        .await
        .unwrap();
        let project = crate::projects::get_project(&pool, &project_id)
            .await
            .unwrap()
            .unwrap();
        let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        let parsed = serde_json::json!({
            "summary_markdown": "Summary here",
            "todos": [{"title": "Follow up", "context": "with team"}]
        });
        let provenance = meeting_writeup::WriteupProvenance {
            provider: "test".into(),
            model: "test".into(),
            privacy: meeting_writeup::WriteupPrivacy::LocalOnly,
        };
        let card_ids = apply_extraction_effects(ApplyInput {
            pool: &pool,
            brain: None,
            project: &project,
            row: &row,
            parsed: &parsed,
            provenance: &provenance,
            user_notes: None,
            transcript_only: "hi",
            truncated: false,
            session_id: None,
            privacy_route: Some("local_only"),
            orchestrator_name: Some("Aria"),
        })
        .await
        .unwrap();
        assert_eq!(card_ids.len(), 1);
        let updated = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(updated.status, TriageStatus::Completed);
        let note_key = format!("note:{}:{}", project_id, saved.note.id);
        let card_key = crate::brain_feed::feed_key("card", &card_ids[0], "created");
        assert!(outbox_has_key(&pool, &note_key).await);
        assert!(outbox_has_key(&pool, &card_key).await);
    }

    async fn outbox_has_key(pool: &Pool<Sqlite>, memory_key: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT 1 FROM brain_feed_outbox WHERE memory_key = ?")
            .bind(memory_key)
            .fetch_optional(pool)
            .await
            .unwrap()
            .is_some()
    }

    #[tokio::test]
    async fn save_meeting_note_enqueues_catalogue_with_durable_row() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let saved = save_meeting_note(
            &pool,
            None,
            &project,
            "rec-outbox",
            true,
            None,
            "## Transcript\n\nqueued",
        )
        .await
        .unwrap();
        let memory_key = format!("note:{}:{}", project, saved.note.id);
        assert!(outbox_has_key(&pool, &memory_key).await);
    }

    #[tokio::test]
    async fn idempotent_save_repairs_missing_catalogue_row() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let body = "## Transcript\n\nrepair me";
        let first = save_meeting_note(&pool, None, &project, "rec-repair", true, None, body)
            .await
            .unwrap();
        let memory_key = format!("note:{}:{}", project, first.note.id);
        sqlx::query("DELETE FROM brain_feed_outbox WHERE memory_key = ?")
            .bind(&memory_key)
            .execute(&pool)
            .await
            .unwrap();
        save_meeting_note(&pool, None, &project, "rec-repair", true, None, body)
            .await
            .unwrap();
        assert!(outbox_has_key(&pool, &memory_key).await);
    }

    #[tokio::test]
    async fn nullable_triage_metadata_roundtrips_absent_and_present_values() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Nullable triage metadata").await;
        let saved = save_meeting_note(&pool, None, &project, "rec-null", true, None, "body")
            .await
            .unwrap();
        sqlx::query("UPDATE meeting_triage SET orchestrator_name = NULL WHERE id = ?")
            .bind(&saved.triage.id)
            .execute(&pool)
            .await
            .unwrap();
        let absent = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert!(absent.orchestrator_name.is_none());
        assert!(absent.writeup_provider.is_none());
        assert!(absent.writeup_model.is_none());
        assert!(absent.writeup_privacy.is_none());
        assert_eq!(provenance_from_row(&absent).provider, "unknown");
        sqlx::query(
            "UPDATE meeting_triage SET orchestrator_name = 'Fixture Orchestrator',
             writeup_provider = 'fixture-provider', writeup_model = 'fixture-model',
             writeup_privacy = 'local_only' WHERE id = ?",
        )
        .bind(&saved.triage.id)
        .execute(&pool)
        .await
        .unwrap();
        let present = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(
            present.orchestrator_name.as_deref(),
            Some("Fixture Orchestrator")
        );
        assert_eq!(
            present.writeup_provider.as_deref(),
            Some("fixture-provider")
        );
        assert_eq!(present.writeup_model.as_deref(), Some("fixture-model"));
        assert_eq!(present.writeup_privacy.as_deref(), Some("local_only"));
    }

    #[tokio::test]
    async fn process_triage_skips_without_cloud_when_local_only() {
        let pool = test_pool().await;
        let project = test_project(&pool, "Record QA").await;
        let saved = save_meeting_note(&pool, None, &project, "rec-skip", true, None, "body")
            .await
            .unwrap();
        sqlx::query("UPDATE meeting_triage SET status = 'running' WHERE id = ?")
            .bind(&saved.triage.id)
            .execute(&pool)
            .await
            .unwrap();
        // An explicit unavailable-local fixture is essential: local_only alone
        // still legitimately selects a configured Ollama route. Never alter the
        // user's/global defaults or attempt a live provider call in this test.
        let cloud_pack = crate::cost_router::packs::ModelPack {
            provider: "fixture-cloud-provider".to_string(),
            model: "fixture-cloud-model".to_string(),
        };
        let plan = meeting_writeup::resolve_meeting_writeup_plan(
            false,
            None,
            true,
            &cloud_pack,
            Some((
                "fixture-session-provider".into(),
                "fixture-session-model".into(),
            )),
        );
        assert_eq!(plan, MeetingWriteupPlan::Skip);
        process_triage_with_plan(&pool, None, &saved.triage.id, Some(plan))
            .await
            .unwrap();
        let row = fetch_by_id(&pool, &saved.triage.id).await.unwrap().unwrap();
        assert_eq!(row.status, TriageStatus::Skipped);
        assert_eq!(row.privacy_route.as_deref(), Some("skipped"));
        assert!(
            row.session_id.is_none(),
            "privacy skip must not create a session"
        );
        assert!(row.result_json.is_none());
        assert!(row.writeup_provider.is_none());
        assert_eq!(row.card_ids_json, "[]");
    }
}
