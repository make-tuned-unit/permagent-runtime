//! Grow post media: brand-aware stills, optional Higgsfield video, send time.
//!
//! Everything here is parameterized by the *current* project and the current
//! user's secrets. No project names, palettes, credentials, or filesystem
//! layouts from any particular install belong in this module.

mod harvest;
mod higgsfield;
mod postiz;
mod publisher;
mod schedule;
mod still;

use sqlx::{Pool, Sqlite};

pub use harvest::content_brief;
pub use higgsfield::{
    credentials_configured, HiggsfieldClient, HiggsfieldCredentials, KEY_ID as HF_KEY_ID,
    KEY_SECRET as HF_KEY_SECRET,
};
pub use postiz::{
    api_key_configured as postiz_configured, base_url as postiz_base_url, channel_label,
    normalize_channel, PostizClient, API_KEY as POSTIZ_API_KEY,
    BASE_URL_KEY as POSTIZ_BASE_URL_KEY, DEFAULT_BASE as POSTIZ_DEFAULT_BASE,
};
pub use publisher::{
    disconnect_channel, publisher_snapshot, start_connect, ConnectStart, PublisherSnapshot,
};
pub use schedule::{recommend_scheduled_for, ScheduleInput};
pub use still::{compose_still, mflux_url, try_mflux_still, StillSpec};

use crate::cards::{self, Card};
use crate::config::paths::Paths;
use crate::events;
use crate::projects::{self, Project, ProjectBrand};

