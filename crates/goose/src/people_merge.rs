//! Merging and deleting directory people.
//!
//! # Why an alias table instead of a re-key
//!
//! A merge would ideally re-point every reference from the duplicate onto the
//! survivor. Our own tables can do that — `person_meetings` and `project_people`
//! key on `people.entity_uuid`, so an UPDATE moves them. The Brain cannot.
//!
//! Spectral has **no entity re-key and no entity delete API** (the same gap
//! `project_graph::delete_graph_triple` documents for triples). Memories are
//! associated with a person through `memory_annotations.who` — a *name* — and
//! through fingerprints derived from content. Rewriting those rows by hand
//! would leave the derived state stale, which is exactly the failure mode we
//! are told not to create. So this module does not touch memories at all.
//!
//! Instead a merge records every identifier the survivor absorbed in
//! `person_aliases`: the duplicate's `entity_uuid`, `canonical_id`,
//! `graph_entity_id` and `display_name`. The `display_name` alias is the
//! load-bearing one — `/api/people/{id}/activity` finds memories by matching
//! the person's name, so absorbing the name is what makes the duplicate's
//! memories show up on the survivor's profile. The `entity_uuid` alias lets a
//! stale client id resolve to the survivor instead of 404-ing.
//!
//! What a merge therefore **keeps** (documented, not silently dropped):
//!
//! * the duplicate's graph entity node itself, and its `entity_fields` — no
//!   delete API exists. Its person→person and person→project edges ARE moved
//!   (copied through `insert_triple`, then the originals deleted through the
//!   existing direct-SQL bridge), so the node is left inert.
//! * Brain memories. They stay under the duplicate's name, and reach the
//!   survivor through the `display_name` alias.
//!
//! # Undo
//!
//! Every merge and delete writes a `person_merge_log` row holding a JSON
//! snapshot of what moved. [`undo_merge`] replays it backwards: the duplicate's
//! `people` row is restored, the moved meetings and project links go back, the
//! aliases are removed, and the moved graph edges are re-asserted and their
//! survivor-side copies deleted. Undo does NOT revert `entity_fields` copied
//! onto the survivor — Spectral has no field-delete — and says so in its
//! report. That is the one lossy edge, and it is bounded: fields are only
//! copied into slots the survivor had left *empty*.

use crate::brain_handle::{MovedTriple, SafeBrain};
use crate::people::{self, Person};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

// ── Duplicate suggestion ───────────────────────────────────────────────────

/// A pair the directory thinks might be the same person, with the evidence.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DuplicateSuggestion {
    /// The person to keep — the older row (earlier `created_at`), because the
    /// older id is the one other rows are most likely to reference.
    pub survivor_uuid: String,
    pub survivor_name: String,
    pub duplicate_uuid: String,
    pub duplicate_name: String,
    /// 0.0–1.0. Anything below [`SUGGESTION_THRESHOLD`] is not returned.
    pub score: f32,
    /// Human-readable evidence, strongest first ("same email", "same name").
    pub reasons: Vec<String>,
}

/// Pairs scoring below this are noise — two different people at one company
/// share a company and nothing else.
pub const SUGGESTION_THRESHOLD: f32 = 0.5;

/// Lowercase, trim, collapse internal whitespace.
pub fn normalize_name(raw: &str) -> String {
    raw.split_whitespace()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase + trim an email. Deliberately does NOT strip Gmail dots or
/// `+tags`: two addresses that differ there are usually deliberate, and a
/// false-positive merge is far more expensive than a missed suggestion.
pub fn normalize_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_lowercase();
    if e.is_empty() || !e.contains('@') {
        return None;
    }
    Some(e)
}

/// Digits only, keeping the last 10 — so `+1 (902) 555-0134` and `902-555-0134`
/// compare equal without needing a real phone-number library.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let digits: Vec<char> = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 7 {
        return None;
    }
    Some(digits[digits.len().saturating_sub(10)..].iter().collect())
}

/// Token-set similarity over names: |intersection| / |union| of the word sets.
/// "Mel Schembri" vs "Melanie Schembri" scores 0.33; "Mel Schembri" vs
/// "Schembri Mel" scores 1.0. Cheap, order-insensitive, and it never claims
/// two unrelated names are close.
pub fn name_token_similarity(a: &str, b: &str) -> f32 {
    let sa: std::collections::BTreeSet<&str> = a.split(' ').filter(|t| !t.is_empty()).collect();
    let sb: std::collections::BTreeSet<&str> = b.split(' ').filter(|t| !t.is_empty()).collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    inter / union
}

/// True when one name looks like a short form of the other: same surname (last
/// token) and one first name is a prefix of the other ("Mel"/"Melanie").
fn is_short_form(a: &str, b: &str) -> bool {
    let (ta, tb): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    if ta.len() < 2 || tb.len() < 2 {
        return false;
    }
    if ta.last() != tb.last() {
        return false;
    }
    let (fa, fb) = (ta[0], tb[0]);
    fa != fb && (fa.starts_with(fb) || fb.starts_with(fa)) && fa.len().min(fb.len()) >= 2
}

/// Score one ordered pair. Returns the score and the evidence behind it.
pub fn score_pair(a: &Person, b: &Person) -> (f32, Vec<String>) {
    let mut score: f32 = 0.0;
    let mut reasons = Vec::new();

    let (ea, eb) = (
        a.email.as_deref().and_then(normalize_email),
        b.email.as_deref().and_then(normalize_email),
    );
    if let (Some(x), Some(y)) = (&ea, &eb) {
        if x == y {
            score = score.max(0.98);
            reasons.push(format!("same email ({x})"));
        }
    }

    let (pa, pb) = (
        a.phone.as_deref().and_then(normalize_phone),
        b.phone.as_deref().and_then(normalize_phone),
    );
    if let (Some(x), Some(y)) = (&pa, &pb) {
        if x == y {
            score = score.max(0.94);
            reasons.push("same phone number".to_string());
        }
    }

    let (na, nb) = (
        normalize_name(&a.display_name),
        normalize_name(&b.display_name),
    );
    if !na.is_empty() && na == nb {
        score = score.max(0.9);
        reasons.push("identical name".to_string());
    } else if is_short_form(&na, &nb) {
        score = score.max(0.72);
        reasons.push(format!("\"{na}\" reads as a short form of \"{nb}\""));
    } else {
        let sim = name_token_similarity(&na, &nb);
        if sim >= 0.5 {
            // 0.5 similarity → 0.55, 1.0 → 0.85. Never enough on its own to
            // clear a same-email pair, always enough to surface for review.
            score = score.max(0.55 + (sim - 0.5) * 0.6);
            reasons.push(format!(
                "similar name ({:.0}% of words shared)",
                sim * 100.0
            ));
        }
    }

    // Company agreement is corroboration, never evidence on its own: it lifts
    // a pair that already has a name/contact signal, and adds nothing to a
    // pair that has none.
    let same_company = match (a.company.as_deref(), b.company.as_deref()) {
        (Some(x), Some(y)) if !x.trim().is_empty() => x.trim().eq_ignore_ascii_case(y.trim()),
        _ => false,
    };
    if same_company && score > 0.0 {
        score = (score + 0.05).min(1.0);
        reasons.push("same company".to_string());
    }

    (score, reasons)
}

