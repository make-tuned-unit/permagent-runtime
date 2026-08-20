//! Person-keyed meetings — the CRM log of time spent with someone.
//!
//! Name-search over notes is not a meeting record: a transcript that happens
//! to mention a display name is incidental. These rows are the first-class
//! log the person profile, the project People panel, and the Home calendar
//! card all read. Creating one best-effort writes Calendar.app; importing
//! matches Calendar.app titles back onto directory people.

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const MEETING_COLS: &str = "id, entity_uuid, title, starts_at, ends_at, notes, calendar_synced, \
     project_id, follow_up_at, follow_up_note, follow_up_done, calendar_uid, \
     created_at, updated_at";

/// Days without contact after which a person is "quiet" on the People tab
/// and in observe_app. Never-contacted counts as quiet.
pub const QUIET_AFTER_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonMeeting {
    pub id: String,
    pub entity_uuid: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub notes: String,
    pub calendar_synced: bool,
    pub project_id: Option<String>,
    pub follow_up_at: Option<String>,
    pub follow_up_note: String,
    pub follow_up_done: bool,
    pub calendar_uid: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A meeting plus the person's display name — Home calendar and project lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonMeetingWithName {
    pub id: String,
    pub entity_uuid: String,
    pub display_name: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub notes: String,
    pub calendar_synced: bool,
    pub project_id: Option<String>,
    pub follow_up_at: Option<String>,
    pub follow_up_note: String,
    pub follow_up_done: bool,
}

#[derive(Debug, Clone)]
pub struct NewMeeting {
    pub entity_uuid: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub notes: String,
    pub project_id: Option<String>,
    pub follow_up_at: Option<String>,
    pub follow_up_note: Option<String>,
    pub calendar_uid: Option<String>,
    /// Write-through to Calendar.app. Imported events set this false.
    pub sync_calendar: bool,
}

impl Default for NewMeeting {
    fn default() -> Self {
        Self {
            entity_uuid: String::new(),
            title: String::new(),
            starts_at: String::new(),
            ends_at: None,
            notes: String::new(),
            project_id: None,
            follow_up_at: None,
            follow_up_note: None,
            calendar_uid: None,
            sync_calendar: true,
        }
    }
}

/// An event read from Calendar.app (or a test double).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub uid: String,
    pub title: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

/// Parse an RFC-3339 instant. Reject anything else so a local `datetime-local`
/// value cannot silently become UTC midnight.
pub fn parse_rfc3339(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| format!("starts_at/ends_at must be RFC-3339 (got {value:?})"))
}

/// RFC-3339, or a calendar date `YYYY-MM-DD` which means 09:00 that local day.
pub fn parse_rfc3339_or_date(value: &str) -> Result<DateTime<Utc>, String> {
    let trimmed = value.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|_| format!("follow_up_at must be RFC-3339 or YYYY-MM-DD (got {value:?})"))?;
    let local = date
        .and_hms_opt(9, 0, 0)
        .ok_or_else(|| "follow_up_at date has no 09:00".to_string())?;
    Ok(Local
        .from_local_datetime(&local)
        .single()
        .unwrap_or_else(|| Local.from_utc_datetime(&local))
        .with_timezone(&Utc))
}

