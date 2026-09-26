//! People (CRM) module — typed person records keyed on an opaque, immutable
//! entity UUID.
//!
//! # Opaque entity_uuid (the persona_id pattern)
//!
//! [`crate::identity::canonical::canonicalize_entity_id`] mints a *name-derived*
//! slug (e.g. `person:jane-doe`). That slug is convenient for cross-referencing
//! Brain graph triples, but it is **driftable**: rename the person and the slug
//! changes. Keying durable CRM rows on a driftable identifier would orphan a
//! person's history on every rename.
//!
//! So this table follows the `persona_id` pattern (see
//! [`crate::config::agent_identity`]): every person gets an opaque, immutable
//! `entity_uuid` minted **once** at first save, and `canonical_id` is a *mutable
//! lookup column* (UNIQUE) that may be rewritten on rename. Downstream rows key
//! on `entity_uuid`, never on the slug.
//!
//! # Scope (v1)
//!
//! Read-only surface (see `routes::people`). The write primitives here
//! ([`upsert_person`], [`rename_canonical_id`]) are the substrate the v1.5
//! Librarian enrichment pass will build on; v1 exposes no write HTTP endpoint
//! and never touches Spectral.

use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

// ── Data types ─────────────────────────────────────────────────────────────

/// A typed CRM person record. Carries attributes the Brain graph `GraphEntity`
/// lacks (role/company/email/phone/last_contact/notes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Person {
    /// Opaque, immutable primary key. Minted once at first save; stable across
    /// any later rename of `canonical_id` / `display_name`.
    pub entity_uuid: String,
    /// Mutable name-derived slug (e.g. `person:jane-doe`). UNIQUE lookup column.
    pub canonical_id: String,
    pub display_name: String,
    pub role: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub last_contact_at: Option<String>,
    /// Graph-only manual attributes (#495 vocab, slice 2b). These have **no
    /// people-table column** — they live solely in the graph's `entity_fields`
    /// and are populated by the read overlay from there. `None` on a fresh
    /// row; set once the manual write path (`FieldSource::Manual`) records one.
    pub birthday: Option<String>,
    pub relationship_strength: Option<String>,
    pub how_met: Option<String>,
    /// Enricher-sourced profile links (#495 slice 4 vocab). Like the three
    /// above these have **no people-table column** — they live only in the
    /// graph's `entity_fields` and reach the client through the read overlay.
    /// Before 2026-08-19 the Enricher wrote these names and the overlay had no
    /// arm for them, so an approved enrichment was stored and never displayed.
    pub linkedin: Option<String>,
    pub x_handle: Option<String>,
    pub facebook: Option<String>,
    pub instagram: Option<String>,
    pub personal_site: Option<String>,
    /// Public headshot URL drawn on the People graph. Direct http(s) image,
    /// graph-`entity_fields`-only, enrichable from a public page (team page,
    /// personal site, press) — not a login-walled scrape.
    pub photo_url: Option<String>,
    /// Manual-only hint written when an enrichment proposal is rejected, so
    /// the next enrich pass can find this person online (company, LinkedIn,
    /// city). Graph-`entity_fields`-only, same overlay as the other extras.
    pub find_online_hints: Option<String>,
    /// Immutable bridge key to the Spectral graph node (bare 64-hex blake3
    /// `EntityId`). Set once at creation, never rewritten on rename — the durable
    /// anchor that carries this identity row to its graph attributes (#255/B).
    /// `None` for rows minted before the bridge that the startup sync hasn't
    /// reached yet.
    pub graph_entity_id: Option<String>,
    /// `"person"` or `"organisation"` (`people.kind`, memory-feed lane
    /// 2026-09-25 — the `dagk/ux-mind` People UI's `peopleKind.ts` reads this
    /// to render an org row differently and skip it in duplicate-person
    /// flagging). Defaults to `"person"`; nothing currently writes
    /// `"organisation"` here (organisation names are refused rather than
    /// filed — see `people_create::classify_person_or_org`), so this is
    /// forward-declared plumbing more than a populated signal today.
    pub kind: String,
    /// True when this row is the operator's own record.
    ///
    /// **Always `false` out of the DB / at construction** (see [`row_to_person`]
    /// and every other builder in this module) — nothing at the storage layer
    /// knows who the owner is. It becomes meaningful only after a caller
    /// overlays [`overlay_owner_self`] using the configured
    /// [`crate::config::agent_identity::OwnerIdentity`] (dagK owner-identity,
    /// 2026-09-26; `routes::people` does this for every people-list read path).
    /// Before that overlay existed this field could never be anything but
    /// `false` — no owner/self identity concept existed anywhere in this
    /// codebase (`config::agent_identity` had no owner record, there was no
    /// `users` table, and the decision-actor constants like `ACTOR_JESSE` are
    /// string literals for decision attribution, not a `people` row link) — so
    /// People → List rendered the operator's own record like any other
    /// contact, including flagging it as a possible duplicate of itself.
    pub is_self: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Mutable attribute bundle for [`upsert_person`]. `None` fields are left
/// untouched on update (and stored NULL on first insert).
#[derive(Debug, Default, Clone)]
pub struct PersonAttrs {
    pub role: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub last_contact_at: Option<String>,
}

/// Read filter for [`list_people`]. All filters AND together; `None` means no
/// constraint. `query` matches display_name / email / company (case-insensitive
/// substring).
#[derive(Debug, Default, Clone)]
pub struct PeopleFilter {
    pub company: Option<String>,
    pub role: Option<String>,
    pub query: Option<String>,
}

