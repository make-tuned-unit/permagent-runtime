use crate::stage::Stage;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{fs, path::Path};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

pub fn open(path: &Path) -> Result<Connection> {
    let connection =
        Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

pub fn open_and_migrate(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut connection = open(path)?;
    migrate(&mut connection)?;
    Ok(connection)
}

pub fn migrate(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;
    for (version, sql) in MIGRATIONS {
        let applied: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=?1)",
            [version],
            |row| row.get(0),
        )?;
        if !applied {
            let transaction = connection.transaction()?;
            transaction.execute_batch(sql)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![version, Utc::now().to_rfc3339()],
            )?;
            transaction.commit()?;
        }
    }
    Ok(())
}

pub fn tick(connection: &Connection, stage: Stage) -> Result<()> {
    // Phase 0 stages deliberately do no work beyond recording their invocation.
    let started = Utc::now().to_rfc3339();
    let finished = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO runs(
            stage, started_at, finished_at, ok, items_in, items_out, error, tokens_in, tokens_out
         ) VALUES (?1, ?2, ?3, 1, 0, 0, NULL, 0, 0)",
        params![stage.to_string(), started, finished],
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct StageStatus {
    pub stage: Stage,
    pub started_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub items_in: Option<i64>,
    pub items_out: Option<i64>,
    pub ok: Option<bool>,
    pub last_error: Option<String>,
}

pub fn statuses(connection: &Connection) -> Result<Vec<StageStatus>> {
    Stage::ALL
        .into_iter()
        .map(|stage| {
            let latest: Option<(String, String, i64, i64, i64)> = connection
                .query_row(
                    "SELECT started_at, finished_at, items_in, items_out, ok
                     FROM runs
                     WHERE stage = ?1
                     ORDER BY id DESC
                     LIMIT 1",
                    [stage.to_string()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()?;
            let last_error = connection
                .query_row(
                    "SELECT error FROM runs WHERE stage = ?1 AND ok = 0 ORDER BY id DESC LIMIT 1",
                    [stage.to_string()],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            let (started_at, duration_ms, items_in, items_out, ok) = match latest {
                Some((start, end, input, output, success)) => {
                    let start_time = DateTime::parse_from_rfc3339(&start)?;
                    let end_time = DateTime::parse_from_rfc3339(&end)?;
                    (
                        Some(start),
                        Some((end_time - start_time).num_milliseconds()),
                        Some(input),
                        Some(output),
                        Some(success != 0),
                    )
                }
                None => (None, None, None, None, None),
            };
            Ok(StageStatus {
                stage,
                started_at,
                duration_ms,
                items_in,
                items_out,
                ok,
                last_error,
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct Draft {
    pub id: i64,
    pub platform: String,
    pub body: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn drafts(connection: &Connection) -> Result<Vec<Draft>> {
    let mut statement = connection
        .prepare("SELECT id,platform,body,status,created_at,updated_at FROM drafts ORDER BY id")?;
    let drafts = statement
        .query_map([], |row| {
            Ok(Draft {
                id: row.get(0)?,
                platform: row.get(1)?,
                body: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(drafts)
}

pub fn draft(connection: &Connection, id: i64) -> Result<Option<Draft>> {
    Ok(connection
        .query_row(
            "SELECT id,platform,body,status,created_at,updated_at FROM drafts WHERE id=?1",
            [id],
            |row| {
                Ok(Draft {
                    id: row.get(0)?,
                    platform: row.get(1)?,
                    body: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        let migration_count: i64 = connection
            .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, 1);
        assert!(connection.prepare("SELECT * FROM drafts").is_ok());
    }

    #[test]
    fn tick_writes_one_complete_run() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        tick(&connection, Stage::Harvest).unwrap();
        let run: (
            i64,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            Option<String>,
            i64,
            i64,
        ) = connection
            .query_row(
                "SELECT count(*), stage, started_at, finished_at, ok, items_in, items_out,
                    error, tokens_in, tokens_out
             FROM runs",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .unwrap();
        let (
            count,
            stage,
            started_at,
            finished_at,
            ok,
            items_in,
            items_out,
            error,
            tokens_in,
            tokens_out,
        ) = run;
        assert_eq!(count, 1);
        assert_eq!(stage, "harvest");
        DateTime::parse_from_rfc3339(&started_at).unwrap();
        DateTime::parse_from_rfc3339(&finished_at).unwrap();
        assert_eq!(ok, 1);
        assert_eq!(items_in, 0);
        assert_eq!(items_out, 0);
        assert_eq!(error, None);
        assert_eq!(tokens_in, 0);
        assert_eq!(tokens_out, 0);
    }
}