fn row_to_meeting(r: &sqlx::sqlite::SqliteRow) -> PersonMeeting {
    PersonMeeting {
        id: r.get("id"),
        entity_uuid: r.get("entity_uuid"),
        title: r.get("title"),
        starts_at: r.get("starts_at"),
        ends_at: r.get("ends_at"),
        notes: r.get("notes"),
        calendar_synced: r.get::<i64, _>("calendar_synced") != 0,
        project_id: r.get("project_id"),
        follow_up_at: r.get("follow_up_at"),
        follow_up_note: r.get("follow_up_note"),
        follow_up_done: r.get::<i64, _>("follow_up_done") != 0,
        calendar_uid: r.get("calendar_uid"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_named(r: &sqlx::sqlite::SqliteRow) -> PersonMeetingWithName {
    PersonMeetingWithName {
        id: r.get("id"),
        entity_uuid: r.get("entity_uuid"),
        display_name: r.get("display_name"),
        title: r.get("title"),
        starts_at: r.get("starts_at"),
        ends_at: r.get("ends_at"),
        notes: r.get("notes"),
        calendar_synced: r.get::<i64, _>("calendar_synced") != 0,
        project_id: r.get("project_id"),
        follow_up_at: r.get("follow_up_at"),
        follow_up_note: r.get("follow_up_note"),
        follow_up_done: r.get::<i64, _>("follow_up_done") != 0,
    }
}

pub async fn list_for_person(
    pool: &Pool<Sqlite>,
    entity_uuid: &str,
) -> Result<Vec<PersonMeeting>, String> {
    let rows = sqlx::query(&format!(
        "SELECT {MEETING_COLS} FROM person_meetings WHERE entity_uuid = ? \
         ORDER BY starts_at DESC"
    ))
    .bind(entity_uuid)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_meeting).collect())
}

pub async fn list_for_project(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<PersonMeetingWithName>, String> {
    let rows = sqlx::query(
        "SELECT m.id, m.entity_uuid, p.display_name, m.title, m.starts_at, m.ends_at, \
         m.notes, m.calendar_synced, m.project_id, m.follow_up_at, m.follow_up_note, \
         m.follow_up_done \
         FROM person_meetings m \
         JOIN people p ON p.entity_uuid = m.entity_uuid \
         WHERE m.project_id = ? \
         ORDER BY m.starts_at DESC \
         LIMIT 40",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_named).collect())
}

/// Meetings whose start falls in `[start, end)` (RFC-3339, UTC). Used by the
/// Home Calendar card to show today's people meetings alongside Calendar.app.
pub async fn list_in_range(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<PersonMeetingWithName>, String> {
    let rows = sqlx::query(
        "SELECT m.id, m.entity_uuid, p.display_name, m.title, m.starts_at, m.ends_at, \
         m.notes, m.calendar_synced, m.project_id, m.follow_up_at, m.follow_up_note, \
         m.follow_up_done \
         FROM person_meetings m \
         JOIN people p ON p.entity_uuid = m.entity_uuid \
         WHERE m.starts_at >= ? AND m.starts_at < ? \
         ORDER BY m.starts_at ASC",
    )
    .bind(start.to_rfc3339())
    .bind(end.to_rfc3339())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_named).collect())
}

/// Open follow-ups whose due instant falls in `[start, end)`.
pub async fn list_follow_ups_in_range(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<PersonMeetingWithName>, String> {
    let rows = sqlx::query(
        "SELECT m.id, m.entity_uuid, p.display_name, m.title, m.starts_at, m.ends_at, \
         m.notes, m.calendar_synced, m.project_id, m.follow_up_at, m.follow_up_note, \
         m.follow_up_done \
         FROM person_meetings m \
         JOIN people p ON p.entity_uuid = m.entity_uuid \
         WHERE m.follow_up_done = 0 \
           AND m.follow_up_at IS NOT NULL AND m.follow_up_at != '' \
           AND m.follow_up_at >= ? AND m.follow_up_at < ? \
         ORDER BY m.follow_up_at ASC",
    )
    .bind(start.to_rfc3339())
    .bind(end.to_rfc3339())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_named).collect())
}

/// Next open follow-up per person (soonest). Used by the directory chips.
pub async fn next_follow_up_by_person(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, String>, String> {
    let rows = sqlx::query(
        "SELECT entity_uuid, MIN(follow_up_at) AS next_at \
         FROM person_meetings \
         WHERE follow_up_done = 0 AND follow_up_at IS NOT NULL AND follow_up_at != '' \
         GROUP BY entity_uuid",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let uuid: String = r.get("entity_uuid");
            let at: Option<String> = r.get("next_at");
            at.map(|at| (uuid, at))
        })
        .collect())
}

/// Latest meeting start per person, for last-contact overlay after Decision A
/// clears the people-table column.
pub async fn latest_starts_by_person(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, String>, String> {
    let rows = sqlx::query(
        "SELECT entity_uuid, MAX(starts_at) AS latest \
         FROM person_meetings GROUP BY entity_uuid",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let uuid: String = r.get("entity_uuid");
            let latest: Option<String> = r.get("latest");
            latest.map(|t| (uuid, t))
        })
        .collect())
}

/// If a meeting is more recent than the (graph-overlaid) last_contact, use it.
/// Manual last_contact still wins when it is newer than any meeting.
pub fn merge_last_contact(people: &mut [crate::people::Person], latest: &HashMap<String, String>) {
    for person in people {
        let Some(meeting_at) = latest.get(&person.entity_uuid) else {
            continue;
        };
        let meeting_dt = parse_rfc3339(meeting_at).ok();
        let current_dt = person
            .last_contact_at
            .as_deref()
            .and_then(|s| parse_rfc3339(s).ok());
        let take_meeting = match (current_dt, meeting_dt) {
            (None, Some(_)) => true,
            (Some(cur), Some(meet)) => meet >= cur,
            _ => false,
        };
        if take_meeting {
            person.last_contact_at = Some(meeting_at.clone());
        }
    }
}