/// Canonical `entity_fields` names for person attributes. Under Decision A
/// (#255) the graph is authoritative for these; the read overlay sources them
/// from `entity_fields` and the slice-2b write path writes the same names. Kept
/// here (plain strings — no Spectral dependency) so both sides share one list.
///
/// `birthday`, `relationship_strength` and `how_met` are #495 manual-only
/// fields with no people-table column — graph-`entity_fields`-only, written
/// exclusively through the `FieldSource::Manual` edit path (slice 2b).
///
/// `linkedin`, `x_handle` and `personal_site` joined this list on 2026-08-19.
/// They are also in [`ENRICHABLE_FIELD_NAMES`]: the Enricher had been writing
/// them since slice 4 while the reader had no arm for them, so approved
/// enrichments were durable and invisible. Every enrichable name must be
/// readable — `enrichable_fields_are_all_readable` fails the build otherwise.
///
/// `facebook` and `instagram` joined on 2026-08-20 — the same public-profile
/// URL shape as LinkedIn, so they are enrichable and manually editable.
///
/// `photo_url` joined on 2026-08-20 — a direct http(s) image for the graph
/// face. Enrichable from a public page; not a login-walled scrape.
///
/// `find_online_hints` is manual-only (written on enrichment reject). It is
/// deliberately absent from [`ENRICHABLE_FIELD_NAMES`].
pub const PERSON_FIELD_NAMES: [&str; 16] = [
    "role",
    "company",
    "email",
    "phone",
    "notes",
    "last_contact_at",
    "birthday",
    "relationship_strength",
    "how_met",
    "linkedin",
    "x_handle",
    "facebook",
    "instagram",
    "personal_site",
    "photo_url",
    "find_online_hints",
];

/// Graph field names that are *synonyms* of a canonical [`PERSON_FIELD_NAMES`]
/// entry rather than attributes of their own: `(written_name, canonical_name)`.
///
/// `job_title` is what the Enricher proposes; `role` is what the CRM row, the
/// `PeopleFilter` and the UI have always called the same thing. Rather than
/// rename a column and a wire key that predate the Enricher, the reader folds
/// the synonym onto the canonical slot. Aliases are deliberately NOT accepted by
/// the manual `PATCH` vocabulary — one writable name per attribute, so a manual
/// edit and an enrichment can never land in two different rows for one concept.
pub const PERSON_FIELD_ALIASES: [(&str, &str); 1] = [("job_title", "role")];

/// Resolve a graph field name to the canonical attribute it populates.
/// Non-alias names pass through unchanged.
pub fn canonical_person_field(field_name: &str) -> &str {
    match PERSON_FIELD_ALIASES
        .iter()
        .find(|(alias, _)| *alias == field_name)
    {
        Some((_, canonical)) => canonical,
        None => field_name,
    }
}

/// Fields the Enricher may propose (#495 slice 4, design ruled 2026-06-24).
/// STRUCTURED, verifiable fields only — lower confabulation. Manual-only
/// fields (email, phone, birthday, notes, relationship_strength, how_met) are
/// deliberately absent: an enrichment proposal naming one is rejected as
/// malformed at decision creation AND skipped at apply time. Independently,
/// Spectral's store never lets an `Enriched` write overwrite a field whose
/// stored provenance is `Manual` — three layers, all load-bearing.
///
/// Every name here MUST be readable: either a [`PERSON_FIELD_NAMES`] entry or a
/// [`PERSON_FIELD_ALIASES`] synonym of one. The two lists drifted apart between
/// slice 2b and slice 4 and nothing caught it, so
/// `enrichable_fields_are_all_readable` now pins the invariant against the
/// reader itself rather than against a second hand-maintained list.
pub const ENRICHABLE_FIELD_NAMES: [&str; 8] = [
    "linkedin",
    "job_title",
    "company",
    "x_handle",
    "facebook",
    "instagram",
    "personal_site",
    "photo_url",
];

impl Person {
    /// Null every graph-sourced attribute — including `last_contact_at`. The
    /// read overlay calls this before applying graph `entity_fields`, so the
    /// response reflects the graph only — never a stale people-table column
    /// (Decision A). The columns remain in the DB as a safety net until the
    /// Step-3 drop, but are not the response source.
    ///
    /// 2026-09-26 (DAG K review fix, #255 follow-up): `last_contact_at` was
    /// briefly EXCLUDED here (memory-feed lane, 2026-09-25) because, unlike
    /// the other columns, it is not solely graph-sourced — it is also bumped
    /// directly on the `people` row when a meeting is logged
    /// ([`crate::person_meetings::bump_last_contact`]) — and clearing it
    /// unconditionally, with nothing recomputing it afterward, would have
    /// regressed to "no last contact" for someone the CRM DID have a date
    /// for. That exclusion broke the general clearable contract instead
    /// (`every_settable_attribute_is_clearable`): a value this method leaves
    /// set can outlive the graph read that dropped it.
    ///
    /// The exclusion turned out to be unnecessary: the read overlay's own
    /// downstream steps already reconstruct `last_contact_at` as
    /// max(graph/manual value, latest meeting) *after* this clear runs —
    /// [`Person::set_attribute`] re-applies a graph-sourced value (a manual
    /// edit lands in `entity_fields`, same as every other attribute here),
    /// and [`crate::person_meetings::merge_last_contact`] then folds in the
    /// latest logged meeting on top, querying `person_meetings` fresh rather
    /// than trusting this struct's pre-clear state. So this method can null
    /// the field like every other one without losing meeting-derived or
    /// manually-set contact dates — only a value that is neither graph-backed
    /// nor meeting-backed (e.g. a bare column artifact from a people-merge
    /// with no corresponding graph field or meeting row) is dropped, which is
    /// the same fate every other Decision-A column already has.
    pub fn clear_attributes(&mut self) {
        self.role = None;
        self.company = None;
        self.email = None;
        self.phone = None;
        self.notes = None;
        self.last_contact_at = None;
        self.birthday = None;
        self.relationship_strength = None;
        self.how_met = None;
        self.linkedin = None;
        self.x_handle = None;
        self.facebook = None;
        self.instagram = None;
        self.personal_site = None;
        self.photo_url = None;
        self.find_online_hints = None;
    }