/// Filenames we will ever write or serve under a card's media dir.
pub fn is_safe_media_filename(name: &str) -> bool {
    matches!(name, "still.png" | "still.svg" | "video.mp4")
        || (name.starts_with("slide-")
            && name.ends_with(".png")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.'))
}

fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn media_dir(project_id: &str, card_id: &str) -> Result<std::path::PathBuf, String> {
    if !is_safe_id(project_id) || !is_safe_id(card_id) {
        return Err("invalid project or card id for media path".into());
    }
    Ok(Paths::grow_media_dir().join(project_id).join(card_id))
}

pub fn resolve_media_file(
    project_id: &str,
    card_id: &str,
    filename: &str,
) -> Result<std::path::PathBuf, String> {
    if !is_safe_media_filename(filename) {
        return Err("invalid media filename".into());
    }
    let dir = media_dir(project_id, card_id)?;
    Ok(dir.join(filename))
}

/// Fill format/channel/schedule/mediaStatus on a new social_post. Does not
/// invent another user's brand or a canned origin story — those come from
/// the project bag, which may be empty.
#[allow(clippy::too_many_arguments)]
pub async fn enrich_new_social_post(
    pool: &Pool<Sqlite>,
    project: &Project,
    title: &str,
    description: Option<&str>,
    mut meta: serde_json::Value,
    format: Option<&str>,
    channel: Option<&str>,
    harvest_kind: Option<&str>,
) -> Result<serde_json::Value, String> {
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    let map = meta.as_object_mut().expect("normalized");

    // Create is always a draft. Approve is the only path to scheduled.
    map.insert(
        cards::POST_STATUS_KEY.to_string(),
        serde_json::json!("draft"),
    );

    let format_in = format.or_else(|| map.get(cards::POST_FORMAT_KEY).and_then(|v| v.as_str()));
    let format = match format_in.map(str::trim).filter(|s| !s.is_empty()) {
        Some(f) => {
            cards::validate_social_format(f)?;
            f.to_string()
        }
        None => infer_format(title, description),
    };
    map.insert(
        cards::POST_FORMAT_KEY.to_string(),
        serde_json::json!(format),
    );

    let channel_in = channel.or_else(|| map.get(cards::POST_CHANNEL_KEY).and_then(|v| v.as_str()));
    let channel = match channel_in.map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => c.to_lowercase(),
        None => infer_channel(project, &format),
    };
    map.insert(
        cards::POST_CHANNEL_KEY.to_string(),
        serde_json::json!(channel),
    );

    let harvest_in = harvest_kind.or_else(|| {
        map.get(cards::POST_HARVEST_KIND_KEY)
            .and_then(|v| v.as_str())
    });
    if let Some(kind) = harvest_in.map(str::trim).filter(|s| !s.is_empty()) {
        cards::validate_harvest_kind(kind)?;
        map.insert(
            cards::POST_HARVEST_KIND_KEY.to_string(),
            serde_json::json!(kind),
        );
    }

    if map
        .get(cards::POST_SCHEDULED_FOR_KEY)
        .and_then(|v| v.as_str())
        .is_none()
    {
        let occupied = occupied_times(pool, &project.id, &channel).await?;
        let not_before = previous_beat_time(map);
        let when = recommend_scheduled_for(ScheduleInput {
            channel: &channel,
            occupied: &occupied,
            not_before,
            now: chrono::Utc::now(),
        });
        map.insert(
            cards::POST_SCHEDULED_FOR_KEY.to_string(),
            serde_json::json!(when.to_rfc3339()),
        );
    } else {
        let when = map
            .get(cards::POST_SCHEDULED_FOR_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        cards::validate_post_metadata(Some(when), None)?;
    }

    map.insert(
        cards::POST_MEDIA_STATUS_KEY.to_string(),
        serde_json::json!("queued"),
    );
    let brand = ProjectBrand::from_metadata(&project.metadata_json);
    if let Some(rev) = brand.updated_at {
        map.insert(
            cards::POST_BRAND_REV_KEY.to_string(),
            serde_json::json!(rev),
        );
    }

    Ok(serde_json::Value::Object(map.clone()))
}

fn infer_format(title: &str, body: Option<&str>) -> String {
    let blob = format!("{} {}", title, body.unwrap_or("")).to_lowercase();
    if blob.contains("reel") || blob.contains("9:16") {
        "reel".into()
    } else if blob.contains("carousel") || blob.contains("slides") {
        "carousel".into()
    } else if blob.contains("shipped") || blob.contains("screenshot") {
        "compose".into()
    } else {
        "text".into()
    }
}

/// Prefer a channel the project's own strategy named. Fall back to LinkedIn
/// for text/compose and Instagram for visual formats — generic, not a
/// particular user's mix.
fn infer_channel(project: &Project, format: &str) -> String {
    if let Some(channels) = project
        .metadata_json
        .get("strategy")
        .and_then(|s| s.get("channels"))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.as_str())
    {
        let lower = channels.to_lowercase();
        if lower.contains("instagram") || lower.contains(" ig") {
            return "ig".into();
        }
        if lower.contains("linkedin") {
            return "li".into();
        }
    }
    match format {
        "reel" | "carousel" => "ig".into(),
        _ => "li".into(),
    }
}

async fn occupied_times(
    pool: &Pool<Sqlite>,
    project_id: &str,
    channel: &str,
) -> Result<Vec<chrono::DateTime<chrono::Utc>>, String> {
    let cards = cards::list_cards(pool, project_id, Some("social_post"), None).await?;
    Ok(cards
        .into_iter()
        .filter(|c| {
            c.metadata_json
                .get(cards::POST_CHANNEL_KEY)
                .and_then(|v| v.as_str())
                == Some(channel)
        })
        .filter_map(|c| {
            c.metadata_json
                .get(cards::POST_SCHEDULED_FOR_KEY)
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc))
        })
        .collect())
}

