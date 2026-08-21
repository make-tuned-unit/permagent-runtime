//! Project association layer — scopes global entities (people, Brain memories)
//! to a project via additive join tables in permagent.db.
//!
//! Two many-to-many relations, both populated manually now (the Enricher /
//! extraction pass writes them later):
//!
//! * [`project_people`](apply schema) — links a project to a [`crate::people::Person`]
//!   by the person's opaque, immutable `entity_uuid`. Real FK; cascades on delete.
//!
//! * [`project_memories`] — links a project to a Brain memory by the **Spectral
//!   `memory.db` id** (no FK; cross-database). This module owns the join rows
//!   only; resolving an id to the memory's content is the route layer's job
//!   (`read_only_brain_conn` against the live Brain), so the dead permagent.db
//!   `memories` table is never touched. See `apply_project_association_schema`.

use crate::people::Person;
use sqlx::{Pool, Row, Sqlite};

// ── project_people ──────────────────────────────────────────────────────────

/// A person associated with a project: the full [`Person`] record plus the
/// association's own fields (`project_role` — the role *within this project*,
/// distinct from the person's global CRM `role` — and `associated_at`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectPerson {
    #[serde(flatten)]
    pub person: Person,
    /// Role within *this* project (the `project_people.role` column). Nullable.
    pub project_role: Option<String>,
    /// When the association was created (`project_people.added_at`).
    pub associated_at: String,
}

/// Associate a person (by opaque `entity_uuid`) with a project.
///
/// Idempotent: re-associating the same pair updates `project_role` rather than
/// erroring (a composite PK makes the upsert a no-op otherwise). Returns an error
/// if the FK is violated (unknown project or person).
pub async fn associate_person(
    pool: &Pool<Sqlite>,
    project_id: &str,
    entity_uuid: &str,
    role: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO project_people (project_id, entity_uuid, role) VALUES (?, ?, ?)
         ON CONFLICT(project_id, entity_uuid) DO UPDATE SET role = excluded.role",
    )
    .bind(project_id)
    .bind(entity_uuid)
    .bind(role)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a person↔project association. Returns `true` if a row was deleted.
pub async fn disassociate_person(
    pool: &Pool<Sqlite>,
    project_id: &str,
    entity_uuid: &str,
) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM project_people WHERE project_id = ? AND entity_uuid = ?")
        .bind(project_id)
        .bind(entity_uuid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

/// List the people associated with a project, newest association first. Joins
/// the live `people` table, so a person deleted from CRM (cascade) drops out.
pub async fn list_project_people(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<ProjectPerson>, String> {
    let rows = sqlx::query(
        "SELECT p.entity_uuid, p.canonical_id, p.display_name, p.role, p.company, \
                p.email, p.phone, p.notes, p.last_contact_at, p.graph_entity_id, \
                p.created_at, p.updated_at, \
                pp.role AS project_role, pp.added_at AS associated_at \
         FROM project_people pp \
         JOIN people p ON p.entity_uuid = pp.entity_uuid \
         WHERE pp.project_id = ? \
         ORDER BY pp.added_at DESC, p.display_name ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ProjectPerson {
            person: Person {
                entity_uuid: r.get("entity_uuid"),
                canonical_id: r.get("canonical_id"),
                display_name: r.get("display_name"),
                role: r.get("role"),
                company: r.get("company"),
                email: r.get("email"),
                phone: r.get("phone"),
                notes: r.get("notes"),
                last_contact_at: r.get("last_contact_at"),
                // Graph-only manual fields (no people-table column); the caller's
                // graph overlay populates them from `entity_fields`.
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
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            },
            project_role: r.get("project_role"),
            associated_at: r.get("associated_at"),
        })
        .collect())
}

/// One project a person belongs to — the reverse of [`list_project_people`].
/// Identity-only: no [`Person`] attributes, so no graph overlay is needed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonProject {
    pub project_id: String,
    pub project_name: String,
    pub project_status: String,
    /// Role within this project (`project_people.role`). Nullable.
    pub role: Option<String>,
    pub added_at: String,
}