    /// Set one attribute by its `entity_fields` field name (graph overlay).
    /// [`PERSON_FIELD_ALIASES`] synonyms resolve to their canonical slot first;
    /// names outside [`PERSON_FIELD_NAMES`] are ignored.
    pub fn set_attribute(&mut self, field_name: &str, value: String) {
        match canonical_person_field(field_name) {
            "role" => self.role = Some(value),
            "company" => self.company = Some(value),
            "email" => self.email = Some(value),
            "phone" => self.phone = Some(value),
            "notes" => self.notes = Some(value),
            // Max-of-sources, not overwrite: whatever is already in
            // `self.last_contact_at` only gets replaced when the incoming
            // value is actually newer. In the normal read-overlay call order
            // (`clear_attributes` then one `set_attribute` per graph field),
            // `self.last_contact_at` is already `None` by the time this runs,
            // so the comparison is usually a no-op — but this still protects
            // a caller that applies more than one source for this field in
            // one pass (e.g. an alias collision, or calling `set_attribute`
            // without a preceding `clear_attributes`) from letting an older
            // value stomp a newer one.
            //
            // Both sides are parsed with `parse_rfc3339_or_date`, not the
            // strict `parse_rfc3339`: a manual edit routes through this same
            // arm, and the UI's own placeholder tells the person to type a
            // bare `YYYY-MM-DD` date, which strict RFC-3339 parsing rejects.
            // A rejected *incoming* value used to fall into `unwrap_or(false)`
            // ("not newer") and get silently discarded instead of applied —
            // the manual edit had no way to ever win.
            "last_contact_at" => {
                let take_graph = match self
                    .last_contact_at
                    .as_deref()
                    .and_then(|c| crate::person_meetings::parse_rfc3339_or_date(c).ok())
                {
                    Some(current) => crate::person_meetings::parse_rfc3339_or_date(&value)
                        .map(|graph_dt| graph_dt >= current)
                        .unwrap_or(false),
                    None => true,
                };
                if take_graph {
                    self.last_contact_at = Some(value);
                }
            }
            "birthday" => self.birthday = Some(value),
            "relationship_strength" => self.relationship_strength = Some(value),
            "how_met" => self.how_met = Some(value),
            "linkedin" => self.linkedin = Some(value),
            "x_handle" => self.x_handle = Some(value),
            "facebook" => self.facebook = Some(value),
            "instagram" => self.instagram = Some(value),
            "personal_site" => self.personal_site = Some(value),
            "photo_url" => self.photo_url = Some(value),
            "find_online_hints" => self.find_online_hints = Some(value),
            _ => {}
        }
    }
}

fn row_to_person(r: &sqlx::sqlite::SqliteRow) -> Person {
    Person {
        entity_uuid: r.get("entity_uuid"),
        canonical_id: r.get("canonical_id"),
        display_name: r.get("display_name"),
        role: r.get("role"),
        company: r.get("company"),
        email: r.get("email"),
        phone: r.get("phone"),
        notes: r.get("notes"),
        last_contact_at: r.get("last_contact_at"),
        // Graph-only fields — no people-table column; the read overlay populates
        // them from `entity_fields`. Never sourced from the DB row.
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
        graph_entity_id: r.get("graph_entity_id"),
        kind: r.get("kind"),
        // No owner/self identity source exists yet — see the field's doc.
        is_self: false,
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const SELECT_COLS: &str = "entity_uuid, canonical_id, display_name, role, company, \
     email, phone, notes, last_contact_at, graph_entity_id, kind, created_at, updated_at";

// ── Write primitives (substrate for v1.5; no HTTP surface in v1) ────────────

/// Get-or-create a person by `canonical_id`.
///
/// On **first save** an opaque `entity_uuid` is minted (`Uuid::now_v7`) and the
/// row inserted. On a subsequent call with the same `canonical_id` the existing
/// row's `entity_uuid` is preserved and only the supplied (`Some`) attributes
/// plus `display_name` are updated. Returns the resulting [`Person`].
pub async fn upsert_person(
    pool: &Pool<Sqlite>,
    canonical_id: &str,
    display_name: &str,
    attrs: &PersonAttrs,
) -> Result<Person, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|e| e.to_string())?;

    let existing: Option<String> =
        sqlx::query_scalar("SELECT entity_uuid FROM people WHERE canonical_id = ?")
            .bind(canonical_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

    let entity_uuid = match existing {
        Some(uuid) => {
            // Update display_name + only the attributes that were supplied.
            // COALESCE(?, col) leaves a column untouched when the bind is NULL.
            sqlx::query(
                "UPDATE people SET
                     display_name = ?,
                     role = COALESCE(?, role),
                     company = COALESCE(?, company),
                     email = COALESCE(?, email),
                     phone = COALESCE(?, phone),
                     notes = COALESCE(?, notes),
                     last_contact_at = COALESCE(?, last_contact_at),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE entity_uuid = ?",
            )
            .bind(display_name)
            .bind(&attrs.role)
            .bind(&attrs.company)
            .bind(&attrs.email)
            .bind(&attrs.phone)
            .bind(&attrs.notes)
            .bind(&attrs.last_contact_at)
            .bind(&uuid)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            uuid
        }
        None => {
            let uuid = Uuid::now_v7().to_string();
            // Persist the graph bridge key immutably at creation (#255/B). Derived
            // from the graph's canonical form (lowercased name), NOT canonical_id —
            // see identity::canonical::graph_entity_id_hex.
            let graph_entity_id =
                crate::identity::canonical::graph_entity_id_hex("person", display_name);
            sqlx::query(
                "INSERT INTO people
                     (entity_uuid, canonical_id, display_name, role, company,
                      email, phone, notes, last_contact_at, graph_entity_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&uuid)
            .bind(canonical_id)
            .bind(display_name)
            .bind(&attrs.role)
            .bind(&attrs.company)
            .bind(&attrs.email)
            .bind(&attrs.phone)
            .bind(&attrs.notes)
            .bind(&attrs.last_contact_at)
            .bind(&graph_entity_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            uuid
        }
    };

    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM people WHERE entity_uuid = ?"
    )))
    .bind(&entity_uuid)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(row_to_person(&row))
}