/// Local-calendar today, as a UTC `[start, end)` window.
pub fn local_today_utc_range() -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Local::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists");
    let start_local = Local.from_local_datetime(&midnight).single().unwrap_or(now);
    let start = start_local.with_timezone(&Utc);
    (start, start + Duration::days(1))
}

pub fn local_import_window() -> (DateTime<Utc>, DateTime<Utc>) {
    let (today, tomorrow) = local_today_utc_range();
    (today - Duration::days(14), tomorrow + Duration::days(7))
}

pub async fn create_meeting(pool: &Pool<Sqlite>, new: NewMeeting) -> Result<PersonMeeting, String> {
    let title = new.title.trim();
    if title.is_empty() {
        return Err("title must not be empty".into());
    }
    let start = parse_rfc3339(&new.starts_at)?;
    let end = match new
        .ends_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => parse_rfc3339(raw)?,
        None => start + Duration::hours(1),
    };
    if end <= start {
        return Err("ends_at must be after starts_at".into());
    }

    let exists: Option<String> =
        sqlx::query_scalar("SELECT entity_uuid FROM people WHERE entity_uuid = ?")
            .bind(&new.entity_uuid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err("Person not found".into());
    }

    let project_id = match new
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(pid) => {
            let found: Option<String> = sqlx::query_scalar("SELECT id FROM projects WHERE id = ?")
                .bind(pid)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?;
            if found.is_none() {
                return Err("Project not found".into());
            }
            Some(pid.to_string())
        }
        None => None,
    };

    let follow_up_at = match new
        .follow_up_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => Some(parse_rfc3339_or_date(raw)?.to_rfc3339()),
        None => None,
    };
    let follow_up_note = new
        .follow_up_note
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let calendar_uid = new
        .calendar_uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let id = Uuid::now_v7().to_string();
    let notes = new.notes.trim().to_string();
    sqlx::query(
        "INSERT INTO person_meetings \
         (id, entity_uuid, title, starts_at, ends_at, notes, calendar_synced, \
          project_id, follow_up_at, follow_up_note, follow_up_done, calendar_uid) \
         VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(&new.entity_uuid)
    .bind(title)
    .bind(start.to_rfc3339())
    .bind(end.to_rfc3339())
    .bind(&notes)
    .bind(&project_id)
    .bind(&follow_up_at)
    .bind(&follow_up_note)
    .bind(&calendar_uid)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut synced = calendar_uid.is_some();
    if new.sync_calendar && calendar_uid.is_none() {
        synced = sync_to_calendar(title, start, end, &notes).await;
    }
    if synced {
        let _ = sqlx::query("UPDATE person_meetings SET calendar_synced = 1 WHERE id = ?")
            .bind(&id)
            .execute(pool)
            .await;
    }

    bump_last_contact(pool, &new.entity_uuid, &start).await?;

    fetch_meeting(pool, &id).await
}

