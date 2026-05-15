use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

use crate::Result;

pub async fn connect(db_path: &Path) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub async fn seed_default_projects(pool: &SqlitePool) -> Result<()> {
    let projects = [
        ("atlas-atlantic", "Atlas Atlantic", "Venture studio main account"),
        ("evntally", "evntally", "Event discovery app"),
        ("world-litter-run", "World Litter Run", "Plogging community and events"),
    ];

    for (slug, name, description) in projects {
        // Skip if already exists (idempotent seeding)
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM projects WHERE slug = ?"
        )
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        if existing.is_some() {
            continue;
        }

        let project_id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (slug, name, description) VALUES (?, ?, ?) RETURNING id"
        )
        .bind(slug)
        .bind(name)
        .bind(description)
        .fetch_one(pool)
        .await?;

        // Default columns for social_post type
        let social_cols = ["Draft", "Scheduled", "Posted", "Failed"];
        for (pos, col_name) in social_cols.iter().enumerate() {
            sqlx::query(
                "INSERT INTO board_columns (project_id, card_type, name, position) VALUES (?, ?, ?, ?)"
            )
            .bind(project_id)
            .bind("social_post")
            .bind(col_name)
            .bind(pos as i64)
            .execute(pool)
            .await?;
        }

        // Default columns for coding_task type (stubbed for later days)
        let code_cols = ["Backlog", "In Progress", "Review", "Done"];
        for (pos, col_name) in code_cols.iter().enumerate() {
            sqlx::query(
                "INSERT INTO board_columns (project_id, card_type, name, position) VALUES (?, ?, ?, ?)"
            )
            .bind(project_id)
            .bind("coding_task")
            .bind(col_name)
            .bind(pos as i64)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}