fn previous_beat_time(
    meta: &serde_json::Map<String, serde_json::Value>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    // beatIndex alone is not enough — the caller must have stamped the
    // previous beat's scheduledFor if they want ordering. We only enforce
    // not_before when it is already on the card (Harvester will set it).
    meta.get("notBefore")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// After a social_post row exists: generate a still (and a Reel video if
/// this user has Higgsfield credentials). Fire-and-forget on the current
/// runtime. Inbox-routed posts that already carry an attachment skip
/// generation — the user supplied the file.
pub fn enqueue_after_create(pool: Pool<Sqlite>, project_id: String, card_id: String) {
    tokio::spawn(async move {
        if let Err(e) = run_media_job(&pool, &project_id, &card_id).await {
            tracing::warn!(
                target: "permagent::grow_media",
                card = %card_id,
                "media job failed: {e}"
            );
            let _ = mark_failed(&pool, &card_id, &e).await;
        }
        events::emit(events::project_changed(&project_id, "grow_media"));
    });
}

pub async fn retry_media(
    pool: &Pool<Sqlite>,
    project_id: &str,
    card_id: &str,
    feedback: Option<&str>,
) -> Result<Card, String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{card_id}' not found"))?;
    if card.project_id != project_id {
        return Err("Card does not belong to this project".into());
    }
    if card.card_type != "social_post" {
        return Err("media retry is only valid for social_post cards".into());
    }
    let status = card
        .metadata_json
        .get(cards::POST_STATUS_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("draft");
    if status != "draft" {
        return Err(format!(
            "only a draft still can be regenerated (status is '{status}')"
        ));
    }
    let media = card
        .metadata_json
        .get(cards::POST_MEDIA_STATUS_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("queued");
    if media == "generating" {
        return Err("a still is already generating for this post".into());
    }

    let notes = feedback
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            card.metadata_json
                .get(cards::POST_MEDIA_FEEDBACK_KEY)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        });

    let mut patch = serde_json::json!({
        cards::POST_MEDIA_STATUS_KEY: "queued",
        cards::POST_MEDIA_ERROR_KEY: serde_json::Value::Null,
    });
    if let Some(notes) = notes {
        patch.as_object_mut().expect("literal").insert(
            cards::POST_MEDIA_FEEDBACK_KEY.to_string(),
            serde_json::json!(notes),
        );
    }
    cards::merge_card_metadata(pool, card_id, patch, true).await?;
    enqueue_after_create(pool.clone(), project_id.to_string(), card_id.to_string());
    cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| "card vanished after retry".into())
}

pub async fn approve_post(
    pool: &Pool<Sqlite>,
    project_id: &str,
    card_id: &str,
) -> Result<Card, String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{card_id}' not found"))?;
    if card.project_id != project_id {
        return Err("Card does not belong to this project".into());
    }
    if card.card_type != "social_post" {
        return Err("approve is only valid for social_post cards".into());
    }
    cards::assert_ready_to_schedule(&card.metadata_json)?;
    let status = card
        .metadata_json
        .get(cards::POST_STATUS_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("draft");
    if status != "draft" {
        return Err(format!(
            "only a draft can be approved (status is '{status}')"
        ));
    }
    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let published = publisher::submit_approved(pool, &project, &card).await?;
    let mut patch = serde_json::json!({ cards::POST_STATUS_KEY: "scheduled" });
    if let Some(ids) = published {
        let obj = patch.as_object_mut().expect("literal");
        obj.insert(
            cards::POST_PUBLISHER_POST_ID_KEY.to_string(),
            serde_json::json!(ids.post_id),
        );
        obj.insert(
            cards::POST_PUBLISHER_INTEGRATION_KEY.to_string(),
            serde_json::json!(ids.integration_id),
        );
    }
    let updated = cards::merge_card_metadata(pool, card_id, patch, true)
        .await?
        .ok_or_else(|| "card vanished during approve".to_string())?;
    events::emit(events::project_changed(project_id, "grow_media"));
    Ok(updated)
}

async fn mark_failed(pool: &Pool<Sqlite>, card_id: &str, error: &str) -> Result<(), String> {
    cards::merge_card_metadata(
        pool,
        card_id,
        serde_json::json!({
            cards::POST_MEDIA_STATUS_KEY: "failed",
            cards::POST_MEDIA_ERROR_KEY: error,
        }),
        true,
    )
    .await?;
    Ok(())
}

