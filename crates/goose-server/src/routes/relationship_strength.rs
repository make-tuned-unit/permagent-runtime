//! Relationship strength — a deterministic, signal-derived score for People.
//!
//! Attio computes "connection strength" from real interaction signal, never a
//! field a human sets by hand (UX_BENCHMARK_RESEARCH.md §6). This module is
//! Permagent's version: a recency-weighted sum over four REAL, timestamped
//! channels the agent already has —
//!
//!   1. **Meetings** (`person_meetings.starts_at`) — the least-noisy signal:
//!      a logged or calendar-synced event actually tied to this person.
//!   2. **Decisions** (`decisions` rows of kind `file_to_project`, whose
//!      `payload_json.people` names this person) — structured and
//!      deliberate: the agent filed something and named them.
//!   3. **Memories** (`memory_annotations.who`, joined to `memories`) — the
//!      Librarian's describe pass inferred this person was involved.
//!      Display-name matched, so it inherits that join's known looseness
//!      (SCOUT_ROUND2 Q1: a rename or a name collision can mis-attribute a
//!      memory) — the weakest of the four.
//!   4. **Last contact** (`people.last_contact_at`, the raw column) — mostly
//!      redundant with (1), since a logged meeting bumps this column too
//!      (`person_meetings::create_meeting`'s `bump_last_contact`), but it is
//!      also the one place a *manually* entered contact date lives when no
//!      meeting/decision/memory exists for it. Weighted lowest specifically
//!      because of that overlap — see `WEIGHT_LAST_CONTACT`.
//!
//! **What is deliberately NOT a channel**: raw chat-session content (no
//! `session_id -> person_id` link exists — SCOUT_ROUND2 Q1), project
//! notes/cards (matched by loose LIKE substring in `person_activity_handler`,
//! noisier than the four above), and `person_merge_proposal` /
//! `person_delete_proposal` decisions (administrative actions ON the person
//! record, not evidence of an active relationship — including a "delete this
//! duplicate" proposal as positive signal would be backwards).
//!
//! ## Naming collision (read before renaming anything to `relationship_strength`)
//!
//! [`permagent::people::Person`] already has a field literally named
//! `relationship_strength: Option<String>` — a **pre-existing, manual,
//! free-text** attribute (#495 slice 2b), labelled "Relationship" in
//! `PersonDetailModal`'s edit form (e.g. "Close friend", "Colleague"). It
//! describes the relationship's *type*, is user-authored, and is exactly the
//! anti-pattern UX_BENCHMARK_RESEARCH §6 warns against for a *strength*
//! value. It cannot be renamed or removed here (it is a live, tested,
//! PATCH-able field several other components read/write); this module's
//! output is therefore wired onto the wire as **`relationship_signal`**, a
//! sibling field, never reusing that key. See the DAG K lane report for the
//! open question this leaves for Jesse (whether the manual field should be
//! renamed/retired now that a real signal exists).
//!
//! ## Normalisation
//!
//! Each channel's recency-weighted hits are capped at 1.0 *before* combining
//! (`score_channel`), so one channel firing very frequently cannot alone blow
//! the score past what "fully present this window" means for that channel.
//! The capped per-channel scores are then combined as a weighted average and
//! scaled to 0-100. Thresholds and weights below are first-cut defaults
//! (flagged for Jesse in the lane report) — there is no usage data yet to
//! calibrate against.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use std::collections::HashMap;

use permagent::people::Person;
use permagent::person_meetings::parse_rfc3339_or_date;

/// Only interactions inside this many days of `now` count toward the score.
/// A meeting from three years ago should not prop up "strong" forever.
pub const STRENGTH_WINDOW_DAYS: i64 = 90;