/// Rename a person: rewrite the mutable `canonical_id` (and `display_name`) for
/// a fixed `entity_uuid`. The opaque `entity_uuid` is unchanged — this is the
/// operation that proves slug drift never orphans a person's identity.
///
/// Returns the updated [`Person`], or `None` if no row has that `entity_uuid`.
pub async fn rename_canonical_id(
    pool: &Pool<Sqlite>,
    entity_uuid: &str,
    new_canonical_id: &str,
    new_display_name: &str,
) -> Result<Option<Person>, String> {
    let affected = sqlx::query(
        "UPDATE people SET
             canonical_id = ?,
             display_name = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE entity_uuid = ?",
    )
    .bind(new_canonical_id)
    .bind(new_display_name)
    .bind(entity_uuid)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();

    if affected == 0 {
        return Ok(None);
    }
    get_by_uuid(pool, entity_uuid).await
}

// ── Read surface (v1) ───────────────────────────────────────────────────────

/// Fetch a single person by opaque `entity_uuid`.
pub async fn get_by_uuid(pool: &Pool<Sqlite>, entity_uuid: &str) -> Result<Option<Person>, String> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM people WHERE entity_uuid = ?"
    )))
    .bind(entity_uuid)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_person))
}

/// Fetch a single person by mutable `canonical_id` slug.
pub async fn get_by_canonical_id(
    pool: &Pool<Sqlite>,
    canonical_id: &str,
) -> Result<Option<Person>, String> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM people WHERE canonical_id = ?"
    )))
    .bind(canonical_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_person))
}

/// Fetch a single person by immutable `graph_entity_id` (bare blake3 hex).
///
/// The *other* identity key. It exists because `canonical_id` and
/// `graph_entity_id` are derived by two different normalizations —
/// [`crate::identity::canonical::canonicalize_entity_id`] strips punctuation
/// while [`crate::identity::canonical::graph_canonical`] preserves it — so
/// "Jean-Luc Picard" and "Jean Luc Picard" share a `canonical_id` but mint
/// different EntityIds. A caller checking whether a person already exists must
/// consult BOTH keys; agreeing on one alone hands back the wrong human.
///
/// Returns the first match: `graph_entity_id` carries an index, not a UNIQUE
/// constraint, and the graph overlay deliberately tolerates several rows sharing
/// one id.
pub async fn get_by_graph_entity_id(
    pool: &Pool<Sqlite>,
    graph_entity_id: &str,
) -> Result<Option<Person>, String> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SELECT_COLS} FROM people WHERE graph_entity_id = ? ORDER BY created_at ASC"
    )))
    .bind(graph_entity_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(row_to_person))
}

/// List / surface people by attribute. Filters AND together; results are
/// ordered by most-recent contact first (NULLs last), then display_name.
pub async fn list_people(
    pool: &Pool<Sqlite>,
    filter: &PeopleFilter,
) -> Result<Vec<Person>, String> {
    // Build the WHERE clause dynamically; bind values positionally afterwards.
    // Archived rows (non-person bridge mints, memory-feed lane 2026-09-25) are
    // never in the default directory — they are archived, not deleted, so a
    // direct-by-id lookup still finds them, but list/search does not.
    let mut clauses: Vec<&str> = vec!["archived_at IS NULL"];
    if filter.company.is_some() {
        clauses.push("company = ?");
    }
    if filter.role.is_some() {
        clauses.push("role = ?");
    }
    if filter.query.is_some() {
        clauses.push("(display_name LIKE ? OR email LIKE ? OR company LIKE ?)");
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    let sql = format!(
        "SELECT {SELECT_COLS} FROM people {where_sql}
         ORDER BY last_contact_at IS NULL, last_contact_at DESC, display_name ASC",
    );

    // `sql` interpolates only fixed clause strings from the allow-list above
    // (`clauses`) plus the const SELECT_COLS/table name — no user input reaches
    // the SQL text itself; values are bound below.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    if let Some(ref company) = filter.company {
        q = q.bind(company.clone());
    }
    if let Some(ref role) = filter.role {
        q = q.bind(role.clone());
    }
    if let Some(ref query) = filter.query {
        let like = format!("%{query}%");
        q = q.bind(like.clone()).bind(like.clone()).bind(like);
    }

    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_person).collect())
}