async fn fetch_meeting(pool: &Pool<Sqlite>, id: &str) -> Result<PersonMeeting, String> {
    let row = sqlx::query(&format!(
        "SELECT {MEETING_COLS} FROM person_meetings WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row_to_meeting(&row))
}

pub async fn set_follow_up_done(
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    entity_uuid: &str,
    done: bool,
) -> Result<PersonMeeting, String> {
    let result = sqlx::query(
        "UPDATE person_meetings SET follow_up_done = ? WHERE id = ? AND entity_uuid = ?",
    )
    .bind(if done { 1 } else { 0 })
    .bind(meeting_id)
    .bind(entity_uuid)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    if result.rows_affected() == 0 {
        return Err("Meeting not found".into());
    }
    fetch_meeting(pool, meeting_id).await
}

async fn bump_last_contact(
    pool: &Pool<Sqlite>,
    entity_uuid: &str,
    at: &DateTime<Utc>,
) -> Result<(), String> {
    let at_s = at.to_rfc3339();
    sqlx::query(
        "UPDATE people SET last_contact_at = ? \
         WHERE entity_uuid = ? \
           AND (last_contact_at IS NULL OR last_contact_at < ?)",
    )
    .bind(&at_s)
    .bind(entity_uuid)
    .bind(&at_s)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Write the meeting into Calendar.app. Best-effort: a permission prompt, a
/// missing calendar, or a non-macOS host must not fail the CRM write.
async fn sync_to_calendar(
    title: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    notes: &str,
) -> bool {
    // Unit tests must not hit Calendar.app: CI has none, and a local run
    // would otherwise leave fixture events on the developer's calendar.
    if cfg!(test) || !cfg!(target_os = "macos") {
        return false;
    }
    const JXA: &str = r#"
function run(argv) {
  var title = argv[0];
  var startIso = argv[1];
  var endIso = argv[2];
  var notes = argv[3] || "";
  var Calendar = Application("Calendar");
  var cals = Calendar.calendars();
  if (!cals || cals.length === 0) {
    throw new Error("no calendars");
  }
  var cal = cals[0];
  cal.events.push(Calendar.Event({
    summary: title,
    startDate: new Date(startIso),
    endDate: new Date(endIso),
    description: notes
  }));
  return "ok";
}
"#;
    let fut = tokio::process::Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(JXA)
        .arg("--")
        .arg(title)
        .arg(start.to_rfc3339())
        .arg(end.to_rfc3339())
        .arg(notes)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(8), fut).await {
        Ok(Ok(out)) if out.status.success() => true,
        _ => false,
    }
}

/// Normalize for name matching: letters/digits stay, everything else is space.
fn normalize_words(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `title` contains the full display name as consecutive tokens.
/// Single-token names shorter than 4 characters never match (too many hits).
pub fn title_mentions_name(title: &str, display_name: &str) -> bool {
    let name = normalize_words(display_name);
    if name.is_empty() {
        return false;
    }
    let name_tokens: Vec<&str> = name.split_whitespace().collect();
    if name_tokens.len() == 1 && name_tokens[0].chars().count() < 4 {
        return false;
    }
    let title_n = normalize_words(title);
    let title_tokens: Vec<&str> = title_n.split_whitespace().collect();
    if title_tokens.len() < name_tokens.len() {
        return false;
    }
    title_tokens
        .windows(name_tokens.len())
        .any(|w| w == name_tokens.as_slice())
}

/// Unique directory person whose full name appears in the event title.
/// Ambiguous (two people match) or none → None. Never guess.
pub fn match_title_to_person<'a>(title: &str, people: &'a [(String, String)]) -> Option<&'a str> {
    let mut hits: Vec<&str> = Vec::new();
    for (uuid, name) in people {
        if title_mentions_name(title, name) {
            hits.push(uuid.as_str());
        }
    }
    if hits.len() == 1 {
        Some(hits[0])
    } else {
        None
    }
}

pub fn parse_calendar_tsv(out: &str) -> Vec<CalendarEvent> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let uid = parts.next()?.trim();
            let title = parts.next()?.trim();
            let start = parse_rfc3339(parts.next()?.trim()).ok()?;
            let end = parse_rfc3339(parts.next()?.trim()).ok()?;
            if uid.is_empty() || title.is_empty() {
                return None;
            }
            Some(CalendarEvent {
                uid: uid.to_string(),
                title: title.to_string(),
                starts_at: start,
                ends_at: if end > start {
                    end
                } else {
                    start + Duration::hours(1)
                },
            })
        })
        .collect()
}

/// Apply already-parsed Calendar.app events onto matching people. Idempotent
/// on `calendar_uid` and on (person, title, start-minute).
pub async fn import_parsed_events(
    pool: &Pool<Sqlite>,
    events: &[CalendarEvent],
    people: &[(String, String)],
) -> Result<usize, String> {
    if events.is_empty() || people.is_empty() {
        return Ok(0);
    }
    let existing_uids: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT calendar_uid FROM person_meetings WHERE calendar_uid IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .collect();

    let mut imported = 0usize;
    for event in events {
        if existing_uids.contains(&event.uid) {
            continue;
        }
        let Some(uuid) = match_title_to_person(&event.title, people) else {
            continue;
        };
        let start_s = event.starts_at.to_rfc3339();
        let dup: Option<String> = sqlx::query_scalar(
            "SELECT id FROM person_meetings \
             WHERE entity_uuid = ? AND lower(title) = lower(?) AND starts_at = ?",
        )
        .bind(uuid)
        .bind(&event.title)
        .bind(&start_s)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        if dup.is_some() {
            continue;
        }
        create_meeting(
            pool,
            NewMeeting {
                entity_uuid: uuid.to_string(),
                title: event.title.clone(),
                starts_at: start_s,
                ends_at: Some(event.ends_at.to_rfc3339()),
                notes: "Imported from Apple Calendar".into(),
                calendar_uid: Some(event.uid.clone()),
                sync_calendar: false,
                ..Default::default()
            },
        )
        .await?;
        imported += 1;
    }
    Ok(imported)
}