/// Meetings are the least-noisy signal available — a logged/calendar-synced
/// event, not an inference.
const WEIGHT_MEETINGS: f64 = 1.0;
/// A `file_to_project` decision naming this person: structured and
/// deliberate, but not evidence of *time spent* the way a meeting is.
const WEIGHT_DECISIONS: f64 = 0.8;
/// Librarian-inferred, display-name matched — the loosest join of the four.
const WEIGHT_MEMORIES: f64 = 0.6;
/// Deliberately the lowest weight: this channel is usually just a mirror of
/// (1) — logging a meeting bumps `last_contact_at` too — so double-counting
/// the same event is the common case, not an edge case. The low weight
/// bounds how much that redundancy can inflate the score (see module docs).
const WEIGHT_LAST_CONTACT: f64 = 0.3;

/// The normalisation ceiling is ONE channel — meetings, the highest-weighted
/// — fully saturated (one very recent hit). Deliberately NOT the sum of all
/// four weights: dividing by the sum would mean a person whose only signal is
/// meetings (the common case — most people never generate a `file_to_project`
/// decision or a Librarian memory) could never score above
/// `WEIGHT_MEETINGS / (sum of all weights) * 100 ≈ 37`, capping "lots of
/// frequent, recent meetings" at "steady" forever. Normalising against the
/// single strongest channel instead means that channel alone can reach 100,
/// and corroborating channels only ever push the score up (clamped at 100)
/// rather than being required to reach it.
const NORMALIZATION_CEILING: f64 = WEIGHT_MEETINGS;

/// Normalised-score ceiling for the "fading" bucket (inclusive). Above it and
/// at or below [`STEADY_MAX`] is "steady"; above `STEADY_MAX` is "strong".
const FADING_MAX: u8 = 29;
const STEADY_MAX: u8 = 64;

/// Every known timestamp for one person, one channel each, un-windowed —
/// [`compute_relationship_strength`] applies the 90-day window itself, so
/// callers can pass everything they have (including a manually-set decade-old
/// `last_contact_at`) without special-casing it.
#[derive(Debug, Clone, Default)]
pub struct RelationshipInputs {
    pub meetings: Vec<DateTime<Utc>>,
    pub decisions: Vec<DateTime<Utc>>,
    pub memories: Vec<DateTime<Utc>>,
    pub last_contact: Vec<DateTime<Utc>>,
}

/// One channel's contribution to the breakdown shown in the UI: how many
/// hits counted toward the current score, and when the most recent one (in
/// or out of the scoring window) was — so "last meeting: 4 months ago" can
/// still be shown even once it has aged out of the 90-day window.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChannelSignal {
    pub count: usize,
    pub most_recent: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RelationshipSignals {
    pub meetings: ChannelSignal,
    pub decisions: ChannelSignal,
    pub memories: ChannelSignal,
    pub last_contact: ChannelSignal,
}

/// Attio's "who am I neglecting" framing (UX_BENCHMARK_RESEARCH §6) —
/// `Fading`/`Dormant` are the buckets the People "Fading" filter chip and the
/// Home `people_attention` card both key off, so both surfaces agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipBucket {
    Strong,
    Steady,
    Fading,
    Dormant,
    /// Never any interaction on any channel, ever — distinct from `Dormant`
    /// (which had signal once, just not recently). Honest-data rule: a
    /// person with zero signal is not "neglected", they're simply unrecorded.
    NoSignal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationshipStrength {
    /// 0-100, normalised.
    pub score: u8,
    pub bucket: RelationshipBucket,
    /// Max timestamp across every channel, regardless of the scoring window
    /// — this is what distinguishes `Dormant` (has one, just old) from
    /// `NoSignal` (has none).
    pub last_interaction_at: Option<String>,
    pub signals: RelationshipSignals,
}