/// List the projects a person belongs to, newest association first.
///
/// Served by `idx_project_people_entity`, so this is cheap even though it reads
/// the join table "backwards". Note that on the live corpus almost every person
/// belongs to at most one project — the interesting cohort is the people this
/// returns *nothing* for, who are unreachable from any project-scoped surface.
pub async fn list_person_projects(
    pool: &Pool<Sqlite>,
    entity_uuid: &str,
) -> Result<Vec<PersonProject>, String> {
    let rows = sqlx::query(
        "SELECT p.id AS project_id, p.name AS project_name, p.status AS project_status, \
                pp.role AS role, pp.added_at AS added_at \
         FROM project_people pp \
         JOIN projects p ON p.id = pp.project_id \
         WHERE pp.entity_uuid = ? \
         ORDER BY pp.added_at DESC, p.name ASC",
    )
    .bind(entity_uuid)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| PersonProject {
            project_id: r.get("project_id"),
            project_name: r.get("project_name"),
            project_status: r.get("project_status"),
            role: r.get("role"),
            added_at: r.get("added_at"),
        })
        .collect())
}

/// Every person↔project pair in one query, grouped by `entity_uuid`.
///
/// The directory's fan-out guard: one statement for the whole table rather than
/// [`list_person_projects`] per row. Only the fields a directory chip needs.
pub async fn project_refs_by_person(
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashMap<String, Vec<ProjectRef>>, String> {
    let rows = sqlx::query(
        "SELECT pp.entity_uuid, p.id AS project_id, p.name AS project_name \
         FROM project_people pp \
         JOIN projects p ON p.id = pp.project_id \
         ORDER BY p.name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut out: std::collections::HashMap<String, Vec<ProjectRef>> =
        std::collections::HashMap::new();
    for r in rows.iter() {
        out.entry(r.get("entity_uuid"))
            .or_default()
            .push(ProjectRef {
                project_id: r.get("project_id"),
                project_name: r.get("project_name"),
            });
    }
    Ok(out)
}

/// The minimal project reference a directory row renders as a chip.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectRef {
    pub project_id: String,
    pub project_name: String,
}

// ── project_memories ────────────────────────────────────────────────────────

/// One project↔memory association row. `memory_id` is a Spectral `memory.db` id;
/// resolving it to content is the route layer's job (live Brain connection).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryAssociation {
    pub memory_id: String,
    pub added_at: String,
}