// ── Owner/self overlay (dagK owner-identity, 2026-09-26) ────────────────────

/// Whether `person` IS the configured owner's own directory record.
///
/// Two ways to match, checked in order, both conservative by design (a false
/// positive here means the operator's own record silently absorbs someone
/// else's — see the People merge/delete audit trail for why that is not a
/// cheap mistake to undo):
///
/// 1. **Explicit link** (`owner.linked_person_id`) — wins whenever set, by
///    `entity_uuid` equality. This is the only path that survives the owner
///    renaming themselves in the directory.
/// 2. **Name fallback** — only when no link is set: a case-insensitive,
///    WHOLE-STRING match against `owner.display_name`. Deliberately not a
///    substring or fuzzy match ("Ada" must not match "Ada Lovelace" or
///    "Ada's Assistant") — a false-positive self-match would hide a real
///    contact from the directory's duplicate flagging and misroute the "You"
///    label onto a stranger who happens to share a first name.
///
/// Returns `false` whenever neither `linked_person_id` nor a non-empty
/// `display_name` is configured — an owner record nobody has ever filled in
/// must never accidentally match every blank-named row.
pub fn is_owner_self(
    person: &Person,
    owner: &crate::config::agent_identity::OwnerIdentity,
) -> bool {
    if let Some(linked) = owner.linked_person_id.as_deref() {
        if !linked.trim().is_empty() {
            return person.entity_uuid == linked;
        }
    }
    match owner.display_name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => person.display_name.trim().eq_ignore_ascii_case(name),
        _ => false,
    }
}