/// Recency-weighted sum for one channel, capped at 1.0 before weighting (see
/// module docs). Returns the UI-facing breakdown alongside the capped score.
fn score_channel(timestamps: &[DateTime<Utc>], now: DateTime<Utc>) -> (ChannelSignal, f64) {
    let mut count = 0usize;
    let mut raw = 0.0f64;
    let mut most_recent: Option<DateTime<Utc>> = None;
    for ts in timestamps {
        let days_ago = (now - *ts).num_seconds() as f64 / 86_400.0;
        if days_ago < 0.0 {
            // Future timestamp: clock skew or bad data. Never let it count,
            // and never let it masquerade as "the most recent interaction".
            continue;
        }
        if most_recent.is_none_or(|m| *ts > m) {
            most_recent = Some(*ts);
        }
        if days_ago <= STRENGTH_WINDOW_DAYS as f64 {
            count += 1;
            raw += 1.0 / (1.0 + days_ago);
        }
    }
    let signal = ChannelSignal {
        count,
        most_recent: most_recent.map(|d| d.to_rfc3339()),
    };
    (signal, raw.min(1.0))
}

fn classify(score: u8, has_last_interaction: bool) -> RelationshipBucket {
    if !has_last_interaction {
        return RelationshipBucket::NoSignal;
    }
    if score == 0 {
        return RelationshipBucket::Dormant;
    }
    if score <= FADING_MAX {
        RelationshipBucket::Fading
    } else if score <= STEADY_MAX {
        RelationshipBucket::Steady
    } else {
        RelationshipBucket::Strong
    }
}