/// Associate a Brain memory (by its Spectral `memory.db` id) with a project.
/// Idempotent — a duplicate pair is a silent no-op.
pub async fn associate_memory(
    pool: &Pool<Sqlite>,
    project_id: &str,
    memory_id: &str,
) -> Result<(), String> {
    sqlx::query("INSERT OR IGNORE INTO project_memories (project_id, memory_id) VALUES (?, ?)")
        .bind(project_id)
        .bind(memory_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a memory↔project association. Returns `true` if a row was deleted.
pub async fn disassociate_memory(
    pool: &Pool<Sqlite>,
    project_id: &str,
    memory_id: &str,
) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM project_memories WHERE project_id = ? AND memory_id = ?")
        .bind(project_id)
        .bind(memory_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

/// List the memory associations for a project, newest first. Returns the
/// Spectral ids + association timestamps only; the caller resolves the ids
/// against the live Brain (`read_only_brain_conn`) and discards orphans.
pub async fn list_project_memory_associations(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Vec<MemoryAssociation>, String> {
    let rows = sqlx::query(
        "SELECT memory_id, added_at FROM project_memories \
         WHERE project_id = ? ORDER BY added_at DESC, memory_id ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| MemoryAssociation {
            memory_id: r.get("memory_id"),
            added_at: r.get("added_at"),
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
        // FK enforcement is off by default per-connection in SQLite; the join
        // table's cascades rely on it, so assert the pragma the app sets.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
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

    async fn a_person(pool: &Pool<Sqlite>, slug: &str, name: &str) -> String {
        upsert_person(pool, slug, name, &PersonAttrs::default())
            .await
            .unwrap()
            .entity_uuid
    }

    #[tokio::test]
    async fn associate_and_list_people() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let alice = a_person(&pool, "person:alice", "Alice").await;
        let bob = a_person(&pool, "person:bob", "Bob").await;

        associate_person(&pool, &proj, &alice, Some("Designer"))
            .await
            .unwrap();
        associate_person(&pool, &proj, &bob, None).await.unwrap();

        let people = list_project_people(&pool, &proj).await.unwrap();
        assert_eq!(people.len(), 2);
        let alice_row = people
            .iter()
            .find(|p| p.person.entity_uuid == alice)
            .unwrap();
        assert_eq!(alice_row.person.display_name, "Alice");
        assert_eq!(alice_row.project_role.as_deref(), Some("Designer"));
    }

    #[tokio::test]
    async fn associate_is_idempotent_and_updates_role() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let alice = a_person(&pool, "person:alice", "Alice").await;

        associate_person(&pool, &proj, &alice, Some("Designer"))
            .await
            .unwrap();
        // Re-associate: no duplicate row, role updated.
        associate_person(&pool, &proj, &alice, Some("Lead"))
            .await
            .unwrap();

        let people = list_project_people(&pool, &proj).await.unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].project_role.as_deref(), Some("Lead"));
    }

    #[tokio::test]
    async fn disassociate_person_works() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let alice = a_person(&pool, "person:alice", "Alice").await;
        associate_person(&pool, &proj, &alice, None).await.unwrap();

        assert!(disassociate_person(&pool, &proj, &alice).await.unwrap());
        // Second delete is a no-op.
        assert!(!disassociate_person(&pool, &proj, &alice).await.unwrap());
        assert!(list_project_people(&pool, &proj).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn person_belongs_to_many_projects() {
        let pool = test_pool().await;
        let p1 = a_project(&pool, "One").await;
        let p2 = a_project(&pool, "Two").await;
        let alice = a_person(&pool, "person:alice", "Alice").await;

        associate_person(&pool, &p1, &alice, None).await.unwrap();
        associate_person(&pool, &p2, &alice, None).await.unwrap();

        assert_eq!(list_project_people(&pool, &p1).await.unwrap().len(), 1);
        assert_eq!(list_project_people(&pool, &p2).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_project_cascades_associations() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;
        let alice = a_person(&pool, "person:alice", "Alice").await;
        associate_person(&pool, &proj, &alice, None).await.unwrap();

        crate::projects::delete_project(&pool, &proj).await.unwrap();

        // Cascade removed the join row; the person itself survives.
        assert!(list_project_people(&pool, &proj).await.unwrap().is_empty());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM people WHERE entity_uuid = ?")
            .bind(&alice)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn memory_association_roundtrip() {
        let pool = test_pool().await;
        let proj = a_project(&pool, "Acme").await;

        // memory_id is an opaque Spectral id — no FK, so any string associates.
        associate_memory(&pool, &proj, "a1b2c3d4e5f60718")
            .await
            .unwrap();
        // Idempotent.
        associate_memory(&pool, &proj, "a1b2c3d4e5f60718")
            .await
            .unwrap();

        let assocs = list_project_memory_associations(&pool, &proj)
            .await
            .unwrap();
        assert_eq!(assocs.len(), 1);
        assert_eq!(assocs[0].memory_id, "a1b2c3d4e5f60718");

        assert!(disassociate_memory(&pool, &proj, "a1b2c3d4e5f60718")
            .await
            .unwrap());
        assert!(list_project_memory_associations(&pool, &proj)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        let pool = test_pool().await;
        // Re-running the apply step on an initialized DB must not error.
        crate::session::spectral_schema::apply_project_association_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::migrate_v19_to_v20(&pool)
            .await
            .unwrap();
    }
}