/// Overlay `is_self` onto every row per [`is_owner_self`].
///
/// Call this after [`crate::people`]'s other overlays
/// (`routes::people::overlay_graph_attributes` / `overlay_meeting_contact`) in
/// every people-list read path, so the owner's own record — however it is
/// identified — renders as "You" (`peopleKind.isSelf` on the UI side) and is
/// excluded from duplicate-flagging (`peopleKind.isContact`). Every row not
/// matched by [`is_owner_self`] is explicitly reset to `false`, so a stale
/// `true` from a previous overlay pass can never survive a re-run against a
/// changed owner record.
pub fn overlay_owner_self(
    people: &mut [Person],
    owner: &crate::config::agent_identity::OwnerIdentity,
) {
    for p in people.iter_mut() {
        p.is_self = is_owner_self(p, owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::spectral_schema;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        spectral_schema::init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn mints_opaque_uuid_at_first_save() {
        let pool = fresh_pool().await;
        let p = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs {
                company: Some("Acme".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(!p.entity_uuid.is_empty());
        assert_ne!(p.entity_uuid, p.canonical_id);
        assert_eq!(p.canonical_id, "person:jane-doe");
        assert_eq!(p.company.as_deref(), Some("Acme"));
    }

    /// `people.kind` defaults to `"person"` for every row minted through the
    /// ordinary write path, and `is_self` is always `false` straight out of the
    /// DB/write layer — it only becomes `true` once a reader applies
    /// [`overlay_owner_self`] using the configured owner record (dagK
    /// owner-identity, 2026-09-26; see the field's doc comment on [`Person`]).
    #[tokio::test]
    async fn kind_defaults_to_person_and_is_self_is_always_false() {
        let pool = fresh_pool().await;
        let p = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs::default(),
        )
        .await
        .unwrap();
        assert_eq!(p.kind, "person");
        assert!(!p.is_self);

        let fetched = get_by_canonical_id(&pool, "person:jane-doe")
            .await
            .unwrap()
            .expect("row exists");
        assert_eq!(fetched.kind, "person");
        assert!(!fetched.is_self);
    }

    #[test]
    fn clear_and_set_attribute_map_field_names() {
        let mut p = Person {
            entity_uuid: "u".into(),
            canonical_id: "person:x".into(),
            display_name: "X".into(),
            role: Some("old".into()),
            company: Some("old".into()),
            email: Some("old".into()),
            phone: Some("old".into()),
            notes: Some("old".into()),
            last_contact_at: Some("old".into()),
            birthday: Some("old".into()),
            relationship_strength: Some("old".into()),
            how_met: Some("old".into()),
            linkedin: Some("old".into()),
            x_handle: Some("old".into()),
            facebook: Some("old".into()),
            instagram: Some("old".into()),
            personal_site: Some("old".into()),
            photo_url: Some("old".into()),
            find_online_hints: Some("old".into()),
            graph_entity_id: Some("hex".into()),
            kind: "person".into(),
            is_self: false,
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        // clear_attributes nulls every graph-sourced attribute (identity kept),
        // including last_contact_at (2026-09-26 DAG K review fix, #255
        // follow-up): a brief exclusion for last_contact_at was removed
        // because the read overlay's own downstream steps — re-applying a
        // graph/manual value via `set_attribute`, then folding in the latest
        // logged meeting via `merge_last_contact` — already reconstruct it
        // after this clear runs, so nothing here needs to skip it. See
        // `every_settable_attribute_is_clearable` for the general contract.
        p.clear_attributes();
        assert_eq!(p.role, None);
        assert_eq!(p.last_contact_at, None);
        assert_eq!(p.birthday, None);
        assert_eq!(p.relationship_strength, None);
        assert_eq!(p.how_met, None);
        assert_eq!(p.display_name, "X");
        assert_eq!(p.graph_entity_id.as_deref(), Some("hex"));

        // set_attribute maps each canonical field name; unknown names are ignored.
        for name in PERSON_FIELD_NAMES {
            p.set_attribute(name, format!("v-{name}"));
        }
        p.set_attribute("not_a_field", "ignored".into());
        assert_eq!(p.role.as_deref(), Some("v-role"));
        assert_eq!(p.company.as_deref(), Some("v-company"));
        assert_eq!(p.email.as_deref(), Some("v-email"));
        assert_eq!(p.phone.as_deref(), Some("v-phone"));
        assert_eq!(p.notes.as_deref(), Some("v-notes"));
        assert_eq!(p.last_contact_at.as_deref(), Some("v-last_contact_at"));
        // The #495 manual-only vocab additions map too.
        assert_eq!(p.birthday.as_deref(), Some("v-birthday"));
        assert_eq!(
            p.relationship_strength.as_deref(),
            Some("v-relationship_strength")
        );
        assert_eq!(p.how_met.as_deref(), Some("v-how_met"));
        // The slice-4 enrichable names map too (2026-08-19).
        assert_eq!(p.linkedin.as_deref(), Some("v-linkedin"));
        assert_eq!(p.x_handle.as_deref(), Some("v-x_handle"));
        assert_eq!(p.facebook.as_deref(), Some("v-facebook"));
        assert_eq!(p.instagram.as_deref(), Some("v-instagram"));
        assert_eq!(p.personal_site.as_deref(), Some("v-personal_site"));
        assert_eq!(p.photo_url.as_deref(), Some("v-photo_url"));
        assert_eq!(p.find_online_hints.as_deref(), Some("v-find_online_hints"));
    }

    /// #3 (memory-feed lane, 2026-09-26): a manual edit of `last_contact_at`
    /// typed as a bare `YYYY-MM-DD` — exactly what the UI's own placeholder
    /// tells the person to type — must not be silently discarded just
    /// because the person already has an older, strictly-RFC-3339
    /// meeting-bumped value in that slot. Before the fix, `set_attribute`
    /// parsed the incoming value with the strict `parse_rfc3339`, which
    /// rejects a bare date, and `.unwrap_or(false)` then treated that parse
    /// failure as "not newer", dropping the edit with no error anywhere.
    #[test]
    fn set_attribute_last_contact_accepts_bare_date_over_older_rfc3339() {
        let mut p = blank_person();
        // A meeting-bumped column value, older than the manual edit below.
        p.last_contact_at = Some("2026-09-01T09:00:00Z".into());

        p.set_attribute("last_contact_at", "2026-09-10".into());

        assert_eq!(
            p.last_contact_at.as_deref(),
            Some("2026-09-10"),
            "a bare YYYY-MM-DD manual edit newer than the existing value must win, \
             not be silently discarded"
        );
    }

    /// The inverse: a bare-date manual edit that is chronologically *older*
    /// than the existing value is still correctly rejected (max-of-sources
    /// is about recency, not about rejecting every unparsed-looking value).
    #[test]
    fn set_attribute_last_contact_rejects_older_bare_date() {
        let mut p = blank_person();
        p.last_contact_at = Some("2026-09-20T09:00:00Z".into());

        p.set_attribute("last_contact_at", "2026-09-10".into());

        assert_eq!(
            p.last_contact_at.as_deref(),
            Some("2026-09-20T09:00:00Z"),
            "an older manual edit must not overwrite a newer existing value"
        );
    }

    /// A `Person` with identity set and every attribute blank.
    fn blank_person() -> Person {
        let mut p = Person {
            entity_uuid: "u".into(),
            canonical_id: "person:x".into(),
            display_name: "X".into(),
            role: None,
            company: None,
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
            graph_entity_id: Some("hex".into()),
            kind: "person".into(),
            is_self: false,
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        p.clear_attributes();
        p
    }

    /// Does the serialized person carry `sentinel` in any attribute slot?
    /// Asks the wire shape rather than a named field, so this stays true for
    /// attributes added later without anyone remembering to extend it.
    fn wire_contains(p: &Person, sentinel: &str) -> bool {
        let json = serde_json::to_value(p).expect("Person serializes");
        json.as_object()
            .expect("Person serializes to an object")
            .values()
            .any(|v| v.as_str() == Some(sentinel))
    }

    /// THE DRIFT GUARD (2026-08-19). Every name the Enricher may write must be
    /// something the read overlay can surface — directly or through a
    /// [`PERSON_FIELD_ALIASES`] synonym. Before this, `job_title`, `linkedin`,
    /// `x_handle` and `personal_site` were all written by an approved
    /// enrichment, stored durably with `source = enriched`, and dropped on the
    /// floor by [`Person::set_attribute`]'s catch-all arm: the user saw an
    /// enrichment succeed and no new data appear.
    ///
    /// Deliberately asserted against the *reader's behaviour* — feed the name
    /// in, look for the value on the wire — not against a second hand-kept
    /// list. A future field added to `ENRICHABLE_FIELD_NAMES` without a
    /// matching `set_attribute` arm fails here; there is no list to forget to
    /// update, which is exactly how the two vocabularies drifted apart before.
    #[test]
    fn enrichable_fields_are_all_readable() {
        for name in ENRICHABLE_FIELD_NAMES {
            let sentinel = format!("sentinel-{name}");
            let mut p = blank_person();
            p.set_attribute(name, sentinel.clone());
            assert!(
                wire_contains(&p, &sentinel),
                "enrichable field `{name}` is written by the Enricher but the \
                 read overlay drops it: `Person::set_attribute` has no arm for \
                 it and `PERSON_FIELD_ALIASES` has no synonym mapping it onto \
                 one. An approved enrichment would be stored and never shown. \
                 Add the arm (and a `Person` slot) or an alias entry."
            );
        }
    }

    #[test]
    fn find_online_hints_is_readable_and_not_enrichable() {
        assert!(PERSON_FIELD_NAMES.contains(&"find_online_hints"));
        assert!(
            !ENRICHABLE_FIELD_NAMES.contains(&"find_online_hints"),
            "find_online_hints is a user-typed reject note, never an Enricher field"
        );
        let mut p = blank_person();
        p.set_attribute(
            "find_online_hints",
            "works at Example Coworking, Halifax".into(),
        );
        assert_eq!(
            p.find_online_hints.as_deref(),
            Some("works at Example Coworking, Halifax")
        );
        p.clear_attributes();
        assert_eq!(p.find_online_hints, None);
    }

    #[test]
    fn facebook_and_instagram_are_enrichable_profile_links() {
        for name in ["facebook", "instagram"] {
            assert!(PERSON_FIELD_NAMES.contains(&name));
            assert!(
                ENRICHABLE_FIELD_NAMES.contains(&name),
                "{name} is a public profile URL, same class as linkedin"
            );
        }
        let mut p = blank_person();
        p.set_attribute("facebook", "https://www.facebook.com/example".into());
        p.set_attribute("instagram", "https://www.instagram.com/example".into());
        assert_eq!(
            p.facebook.as_deref(),
            Some("https://www.facebook.com/example")
        );
        assert_eq!(
            p.instagram.as_deref(),
            Some("https://www.instagram.com/example")
        );
    }

    #[test]
    fn photo_url_is_enrichable_and_readable() {
        assert!(PERSON_FIELD_NAMES.contains(&"photo_url"));
        assert!(
            ENRICHABLE_FIELD_NAMES.contains(&"photo_url"),
            "photo_url is a public image URL, same class as linkedin"
        );
        let mut p = blank_person();
        p.set_attribute("photo_url", "https://cdn.example.com/ada.jpg".into());
        assert_eq!(
            p.photo_url.as_deref(),
            Some("https://cdn.example.com/ada.jpg")
        );
        p.clear_attributes();
        assert_eq!(p.photo_url, None);
    }

    /// The reader's other half: whatever `set_attribute` can populate,
    /// `clear_attributes` must be able to null. The overlay clears first so the
    /// response reflects the graph alone (Decision A, #255); an attribute that
    /// can be set but not cleared would let a stale value survive a graph read
    /// that no longer carries it.
    #[test]
    fn every_settable_attribute_is_clearable() {
        for name in PERSON_FIELD_NAMES
            .iter()
            .chain(PERSON_FIELD_ALIASES.iter().map(|(alias, _)| alias))
        {
            let sentinel = format!("sentinel-{name}");
            let mut p = blank_person();
            p.set_attribute(name, sentinel.clone());
            assert!(
                wire_contains(&p, &sentinel),
                "`{name}` is in the read vocabulary but `set_attribute` ignores it"
            );
            p.clear_attributes();
            assert!(
                !wire_contains(&p, &sentinel),
                "`clear_attributes` leaves `{name}` set — a stale value can \
                 outlive the graph read that dropped it (Decision A, #255)"
            );
        }
    }

    /// `job_title` (what the Enricher proposes) and `role` (what the CRM row,
    /// the filter and the UI call it) are one attribute. The alias folds the
    /// former onto the latter; the canonical name wins when both are present so
    /// a manual edit is never overwritten by an enrichment of the synonym —
    /// the collision Spectral's same-name provenance rule cannot see.
    #[test]
    fn job_title_aliases_role() {
        assert_eq!(canonical_person_field("job_title"), "role");
        assert_eq!(canonical_person_field("company"), "company");
        assert_eq!(canonical_person_field("not_a_field"), "not_a_field");

        let mut p = blank_person();
        p.set_attribute("job_title", "Director of Field Operations".into());
        assert_eq!(p.role.as_deref(), Some("Director of Field Operations"));

        // No alias may name an attribute that is itself directly writable, or
        // the manual PATCH path and the Enricher get two rows for one concept.
        for (alias, canonical) in PERSON_FIELD_ALIASES {
            assert!(
                !PERSON_FIELD_NAMES.contains(&alias),
                "`{alias}` is both an alias and a canonical name"
            );
            assert!(
                PERSON_FIELD_NAMES.contains(&canonical),
                "alias `{alias}` points at `{canonical}`, which is not readable"
            );
        }
    }

    #[tokio::test]
    async fn sets_graph_entity_id_at_creation() {
        let pool = fresh_pool().await;
        let p = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs::default(),
        )
        .await
        .unwrap();

        // Bridge key derived from the graph canonical ("jane doe"), NOT the
        // dash-slug canonical_id — bare 64-hex, immutable across later renames.
        let expect = crate::identity::canonical::graph_entity_id_hex("person", "Jane Doe");
        assert_eq!(p.graph_entity_id.as_deref(), Some(expect.as_str()));
        assert_eq!(expect.len(), 64);

        let after = rename_canonical_id(&pool, &p.entity_uuid, "person:jane-smith", "Jane Smith")
            .await
            .unwrap()
            .unwrap();
        // Rename leaves the graph anchor untouched (immutable).
        assert_eq!(after.graph_entity_id, p.graph_entity_id);
    }

    #[tokio::test]
    async fn upsert_is_idempotent_on_canonical_id() {
        let pool = fresh_pool().await;
        let first = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs::default(),
        )
        .await
        .unwrap();
        let second = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs {
                role: Some("CTO".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Same row (uuid preserved), attrs merged.
        assert_eq!(first.entity_uuid, second.entity_uuid);
        assert_eq!(second.role.as_deref(), Some("CTO"));

        let all = list_people(&pool, &PeopleFilter::default()).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn entity_uuid_stable_across_rename() {
        let pool = fresh_pool().await;
        let before = upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs::default(),
        )
        .await
        .unwrap();

        let after = rename_canonical_id(
            &pool,
            &before.entity_uuid,
            "person:jane-smith",
            "Jane Smith",
        )
        .await
        .unwrap()
        .expect("person exists");

        // The slug changed, the uuid did NOT.
        assert_eq!(before.entity_uuid, after.entity_uuid);
        assert_ne!(before.canonical_id, after.canonical_id);
        assert_eq!(after.canonical_id, "person:jane-smith");
        assert_eq!(after.display_name, "Jane Smith");

        // Old slug no longer resolves; uuid still does.
        assert!(get_by_canonical_id(&pool, "person:jane-doe")
            .await
            .unwrap()
            .is_none());
        assert!(get_by_uuid(&pool, &before.entity_uuid)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn lists_and_filters_by_attribute() {
        let pool = fresh_pool().await;
        upsert_person(
            &pool,
            "person:jane-doe",
            "Jane Doe",
            &PersonAttrs {
                company: Some("Acme".into()),
                role: Some("CTO".into()),
                last_contact_at: Some("2026-06-18T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        upsert_person(
            &pool,
            "person:john-roe",
            "John Roe",
            &PersonAttrs {
                company: Some("Globex".into()),
                role: Some("CTO".into()),
                last_contact_at: Some("2026-06-19T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // By company.
        let acme = list_people(
            &pool,
            &PeopleFilter {
                company: Some("Acme".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].display_name, "Jane Doe");

        // By role — both match; most-recent contact first.
        let ctos = list_people(
            &pool,
            &PeopleFilter {
                role: Some("CTO".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(ctos.len(), 2);
        assert_eq!(ctos[0].display_name, "John Roe");

        // By free-text query.
        let q = list_people(
            &pool,
            &PeopleFilter {
                query: Some("globex".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].display_name, "John Roe");
    }

    // ── Owner/self overlay (dagK owner-identity, 2026-09-26) ────────────

    use crate::config::agent_identity::OwnerIdentity;

    #[test]
    fn unconfigured_owner_matches_nobody() {
        let p = blank_person();
        assert!(!is_owner_self(&p, &OwnerIdentity::default()));
    }

    #[test]
    fn explicit_link_wins_by_entity_uuid_regardless_of_name() {
        let mut p = blank_person();
        p.entity_uuid = "person-uuid-1".into();
        p.display_name = "Totally Different Name".into();
        let owner = OwnerIdentity {
            display_name: Some("Ignored Because Linked".into()),
            email: None,
            linked_person_id: Some("person-uuid-1".into()),
        };
        assert!(is_owner_self(&p, &owner));
    }

    #[test]
    fn explicit_link_does_not_match_a_different_person() {
        let mut p = blank_person();
        p.entity_uuid = "person-uuid-1".into();
        let owner = OwnerIdentity {
            linked_person_id: Some("person-uuid-2".into()),
            ..Default::default()
        };
        assert!(!is_owner_self(&p, &owner));
    }

    #[test]
    fn name_fallback_matches_case_insensitively_when_unlinked() {
        let mut p = blank_person();
        p.display_name = "Jane Doe".into();
        let owner = OwnerIdentity {
            display_name: Some("jane DOE".into()),
            ..Default::default()
        };
        assert!(is_owner_self(&p, &owner));
    }

    /// Deliberately conservative: a first-name-only owner record must not
    /// absorb a directory row that merely starts with it — the false-positive
    /// this whole function exists to avoid.
    #[test]
    fn name_fallback_requires_a_whole_string_match_not_a_prefix() {
        let mut p = blank_person();
        p.display_name = "Jane Doe".into();
        let owner = OwnerIdentity {
            display_name: Some("Jane".into()),
            ..Default::default()
        };
        assert!(!is_owner_self(&p, &owner));
    }

    #[test]
    fn blank_owner_display_name_matches_nobody() {
        let mut p = blank_person();
        p.display_name = String::new();
        let owner = OwnerIdentity {
            display_name: Some("   ".into()),
            ..Default::default()
        };
        assert!(!is_owner_self(&p, &owner));
    }

    #[test]
    fn overlay_sets_true_for_the_match_and_false_for_everyone_else() {
        let mut jane = blank_person();
        jane.entity_uuid = "u-jane".into();
        jane.display_name = "Jane Doe".into();
        let mut john = blank_person();
        john.entity_uuid = "u-john".into();
        john.display_name = "John Roe".into();
        let mut rows = vec![jane, john];

        let owner = OwnerIdentity {
            display_name: Some("Jane Doe".into()),
            ..Default::default()
        };
        overlay_owner_self(&mut rows, &owner);
        assert!(rows[0].is_self);
        assert!(!rows[1].is_self);
    }

    /// A row overlaid `true` on a previous pass must flip back to `false` once
    /// the owner record no longer matches it — the overlay must not just set
    /// `true` and leave everyone else's prior value untouched.
    #[test]
    fn overlay_clears_a_stale_true_when_the_owner_record_changes() {
        let mut p = blank_person();
        p.entity_uuid = "u-jane".into();
        p.display_name = "Jane Doe".into();
        p.is_self = true;
        let mut rows = vec![p];

        overlay_owner_self(&mut rows, &OwnerIdentity::default());
        assert!(!rows[0].is_self);
    }
}