/// The pure scoring function — no DB, no clock reads (caller supplies `now`),
/// deterministic for a given `inputs`/`now` pair. Every other function in this
/// module exists to build `inputs` from the real database.
pub fn compute_relationship_strength(
    inputs: &RelationshipInputs,
    now: DateTime<Utc>,
) -> RelationshipStrength {
    let (meetings, meetings_score) = score_channel(&inputs.meetings, now);
    let (decisions, decisions_score) = score_channel(&inputs.decisions, now);
    let (memories, memories_score) = score_channel(&inputs.memories, now);
    let (last_contact, last_contact_score) = score_channel(&inputs.last_contact, now);

    let weighted = meetings_score * WEIGHT_MEETINGS
        + decisions_score * WEIGHT_DECISIONS
        + memories_score * WEIGHT_MEMORIES
        + last_contact_score * WEIGHT_LAST_CONTACT;
    let score = ((weighted / NORMALIZATION_CEILING) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;

    let last_interaction_at = inputs
        .meetings
        .iter()
        .chain(inputs.decisions.iter())
        .chain(inputs.memories.iter())
        .chain(inputs.last_contact.iter())
        .filter(|ts| **ts <= now)
        .max()
        .map(|dt| dt.to_rfc3339());

    let bucket = classify(score, last_interaction_at.is_some());

    RelationshipStrength {
        score,
        bucket,
        last_interaction_at,
        signals: RelationshipSignals {
            meetings,
            decisions,
            memories,
            last_contact,
        },
    }
}

/// True the moment a person's bucket newly crosses from an engaged bucket
/// (`Strong`/`Steady`) into a needs-attention one (`Fading`/`Dormant`) —
/// found by comparing `now` against `now - 1 day` on the SAME fixed
/// timestamps, rather than a persisted "last known bucket" table (there is
/// no schema-migration path available to this edit-only lane; see the lane
/// report). Returns the current (crossing) score on a transition.
///
/// Caveat, flagged for Jesse: this only catches a transition if it is
/// checked within roughly a day of the actual crossing instant (it runs from
/// a page-load hook, not a guaranteed daily sweep) — a person whose profile
/// isn't opened for a week after they go quiet will not get a memory. A
/// proper nightly sweep beside the Librarian's other sweeps would close this
/// gap; out of scope for an edit-only, no-cargo lane.
pub fn crossed_into_attention(
    inputs: &RelationshipInputs,
    now: DateTime<Utc>,
) -> Option<RelationshipStrength> {
    let today = compute_relationship_strength(inputs, now);
    let yesterday = compute_relationship_strength(inputs, now - chrono::Duration::days(1));
    let needs_attention_now = matches!(
        today.bucket,
        RelationshipBucket::Fading | RelationshipBucket::Dormant
    );
    let was_engaged = matches!(
        yesterday.bucket,
        RelationshipBucket::Strong | RelationshipBucket::Steady
    );
    (needs_attention_now && was_engaged).then_some(today)
}

fn transition_content(display_name: &str, strength: &RelationshipStrength) -> String {
    let bucket_word = match strength.bucket {
        RelationshipBucket::Fading => "fading",
        RelationshipBucket::Dormant => "dormant",
        RelationshipBucket::Strong | RelationshipBucket::Steady | RelationshipBucket::NoSignal => {
            "changed"
        }
    };
    match &strength.last_interaction_at {
        Some(at) => format!(
            "Relationship with {display_name} is now {bucket_word} (score {}/100) — last interaction {at}.",
            strength.score
        ),
        None => format!(
            "Relationship with {display_name} is now {bucket_word} (score {}/100).",
            strength.score
        ),
    }
}

// ── Bulk data-gathering (impure) ─────────────────────────────────────────

/// Every display name each person answers to (their own plus any merge
/// aliases), bulk-fetched in one query rather than one `names_for` call per
/// person — mirrors [`permagent::people_merge::names_for`] but for a whole
/// directory page at once (never per-row: this backs a page load).
async fn display_names_by_person(
    pool: &Pool<Sqlite>,
    people: &[Person],
) -> HashMap<String, Vec<String>> {
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    for p in people {
        if !p.display_name.trim().is_empty() {
            names
                .entry(p.entity_uuid.clone())
                .or_default()
                .push(p.display_name.clone());
        }
    }
    let rows = sqlx::query(
        "SELECT entity_uuid, alias_value FROM person_aliases WHERE alias_kind = 'display_name'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for r in rows {
        let uuid: String = r.get("entity_uuid");
        let Some(list) = names.get_mut(&uuid) else {
            continue; // not one of the people this call is scoring
        };
        let alias: String = r.get("alias_value");
        if alias.trim().is_empty() {
            continue;
        }
        if !list.iter().any(|n| n.eq_ignore_ascii_case(&alias)) {
            list.push(alias);
        }
    }
    names
}

/// Lowercased name → every person who answers to it. A name can map to more
/// than one person (two people sharing a first name); an ambiguous hit is
/// attributed to all of them, the same known limitation the existing
/// `memory_annotations.who` join already has (SCOUT_ROUND2 Q1).
fn reverse_name_index(
    names_by_person: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    for (uuid, names) in names_by_person {
        for name in names {
            index
                .entry(name.to_lowercase())
                .or_default()
                .push(uuid.clone());
        }
    }
    index
}

async fn meeting_starts_by_person(pool: &Pool<Sqlite>) -> HashMap<String, Vec<DateTime<Utc>>> {
    let rows = sqlx::query("SELECT entity_uuid, starts_at FROM person_meetings")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let mut out: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
    for r in rows {
        let uuid: String = r.get("entity_uuid");
        let starts_at: String = r.get("starts_at");
        if let Ok(ts) = parse_rfc3339_or_date(&starts_at) {
            out.entry(uuid).or_default().push(ts);
        }
    }
    out
}

async fn last_contact_by_person(pool: &Pool<Sqlite>) -> HashMap<String, DateTime<Utc>> {
    let rows = sqlx::query(
        "SELECT entity_uuid, last_contact_at FROM people \
         WHERE last_contact_at IS NOT NULL AND last_contact_at != ''",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out = HashMap::new();
    for r in rows {
        let uuid: String = r.get("entity_uuid");
        let raw: String = r.get("last_contact_at");
        if let Ok(ts) = parse_rfc3339_or_date(&raw) {
            out.insert(uuid, ts);
        }
    }
    out
}

/// The `people` field of every `file_to_project` decision payload, paired
/// with that decision's `created_at`. Display names only — matched against
/// [`reverse_name_index`] by the caller.
#[derive(Debug, serde::Deserialize)]
struct FileToProjectPeopleOnly {
    #[serde(default)]
    people: Vec<String>,
}

async fn file_to_project_hits(pool: &Pool<Sqlite>) -> Vec<(String, DateTime<Utc>)> {
    let rows = sqlx::query(
        "SELECT payload_json, created_at FROM decisions WHERE kind = 'file_to_project'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let created_at: String = r.get("created_at");
        let Ok(ts) = parse_rfc3339_or_date(&created_at) else {
            continue;
        };
        let payload_json: String = r.get("payload_json");
        let Ok(parsed) = serde_json::from_str::<FileToProjectPeopleOnly>(&payload_json) else {
            continue;
        };
        for name in parsed.people {
            if !name.trim().is_empty() {
                out.push((name, ts));
            }
        }
    }
    out
}

/// Every `(display_name, created_at)` pair a Brain memory's `who` annotation
/// names, across the whole `memories`/`memory_annotations` join — the bulk
/// version of the per-person scan `person_activity_handler` already runs.
/// Takes an open connection (rather than opening `memory.db` itself) so
/// tests can seed an in-memory Brain schema instead of touching a real path.
fn memory_hits_by_name(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, DateTime<Utc>)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.created_at, a.who FROM memories m \
             JOIN memory_annotations a ON a.memory_id = m.id \
             WHERE a.who IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (Some(created_at), Some(who)) = row else {
            continue;
        };
        let Ok(ts) = parse_rfc3339_or_date(&created_at) else {
            continue;
        };
        let refs: Vec<serde_json::Value> = serde_json::from_str(&who).unwrap_or_default();
        for r in refs {
            if let Some(name) = r.get("display_name").and_then(|v| v.as_str()) {
                if !name.trim().is_empty() {
                    out.push((name.to_string(), ts));
                }
            }
        }
    }
    Ok(out)
}

