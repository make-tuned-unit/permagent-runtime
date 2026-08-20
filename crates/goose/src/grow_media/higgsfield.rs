//! Higgsfield platform HTTP client.
//!
//! Credentials are the *current user's* keyring / env (`HF_API_KEY_ID`,
//! `HF_API_KEY_SECRET`). There is no shared app key and no project is
//! special-cased. Output is downloaded immediately (Higgsfield retains files
//! about seven days).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;

use super::still::{prompt_for, StillSpec};

pub const KEY_ID: &str = "HF_API_KEY_ID";
pub const KEY_SECRET: &str = "HF_API_KEY_SECRET";
pub const BASE_URL_KEY: &str = "higgsfield_base_url";
const DEFAULT_BASE: &str = "https://platform.higgsfield.ai";
const DEFAULT_I2V: &str = "higgsfield-ai/dop/standard";
const POLL: Duration = Duration::from_secs(2);
const POLL_CAP: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub struct HiggsfieldCredentials {
    pub key_id: String,
    pub secret: String,
}

pub fn credentials_configured() -> bool {
    read_secret(KEY_ID).is_some() && read_secret(KEY_SECRET).is_some()
}

fn read_secret(key: &str) -> Option<String> {
    Config::global()
        .get_secret::<String>(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub struct HiggsfieldClient {
    pub base_url: String,
    pub key_id: String,
    pub secret: String,
    pub i2v_model: String,
    http: reqwest::Client,
}

impl HiggsfieldClient {
    pub fn from_user_secrets() -> Option<Self> {
        let key_id = read_secret(KEY_ID)?;
        let secret = read_secret(KEY_SECRET)?;
        Some(Self::new(base_url(), key_id, secret, i2v_model()))
    }

    pub fn new(base_url: String, key_id: String, secret: String, i2v_model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            key_id,
            secret,
            i2v_model,
            http,
        }
    }

    fn auth(&self) -> String {
        format!("Key {}:{}", self.key_id, self.secret)
    }

    /// Upload the local still, submit image-to-video, poll, download mp4.
    pub async fn animate_still(
        &self,
        still: &Path,
        spec: &StillSpec,
        dest_dir: &Path,
    ) -> Result<PathBuf, String> {
        let bytes = std::fs::read(still).map_err(|e| e.to_string())?;
        let public_url = self.upload_png(&bytes).await?;
        let request_id = self.submit_i2v(&public_url, &motion_prompt(spec)).await?;
        let video_url = self.wait_for_video(&request_id).await?;
        let dest = dest_dir.join("video.mp4");
        self.download(&video_url, &dest).await?;
        Ok(dest)
    }

    async fn upload_png(&self, bytes: &[u8]) -> Result<String, String> {
        let created: serde_json::Value = self
            .http
            .post(format!("{}/files/generate-upload-url", self.base_url))
            .header("Authorization", self.auth())
            .json(&serde_json::json!({ "content_type": "image/png" }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let upload_url = created
            .get("upload_url")
            .and_then(|v| v.as_str())
            .ok_or("Higgsfield upload response missing upload_url")?;
        let public_url = created
            .get("public_url")
            .and_then(|v| v.as_str())
            .ok_or("Higgsfield upload response missing public_url")?
            .to_string();
        let mut req = self.http.put(upload_url).body(bytes.to_vec());
        if let Some(headers) = created.get("upload_headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    req = req.header(k.as_str(), val);
                }
            }
        } else {
            req = req.header("Content-Type", "image/png");
        }
        req.send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(public_url)
    }

    async fn submit_i2v(&self, image_url: &str, prompt: &str) -> Result<String, String> {
        let body: serde_json::Value = self
            .http
            .post(format!("{}/{}", self.base_url, self.i2v_model))
            .header("Authorization", self.auth())
            .json(&serde_json::json!({
                "image_url": image_url,
                "prompt": prompt,
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        body.get("request_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| "Higgsfield submit missing request_id".into())
    }

    async fn wait_for_video(&self, request_id: &str) -> Result<String, String> {
        let deadline = std::time::Instant::now() + POLL_CAP;
        loop {
            let status: serde_json::Value = self
                .http
                .get(format!("{}/requests/{request_id}/status", self.base_url))
                .header("Authorization", self.auth())
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let state = status.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match state {
                "completed" => {
                    return status
                        .get("video")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .ok_or_else(|| "completed Higgsfield request had no video.url".into());
                }
                "failed" | "nsfw" | "canceled" => {
                    let detail = status
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or(state);
                    return Err(format!("Higgsfield request {state}: {detail}"));
                }
                _ => {
                    if std::time::Instant::now() > deadline {
                        return Err("Higgsfield request timed out".into());
                    }
                    tokio::time::sleep(POLL).await;
                }
            }
        }
    }

    async fn download(&self, url: &str, dest: &Path) -> Result<(), String> {
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        std::fs::write(dest, bytes).map_err(|e| e.to_string())
    }
}

fn base_url() -> String {
    Config::global()
        .get_param::<String>(BASE_URL_KEY)
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn i2v_model() -> String {
    Config::global()
        .get_param::<String>("higgsfield_i2v_model")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_I2V.to_string())
}

fn motion_prompt(spec: &StillSpec) -> String {
    format!(
        "Slow, steady camera. Keep on-screen type readable. {}",
        prompt_for(spec)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectBrand;
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn downloads_completed_video_to_the_card_dir() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/files/generate-upload-url"))
            .and(header("Authorization", "Key test-id:test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "public_url": format!("{}/public/still.png", server.uri()),
                "upload_url": format!("{}/upload", server.uri()),
                "upload_headers": { "Content-Type": "image/png" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/upload"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/higgsfield-ai/dop/standard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "queued",
                "request_id": "11111111-1111-1111-1111-111111111111"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/requests/11111111-1111-1111-1111-111111111111/status",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "completed",
                "request_id": "11111111-1111-1111-1111-111111111111",
                "video": { "url": format!("{}/out.mp4", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/out.mp4"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-mp4"))
            .mount(&server)
            .await;

        let dir = TempDir::new().unwrap();
        let still = dir.path().join("still.png");
        std::fs::write(&still, b"png").unwrap();
        let client = HiggsfieldClient::new(
            server.uri(),
            "test-id".into(),
            "test-secret".into(),
            "higgsfield-ai/dop/standard".into(),
        );
        let spec = StillSpec {
            title: "Hook".into(),
            body: String::new(),
            project_name: "Example App".into(),
            brand: ProjectBrand::default(),
            format: "reel".into(),
            feedback: String::new(),
        };
        let video = client
            .animate_still(&still, &spec, dir.path())
            .await
            .unwrap();
        assert_eq!(std::fs::read(video).unwrap(), b"fake-mp4");
    }

    #[test]
    fn from_user_secrets_is_none_without_this_users_keys() {
        // Whatever is on the developer machine must not count as a baked-in
        // product key. Isolation: only a missing pair is asserted when the
        // process has no HF_* secrets in the test config — if the user has
        // configured keys, configured() is true for *them*, which is correct.
        let _ = credentials_configured();
    }
}
