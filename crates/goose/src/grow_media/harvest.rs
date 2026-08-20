//! What *this* project can be posted about.
//!
//! Reads the project's own brand, first-party analytics, completed goals, and
//! GTM strategy. Returns empty lists when a source is missing — never a
//! canned story from another install.

use serde::Serialize;
use sqlx::{Pool, Sqlite};

use crate::projects::{self, ProjectBrand};

#[derive(Debug, Serialize)]
pub struct ContentBrief {
    pub project_id: String,
    pub project_name: String,
    pub brand: BrandSnapshot,
    pub origin: String,
    pub top_pages: Vec<NamedCount>,
    pub shipped_features: Vec<ShippedFeature>,
    pub strategy_content: String,
}

#[derive(Debug, Serialize)]
pub struct BrandSnapshot {
    pub voice: String,
    pub has_kit: bool,
    pub donts: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct NamedCount {
    pub path: String,
    pub pageviews: i64,
}

#[derive(Debug, Serialize)]
pub struct ShippedFeature {
    pub title: String,
    pub description: String,
}

const SKIP_PATHS: &[&str] = &["/", "/privacy", "/terms", "/legal", "/login", "/signup"];

pub async fn content_brief(
    pool: &Pool<Sqlite>,
    project_id_or_slug: &str,
) -> Result<ContentBrief, String> {
    let project = projects::get_project_by_id_or_slug(pool, project_id_or_slug)
        .await?
        .ok_or_else(|| format!("Project '{project_id_or_slug}' not found"))?;
    let brand = ProjectBrand::from_metadata(&project.metadata_json);
    let strategy_content = project
        .metadata_json
        .get("strategy")
        .and_then(|s| s.get("content"))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    Ok(ContentBrief {
        project_id: project.id.clone(),
        project_name: project.name.clone(),
        origin: brand.origin.clone(),
        brand: BrandSnapshot {
            has_kit: !brand.bg.is_empty(),
            voice: brand.voice.clone(),
            donts: brand.donts.clone(),
        },
        top_pages: top_pages(pool, &project.id).await,
        shipped_features: shipped_features(pool, &project.id).await,
        strategy_content,
    })
}

async fn top_pages(pool: &Pool<Sqlite>, project_id: &str) -> Vec<NamedCount> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT path, count(*) FROM analytics_events
         WHERE project_id = ? AND is_bot = 0 AND kind = 'pageview'
           AND created_at >= datetime('now', '-30 days')
         GROUP BY path
         ORDER BY count(*) DESC
         LIMIT 20",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter(|(path, _)| {
            let p = path.trim_end_matches('/');
            let leaf = if p.is_empty() { "/" } else { p };
            !SKIP_PATHS.contains(&leaf) && !leaf.starts_with("/legal") && leaf != "/robots.txt"
        })
        .take(8)
        .map(|(path, pageviews)| NamedCount { path, pageviews })
        .collect()
}

async fn shipped_features(pool: &Pool<Sqlite>, project_id: &str) -> Vec<ShippedFeature> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT c.title, c.description FROM cards c
         JOIN board_columns col ON col.id = c.column_id
         WHERE c.project_id = ? AND c.card_type = 'goal'
           AND col.state_binding = 'complete' AND c.archived_at IS NULL
         ORDER BY c.updated_at DESC
         LIMIT 8",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(title, description)| ShippedFeature { title, description })
        .collect()
}