/// Bulk relationship strength for a set of people — one shared query pass
/// (never per-person) backing both `GET /api/people/directory`
/// (`relationship_signal` on [`crate::routes::people::DirectoryPerson`]) and
/// the Home `people_attention` card, so both read the identical score (one
/// source of truth).
///
/// `feed_transitions`: when true, a person whose bucket newly crosses into
/// `Fading`/`Dormant` (see [`crossed_into_attention`]) gets one upserted
/// Brain memory keyed `person:{id}:relationship`. Only the directory-page
/// call site sets this — the dashboard card call reads the same scores
/// without re-triggering the feed, so a transition is recorded once per
/// code path rather than from every consumer.
pub async fn relationship_strength_by_person(
    pool: &Pool<Sqlite>,
    people: &[Person],
    feed_transitions: bool,
) -> HashMap<String, RelationshipStrength> {
    let mut out = HashMap::new();
    if people.is_empty() {
        return out;
    }
    let now = Utc::now();

    let names_by_person = display_names_by_person(pool, people).await;
    let name_index = reverse_name_index(&names_by_person);

    let meetings_map = meeting_starts_by_person(pool).await;
    let last_contact_map = last_contact_by_person(pool).await;
    let decision_hits = file_to_project_hits(pool).await;
    let memory_hits = tokio::task::spawn_blocking(|| {
        crate::brain_ops::read_only_brain_conn()
            .map_err(|e| e.to_string())
            .and_then(|conn| memory_hits_by_name(&conn))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    .unwrap_or_default();

    let mut decisions_by_person: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
    for (name, ts) in &decision_hits {
        if let Some(uuids) = name_index.get(&name.to_lowercase()) {
            for uuid in uuids {
                decisions_by_person
                    .entry(uuid.clone())
                    .or_default()
                    .push(*ts);
            }
        }
    }
    let mut memories_by_person: HashMap<String, Vec<DateTime<Utc>>> = HashMap::new();
    for (name, ts) in &memory_hits {
        if let Some(uuids) = name_index.get(&name.to_lowercase()) {
            for uuid in uuids {
                memories_by_person
                    .entry(uuid.clone())
                    .or_default()
                    .push(*ts);
            }
        }
    }

    for person in people {
        let uuid = &person.entity_uuid;
        let inputs = RelationshipInputs {
            meetings: meetings_map.get(uuid).cloned().unwrap_or_default(),
            decisions: decisions_by_person.get(uuid).cloned().unwrap_or_default(),
            memories: memories_by_person.get(uuid).cloned().unwrap_or_default(),
            last_contact: last_contact_map.get(uuid).cloned().into_iter().collect(),
        };
        let strength = compute_relationship_strength(&inputs, now);
        if feed_transitions {
            if let Some(current) = crossed_into_attention(&inputs, now) {
                let content = transition_content(&person.display_name, &current);
                permagent::brain_feed::feed(
                    pool,
                    permagent::brain_feed::FeedSource::Person,
                    permagent::brain_feed::feed_key("person", uuid, "relationship"),
                    None,
                    content,
                )
                .await;
            }
        }
        out.insert(uuid.clone(), strength);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn days_ago(now: DateTime<Utc>, n: i64) -> DateTime<Utc> {
        now - Duration::days(n)
    }

    #[test]
    fn empty_inputs_is_no_signal() {
        let now = Utc::now();
        let strength = compute_relationship_strength(&RelationshipInputs::default(), now);
        assert_eq!(strength.score, 0);
        assert_eq!(strength.bucket, RelationshipBucket::NoSignal);
        assert_eq!(strength.last_interaction_at, None);
        assert_eq!(strength.signals.meetings.count, 0);
    }

    #[test]
    fn a_meeting_older_than_the_window_is_dormant_not_no_signal() {
        let now = Utc::now();
        let inputs = RelationshipInputs {
            meetings: vec![days_ago(now, STRENGTH_WINDOW_DAYS + 30)],
            ..Default::default()
        };
        let strength = compute_relationship_strength(&inputs, now);
        assert_eq!(strength.score, 0);
        assert_eq!(strength.bucket, RelationshipBucket::Dormant);
        assert!(strength.last_interaction_at.is_some());
        // The old meeting aged out of the scoring window, but it is still the
        // most recent hit reported in the breakdown.
        assert_eq!(strength.signals.meetings.count, 0);
        assert!(strength.signals.meetings.most_recent.is_some());
    }

    #[test]
    fn a_meeting_today_is_strong() {
        let now = Utc::now();
        let inputs = RelationshipInputs {
            meetings: vec![now],
            ..Default::default()
        };
        let strength = compute_relationship_strength(&inputs, now);
        assert_eq!(strength.bucket, RelationshipBucket::Strong);
        assert_eq!(strength.signals.meetings.count, 1);
        assert!(strength.score > STEADY_MAX);
    }

    #[test]
    fn decay_reduces_score_as_time_passes() {
        let now = Utc::now();
        let fresh = compute_relationship_strength(
            &RelationshipInputs {
                meetings: vec![days_ago(now, 1)],
                ..Default::default()
            },
            now,
        );
        let older = compute_relationship_strength(
            &RelationshipInputs {
                meetings: vec![days_ago(now, 60)],
                ..Default::default()
            },
            now,
        );
        assert!(
            fresh.score > older.score,
            "fresh={} older={}",
            fresh.score,
            older.score
        );
    }

    #[test]
    fn meetings_outweigh_memories_at_equal_recency() {
        let now = Utc::now();
        let meeting_only = compute_relationship_strength(
            &RelationshipInputs {
                meetings: vec![days_ago(now, 5)],
                ..Default::default()
            },
            now,
        );
        let memory_only = compute_relationship_strength(
            &RelationshipInputs {
                memories: vec![days_ago(now, 5)],
                ..Default::default()
            },
            now,
        );
        assert!(meeting_only.score > memory_only.score);
    }

    #[test]
    fn buckets_span_the_full_range() {
        let now = Utc::now();
        // Steady: a regular cadence, but nothing in the last few days.
        let steady = compute_relationship_strength(
            &RelationshipInputs {
                meetings: vec![
                    days_ago(now, 3),
                    days_ago(now, 10),
                    days_ago(now, 20),
                    days_ago(now, 35),
                    days_ago(now, 50),
                ],
                ..Default::default()
            },
            now,
        );
        assert_eq!(
            steady.bucket,
            RelationshipBucket::Steady,
            "score={}",
            steady.score
        );

        // Fading: one interaction, well into the window.
        let fading = compute_relationship_strength(
            &RelationshipInputs {
                meetings: vec![days_ago(now, 70)],
                ..Default::default()
            },
            now,
        );
        assert_eq!(
            fading.bucket,
            RelationshipBucket::Fading,
            "score={}",
            fading.score
        );
    }

    #[test]
    fn crossed_into_attention_fires_only_on_the_transition_day() {
        let now = Utc::now();
        // A single meeting 3 days old: at `now - 1 day` it was only 2 days
        // old (score ≈33, steady); at `now` it is 3 days old (score ≈25,
        // fading) — the exact boundary crossing this function exists to
        // catch.
        let inputs = RelationshipInputs {
            meetings: vec![days_ago(now, 3)],
            ..Default::default()
        };
        let today = compute_relationship_strength(&inputs, now);
        assert_eq!(
            today.bucket,
            RelationshipBucket::Fading,
            "score={} (fixture no longer straddles the boundary)",
            today.score
        );
        assert!(crossed_into_attention(&inputs, now).is_some());

        // No transition once already fading/dormant yesterday too.
        let stale_inputs = RelationshipInputs {
            meetings: vec![days_ago(now, STRENGTH_WINDOW_DAYS + 10)],
            ..Default::default()
        };
        assert!(crossed_into_attention(&stale_inputs, now).is_none());
    }

    #[test]
    fn crossed_into_attention_none_when_still_engaged() {
        let now = Utc::now();
        let inputs = RelationshipInputs {
            meetings: vec![days_ago(now, 2)],
            ..Default::default()
        };
        assert!(crossed_into_attention(&inputs, now).is_none());
    }

    #[test]
    fn reverse_name_index_maps_case_insensitively_and_handles_collisions() {
        let mut names_by_person = HashMap::new();
        names_by_person.insert("uuid-1".to_string(), vec!["Jane Doe".to_string()]);
        names_by_person.insert("uuid-2".to_string(), vec!["JANE DOE".to_string()]);
        let index = reverse_name_index(&names_by_person);
        let matched = index.get("jane doe").expect("indexed");
        assert_eq!(matched.len(), 2);
    }

    fn brain_schema(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE memories (
                 id TEXT PRIMARY KEY,
                 content TEXT NOT NULL,
                 description TEXT,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE memory_annotations (
                 id TEXT PRIMARY KEY,
                 memory_id TEXT NOT NULL,
                 who TEXT,
                 FOREIGN KEY (memory_id) REFERENCES memories(id)
             );",
        )
        .unwrap();
    }

    #[test]
    fn memory_hits_by_name_reads_display_name_refs() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        brain_schema(&conn);
        conn.execute(
            "INSERT INTO memories (id, content, description, created_at) \
             VALUES ('m1', 'lunch with jane', 'lunch', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_annotations (id, memory_id, who) \
             VALUES ('a1', 'm1', '[{\"display_name\":\"Jane Doe\"}]')",
            [],
        )
        .unwrap();
        // A memory with no `who` must not appear.
        conn.execute(
            "INSERT INTO memories (id, content, description, created_at) \
             VALUES ('m2', 'no annotation', NULL, '2026-01-02T00:00:00Z')",
            [],
        )
        .unwrap();

        let hits = memory_hits_by_name(&conn).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "Jane Doe");
    }
}