/// Rank likely-duplicate pairs across the directory, best first.
///
/// `survivor` is the person with the earlier `created_at` (ties broken by
/// `entity_uuid` so the result is deterministic) — the older id is the one
/// other rows are most likely to already reference. The caller is free to swap
/// the direction; this is a suggestion, not a decision.
pub fn suggest_duplicates(people: &[Person], limit: usize) -> Vec<DuplicateSuggestion> {
    let mut out = Vec::new();
    for i in 0..people.len() {
        for j in (i + 1)..people.len() {
            let (a, b) = (&people[i], &people[j]);
            let (score, reasons) = score_pair(a, b);
            if score < SUGGESTION_THRESHOLD {
                continue;
            }
            let (survivor, duplicate) = if (a.created_at.as_str(), a.entity_uuid.as_str())
                <= (b.created_at.as_str(), b.entity_uuid.as_str())
            {
                (a, b)
            } else {
                (b, a)
            };
            out.push(DuplicateSuggestion {
                survivor_uuid: survivor.entity_uuid.clone(),
                survivor_name: survivor.display_name.clone(),
                duplicate_uuid: duplicate.entity_uuid.clone(),
                duplicate_name: duplicate.display_name.clone(),
                score,
                reasons,
            });
        }
    }
    out.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.survivor_uuid.cmp(&y.survivor_uuid))
            .then_with(|| x.duplicate_uuid.cmp(&y.duplicate_uuid))
    });
    out.truncate(limit);
    out
}

// ── Preview ────────────────────────────────────────────────────────────────

/// One graph field the merge would copy onto the survivor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldMove {
    pub field_name: String,
    pub value: String,
    /// `manual` | `enriched` | other — Spectral's provenance, preserved on copy.
    pub source: String,
}

/// A project link the merge would move (or drop, when the survivor already has it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectLinkMove {
    pub project_id: String,
    pub project_name: String,
    pub role: Option<String>,
    /// True when the survivor is already on this project — the duplicate's row
    /// is dropped rather than moved (the primary key is (project, person)).
    pub survivor_already_linked: bool,
}

/// Everything a merge would do, computed without writing anything.
#[derive(Debug, Clone, Serialize)]
pub struct MergePreview {
    pub survivor: Person,
    pub duplicate: Person,
    /// Meetings that move from the duplicate to the survivor.
    pub meetings: usize,
    /// Of those, how many carry an open (undone) follow-up.
    pub open_follow_ups: usize,
    pub project_links: Vec<ProjectLinkMove>,
    /// Contact/graph fields copied because the survivor's slot is empty.
    pub fields: Vec<FieldMove>,
    /// Fields the survivor already has, which the merge will NOT overwrite.
    pub fields_kept_from_survivor: Vec<String>,
    /// Identifiers the survivor will absorb (see the module docs).
    pub aliases: Vec<String>,
    /// Graph edges that will be re-pointed at the survivor.
    pub graph_edges: usize,
    /// Plain-language statement of what the merge does not move.
    pub retained: Vec<String>,
}

fn field_source_str(source: spectral::ingest::FieldSource) -> String {
    match source {
        spectral::ingest::FieldSource::Manual => "manual".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn parse_field_source(s: &str) -> spectral::ingest::FieldSource {
    match s {
        "manual" => spectral::ingest::FieldSource::Manual,
        _ => spectral::ingest::FieldSource::Enriched,
    }
}

async fn load_pair(
    pool: &Pool<Sqlite>,
    survivor_uuid: &str,
    duplicate_uuid: &str,
) -> Result<(Person, Person), String> {
    if survivor_uuid == duplicate_uuid {
        return Err("A person cannot be merged into themselves.".to_string());
    }
    let survivor = people::get_by_uuid(pool, survivor_uuid)
        .await?
        .ok_or_else(|| format!("Person {survivor_uuid} not found"))?;
    let duplicate = people::get_by_uuid(pool, duplicate_uuid)
        .await?
        .ok_or_else(|| format!("Person {duplicate_uuid} not found"))?;
    Ok((survivor, duplicate))
}

/// Compute what [`merge_people`] would do. Read-only.
pub async fn preview_merge(
    pool: &Pool<Sqlite>,
    brain: Option<&SafeBrain>,
    survivor_uuid: &str,
    duplicate_uuid: &str,
) -> Result<MergePreview, String> {
    let (survivor, duplicate) = load_pair(pool, survivor_uuid, duplicate_uuid).await?;

    let meetings: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM person_meetings WHERE entity_uuid = ?")
            .bind(duplicate_uuid)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let open_follow_ups: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM person_meetings \
         WHERE entity_uuid = ? AND follow_up_at IS NOT NULL AND follow_up_done = 0",
    )
    .bind(duplicate_uuid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    let project_links = project_link_moves(pool, survivor_uuid, duplicate_uuid).await?;

    let (fields, fields_kept_from_survivor, graph_edges) =
        field_and_edge_plan(brain, &survivor, &duplicate).await;

    Ok(MergePreview {
        aliases: alias_values(&duplicate),
        survivor,
        duplicate,
        meetings: meetings as usize,
        open_follow_ups: open_follow_ups as usize,
        project_links,
        fields,
        fields_kept_from_survivor,
        graph_edges,
        retained: vec![
            "Brain memories stay where they are — they reach the survivor through the \
             absorbed name, because Spectral has no memory re-key API."
                .to_string(),
            "The duplicate's graph entity node and its stored fields are left in place \
             (Spectral has no entity-delete API); its edges are moved, so the node is inert."
                .to_string(),
        ],
    })
}

async fn project_link_moves(
    pool: &Pool<Sqlite>,
    survivor_uuid: &str,
    duplicate_uuid: &str,
) -> Result<Vec<ProjectLinkMove>, String> {
    let rows = sqlx::query(
        "SELECT pp.project_id, COALESCE(pr.name, pp.project_id) AS project_name, pp.role, \
                EXISTS(SELECT 1 FROM project_people s \
                       WHERE s.project_id = pp.project_id AND s.entity_uuid = ?) AS survivor_has \
         FROM project_people pp \
         LEFT JOIN projects pr ON pr.id = pp.project_id \
         WHERE pp.entity_uuid = ? \
         ORDER BY pp.added_at",
    )
    .bind(survivor_uuid)
    .bind(duplicate_uuid)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| ProjectLinkMove {
            project_id: r.get("project_id"),
            project_name: r.get("project_name"),
            role: r.get("role"),
            survivor_already_linked: r.get::<i64, _>("survivor_has") != 0,
        })
        .collect())
}

