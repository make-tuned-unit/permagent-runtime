//! Per-project social channel bindings, backed by Postiz.
//!
//! The Postiz API key is per-install. Which Instagram (or LinkedIn, or X)
//! account a project posts to lives on *that project* (`metadata_json.publisher`).
//! Connecting Grocery Saver's Instagram does not connect any other project.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};

use super::postiz::{
    api_key_configured, channel_label, normalize_channel, providers_for, Integration, PostizClient,
};
use super::resolve_media_file;
use crate::cards::{self, Card};
use crate::events;
use crate::projects::{self, Project, UpdateProject};

const PENDING_TTL_MINUTES: i64 = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelBinding {
    pub integration_id: String,
    pub identifier: String,
    pub name: String,
    pub profile: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConnect {
    pub channel: String,
    pub providers: Vec<String>,
    pub seen_ids: Vec<String>,
    pub started_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherSnapshot {
    pub configured: bool,
    pub base_url: String,
    pub channels: HashMap<String, ChannelBinding>,
    pub pending: Option<PendingConnect>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStart {
    pub url: String,
    pub channel: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct PublishIds {
    pub post_id: String,
    pub integration_id: String,
}

pub async fn publisher_snapshot(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<PublisherSnapshot, String> {
    let _ = complete_pending(pool, project_id).await;
    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    Ok(snapshot_from(&project))
}

fn snapshot_from(project: &Project) -> PublisherSnapshot {
    PublisherSnapshot {
        configured: api_key_configured(),
        base_url: super::postiz::base_url(),
        channels: channels_of(project),
        pending: pending_of(project),
    }
}

fn publisher_bag(project: &Project) -> serde_json::Value {
    project
        .metadata_json
        .get("publisher")
        .cloned()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn channels_of(project: &Project) -> HashMap<String, ChannelBinding> {
    let mut out = HashMap::new();
    let Some(map) = publisher_bag(project)
        .get("channels")
        .and_then(|v| v.as_object())
        .cloned()
    else {
        return out;
    };
    for (k, v) in map {
        let Some(channel) = normalize_channel(&k) else {
            continue;
        };
        let Some(id) = v.get("integrationId").and_then(|x| x.as_str()) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        out.insert(
            channel.to_string(),
            ChannelBinding {
                integration_id: id.to_string(),
                identifier: v
                    .get("identifier")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                profile: v
                    .get("profile")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
        );
    }
    out
}

fn pending_of(project: &Project) -> Option<PendingConnect> {
    let v = publisher_bag(project).get("pending")?.clone();
    serde_json::from_value(v).ok()
}

async fn write_publisher(
    pool: &Pool<Sqlite>,
    project_id: &str,
    mut bag: serde_json::Value,
) -> Result<Project, String> {
    if !bag.is_object() {
        bag = serde_json::json!({});
    }
    let existing = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let mut metadata = existing.metadata_json.clone();
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    metadata
        .as_object_mut()
        .expect("normalized")
        .insert("publisher".to_string(), bag);
    projects::update_project(
        pool,
        project_id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await?
    .ok_or_else(|| "project vanished".to_string())
}

fn client() -> Result<PostizClient, String> {
    PostizClient::from_user_secrets().ok_or_else(|| {
        "Save a Postiz API key in Grow first (once per install). Then connect Instagram or LinkedIn for this project."
            .into()
    })
}

/// Open the provider login in the user's browser and remember which new
/// integration to bind onto *this* project when it appears.
pub async fn start_connect(
    pool: &Pool<Sqlite>,
    project_id: &str,
    channel_raw: &str,
) -> Result<ConnectStart, String> {
    let channel = normalize_channel(channel_raw)
        .ok_or_else(|| format!("Unknown channel '{channel_raw}'. Use ig, li, or x."))?;
    let providers = providers_for(channel);
    if providers.is_empty() {
        return Err(format!("No Postiz provider for {channel}"));
    }
    let client = client()?;
    let existing = client.list_integrations().await?;
    let seen_ids: Vec<String> = existing.iter().map(|i| i.id.clone()).collect();

    let mut last_err = "Postiz did not return a login URL".to_string();
    let mut url = None;
    for identifier in providers {
        match client.oauth_url(identifier, None).await {
            Ok(u) => {
                url = Some(u);
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let url = url.ok_or(last_err)?;

    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let mut bag = publisher_bag(&project);
    bag.as_object_mut().expect("object").insert(
        "pending".to_string(),
        serde_json::to_value(PendingConnect {
            channel: channel.to_string(),
            providers: providers.iter().map(|s| s.to_string()).collect(),
            seen_ids,
            started_at: Utc::now().to_rfc3339(),
        })
        .expect("pending"),
    );
    write_publisher(pool, project_id, bag).await?;
    events::emit(events::project_changed(project_id, "publisher"));

    let _ = webbrowser::open(&url);
    Ok(ConnectStart {
        url,
        channel: channel.to_string(),
        label: channel_label(channel).to_string(),
    })
}

pub async fn disconnect_channel(
    pool: &Pool<Sqlite>,
    project_id: &str,
    channel_raw: &str,
) -> Result<PublisherSnapshot, String> {
    let channel =
        normalize_channel(channel_raw).ok_or_else(|| format!("Unknown channel '{channel_raw}'"))?;
    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let mut bag = publisher_bag(&project);
    if let Some(channels) = bag.get_mut("channels").and_then(|v| v.as_object_mut()) {
        channels.remove(channel);
    }
    write_publisher(pool, project_id, bag).await?;
    events::emit(events::project_changed(project_id, "publisher"));
    publisher_snapshot(pool, project_id).await
}

/// If a login finished, bind the new Postiz integration onto this project.
pub async fn complete_pending(
    pool: &Pool<Sqlite>,
    project_id: &str,
) -> Result<Option<ChannelBinding>, String> {
    let project = match projects::get_project(pool, project_id).await? {
        Some(p) => p,
        None => return Ok(None),
    };
    let Some(pending) = pending_of(&project) else {
        return Ok(None);
    };
    if pending_expired(&pending) {
        clear_pending(pool, project_id).await?;
        return Ok(None);
    }
    let Ok(client) = client() else {
        return Ok(None);
    };
    let list = client.list_integrations().await?;
    let seen: HashSet<&str> = pending.seen_ids.iter().map(|s| s.as_str()).collect();
    let found = list.into_iter().find(|i| {
        !i.disabled
            && !i.id.is_empty()
            && !seen.contains(i.id.as_str())
            && pending.providers.iter().any(|p| p == &i.identifier)
    });
    let Some(found) = found else {
        return Ok(None);
    };
    bind_integration(pool, project_id, &pending.channel, &found).await
}

fn pending_expired(pending: &PendingConnect) -> bool {
    DateTime::parse_from_rfc3339(&pending.started_at)
        .ok()
        .map(|t| Utc::now().signed_duration_since(t.with_timezone(&Utc)))
        .is_some_and(|d| d.num_minutes() >= PENDING_TTL_MINUTES)
}

async fn clear_pending(pool: &Pool<Sqlite>, project_id: &str) -> Result<(), String> {
    let project = match projects::get_project(pool, project_id).await? {
        Some(p) => p,
        None => return Ok(()),
    };
    let mut bag = publisher_bag(&project);
    if let Some(obj) = bag.as_object_mut() {
        obj.remove("pending");
    }
    write_publisher(pool, project_id, bag).await?;
    Ok(())
}

async fn bind_integration(
    pool: &Pool<Sqlite>,
    project_id: &str,
    channel: &str,
    found: &Integration,
) -> Result<Option<ChannelBinding>, String> {
    let binding = ChannelBinding {
        integration_id: found.id.clone(),
        identifier: found.identifier.clone(),
        name: found.name.clone(),
        profile: found.profile.clone(),
    };
    let project = projects::get_project(pool, project_id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let mut bag = publisher_bag(&project);
    {
        let obj = bag.as_object_mut().expect("object");
        let channels = obj
            .entry("channels")
            .or_insert_with(|| serde_json::json!({}));
        if !channels.is_object() {
            *channels = serde_json::json!({});
        }
        channels.as_object_mut().expect("channels").insert(
            channel.to_string(),
            serde_json::to_value(&binding).expect("binding"),
        );
        obj.remove("pending");
    }
    write_publisher(pool, project_id, bag).await?;
    events::emit(events::project_changed(project_id, "publisher"));
    Ok(Some(binding))
}

/// When Postiz is configured, upload media and schedule on this project's
/// connected account. When it is not, Approve stays calendar-only.
pub async fn submit_approved(
    pool: &Pool<Sqlite>,
    project: &Project,
    card: &Card,
) -> Result<Option<PublishIds>, String> {
    if !api_key_configured() {
        return Ok(None);
    }
    let _ = complete_pending(pool, &project.id).await;
    let project = projects::get_project(pool, &project.id)
        .await?
        .ok_or_else(|| "project not found".to_string())?;
    let channel_raw = card
        .metadata_json
        .get(cards::POST_CHANNEL_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let channel = normalize_channel(channel_raw).ok_or_else(|| {
        format!(
            "This post has no network channel (ig / li / x). Set channel on the card, then connect that account for this project."
        )
    })?;
    let binding = channels_of(&project).remove(channel).ok_or_else(|| {
        format!(
            "Connect {} for this project first (Grow → Connect {}). The post was not scheduled.",
            channel_label(channel),
            channel_label(channel)
        )
    })?;
    let client = client()?;
    let images = upload_card_media(&client, &project.id, card).await?;
    let when = card
        .metadata_json
        .get(cards::POST_SCHEDULED_FOR_KEY)
        .and_then(|v| v.as_str())
        .ok_or("Cannot schedule a post with no scheduledFor instant.")?;
    let format = card
        .metadata_json
        .get(cards::POST_FORMAT_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("text");
    let body = schedule_body(card, &binding, &images, when, format);
    let created = client.create_post(&body).await?;
    let first = created
        .into_iter()
        .next()
        .ok_or_else(|| "Postiz accepted the post but returned no id".to_string())?;
    Ok(Some(PublishIds {
        post_id: first.post_id,
        integration_id: if first.integration.is_empty() {
            binding.integration_id
        } else {
            first.integration
        },
    }))
}

async fn upload_card_media(
    client: &PostizClient,
    project_id: &str,
    card: &Card,
) -> Result<Vec<serde_json::Value>, String> {
    let mut files: Vec<String> = card
        .metadata_json
        .get(cards::POST_MEDIA_KEY)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("file").and_then(|f| f.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let format = card
        .metadata_json
        .get(cards::POST_FORMAT_KEY)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if format == "reel" && !files.iter().any(|f| f.ends_with(".mp4")) {
        files.push("video.mp4".into());
    }
    if !files.iter().any(|f| f.ends_with(".png")) {
        files.insert(0, "still.png".into());
    }

    let mut images = Vec::new();
    for name in files {
        let path = match resolve_media_file(project_id, &card.id, &name) {
            Ok(p) if p.is_file() => p,
            _ => continue,
        };
        let uploaded = client.upload(&path).await?;
        images.push(serde_json::json!({
            "id": uploaded.id,
            "path": uploaded.path,
        }));
    }
    Ok(images)
}

fn schedule_body(
    card: &Card,
    binding: &ChannelBinding,
    images: &[serde_json::Value],
    when: &str,
    format: &str,
) -> serde_json::Value {
    let content = post_content(&card.title, Some(&card.description));
    let in_past = DateTime::parse_from_rfc3339(when)
        .ok()
        .is_some_and(|t| t.with_timezone(&Utc) <= Utc::now());
    let post_type = if in_past { "now" } else { "schedule" };
    let settings = provider_settings(&binding.identifier, format, images);
    serde_json::json!({
        "type": post_type,
        "date": when,
        "shortLink": false,
        "tags": [],
        "posts": [{
            "integration": { "id": binding.integration_id },
            "value": [{
                "content": content,
                "image": images,
            }],
            "settings": settings,
        }],
    })
}

fn post_content(title: &str, description: Option<&str>) -> String {
    let title = title.trim();
    let body = description.unwrap_or("").trim();
    if body.is_empty() {
        title.to_string()
    } else if title.is_empty() {
        body.to_string()
    } else {
        format!("{title}\n\n{body}")
    }
}

fn provider_settings(
    identifier: &str,
    format: &str,
    images: &[serde_json::Value],
) -> serde_json::Value {
    let has_video = images.iter().any(|img| {
        img.get("path")
            .and_then(|v| v.as_str())
            .is_some_and(|p| p.to_ascii_lowercase().contains(".mp4"))
            || img
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id.contains("mp4"))
    });
    let ig_type = if format == "reel" || has_video {
        "reel"
    } else {
        "post"
    };
    match identifier {
        "instagram" | "instagram-standalone" => serde_json::json!({
            "__type": identifier,
            "post_type": ig_type,
        }),
        "linkedin" | "linkedin-page" => serde_json::json!({
            "__type": identifier,
        }),
        "x" => serde_json::json!({
            "__type": "x",
            "who_can_reply_post": "everyone",
        }),
        other => serde_json::json!({ "__type": other }),
    }
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

    #[tokio::test]
    async fn binding_stays_on_this_project_only() {
        let pool = pool().await;
        let a = projects::create_project(
            &pool,
            CreateProject {
                name: "Project A".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let b = projects::create_project(
            &pool,
            CreateProject {
                name: "Project B".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let found = Integration {
            id: "int-ig".into(),
            name: "Shop".into(),
            identifier: "instagram-standalone".into(),
            profile: "shop".into(),
            disabled: false,
        };
        bind_integration(&pool, &a.id, "ig", &found).await.unwrap();
        let snap_a = snapshot_from(&projects::get_project(&pool, &a.id).await.unwrap().unwrap());
        let snap_b = snapshot_from(&projects::get_project(&pool, &b.id).await.unwrap().unwrap());
        assert_eq!(snap_a.channels["ig"].integration_id, "int-ig");
        assert!(snap_b.channels.is_empty());
    }

    #[test]
    fn schedule_body_uses_the_bound_integration() {
        let card = Card {
            id: "c1".into(),
            project_id: "p1".into(),
            card_type: "social_post".into(),
            title: "Hook".into(),
            description: "Body copy.".into(),
            column_id: "col".into(),
            position: 0,
            created_by: String::new(),
            assigned_to: None,
            metadata_json: serde_json::json!({}),
            created_at: String::new(),
            updated_at: String::new(),
            archived_at: None,
        };
        let binding = ChannelBinding {
            integration_id: "int-ig".into(),
            identifier: "instagram-standalone".into(),
            name: "Shop".into(),
            profile: "shop".into(),
        };
        let body = schedule_body(
            &card,
            &binding,
            &[serde_json::json!({"id": "img", "path": "https://x/still.png"})],
            "2026-08-21T15:00:00Z",
            "text",
        );
        assert_eq!(body["posts"][0]["integration"]["id"], "int-ig");
        assert_eq!(
            body["posts"][0]["settings"]["__type"],
            "instagram-standalone"
        );
        assert_eq!(body["posts"][0]["settings"]["post_type"], "post");
        assert!(body["posts"][0]["value"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Hook"));
    }
}
