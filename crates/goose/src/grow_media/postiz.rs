//! Postiz public HTTP client.
//!
//! Postiz is a separate program (Cloud or a self-hosted instance). This crate
//! talks to it over HTTP and never vendors Postiz source. The API key lives in
//! *this user's* secret store. There is no shared app key and no project is
//! special-cased.
//!
//! Auth is the raw key in `Authorization` (not `Bearer`). Default base is
//! Postiz Cloud so Instagram/LinkedIn OAuth apps stay with Postiz.

use std::path::Path;
use std::time::Duration;

use crate::config::Config;

pub const API_KEY: &str = "POSTIZ_API_KEY";
pub const BASE_URL_KEY: &str = "postiz_base_url";
pub const DEFAULT_BASE: &str = "https://api.postiz.com/public/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    pub id: String,
    pub name: String,
    pub identifier: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct CreatedPost {
    pub post_id: String,
    pub integration: String,
}

pub fn api_key_configured() -> bool {
    read_secret(API_KEY).is_some()
}

pub fn base_url() -> String {
    Config::global()
        .get_param::<String>(BASE_URL_KEY)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn read_secret(key: &str) -> Option<String> {
    Config::global()
        .get_secret::<String>(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub struct PostizClient {
    pub base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl PostizClient {
    pub fn from_user_secrets() -> Option<Self> {
        let api_key = read_secret(API_KEY)?;
        Some(Self::new(base_url(), api_key))
    }

    pub fn new(base_url: String, api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn json_error(resp: reqwest::Response) -> String {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let clip: String = body.chars().take(400).collect();
        if clip.is_empty() {
            format!("Postiz HTTP {status}")
        } else {
            format!("Postiz HTTP {status}: {clip}")
        }
    }

    pub async fn list_integrations(&self) -> Result<Vec<Integration>, String> {
        let resp = self
            .http
            .get(self.url("/integrations"))
            .header("Authorization", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(Self::json_error(resp).await);
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let items = if let Some(arr) = value.as_array() {
            arr.clone()
        } else if let Some(arr) = value.get("integrations").and_then(|v| v.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };
        Ok(items
            .into_iter()
            .filter_map(|v| {
                let id = v.get("id")?.as_str()?.to_string();
                if id.is_empty() {
                    return None;
                }
                Some(Integration {
                    id,
                    name: v
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    identifier: v
                        .get("identifier")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    profile: v
                        .get("profile")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    disabled: v.get("disabled").and_then(|x| x.as_bool()).unwrap_or(false),
                })
            })
            .collect())
    }

    /// OAuth URL for a Postiz provider identifier (`instagram-standalone`, …).
    pub async fn oauth_url(
        &self,
        identifier: &str,
        refresh_id: Option<&str>,
    ) -> Result<String, String> {
        let mut path = format!("/social/{identifier}");
        if let Some(id) = refresh_id.filter(|s| !s.is_empty()) {
            path.push_str(&format!("?refresh={}", urlencoding::encode(id)));
        }
        let resp = self
            .http
            .get(self.url(&path))
            .header("Authorization", &self.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(Self::json_error(resp).await);
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        value
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "Postiz connect response missing url".into())
    }

    pub async fn upload(&self, file: &Path) -> Result<UploadedFile, String> {
        let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("media.bin")
            .to_string();
        let mime = mime_for(&name);
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name)
            .mime_str(mime)
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = self
            .http
            .post(self.url("/upload"))
            .header("Authorization", &self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(Self::json_error(resp).await);
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Postiz upload response missing id")?
            .to_string();
        let path = value
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or("Postiz upload response missing path")?
            .to_string();
        Ok(UploadedFile { id, path })
    }

    pub async fn create_post(&self, body: &serde_json::Value) -> Result<Vec<CreatedPost>, String> {
        let resp = self
            .http
            .post(self.url("/posts"))
            .header("Authorization", &self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(Self::json_error(resp).await);
        }
        let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let items = if let Some(arr) = value.as_array() {
            arr.clone()
        } else if let Some(arr) = value.get("posts").and_then(|v| v.as_array()) {
            arr.clone()
        } else {
            vec![value]
        };
        Ok(items
            .into_iter()
            .filter_map(|v| {
                let post_id = v
                    .get("postId")
                    .or_else(|| v.get("id"))
                    .and_then(|x| x.as_str())?
                    .to_string();
                let integration = v
                    .get("integration")
                    .map(|x| match x {
                        serde_json::Value::String(s) => s.clone(),
                        other => other
                            .get("id")
                            .and_then(|id| id.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .unwrap_or_default();
                Some(CreatedPost {
                    post_id,
                    integration,
                })
            })
            .collect())
    }
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// Grow channel slug → Postiz provider identifiers to try, in order.
pub fn providers_for(channel: &str) -> &'static [&'static str] {
    match normalize_channel(channel) {
        Some("ig") => &["instagram-standalone", "instagram"],
        Some("li") => &["linkedin", "linkedin-page"],
        Some("x") => &["x"],
        _ => &[],
    }
}

pub fn normalize_channel(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ig" | "instagram" | "instagram-standalone" => Some("ig"),
        "li" | "linkedin" | "linkedin-page" => Some("li"),
        "x" | "twitter" | "twitter-x" => Some("x"),
        _ => None,
    }
}

pub fn channel_label(channel: &str) -> &'static str {
    match normalize_channel(channel) {
        Some("ig") => "Instagram",
        Some("li") => "LinkedIn",
        Some("x") => "X",
        _ => "that network",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn channel_slugs_map_to_postiz_providers() {
        assert_eq!(normalize_channel("Instagram"), Some("ig"));
        assert_eq!(providers_for("ig"), ["instagram-standalone", "instagram"]);
        assert_eq!(providers_for("li"), ["linkedin", "linkedin-page"]);
        assert_eq!(channel_label("ig"), "Instagram");
        assert!(providers_for("tiktok").is_empty());
    }

    #[tokio::test]
    async fn lists_integrations_with_raw_api_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/integrations"))
            .and(header("Authorization", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "int-1",
                    "name": "Example Shop",
                    "identifier": "instagram-standalone",
                    "profile": "exampleshop",
                    "disabled": false
                }
            ])))
            .mount(&server)
            .await;
        let client = PostizClient::new(server.uri(), "test-key".into());
        let list = client.list_integrations().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "int-1");
        assert_eq!(list[0].identifier, "instagram-standalone");
    }

    #[tokio::test]
    async fn oauth_url_and_upload_and_create_post() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/social/instagram-standalone"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": "https://www.instagram.com/accounts/login/"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "img-1",
                "path": "https://uploads.example/still.png"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/posts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "postId": "post-9", "integration": "int-1" }
            ])))
            .mount(&server)
            .await;

        let client = PostizClient::new(server.uri(), "test-key".into());
        let url = client
            .oauth_url("instagram-standalone", None)
            .await
            .unwrap();
        assert!(url.contains("instagram.com"));

        let dir = TempDir::new().unwrap();
        let still = dir.path().join("still.png");
        std::fs::write(&still, b"png").unwrap();
        let uploaded = client.upload(&still).await.unwrap();
        assert_eq!(uploaded.id, "img-1");

        let created = client
            .create_post(&serde_json::json!({
                "type": "schedule",
                "date": "2026-08-21T15:00:00.000Z",
                "shortLink": false,
                "tags": [],
                "posts": []
            }))
            .await
            .unwrap();
        assert_eq!(created[0].post_id, "post-9");
    }
}