/// Which graph fields would be copied, which the survivor keeps, and how many
/// graph edges hang off the duplicate. Degrades to "nothing" with no Brain.
async fn field_and_edge_plan(
    brain: Option<&SafeBrain>,
    survivor: &Person,
    duplicate: &Person,
) -> (Vec<FieldMove>, Vec<String>, usize) {
    let (Some(brain), Some(dup_hex), Some(surv_hex)) = (
        brain,
        duplicate.graph_entity_id.as_deref(),
        survivor.graph_entity_id.as_deref(),
    ) else {
        return (Vec::new(), Vec::new(), 0);
    };
    let (Ok(dup_id), Ok(surv_id)) = (
        dup_hex.parse::<spectral::core::entity_id::EntityId>(),
        surv_hex.parse::<spectral::core::entity_id::EntityId>(),
    ) else {
        return (Vec::new(), Vec::new(), 0);
    };
    let dup_fields = brain.get_entity_fields(dup_id).await.unwrap_or_default();
    let surv_fields = brain.get_entity_fields(surv_id).await.unwrap_or_default();
    let held: std::collections::HashSet<String> = surv_fields
        .iter()
        .map(|f| people::canonical_person_field(&f.field_name).to_string())
        .collect();

    let mut moves = Vec::new();
    let mut kept = Vec::new();
    for f in dup_fields {
        let canonical = people::canonical_person_field(&f.field_name).to_string();
        if held.contains(&canonical) {
            kept.push(canonical);
        } else {
            moves.push(FieldMove {
                field_name: f.field_name.clone(),
                value: f.value.clone(),
                source: field_source_str(f.source),
            });
        }
    }
    kept.sort();
    kept.dedup();

    let edges = brain
        .person_edges(dup_hex)
        .await
        .map(|e| e.len())
        .unwrap_or(0);
    (moves, kept, edges)
}

/// The identifiers a survivor absorbs from a duplicate, as `kind:value`.
fn alias_pairs(duplicate: &Person) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("entity_uuid", duplicate.entity_uuid.clone()),
        ("canonical_id", duplicate.canonical_id.clone()),
    ];
    if let Some(g) = duplicate.graph_entity_id.as_deref() {
        out.push(("graph_entity_id", g.to_string()));
    }
    if !duplicate.display_name.trim().is_empty() {
        out.push(("display_name", duplicate.display_name.trim().to_string()));
    }
    out
}

/// Same identifier set as [`alias_pairs`], from the snapshot's copy of the row.
fn alias_pairs_from_snapshot(row: &PersonRowSnapshot) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("entity_uuid", row.entity_uuid.clone()),
        ("canonical_id", row.canonical_id.clone()),
    ];
    if let Some(g) = row.graph_entity_id.as_deref() {
        out.push(("graph_entity_id", g.to_string()));
    }
    if !row.display_name.trim().is_empty() {
        out.push(("display_name", row.display_name.trim().to_string()));
    }
    out
}

fn alias_values(duplicate: &Person) -> Vec<String> {
    alias_pairs(duplicate)
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect()
}

// ── Merge ──────────────────────────────────────────────────────────────────

/// The undo record. Serialized into `person_merge_log.snapshot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSnapshot {
    /// The duplicate's `people` row, verbatim, so undo can put it back.
    pub duplicate_row: PersonRowSnapshot,
    /// `person_meetings.id` values that moved to the survivor.
    pub moved_meeting_ids: Vec<String>,
    /// Project links that moved (survivor_already_linked ones were dropped).
    pub project_links: Vec<ProjectLinkMove>,
    /// Fields written onto the survivor. Undo cannot unwrite these.
    pub copied_fields: Vec<FieldMove>,
    /// Graph edges re-pointed at the survivor.
    pub moved_edges: Vec<MovedTriple>,
    pub survivor_graph_entity_id: Option<String>,
    pub duplicate_graph_entity_id: Option<String>,
}

/// A `people` row captured verbatim so undo can restore it byte-for-byte.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonRowSnapshot {
    pub entity_uuid: String,
    pub canonical_id: String,
    pub display_name: String,
    pub role: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub last_contact_at: Option<String>,
    pub graph_entity_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

