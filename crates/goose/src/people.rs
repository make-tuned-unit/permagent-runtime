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
    /// Immutable bridge key to the Spectral graph node (bare 64-hex blake3
    /// `EntityId`). Set once at creation, never rewritten on rename — the durable
    /// anchor that carries this identity row to its graph attributes (#255/B).
    /// `None` for rows minted before the bridge that the startup sync hasn't
    /// reached yet.
    pub graph_entity_id: Option<String>,
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
/// The last three (`birthday`, `relationship_strength`, `how_met`) are #495
/// manual-only fields with no people-table column — graph-`entity_fields`-only,
/// written exclusively through the `FieldSource::Manual` edit path (slice 2b).
pub const PERSON_FIELD_NAMES: [&str; 9] = [
    "role",
    "company",
    "email",
    "phone",
    "notes",
    "last_contact_at",
    "birthday",
    "relationship_strength",
    "how_met",
];

/// Fields the Enricher may propose (#495 slice 4, design ruled 2026-06-24).
/// STRUCTURED, verifiable fields only — lower confabulation. Manual-only
/// fields (email, phone, birthday, notes, relationship_strength, how_met) are
/// deliberately absent: an enrichment proposal naming one is rejected as
/// malformed at decision creation AND skipped at apply time. Independently,
/// Spectral's store never lets an `Enriched` write overwrite a field whose
/// stored provenance is `Manual` — three layers, all load-bearing.
pub const ENRICHABLE_FIELD_NAMES: [&str; 5] = [
    "linkedin",
    "job_title",
    "company",
    "x_handle",
    "personal_site",
];

impl Person {
    /// Null every graph-sourced attribute. The read overlay calls this before
    /// applying graph `entity_fields`, so the response reflects the graph only —
    /// never a stale people-table column (Decision A). The columns remain in the
    /// DB as a safety net until the Step-3 drop, but are not the response source.
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
    }

    /// Set one attribute by its `entity_fields` field name (graph overlay). Names
    /// outside [`PERSON_FIELD_NAMES`] are ignored.
    pub fn set_attribute(&mut self, field_name: &str, value: String) {
        match field_name {
            "role" => self.role = Some(value),
            "company" => self.company = Some(value),
            "email" => self.email = Some(value),
            "phone" => self.phone = Some(value),
            "notes" => self.notes = Some(value),
            "last_contact_at" => self.last_contact_at = Some(value),
            "birthday" => self.birthday = Some(value),
            "relationship_strength" => self.relationship_strength = Some(value),
            "how_met" => self.how_met = Some(value),
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
        graph_entity_id: r.get("graph_entity_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const SELECT_COLS: &str = "entity_uuid, canonical_id, display_name, role, company, \
     email, phone, notes, last_contact_at, graph_entity_id, created_at, updated_at";

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

    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLS} FROM people WHERE entity_uuid = ?"
    ))
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
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLS} FROM people WHERE entity_uuid = ?"
    ))
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
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLS} FROM people WHERE canonical_id = ?"
    ))
    .bind(canonical_id)
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
    let mut clauses: Vec<&str> = Vec::new();
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

    let mut q = sqlx::query(&sql);
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
            graph_entity_id: Some("hex".into()),
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        // clear_attributes nulls every graph-sourced attribute (identity kept).
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
}