async fn read_calendar_range(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>, String> {
    if cfg!(test) || !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    const JXA: &str = r#"
function run(argv) {
  var start = new Date(argv[0]);
  var end = new Date(argv[1]);
  var Calendar = Application("Calendar");
  var lines = [];
  var cals = Calendar.calendars();
  for (var i = 0; i < cals.length; i++) {
    var evs;
    try {
      evs = cals[i].events.whose({
        _and: [
          { startDate: { '>=': start } },
          { startDate: { '<': end } }
        ]
      })();
    } catch (e) {
      evs = [];
    }
    for (var j = 0; j < evs.length; j++) {
      var e = evs[j];
      var uid = "";
      try { uid = String(e.uid()); } catch (ex) { uid = ""; }
      var summary = "";
      try { summary = String(e.summary()); } catch (ex) { summary = ""; }
      var s = e.startDate();
      var en = e.endDate();
      if (!uid) uid = summary + "|" + s.toISOString();
      lines.push([uid, summary, s.toISOString(), en.toISOString()].join("\t"));
    }
  }
  return lines.join("\n");
}
"#;
    let fut = tokio::process::Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(JXA)
        .arg("--")
        .arg(start.to_rfc3339())
        .arg(end.to_rfc3339())
        .output();
    let output = match tokio::time::timeout(std::time::Duration::from_secs(8), fut).await {
        Ok(Ok(out)) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return Ok(Vec::new()),
    };
    Ok(parse_calendar_tsv(&output))
}

/// Pull Calendar.app events whose titles uniquely mention a directory person.
/// Best-effort: permission failure returns 0, never an error the UI must handle.
pub async fn import_matching_events(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<usize, String> {
    let events = read_calendar_range(start, end).await?;
    if events.is_empty() {
        return Ok(0);
    }
    let rows = sqlx::query("SELECT entity_uuid, display_name FROM people")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let people: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.get("entity_uuid"), r.get("display_name")))
        .collect();
    import_parsed_events(pool, &events, &people).await
}