async fn run_media_job(pool: &Pool<Sqlite>, project_id: &str, card_id: &str) -> Result<(), String> {
    let card = cards::get_card(pool, card_id)
        .await?
        .ok_or_else(|| format!("Card '{card_id}' not found"))?;
    if card.card_type != "social_post" {
        return Ok(());
    }
    // User-supplied file (inbox route): do not overwrite it with a generated still.
    if card.metadata_json.get("attachment").is_some()
        && card
            .metadata_json
            .get(cards::POST_MEDIA_KEY)
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
    {
        cards::merge_card_metadata(
            pool,
            card_id,
            serde_json::json!({ cards::POST_MEDIA_STATUS_KEY: "ready" }),
            true,
        )
        .await?;
        return Ok(());
    }
    if card.metadata_json.get("attachment").is_some() {
        cards::merge_card_metadata(
            pool,
            card_id,
            serde_json::json!({ cards::POST_MEDIA_STATUS_KEY: "ready" }),
            true,
        )
        .await?;
        return Ok(());
    }

    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let brand = ProjectBrand::from_metadata(&project.metadata_json);
    let format = card
        .metadata_json
        .get(cards::POST_FORMAT_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    cards::merge_card_metadata(
        pool,
        card_id,
        serde_json::json!({ cards::POST_MEDIA_STATUS_KEY: "generating" }),
        true,
    )
    .await?;

    let dir = media_dir(project_id, card_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let spec = StillSpec {
        title: card.title.clone(),
        body: card.description.clone(),
        project_name: project.name.clone(),
        brand: brand.clone(),
        format: format.to_string(),
        feedback: card
            .metadata_json
            .get(cards::POST_MEDIA_FEEDBACK_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };

    let (still_source, still_path) = match try_mflux_still(&spec, &dir).await {
        Ok(path) => ("mflux", path),
        Err(mflux_err) => {
            tracing::info!(
                target: "permagent::grow_media",
                card = %card_id,
                "mflux unavailable ({mflux_err}); composing a branded still"
            );
            ("compose", compose_still(&spec, &dir)?)
        }
    };

    let mut media = vec![serde_json::json!({
        "kind": "still",
        "file": still_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("still.png"),
        "source": still_source,
        "prompt": still::prompt_for(&spec),
    })];

    let mut media_error: Option<String> = None;
    if format == "reel" {
        match HiggsfieldClient::from_user_secrets() {
            Some(client) => match client.animate_still(&still_path, &spec, &dir).await {
                Ok(video_path) => {
                    media.push(serde_json::json!({
                        "kind": "video",
                        "file": video_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("video.mp4"),
                        "source": "higgsfield",
                    }));
                }
                Err(e) => {
                    media_error = Some(format!("still ready; video failed: {e}"));
                }
            },
            None => {
                media_error = Some(
                    "still ready; Reel video skipped — this user has not connected Higgsfield"
                        .into(),
                );
            }
        }
    }

    let mut patch = serde_json::json!({
        cards::POST_MEDIA_STATUS_KEY: "ready",
        cards::POST_MEDIA_KEY: media,
        cards::POST_BRAND_REV_KEY: brand.updated_at,
    });
    if let Some(err) = media_error {
        patch.as_object_mut().expect("literal").insert(
            cards::POST_MEDIA_ERROR_KEY.to_string(),
            serde_json::json!(err),
        );
    } else {
        patch.as_object_mut().expect("literal").insert(
            cards::POST_MEDIA_ERROR_KEY.to_string(),
            serde_json::Value::Null,
        );
    }
    cards::merge_card_metadata(pool, card_id, patch, true).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::CreateProject;
    use crate::session::spectral_schema::init_spectral_db;

    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        init_spectral_db(&pool).await.unwrap();
        pool
    }

    #[test]
    fn media_filenames_reject_traversal() {
        assert!(is_safe_media_filename("still.png"));
        assert!(is_safe_media_filename("video.mp4"));
        assert!(!is_safe_media_filename("../secrets.png"));
        assert!(!is_safe_media_filename("/etc/passwd"));
        assert!(!is_safe_media_filename("still.png.bak"));
    }

    #[test]
    fn media_dir_rejects_hostile_ids() {
        assert!(media_dir("..", "x").is_err());
        assert!(media_dir("abc", "foo/bar").is_err());
        assert!(media_dir("proj-1", "card-2").is_ok());
    }

    #[tokio::test]
    async fn enrich_forces_draft_and_picks_a_time() {
        let pool = pool().await;
        let project = projects::create_project(
            &pool,
            CreateProject {
                name: "Example App".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let meta = enrich_new_social_post(
            &pool,
            &project,
            "We shipped search",
            Some("Filter by date."),
            serde_json::json!({ cards::POST_STATUS_KEY: "scheduled" }),
            None,
            None,
            Some("feature"),
        )
        .await
        .unwrap();
        assert_eq!(meta[cards::POST_STATUS_KEY], "draft");
        assert_eq!(meta[cards::POST_HARVEST_KIND_KEY], "feature");
        assert_eq!(meta[cards::POST_MEDIA_STATUS_KEY], "queued");
        let when = meta[cards::POST_SCHEDULED_FOR_KEY].as_str().unwrap();
        chrono::DateTime::parse_from_rfc3339(when).unwrap();
    }

    #[test]
    fn preserve_media_keys_keeps_server_fields() {
        let existing = serde_json::json!({
            "postStatus": "draft",
            "mediaStatus": "ready",
            "media": [{"file": "still.png"}],
            "mediaFeedback": "darker",
        });
        let incoming = serde_json::json!({
            "postStatus": "draft",
            "scheduledFor": "2026-08-21T15:00:00Z",
            "mediaStatus": "queued",
        });
        let out = cards::preserve_media_keys(&existing, incoming);
        assert_eq!(out["mediaStatus"], "ready");
        assert_eq!(out["media"][0]["file"], "still.png");
        assert_eq!(out["mediaFeedback"], "darker");
        assert_eq!(out["scheduledFor"], "2026-08-21T15:00:00Z");
    }

    #[tokio::test]
    async fn retry_keeps_copy_and_stores_feedback() {
        let pool = pool().await;
        let project = projects::create_project(
            &pool,
            CreateProject {
                name: "Example App".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let card = cards::create_card(
            &pool,
            cards::CreateCard {
                project_id: project.id.clone(),
                title: "Keep this hook".into(),
                description: Some("Keep this body.".into()),
                card_type: Some("social_post".into()),
                column_id: None,
                created_by: Some("user".into()),
                metadata_json: Some(serde_json::json!({
                    "postStatus": "draft",
                    "mediaStatus": "ready",
                    "scheduledFor": "2026-08-21T15:00:00Z",
                })),
            },
        )
        .await
        .unwrap();
        let out = retry_media(&pool, &project.id, &card.id, Some("darker, less type"))
            .await
            .unwrap();
        assert_eq!(out.title, "Keep this hook");
        assert_eq!(out.description, "Keep this body.");
        assert_eq!(out.metadata_json["mediaFeedback"], "darker, less type");
        assert_eq!(out.metadata_json["mediaStatus"], "queued");
        assert_ne!(out.metadata_json["postStatus"], "scheduled");
    }

    #[test]
    fn schedule_gate_requires_ready_media_and_a_time() {
        assert!(cards::assert_ready_to_schedule(&serde_json::json!({})).is_err());
        assert!(cards::assert_ready_to_schedule(&serde_json::json!({
            "mediaStatus": "ready"
        }))
        .is_err());
        assert!(cards::assert_ready_to_schedule(&serde_json::json!({
            "mediaStatus": "generating",
            "scheduledFor": "2026-08-20T12:00:00Z"
        }))
        .is_err());
        assert!(cards::assert_ready_to_schedule(&serde_json::json!({
            "mediaStatus": "ready",
            "scheduledFor": "2026-08-20T12:00:00Z"
        }))
        .is_ok());
    }
}