async fn snapshot_person_row(
    pool: &Pool<Sqlite>,
    entity_uuid: &str,
) -> Result<PersonRowSnapshot, String> {
    let r = sqlx::query(
        "SELECT entity_uuid, canonical_id, display_name, role, company, email, phone, notes, \
                last_contact_at, graph_entity_id, created_at, updated_at \
         FROM people WHERE entity_uuid = ?",
    )
    .bind(entity_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Person {entity_uuid} not found"))?;
    Ok(PersonRowSnapshot {
        entity_uuid: r.get("entity_uuid"),
        canonical_id: r.get("canonical_id"),
        display_name: r.get("display_name"),
        role: r.get("role"),
        company: r.get("company"),
        email: r.get("email"),
        phone: r.get("phone"),
        notes: r.get("notes"),
        last_contact_at: r.get("last_contact_at"),
        graph_entity_id: r.get("graph_entity_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

/// What a merge actually did.
#[derive(Debug, Clone, Serialize)]
pub struct MergeReport {
    /// The `person_merge_log` id — pass it to [`undo_merge`].
    pub merge_id: String,
    pub survivor_uuid: String,
    pub survivor_name: String,
    pub duplicate_uuid: String,
    pub duplicate_name: String,
    pub meetings_moved: usize,
    pub project_links_moved: usize,
    pub project_links_dropped: usize,
    pub fields_copied: usize,
    pub graph_edges_moved: usize,
    pub aliases_recorded: usize,
    pub summary: String,
}

/// Merge `duplicate_uuid` into `survivor_uuid`.
///
/// The survivor's `entity_uuid`, `canonical_id` and `graph_entity_id` are never
/// touched — a merge must not change the id anything else references. See the
/// module docs for exactly what moves and what is kept.
///
/// **This function does not gate itself.** Confirmation lives at the callers:
/// the HTTP route requires an explicit `confirm`, and the agent tool files a
/// Decision Inbox card that a person has to approve.
pub async fn merge_people(
    pool: &Pool<Sqlite>,
    brain: Option<&SafeBrain>,
    survivor_uuid: &str,
    duplicate_uuid: &str,
) -> Result<MergeReport, String> {
    let (survivor, duplicate) = load_pair(pool, survivor_uuid, duplicate_uuid).await?;
    let duplicate_row = snapshot_person_row(pool, duplicate_uuid).await?;
    let project_links = project_link_moves(pool, survivor_uuid, duplicate_uuid).await?;
    let (planned_fields, _, _) = field_and_edge_plan(brain, &survivor, &duplicate).await;

    // 1. Meetings. Capture the ids BEFORE the update, so undo knows exactly
    //    which of the survivor's meetings came from the duplicate.
    let moved_meeting_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM person_meetings WHERE entity_uuid = ?")
            .bind(duplicate_uuid)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE person_meetings SET entity_uuid = ? WHERE entity_uuid = ?")
        .bind(survivor_uuid)
        .bind(duplicate_uuid)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 2. Project links. INSERT OR IGNORE keeps the survivor's own role when
    //    both are on a project; the duplicate's row then falls away with it.
    let mut links_moved = 0usize;
    let mut links_dropped = 0usize;
    for link in &project_links {
        if link.survivor_already_linked {
            links_dropped += 1;
        } else {
            sqlx::query(
                "INSERT OR IGNORE INTO project_people (project_id, entity_uuid, role) \
                 VALUES (?, ?, ?)",
            )
            .bind(&link.project_id)
            .bind(survivor_uuid)
            .bind(link.role.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            links_moved += 1;
        }
    }
    sqlx::query("DELETE FROM project_people WHERE entity_uuid = ?")
        .bind(duplicate_uuid)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    // 3. Contact columns the survivor left blank. Column-level only; graph
    //    fields are handled below and are authoritative for attributes.
    sqlx::query(
        "UPDATE people SET \
           role            = COALESCE(NULLIF(TRIM(role), ''), ?), \
           company         = COALESCE(NULLIF(TRIM(company), ''), ?), \
           email           = COALESCE(NULLIF(TRIM(email), ''), ?), \
           phone           = COALESCE(NULLIF(TRIM(phone), ''), ?), \
           notes           = COALESCE(NULLIF(TRIM(notes), ''), ?), \
           last_contact_at = MAX(COALESCE(last_contact_at, ''), COALESCE(?, '')), \
           updated_at      = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE entity_uuid = ?",
    )
    .bind(duplicate.role.as_deref())
    .bind(duplicate.company.as_deref())
    .bind(duplicate.email.as_deref())
    .bind(duplicate.phone.as_deref())
    .bind(duplicate.notes.as_deref())
    .bind(duplicate.last_contact_at.as_deref())
    .bind(survivor_uuid)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    // `MAX(x, '')` yields '' when both are empty; normalise that back to NULL.
    sqlx::query(
        "UPDATE people SET last_contact_at = NULL WHERE entity_uuid = ? AND last_contact_at = ''",
    )
    .bind(survivor_uuid)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    // 4. Graph: copy the duplicate's edges onto the survivor through Spectral's
    //    own insert, then delete the originals through the documented bridge.
    let mut moved_edges: Vec<MovedTriple> = Vec::new();
    if let (Some(brain), Some(dup_hex), Some(surv_hex)) = (
        brain,
        duplicate.graph_entity_id.as_deref(),
        survivor.graph_entity_id.as_deref(),
    ) {
        match brain.copy_entity_triples(dup_hex, surv_hex).await {
            Ok(edges) => {
                let db = crate::config::paths::Paths::brain_dir().join("graph.sqlite");
                for e in &edges {
                    if let Err(err) = crate::project_graph::delete_graph_triple(
                        &db,
                        &e.from_id,
                        &e.to_id,
                        &e.predicate,
                    ) {
                        tracing::warn!(
                            target: "permagent::people_merge",
                            error = %err,
                            "could not delete the duplicate's original graph edge"
                        );
                    }
                }
                moved_edges = edges;
            }
            Err(e) => tracing::warn!(
                target: "permagent::people_merge",
                error = %e,
                "graph edges were not moved; the duplicate's node keeps them"
            ),
        }

        // 5. Graph fields into slots the survivor left empty, provenance kept.
        if let Ok(surv_id) = surv_hex.parse::<spectral::core::entity_id::EntityId>() {
            for f in &planned_fields {
                if let Err(e) = brain
                    .set_entity_field(
                        surv_id,
                        &f.field_name,
                        &f.value,
                        parse_field_source(&f.source),
                        None,
                    )
                    .await
                {
                    tracing::warn!(
                        target: "permagent::people_merge",
                        field = %f.field_name,
                        error = %e,
                        "could not copy a graph field onto the survivor"
                    );
                }
            }
        }
    }

    // 6. Aliases + log, then drop the duplicate row (cascading anything left).
    let merge_id = Uuid::new_v4().to_string();
    let snapshot = MergeSnapshot {
        duplicate_row,
        moved_meeting_ids: moved_meeting_ids.clone(),
        project_links: project_links.clone(),
        copied_fields: planned_fields.clone(),
        moved_edges: moved_edges.clone(),
        survivor_graph_entity_id: survivor.graph_entity_id.clone(),
        duplicate_graph_entity_id: duplicate.graph_entity_id.clone(),
    };
    let summary = format!(
        "Merged \"{}\" into \"{}\": {} meeting(s), {} project link(s), {} field(s), {} graph edge(s)",
        duplicate.display_name,
        survivor.display_name,
        moved_meeting_ids.len(),
        links_moved,
        planned_fields.len(),
        moved_edges.len(),
    );

    sqlx::query(
        "INSERT INTO person_merge_log (id, kind, survivor_uuid, duplicate_uuid, summary, snapshot) \
         VALUES (?, 'merge', ?, ?, ?, ?)",
    )
    .bind(&merge_id)
    .bind(survivor_uuid)
    .bind(duplicate_uuid)
    .bind(&summary)
    .bind(serde_json::to_string(&snapshot).map_err(|e| e.to_string())?)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Drop the duplicate before recording its identifiers as aliases, so no
    // window exists in which the same uuid is both a live person and an alias
    // pointing somewhere else.
    sqlx::query("DELETE FROM people WHERE entity_uuid = ?")
        .bind(duplicate_uuid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut aliases_recorded = 0usize;
    for (kind, value) in alias_pairs(&duplicate) {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO person_aliases (id, entity_uuid, alias_kind, alias_value, merge_id) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(survivor_uuid)
        .bind(kind)
        .bind(&value)
        .bind(&merge_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        aliases_recorded += res.rows_affected() as usize;
    }

    crate::events::emit(crate::events::person_merged(
        survivor_uuid,
        duplicate_uuid,
        &merge_id,
    ));
    crate::events::emit(crate::events::person_changed("", survivor_uuid, "merged"));

    Ok(MergeReport {
        merge_id,
        survivor_uuid: survivor_uuid.to_string(),
        survivor_name: survivor.display_name.clone(),
        duplicate_uuid: duplicate_uuid.to_string(),
        duplicate_name: duplicate.display_name.clone(),
        meetings_moved: moved_meeting_ids.len(),
        project_links_moved: links_moved,
        project_links_dropped: links_dropped,
        fields_copied: planned_fields.len(),
        graph_edges_moved: moved_edges.len(),
        aliases_recorded,
        summary,
    })
}

// ── Undo ───────────────────────────────────────────────────────────────────

/// What an undo restored, and what it could not.
#[derive(Debug, Clone, Serialize)]
pub struct UndoReport {
    pub merge_id: String,
    pub restored_uuid: String,
    pub restored_name: String,
    pub meetings_restored: usize,
    pub project_links_restored: usize,
    pub graph_edges_restored: usize,
    pub aliases_removed: usize,
    /// Things undo deliberately did not revert, in plain language.
    pub not_reverted: Vec<String>,
}

/// Reverse a merge from its snapshot. Idempotent-ish: a merge that has already
/// been undone is refused rather than replayed.
pub async fn undo_merge(
    pool: &Pool<Sqlite>,
    brain: Option<&SafeBrain>,
    merge_id: &str,
) -> Result<UndoReport, String> {
    let row = sqlx::query(
        "SELECT survivor_uuid, duplicate_uuid, snapshot, undone_at, kind \
         FROM person_merge_log WHERE id = ?",
    )
    .bind(merge_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("No such merge: {merge_id}"))?;
    if row.get::<Option<String>, _>("undone_at").is_some() {
        return Err("That merge has already been undone.".to_string());
    }
    if row.get::<String, _>("kind") != "merge" {
        return Err("That log entry is a delete, not a merge — use the delete undo.".to_string());
    }
    let survivor_uuid: String = row
        .get::<Option<String>, _>("survivor_uuid")
        .ok_or("Merge log row has no survivor")?;
    let snapshot: MergeSnapshot = serde_json::from_str(&row.get::<String, _>("snapshot"))
        .map_err(|e| format!("merge snapshot unreadable: {e}"))?;
    let dup = &snapshot.duplicate_row;

    // 1. Aliases first — the duplicate's uuid/canonical_id must stop being an
    //    alias before the row that owns them comes back.
    let aliases_removed = sqlx::query("DELETE FROM person_aliases WHERE merge_id = ?")
        .bind(merge_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected() as usize;

    // 2. The row itself, verbatim. `canonical_id` is UNIQUE, and `INSERT OR
    //    REPLACE` resolves a UNIQUE conflict by DELETING the conflicting row —
    //    so a person created under that slug since the merge would be silently
    //    destroyed by the undo. Refuse instead, and say what is in the way.
    let squatter: Option<String> = sqlx::query_scalar(
        "SELECT entity_uuid FROM people WHERE canonical_id = ? AND entity_uuid != ?",
    )
    .bind(&dup.canonical_id)
    .bind(&dup.entity_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(other) = squatter {
        // Put the aliases back — this undo is not happening.
        for (kind, value) in alias_pairs_from_snapshot(dup) {
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO person_aliases \
                   (id, entity_uuid, alias_kind, alias_value, merge_id) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&survivor_uuid)
            .bind(kind)
            .bind(&value)
            .bind(merge_id)
            .execute(pool)
            .await;
        }
        return Err(format!(
            "Cannot undo: \"{}\" was created under the slug {} since the merge (id {other}). \
             Rename that person first.",
            dup.display_name, dup.canonical_id
        ));
    }
    sqlx::query(
        "INSERT OR REPLACE INTO people \
           (entity_uuid, canonical_id, display_name, role, company, email, phone, notes, \
            last_contact_at, graph_entity_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&dup.entity_uuid)
    .bind(&dup.canonical_id)
    .bind(&dup.display_name)
    .bind(dup.role.as_deref())
    .bind(dup.company.as_deref())
    .bind(dup.email.as_deref())
    .bind(dup.phone.as_deref())
    .bind(dup.notes.as_deref())
    .bind(dup.last_contact_at.as_deref())
    .bind(dup.graph_entity_id.as_deref())
    .bind(&dup.created_at)
    .bind(&dup.updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 3. Meetings and project links go back by id, so a meeting the survivor
    //    logged *after* the merge is left alone.
    let mut meetings_restored = 0usize;
    for id in &snapshot.moved_meeting_ids {
        meetings_restored += sqlx::query(
            "UPDATE person_meetings SET entity_uuid = ? WHERE id = ? AND entity_uuid = ?",
        )
        .bind(&dup.entity_uuid)
        .bind(id)
        .bind(&survivor_uuid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected() as usize;
    }

    let mut links_restored = 0usize;
    for link in &snapshot.project_links {
        sqlx::query(
            "INSERT OR IGNORE INTO project_people (project_id, entity_uuid, role) VALUES (?, ?, ?)",
        )
        .bind(&link.project_id)
        .bind(&dup.entity_uuid)
        .bind(link.role.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        links_restored += 1;
        if !link.survivor_already_linked {
            sqlx::query("DELETE FROM project_people WHERE project_id = ? AND entity_uuid = ?")
                .bind(&link.project_id)
                .bind(&survivor_uuid)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // 4. Graph edges: put the originals back, and remove the survivor-side
    //    copies the merge created (never one the survivor already had).
    let mut edges_restored = 0usize;
    if let (Some(brain), Some(dup_hex), Some(surv_hex)) = (
        brain,
        snapshot.duplicate_graph_entity_id.as_deref(),
        snapshot.survivor_graph_entity_id.as_deref(),
    ) {
        let db = crate::config::paths::Paths::brain_dir().join("graph.sqlite");
        for e in &snapshot.moved_edges {
            if brain
                .restore_triple(&e.from_id, &e.to_id, &e.predicate)
                .await
                .unwrap_or(false)
            {
                edges_restored += 1;
            }
            if e.survivor_already_had_it {
                continue;
            }
            let new_from = if e.from_id == dup_hex {
                surv_hex
            } else {
                &e.from_id
            };
            let new_to = if e.to_id == dup_hex {
                surv_hex
            } else {
                &e.to_id
            };
            let _ = crate::project_graph::delete_graph_triple(&db, new_from, new_to, &e.predicate);
        }
    }

    sqlx::query(
        "UPDATE person_merge_log SET undone_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?",
    )
    .bind(merge_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    crate::events::emit(crate::events::person_changed(
        "",
        &dup.entity_uuid,
        "created",
    ));
    crate::events::emit(crate::events::person_changed("", &survivor_uuid, "updated"));

    Ok(UndoReport {
        merge_id: merge_id.to_string(),
        restored_uuid: dup.entity_uuid.clone(),
        restored_name: dup.display_name.clone(),
        meetings_restored,
        project_links_restored: links_restored,
        graph_edges_restored: edges_restored,
        aliases_removed,
        not_reverted: if snapshot.copied_fields.is_empty() {
            Vec::new()
        } else {
            vec![format!(
                "{} graph field(s) copied onto the survivor stay there — Spectral has no \
                 field-delete API. They only filled slots the survivor had left empty.",
                snapshot.copied_fields.len()
            )]
        },
    })
}

// ── Delete ─────────────────────────────────────────────────────────────────

/// What a delete removed, and what it left behind.
#[derive(Debug, Clone, Serialize)]
pub struct DeleteReport {
    pub entity_uuid: String,
    pub display_name: String,
    /// The `person_merge_log` id holding the snapshot of what was deleted.
    pub log_id: String,
    pub meetings_deleted: usize,
    pub project_links_deleted: usize,
    pub graph_edges_deleted: usize,
    pub aliases_deleted: usize,
    /// Plain-language statement of what survives the delete.
    pub retained: Vec<String>,
}

/// Delete a person from the directory.
///
/// Removes: the `people` row, and by ON DELETE CASCADE their `person_meetings`
/// (including follow-ups), `project_people` links and absorbed
/// `person_aliases`. Removes their person↔person and person→project graph
/// edges through the documented direct-SQL bridge.
///
/// Keeps, and says so: the graph entity node and its `entity_fields` (Spectral
/// exposes no entity delete), and Brain memories that mention them — those are
/// records of things that happened, keyed by name and content, not rows owned
/// by this person. Deleting them is `Brain::forget` on specific memory keys,
/// which is a separate, explicit act.
///
/// Like [`merge_people`], this does not gate itself; the route and the tool do.
pub async fn delete_person(
    pool: &Pool<Sqlite>,
    brain: Option<&SafeBrain>,
    entity_uuid: &str,
) -> Result<DeleteReport, String> {
    let person = people::get_by_uuid(pool, entity_uuid)
        .await?
        .ok_or_else(|| format!("Person {entity_uuid} not found"))?;
    let row = snapshot_person_row(pool, entity_uuid).await?;

    let meetings: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM person_meetings WHERE entity_uuid = ?")
            .bind(entity_uuid)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_people WHERE entity_uuid = ?")
            .bind(entity_uuid)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let aliases: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM person_aliases WHERE entity_uuid = ?")
            .bind(entity_uuid)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let project_links = project_link_moves(pool, entity_uuid, entity_uuid).await?;

    let mut edges_deleted = 0usize;
    let mut deleted_edges: Vec<MovedTriple> = Vec::new();
    if let (Some(brain), Some(hex)) = (brain, person.graph_entity_id.as_deref()) {
        if let Ok(edges) = brain.person_edges(hex).await {
            let db = crate::config::paths::Paths::brain_dir().join("graph.sqlite");
            for e in edges {
                if crate::project_graph::delete_graph_triple(
                    &db,
                    &e.from_id,
                    &e.to_id,
                    &e.predicate,
                )
                .unwrap_or(0)
                    > 0
                {
                    edges_deleted += 1;
                }
                deleted_edges.push(MovedTriple {
                    from_id: e.from_id,
                    to_id: e.to_id,
                    predicate: e.predicate,
                    survivor_already_had_it: false,
                });
            }
        }
    }

    let log_id = Uuid::new_v4().to_string();
    let snapshot = MergeSnapshot {
        duplicate_row: row,
        moved_meeting_ids: sqlx::query_scalar(
            "SELECT id FROM person_meetings WHERE entity_uuid = ?",
        )
        .bind(entity_uuid)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?,
        project_links,
        copied_fields: Vec::new(),
        moved_edges: deleted_edges,
        survivor_graph_entity_id: None,
        duplicate_graph_entity_id: person.graph_entity_id.clone(),
    };
    sqlx::query(
        "INSERT INTO person_merge_log (id, kind, survivor_uuid, duplicate_uuid, summary, snapshot) \
         VALUES (?, 'delete', NULL, ?, ?, ?)",
    )
    .bind(&log_id)
    .bind(entity_uuid)
    .bind(format!(
        "Deleted \"{}\": {meetings} meeting(s), {links} project link(s), {edges_deleted} graph edge(s)",
        person.display_name
    ))
    .bind(serde_json::to_string(&snapshot).map_err(|e| e.to_string())?)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM people WHERE entity_uuid = ?")
        .bind(entity_uuid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    crate::events::emit(crate::events::person_changed("", entity_uuid, "deleted"));

    Ok(DeleteReport {
        entity_uuid: entity_uuid.to_string(),
        display_name: person.display_name.clone(),
        log_id,
        meetings_deleted: meetings as usize,
        project_links_deleted: links as usize,
        graph_edges_deleted: edges_deleted,
        aliases_deleted: aliases as usize,
        retained: vec![
            "Their graph entity node and its stored fields stay — Spectral has no \
             entity-delete API. With their edges gone the node is inert."
                .to_string(),
            "Brain memories that mention them stay. They record things that happened; \
             removing one is a separate, explicit forget."
                .to_string(),
        ],
    })
}

// ── Alias lookup ───────────────────────────────────────────────────────────

/// Resolve an id that may belong to a person who was merged away. Returns the
/// surviving `entity_uuid`, or `None` when the id is not an absorbed alias.
pub async fn resolve_alias(pool: &Pool<Sqlite>, id: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar(
        "SELECT entity_uuid FROM person_aliases \
         WHERE alias_value = ? AND alias_kind IN ('entity_uuid','canonical_id','graph_entity_id')",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())
}

/// Every name this person answers to — their own plus any absorbed by a merge.
/// `/api/people/{id}/activity` searches all of them, which is what carries a
/// merged-away person's memories onto the survivor's profile.
pub async fn names_for(pool: &Pool<Sqlite>, person: &Person) -> Vec<String> {
    let mut names = vec![person.display_name.clone()];
    let absorbed: Vec<String> = sqlx::query_scalar(
        "SELECT alias_value FROM person_aliases \
         WHERE entity_uuid = ? AND alias_kind = 'display_name'",
    )
    .bind(&person.entity_uuid)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for n in absorbed {
        if !names
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&n))
        {
            names.push(n);
        }
    }
    names.retain(|n| !n.trim().is_empty());
    names
}

/// The merge/delete log, newest first — the audit trail behind the undo button.
#[derive(Debug, Clone, Serialize)]
pub struct MergeLogEntry {
    pub id: String,
    pub kind: String,
    pub survivor_uuid: Option<String>,
    pub duplicate_uuid: String,
    pub summary: String,
    pub undone_at: Option<String>,
    pub created_at: String,
}

pub async fn list_merge_log(pool: &Pool<Sqlite>, limit: i64) -> Result<Vec<MergeLogEntry>, String> {
    let rows = sqlx::query(
        "SELECT id, kind, survivor_uuid, duplicate_uuid, summary, undone_at, created_at \
         FROM person_merge_log ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| MergeLogEntry {
            id: r.get("id"),
            kind: r.get("kind"),
            survivor_uuid: r.get("survivor_uuid"),
            duplicate_uuid: r.get("duplicate_uuid"),
            summary: r.get("summary"),
            undone_at: r.get("undone_at"),
            created_at: r.get("created_at"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::{upsert_person, PersonAttrs};
    use crate::projects::{create_project, CreateProject};
    use crate::session::spectral_schema::init_spectral_db;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // The cascades this module relies on are off by default per connection.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    async fn a_person(pool: &Pool<Sqlite>, slug: &str, name: &str, attrs: PersonAttrs) -> Person {
        upsert_person(pool, &format!("person:{slug}"), name, &attrs)
            .await
            .unwrap()
    }

    async fn a_project(pool: &Pool<Sqlite>, name: &str) -> String {
        create_project(
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

    /// Insert a meeting row directly. `create_meeting` write-throughs to
    /// Calendar.app; this test cares about the row, not the calendar.
    async fn a_meeting(pool: &Pool<Sqlite>, entity_uuid: &str, id: &str, follow_up: Option<&str>) {
        sqlx::query(
            "INSERT INTO person_meetings \
               (id, entity_uuid, title, starts_at, notes, follow_up_at, follow_up_done) \
             VALUES (?, ?, 'Coffee', '2026-08-01T10:00:00Z', '', ?, 0)",
        )
        .bind(id)
        .bind(entity_uuid)
        .bind(follow_up)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn meeting_owner(pool: &Pool<Sqlite>, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT entity_uuid FROM person_meetings WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    // ── Suggestion scoring ────────────────────────────────────────────────

    #[test]
    fn normalizers_fold_the_differences_that_do_not_matter() {
        assert_eq!(normalize_name("  Mel   SCHEMBRI "), "mel schembri");
        assert_eq!(
            normalize_email(" Mel@Example.COM ").as_deref(),
            Some("mel@example.com")
        );
        assert_eq!(normalize_email("not-an-email"), None);
        assert_eq!(
            normalize_phone("+1 (902) 555-0134").as_deref(),
            Some("9025550134")
        );
        assert_eq!(
            normalize_phone("902-555-0134").as_deref(),
            Some("9025550134")
        );
        // Too short to be a phone number at all.
        assert_eq!(normalize_phone("555"), None);
    }

    #[test]
    fn name_similarity_is_order_insensitive_and_honest_about_strangers() {
        assert_eq!(name_token_similarity("mel schembri", "schembri mel"), 1.0);
        assert!(name_token_similarity("mel schembri", "ashley lecroy") == 0.0);
        let partial = name_token_similarity("mel schembri", "melanie schembri");
        assert!((0.3..0.4).contains(&partial), "got {partial}");
    }

    #[tokio::test]
    async fn suggestions_rank_email_above_name_and_ignore_strangers() {
        let pool = test_pool().await;
        let a = a_person(
            &pool,
            "mel-a",
            "Mel Schembri",
            PersonAttrs {
                email: Some("mel@example.com".into()),
                ..Default::default()
            },
        )
        .await;
        let b = a_person(
            &pool,
            "mel-b",
            "Melanie Schembri",
            PersonAttrs {
                email: Some("MEL@example.com".into()),
                ..Default::default()
            },
        )
        .await;
        let c = a_person(&pool, "ash", "Ashley Lecroy", PersonAttrs::default()).await;
        let d = a_person(&pool, "ash-2", "Ashley Lecroy", PersonAttrs::default()).await;
        let e = a_person(&pool, "sabaa", "Sabaa Quao", PersonAttrs::default()).await;

        let people = vec![a.clone(), b.clone(), c.clone(), d.clone(), e.clone()];
        let out = suggest_duplicates(&people, 10);

        // The shared email pair outranks the shared-name pair.
        assert!(out[0].score > out[1].score, "{out:#?}");
        assert!(out[0].reasons.iter().any(|r| r.contains("same email")));
        assert_eq!(out[1].reasons, vec!["identical name".to_string()]);
        // Sabaa Quao pairs with nobody.
        assert!(
            !out.iter()
                .any(|s| s.survivor_uuid == e.entity_uuid || s.duplicate_uuid == e.entity_uuid),
            "a stranger was suggested as a duplicate: {out:#?}"
        );
        // Exactly the two real pairs.
        assert_eq!(out.len(), 2, "{out:#?}");
        // The older row is the proposed survivor.
        assert_eq!(out[0].survivor_uuid, a.entity_uuid);
        assert_eq!(out[0].duplicate_uuid, b.entity_uuid);
    }

    #[test]
    fn same_company_alone_is_never_a_duplicate() {
        let base = |slug: &str, name: &str| Person {
            entity_uuid: slug.to_string(),
            canonical_id: format!("person:{slug}"),
            display_name: name.to_string(),
            role: None,
            company: Some("LAUFT".into()),
            email: None,
            phone: None,
            notes: None,
            last_contact_at: None,
            birthday: None,
            relationship_strength: None,
            how_met: None,
            linkedin: None,
            x_handle: None,
            facebook: None,
            instagram: None,
            personal_site: None,
            photo_url: None,
            find_online_hints: None,
            graph_entity_id: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        let (score, reasons) = score_pair(&base("a", "Mel Schembri"), &base("b", "Sabaa Quao"));
        assert_eq!(score, 0.0, "reasons: {reasons:?}");
        assert!(reasons.is_empty());
    }

    // ── Preview ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn preview_counts_what_moves_without_moving_it() {
        let pool = test_pool().await;
        let keep = a_person(&pool, "keep", "Mel Schembri", PersonAttrs::default()).await;
        let dup = a_person(&pool, "dup", "Mel S", PersonAttrs::default()).await;
        let shared = a_project(&pool, "LAUFT").await;
        let only_dup = a_project(&pool, "Love Nova Scotia").await;
        crate::project_association::associate_person(&pool, &shared, &keep.entity_uuid, None)
            .await
            .unwrap();
        crate::project_association::associate_person(&pool, &shared, &dup.entity_uuid, None)
            .await
            .unwrap();
        crate::project_association::associate_person(&pool, &only_dup, &dup.entity_uuid, None)
            .await
            .unwrap();
        a_meeting(&pool, &dup.entity_uuid, "m1", Some("2026-09-01T10:00:00Z")).await;
        a_meeting(&pool, &dup.entity_uuid, "m2", None).await;

        let preview = preview_merge(&pool, None, &keep.entity_uuid, &dup.entity_uuid)
            .await
            .unwrap();
        assert_eq!(preview.meetings, 2);
        assert_eq!(preview.open_follow_ups, 1);
        assert_eq!(preview.project_links.len(), 2);
        assert_eq!(
            preview
                .project_links
                .iter()
                .filter(|l| l.survivor_already_linked)
                .count(),
            1
        );
        assert!(preview
            .aliases
            .iter()
            .any(|a| a == &format!("entity_uuid: {}", dup.entity_uuid)));
        assert!(preview.aliases.iter().any(|a| a == "display_name: Mel S"));
        assert!(
            !preview.retained.is_empty(),
            "the preview must state its limits"
        );

        // Nothing moved.
        assert_eq!(
            meeting_owner(&pool, "m1").await.as_deref(),
            Some(dup.entity_uuid.as_str())
        );
        assert!(people::get_by_uuid(&pool, &dup.entity_uuid)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn a_person_cannot_be_merged_into_themselves() {
        let pool = test_pool().await;
        let p = a_person(&pool, "solo", "Mel Schembri", PersonAttrs::default()).await;
        let err = preview_merge(&pool, None, &p.entity_uuid, &p.entity_uuid)
            .await
            .unwrap_err();
        assert!(err.contains("cannot be merged into themselves"), "{err}");
    }

    // ── Merge ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn merge_moves_everything_keeps_the_survivor_id_and_deletes_the_duplicate() {
        let pool = test_pool().await;
        let keep = a_person(
            &pool,
            "keep",
            "Mel Schembri",
            PersonAttrs {
                email: Some("mel@example.com".into()),
                ..Default::default()
            },
        )
        .await;
        let dup = a_person(
            &pool,
            "dup",
            "Mel S",
            PersonAttrs {
                // The survivor has no phone; the duplicate's fills the gap.
                phone: Some("902-555-0134".into()),
                // The survivor HAS an email; the duplicate's must not win.
                email: Some("stale@example.com".into()),
                ..Default::default()
            },
        )
        .await;
        let shared = a_project(&pool, "LAUFT").await;
        let only_dup = a_project(&pool, "Love Nova Scotia").await;
        crate::project_association::associate_person(&pool, &shared, &keep.entity_uuid, None)
            .await
            .unwrap();
        crate::project_association::associate_person(&pool, &shared, &dup.entity_uuid, None)
            .await
            .unwrap();
        crate::project_association::associate_person(
            &pool,
            &only_dup,
            &dup.entity_uuid,
            Some("advisor"),
        )
        .await
        .unwrap();
        a_meeting(&pool, &dup.entity_uuid, "m1", Some("2026-09-01T10:00:00Z")).await;
        a_meeting(&pool, &keep.entity_uuid, "own", None).await;

        let mut rx = crate::events::subscribe();
        let report = merge_people(&pool, None, &keep.entity_uuid, &dup.entity_uuid)
            .await
            .unwrap();

        // Survivor id is stable.
        let survivor = people::get_by_uuid(&pool, &keep.entity_uuid)
            .await
            .unwrap()
            .expect("survivor still there under the same id");
        assert_eq!(survivor.entity_uuid, keep.entity_uuid);
        assert_eq!(survivor.canonical_id, keep.canonical_id);

        // Duplicate is gone.
        assert!(people::get_by_uuid(&pool, &dup.entity_uuid)
            .await
            .unwrap()
            .is_none());

        // Meetings moved; the survivor's own meeting is untouched.
        assert_eq!(
            meeting_owner(&pool, "m1").await.as_deref(),
            Some(keep.entity_uuid.as_str())
        );
        assert_eq!(
            meeting_owner(&pool, "own").await.as_deref(),
            Some(keep.entity_uuid.as_str())
        );
        assert_eq!(report.meetings_moved, 1);

        // Project links: the unique one moved, the shared one was dropped.
        let links = crate::project_association::list_person_projects(&pool, &keep.entity_uuid)
            .await
            .unwrap();
        assert_eq!(links.len(), 2, "{links:#?}");
        assert_eq!(report.project_links_moved, 1);
        assert_eq!(report.project_links_dropped, 1);

        // Blank survivor columns filled; populated ones kept.
        assert_eq!(survivor.phone.as_deref(), Some("902-555-0134"));
        assert_eq!(survivor.email.as_deref(), Some("mel@example.com"));

        // Aliases recorded, so the dead id and the absorbed name still resolve.
        assert_eq!(
            resolve_alias(&pool, &dup.entity_uuid)
                .await
                .unwrap()
                .as_deref(),
            Some(keep.entity_uuid.as_str())
        );
        let names = names_for(&pool, &survivor).await;
        assert!(names.contains(&"Mel S".to_string()), "{names:?}");
        assert!(names.contains(&"Mel Schembri".to_string()), "{names:?}");

        // The bus was told — the graph updates without a reload (#1090).
        let mut saw_merged = false;
        while let Ok(event) = rx.try_recv() {
            if event.event_type == crate::events::PermagentEventType::PersonMerged
                && event.payload.get("survivor_uuid").and_then(|v| v.as_str())
                    == Some(keep.entity_uuid.as_str())
                && event.payload.get("duplicate_uuid").and_then(|v| v.as_str())
                    == Some(dup.entity_uuid.as_str())
            {
                saw_merged = true;
            }
        }
        assert!(saw_merged, "merge did not emit person_merged");

        // And the whole thing is logged.
        let log = list_merge_log(&pool, 10).await.unwrap();
        assert!(log
            .iter()
            .any(|e| e.id == report.merge_id && e.kind == "merge"));
    }

    #[tokio::test]
    async fn undo_puts_the_duplicate_back_exactly_as_it_was() {
        let pool = test_pool().await;
        let keep = a_person(&pool, "keep", "Mel Schembri", PersonAttrs::default()).await;
        let dup = a_person(
            &pool,
            "dup",
            "Mel S",
            PersonAttrs {
                phone: Some("902-555-0134".into()),
                ..Default::default()
            },
        )
        .await;
        let only_dup = a_project(&pool, "Love Nova Scotia").await;
        crate::project_association::associate_person(&pool, &only_dup, &dup.entity_uuid, None)
            .await
            .unwrap();
        a_meeting(&pool, &dup.entity_uuid, "m1", None).await;

        let report = merge_people(&pool, None, &keep.entity_uuid, &dup.entity_uuid)
            .await
            .unwrap();
        // A meeting logged on the survivor AFTER the merge must not be dragged back.
        a_meeting(&pool, &keep.entity_uuid, "after", None).await;

        let undo = undo_merge(&pool, None, &report.merge_id).await.unwrap();
        assert_eq!(undo.restored_uuid, dup.entity_uuid);
        assert_eq!(undo.meetings_restored, 1);

        let back = people::get_by_uuid(&pool, &dup.entity_uuid)
            .await
            .unwrap()
            .expect("duplicate restored");
        assert_eq!(back.canonical_id, dup.canonical_id);
        assert_eq!(back.display_name, "Mel S");
        assert_eq!(
            back.created_at, dup.created_at,
            "created_at must be verbatim"
        );

        assert_eq!(
            meeting_owner(&pool, "m1").await.as_deref(),
            Some(dup.entity_uuid.as_str())
        );
        assert_eq!(
            meeting_owner(&pool, "after").await.as_deref(),
            Some(keep.entity_uuid.as_str()),
            "a meeting logged after the merge stays with the survivor"
        );

        // The project link went back and left the survivor.
        let dup_links = crate::project_association::list_person_projects(&pool, &dup.entity_uuid)
            .await
            .unwrap();
        assert_eq!(dup_links.len(), 1);
        let keep_links = crate::project_association::list_person_projects(&pool, &keep.entity_uuid)
            .await
            .unwrap();
        assert!(keep_links.is_empty(), "{keep_links:#?}");

        // The aliases are released, so the restored row owns its own id again.
        assert_eq!(resolve_alias(&pool, &dup.entity_uuid).await.unwrap(), None);

        // A second undo is refused rather than replayed.
        let err = undo_merge(&pool, None, &report.merge_id).await.unwrap_err();
        assert!(err.contains("already been undone"), "{err}");
    }

    // ── Delete ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_cascades_meetings_and_links_and_says_what_it_keeps() {
        let pool = test_pool().await;
        let p = a_person(&pool, "gone", "Ashley Lecroy", PersonAttrs::default()).await;
        let project = a_project(&pool, "LAUFT").await;
        crate::project_association::associate_person(&pool, &project, &p.entity_uuid, None)
            .await
            .unwrap();
        a_meeting(&pool, &p.entity_uuid, "m1", Some("2026-09-01T10:00:00Z")).await;
        a_meeting(&pool, &p.entity_uuid, "m2", None).await;

        let mut rx = crate::events::subscribe();
        let report = delete_person(&pool, None, &p.entity_uuid).await.unwrap();
        assert_eq!(report.meetings_deleted, 2);
        assert_eq!(report.project_links_deleted, 1);
        assert!(
            !report.retained.is_empty(),
            "a delete must state what it keeps"
        );

        assert!(people::get_by_uuid(&pool, &p.entity_uuid)
            .await
            .unwrap()
            .is_none());
        assert_eq!(meeting_owner(&pool, "m1").await, None, "meetings cascade");
        assert_eq!(meeting_owner(&pool, "m2").await, None);
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_people WHERE entity_uuid = ?")
                .bind(&p.entity_uuid)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0, "project links cascade");

        // Snapshot survives the row it describes — that is the audit trail.
        let log = list_merge_log(&pool, 10).await.unwrap();
        let entry = log.iter().find(|e| e.id == report.log_id).expect("logged");
        assert_eq!(entry.kind, "delete");
        assert_eq!(entry.duplicate_uuid, p.entity_uuid);

        let mut saw_deleted = false;
        while let Ok(event) = rx.try_recv() {
            if event.event_type == crate::events::PermagentEventType::PersonChanged
                && event.payload.get("change").and_then(|v| v.as_str()) == Some("deleted")
                && event.payload.get("entity_uuid").and_then(|v| v.as_str())
                    == Some(p.entity_uuid.as_str())
            {
                saw_deleted = true;
            }
        }
        assert!(saw_deleted, "delete did not emit person_changed(deleted)");
    }

    #[tokio::test]
    async fn deleting_a_survivor_takes_its_absorbed_aliases_with_it() {
        let pool = test_pool().await;
        let keep = a_person(&pool, "keep", "Mel Schembri", PersonAttrs::default()).await;
        let dup = a_person(&pool, "dup", "Mel S", PersonAttrs::default()).await;
        merge_people(&pool, None, &keep.entity_uuid, &dup.entity_uuid)
            .await
            .unwrap();
        delete_person(&pool, None, &keep.entity_uuid).await.unwrap();

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM person_aliases")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "aliases must cascade with the survivor");
        // And the dead duplicate id no longer resolves to anyone.
        assert_eq!(resolve_alias(&pool, &dup.entity_uuid).await.unwrap(), None);
    }
}