pub fn is_quiet(last_contact_at: Option<&str>, now: DateTime<Utc>) -> bool {
    match last_contact_at.and_then(|s| parse_rfc3339(s).ok()) {
        None => true,
        Some(at) => (now - at).num_days() >= QUIET_AFTER_DAYS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::{upsert_person, PersonAttrs};
    use crate::session::spectral_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        spectral_schema::init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn ada(pool: &Pool<Sqlite>) -> crate::people::Person {
        upsert_person(
            pool,
            "person:ada-lovelace",
            "Ada Lovelace",
            &PersonAttrs::default(),
        )
        .await
        .unwrap()
    }

    #[test]
    fn title_match_requires_the_full_name() {
        assert!(title_mentions_name(
            "Coffee with Ada Lovelace",
            "Ada Lovelace"
        ));
        assert!(title_mentions_name("Ada Lovelace / 1:1", "Ada Lovelace"));
        assert!(!title_mentions_name("Coffee with Ada", "Ada Lovelace"));
        assert!(!title_mentions_name("standup", "Ada Lovelace"));
        assert!(!title_mentions_name("Lunch with Ada", "Ada"));
    }

    #[test]
    fn unique_match_skips_ambiguous_names() {
        let people = vec![
            ("u-ada".into(), "Ada Lovelace".into()),
            ("u-bea".into(), "Bea King".into()),
        ];
        assert_eq!(
            match_title_to_person("Coffee with Ada Lovelace", &people),
            Some("u-ada")
        );
        assert_eq!(match_title_to_person("Team standup", &people), None);
        let two_adas = vec![
            ("u-ada".into(), "Ada Lovelace".into()),
            ("u-ada2".into(), "Ada Lovelace Smith".into()),
        ];
        assert_eq!(
            match_title_to_person("Coffee with Ada Lovelace Smith", &two_adas),
            None
        );
    }

    #[tokio::test]
    async fn create_lists_against_the_person() {
        let pool = fresh().await;
        let person = ada(&pool).await;
        let meeting = create_meeting(
            &pool,
            NewMeeting {
                entity_uuid: person.entity_uuid.clone(),
                title: "Coffee".into(),
                starts_at: "2026-08-20T15:00:00Z".into(),
                notes: "Follow up on the graph".into(),
                follow_up_at: Some("2026-08-27".into()),
                follow_up_note: Some("Send recap".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(meeting.title, "Coffee");
        assert_eq!(
            meeting.ends_at.as_deref(),
            Some("2026-08-20T16:00:00+00:00")
        );
        assert!(!meeting.calendar_synced, "CI has no Calendar.app");
        assert!(meeting.follow_up_at.is_some());
        assert_eq!(meeting.follow_up_note, "Send recap");
        assert!(!meeting.follow_up_done);

        let listed = list_for_person(&pool, &person.entity_uuid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, meeting.id);

        let row: Option<String> =
            sqlx::query_scalar("SELECT last_contact_at FROM people WHERE entity_uuid = ?")
                .bind(&person.entity_uuid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(row.unwrap().starts_with("2026-08-20T15:00:00"));
    }

    #[tokio::test]
    async fn rejects_unknown_person_and_bad_times() {
        let pool = fresh().await;
        let err = create_meeting(
            &pool,
            NewMeeting {
                entity_uuid: "00000000-0000-0000-0000-000000000000".into(),
                title: "Ghost".into(),
                starts_at: "2026-08-20T15:00:00Z".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("Person not found"), "{err}");

        let person = ada(&pool).await;
        let err = create_meeting(
            &pool,
            NewMeeting {
                entity_uuid: person.entity_uuid,
                title: "Backwards".into(),
                starts_at: "2026-08-20T16:00:00Z".into(),
                ends_at: Some("2026-08-20T15:00:00Z".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("ends_at"), "{err}");
    }

    #[tokio::test]
    async fn range_query_joins_the_display_name() {
        let pool = fresh().await;
        let person = ada(&pool).await;
        create_meeting(
            &pool,
            NewMeeting {
                entity_uuid: person.entity_uuid,
                title: "Coffee".into(),
                starts_at: "2026-08-20T15:00:00Z".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let start = parse_rfc3339("2026-08-20T00:00:00Z").unwrap();
        let end = parse_rfc3339("2026-08-21T00:00:00Z").unwrap();
        let rows = list_in_range(&pool, start, end).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name, "Ada Lovelace");
        assert_eq!(rows[0].title, "Coffee");
    }

    #[tokio::test]
    async fn import_attaches_unique_titles_and_skips_duplicates() {
        let pool = fresh().await;
        let person = ada(&pool).await;
        let events = vec![CalendarEvent {
            uid: "cal-1".into(),
            title: "Coffee with Ada Lovelace".into(),
            starts_at: parse_rfc3339("2026-08-19T14:00:00Z").unwrap(),
            ends_at: parse_rfc3339("2026-08-19T15:00:00Z").unwrap(),
        }];
        let people = vec![(person.entity_uuid.clone(), person.display_name.clone())];
        let n = import_parsed_events(&pool, &events, &people).await.unwrap();
        assert_eq!(n, 1);
        let n = import_parsed_events(&pool, &events, &people).await.unwrap();
        assert_eq!(n, 0);
        let listed = list_for_person(&pool, &person.entity_uuid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].calendar_uid.as_deref(), Some("cal-1"));
        assert!(listed[0].calendar_synced);
    }

    #[tokio::test]
    async fn follow_up_can_be_marked_done() {
        let pool = fresh().await;
        let person = ada(&pool).await;
        let meeting = create_meeting(
            &pool,
            NewMeeting {
                entity_uuid: person.entity_uuid.clone(),
                title: "Coffee".into(),
                starts_at: "2026-08-20T15:00:00Z".into(),
                follow_up_at: Some("2026-08-27T12:00:00Z".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let updated = set_follow_up_done(&pool, &meeting.id, &person.entity_uuid, true)
            .await
            .unwrap();
        assert!(updated.follow_up_done);
        let start = parse_rfc3339("2026-08-27T00:00:00Z").unwrap();
        let end = parse_rfc3339("2026-08-28T00:00:00Z").unwrap();
        assert!(list_follow_ups_in_range(&pool, start, end)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn quiet_is_never_or_older_than_thirty_days() {
        let now = parse_rfc3339("2026-08-20T12:00:00Z").unwrap();
        assert!(is_quiet(None, now));
        assert!(is_quiet(Some("2026-07-01T12:00:00Z"), now));
        assert!(!is_quiet(Some("2026-08-18T12:00:00Z"), now));
    }
}
