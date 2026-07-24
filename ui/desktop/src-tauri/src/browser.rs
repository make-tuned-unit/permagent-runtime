use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewBuilder, WebviewUrl};

// ── File-intake inbox (epic #392 / #393) ────────────────────────────────────
//
// Downloads in the in-app browser webview are redirected onto disk under
// `~/.permagent/inbox/` and recorded as a metadata row in permagent.db via the
// daemon's `POST /api/inbox`. The webview fires `on_download` in this (desktop)
// process; persistence lives in the daemon, so we POST over the same
// token-authenticated localhost seam `activity.rs` uses. macOS does not report
// the final saved path on the `Finished` event, so we remember the destination
// we set on `Requested` and use that.

/// A download whose destination we redirected on `Requested`, awaiting `Finished`.
struct PendingInboxDownload {
    /// Source URL the file was downloaded from.
    url: String,
    /// Absolute on-disk path inside the inbox directory.
    abs_path: PathBuf,
    /// On-disk basename, also the `disk_path` relative to the inbox dir.
    filename: String,
}

fn inbox_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".permagent/inbox")
}

fn read_daemon_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".permagent/secrets/daemon_token.json");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("token")?.as_str().map(|s| s.to_string())
}

/// Strip any path components and unsafe characters from a suggested filename.
fn sanitize_filename(name: &str) -> String {
    let base = name.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Split `name` into (stem, extension) on the last dot. Extension is empty when
/// there is no dot or it is a leading dot (dotfile).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Return a filename that does not collide with an existing file in `dir`.
/// Appends `-1`, `-2`, … before the extension; falls back to a uuid prefix.
fn dedupe_filename(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    for n in 1..1000 {
        let candidate = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{}-{name}", uuid::Uuid::new_v4())
}

/// Ensure the inbox dir exists and compute a sanitized, collision-free target
/// from the browser's suggested destination. Returns (absolute path, filename).
fn prepare_inbox_destination(suggested: &Path) -> Result<(PathBuf, String), String> {
    let dir = inbox_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create inbox dir: {e}"))?;
    let raw = suggested
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let unique = dedupe_filename(&dir, &sanitize_filename(raw));
    Ok((dir.join(&unique), unique))
}

/// Best-effort content-type from the filename extension. `None` when unknown —
/// the column is nullable and a later pass can sniff bytes if needed.
fn guess_content_type(name: &str) -> Option<&'static str> {
    let (_, ext) = split_ext(name);
    let ct = match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => return None,
    };
    Some(ct)
}

/// POST the inbox metadata row to the daemon. Mirrors `activity.rs`'s
/// token-authenticated localhost call.
async fn record_inbox_file(pending: PendingInboxDownload) -> Result<(), String> {
    let token = read_daemon_token().ok_or("failed to read daemon token")?;
    let size_bytes = std::fs::metadata(&pending.abs_path)
        .ok()
        .map(|m| m.len() as i64);
    let body = serde_json::json!({
        "filename": pending.filename,
        "original_url": pending.url,
        "content_type": guess_content_type(&pending.filename),
        "size_bytes": size_bytes,
        "disk_path": pending.filename,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:3001/api/inbox")
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("daemon request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("daemon returned {status}: {text}"))
    }
}

struct BrowserWebview {
    _label: String,
}

pub struct BrowserSessions(Mutex<HashMap<String, BrowserWebview>>);

impl BrowserSessions {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

static WEBVIEW_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[derive(Clone, Serialize)]
struct BrowserNavigatedPayload {
    webview_id: String,
    url: String,
}

#[derive(Clone, Serialize)]
struct BrowserTitleChangedPayload {
    webview_id: String,
    title: String,
}

#[tauri::command]
pub async fn create_browser_webview(
    app: AppHandle,
    url: String,
    window_label: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    let id = WEBVIEW_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let label = format!("browser-{id}");
    let parsed_url: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview_url = WebviewUrl::External(parsed_url);

    let owner = window_label.unwrap_or_else(|| "main".to_string());
    let window = app
        .get_window(&owner)
        .ok_or_else(|| format!("Window {owner} not found"))?;

    let nav_id = label.clone();
    let nav_app = app.clone();
    let title_id = label.clone();
    let title_app = app.clone();
    // Pending downloads keyed by source URL, carried Requested -> Finished.
    let pending: Arc<Mutex<HashMap<String, PendingInboxDownload>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let builder = WebviewBuilder::new(&label, webview_url)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15")
        .on_download(move |_webview, event| {
            match event {
                tauri::webview::DownloadEvent::Requested { url, destination } => {
                    match prepare_inbox_destination(destination) {
                        Ok((abs_path, filename)) => {
                            let key = url.to_string();
                            *destination = abs_path.clone();
                            if let Ok(mut map) = pending.lock() {
                                map.insert(
                                    key.clone(),
                                    PendingInboxDownload {
                                        url: key,
                                        abs_path,
                                        filename,
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("[permagent-app] inbox: prepare destination failed: {e}");
                        }
                    }
                    // Allow the download regardless; redirection is best-effort.
                    true
                }
                tauri::webview::DownloadEvent::Finished { url, path: _, success } => {
                    let entry = pending.lock().ok().and_then(|mut m| m.remove(&url.to_string()));
                    if success {
                        if let Some(entry) = entry {
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = record_inbox_file(entry).await {
                                    eprintln!("[permagent-app] inbox: record failed: {e}");
                                }
                            });
                        }
                    }
                    true
                }
                _ => true,
            }
        })
        .on_navigation(move |nav_url: &url::Url| {
            let _ = nav_app.emit(
                "browser_navigated",
                BrowserNavigatedPayload {
                    webview_id: nav_id.clone(),
                    url: nav_url.to_string(),
                },
            );
            true
        })
        .on_document_title_changed(move |_webview, title| {
            let _ = title_app.emit(
                "browser_title_changed",
                BrowserTitleChangedPayload {
                    webview_id: title_id.clone(),
                    title,
                },
            );
        });

    let position = tauri::Position::Logical(tauri::LogicalPosition::new(x, y));
    let size = tauri::Size::Logical(tauri::LogicalSize::new(width, height));

    window
        .add_child(builder, position, size)
        .map_err(|e| format!("Failed to create webview: {e}"))?;

    app.state::<BrowserSessions>().0.lock().unwrap().insert(
        label.clone(),
        BrowserWebview {
            _label: label.clone(),
        },
    );

    Ok(label)
}

/// Move the existing native browser webview to another shell window. Tauri's
/// reparent operation preserves the WKWebView/WebView2 instance, including its
/// DOM, navigation history, cookies and injected capability context.
#[tauri::command]
pub async fn reparent_browser(
    app: AppHandle,
    webview_id: String,
    window_label: String,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let window = app
        .get_window(&window_label)
        .ok_or_else(|| format!("Window {window_label} not found"))?;
    webview
        .reparent(&window)
        .map_err(|e| format!("Reparent failed: {e}"))
}

#[tauri::command]
pub async fn navigate_browser(
    app: AppHandle,
    webview_id: String,
    url: String,
) -> Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("Invalid URL: {e}"))?;
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .navigate(parsed)
        .map_err(|e| format!("Navigation failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn update_browser_bounds(
    app: AppHandle,
    webview_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)))
        .map_err(|e| format!("Set position failed: {e}"))?;
    webview
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
        .map_err(|e| format!("Set size failed: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn hide_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    if let Some(webview) = app.get_webview(&webview_id) {
        webview
            .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                -10000.0, -10000.0,
            )))
            .map_err(|e| format!("Hide failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn close_browser(app: AppHandle, webview_id: String) -> Result<(), String> {
    app.state::<BrowserSessions>()
        .0
        .lock()
        .unwrap()
        .remove(&webview_id);
    if let Some(webview) = app.get_webview(&webview_id) {
        webview.close().map_err(|e| format!("Close failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn zoom_browser(
    app: AppHandle,
    webview_id: String,
    zoom_level: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let js = format!("document.body.style.zoom = '{:.0}%'", zoom_level * 100.0);
    webview.eval(&js).map_err(|e| format!("Zoom failed: {e}"))?;
    Ok(())
}

const MAX_CONTENT_CHARS: usize = 16000;

#[derive(Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub title: String,
    pub url: String,
    pub content: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub truncated: bool,
}

fn default_status() -> String {
    "ok".to_string()
}

fn previous_char_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn truncate_page_content(content: &mut String) -> bool {
    if content.len() <= MAX_CONTENT_CHARS {
        return false;
    }

    // Truncate at nearest paragraph or sentence boundary within the last 500 bytes.
    let search_start = previous_char_boundary(content, MAX_CONTENT_CHARS.saturating_sub(500));
    let max_end = previous_char_boundary(content, MAX_CONTENT_CHARS);
    let search_window = content.get(search_start..max_end).unwrap_or_default();
    let cut_at = search_window
        .rfind("\n\n")
        .or_else(|| search_window.rfind(". "))
        .or_else(|| search_window.rfind('\n'))
        .map(|pos| search_start + pos)
        .unwrap_or(max_end);

    content.truncate(cut_at);
    content.push_str("\n\n[content truncated]");
    true
}

#[tauri::command]
pub async fn get_page_content(app: AppHandle, webview_id: String) -> Result<PageContent, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;

    let (tx, rx) = std::sync::mpsc::channel::<String>();

    let js = r#"
        (function() {
            var title = document.title || '';
            var url = location.href || '';
            var content = document.body ? document.body.innerText : '';
            return JSON.stringify({ title: title, url: url, content: content });
        })()
    "#;

    webview
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .map_err(|e| format!("eval failed: {e}"))?;

    let raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "Timed out waiting for page content".to_string())?;

    // eval_with_callback JSON-serializes the JS return value, so the callback
    // receives a double-encoded string. Unwrap one layer.
    let json_str: String = serde_json::from_str(&raw).unwrap_or_else(|_| raw.clone());
    let mut page: PageContent =
        serde_json::from_str(&json_str).map_err(|e| format!("Parse failed: {e}"))?;

    if truncate_page_content(&mut page.content) {
        page.truncated = true;
    }

    Ok(page)
}

// ── Act-on-page: a11y-ref snapshot + click/type/select (#649 / #622) ─────────
//
// Extends the read-page seam (`get_page_content`) with two capabilities the
// agent grounds actions on: snapshot the page's INTERACTIVE elements as stable
// `data-permagent-ref` handles, then act on a ref. The grounding logic lives in
// `browser_grounding.js` (the single source of truth, also exercised by the
// vitest+jsdom suite); here we only inject it and parse the JSON it returns.

/// Injected verbatim into the page; defines `__permagentSnapshot` / `__permagentAct`.
const GROUNDING_JS: &str = include_str!("browser_grounding.js");

/// Cap on interactive elements per snapshot — bounds the token cost the same way
/// `MAX_CONTENT_CHARS` bounds read_browser_content. JS flags `truncated` when hit.
const MAX_SNAPSHOT_ELEMENTS: usize = 150;

#[derive(Clone, Serialize, Deserialize)]
pub struct SnapshotElement {
    #[serde(rename = "ref")]
    pub ref_id: u32,
    pub role: String,
    pub name: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub url: String,
    pub elements: Vec<SnapshotElement>,
    #[serde(default)]
    pub truncated: bool,
    /// "ok", "refused_scheme" (non-http(s) page), or "error".
    #[serde(default = "default_status")]
    pub status: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ActResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A FRESH snapshot taken after a successful act, so the caller never grounds
    /// on a stale ref (Playwright-MCP discipline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<PageSnapshot>,
}

/// Args handed to `__permagentAct` — serialized to a JSON object literal and
/// injected. serde_json emits valid, escaped JSON, so the agent-supplied `value`
/// cannot break out of the string (injection-safe).
#[derive(Serialize)]
struct ActArgs<'a> {
    #[serde(rename = "ref")]
    ref_id: u32,
    action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a str>,
}

fn snapshot_js(cap: usize) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nreturn JSON.stringify(__permagentSnapshot({cap}));\n}})()"
    )
}

fn act_js(args_json: &str, cap: usize) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nreturn JSON.stringify(__permagentAct({args_json}, {cap}));\n}})()"
    )
}

/// Inject `js` (which must evaluate to a JSON string) and parse it as `T`.
/// Mirrors `get_page_content`'s eval_with_callback + double-decode: the eval
/// bridge JSON-serializes the JS return value, so the callback receives a
/// double-encoded string — unwrap one layer, then parse.
fn eval_returning_json<R: tauri::Runtime, T: serde::de::DeserializeOwned>(
    webview: &tauri::Webview<R>,
    js: &str,
    timeout: std::time::Duration,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    webview
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .map_err(|e| format!("eval failed: {e}"))?;
    let raw = rx
        .recv_timeout(timeout)
        .map_err(|_| "Timed out waiting for the page to respond".to_string())?;
    let json_str: String = serde_json::from_str(&raw).unwrap_or_else(|_| raw.clone());
    serde_json::from_str(&json_str).map_err(|e| format!("Parse failed: {e}"))
}

/// List the page's interactive elements as stable refs (#649). Injects the same
/// grounding script the read path uses, capped at `MAX_SNAPSHOT_ELEMENTS`.
#[tauri::command]
pub async fn get_page_snapshot(app: AppHandle, webview_id: String) -> Result<PageSnapshot, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let js = snapshot_js(MAX_SNAPSHOT_ELEMENTS);
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(5))
}

/// Act on a ref from `get_page_snapshot`: click / type / select (#649). Returns
/// success plus a fresh snapshot. The action set is validated here (defense in
/// depth — the daemon route validates too). The in-page scheme guard in
/// `__permagentAct` refuses non-http(s) pages.
#[tauri::command]
pub async fn act_on_ref(
    app: AppHandle,
    webview_id: String,
    ref_id: u32,
    action: String,
    value: Option<String>,
) -> Result<ActResult, String> {
    if !matches!(action.as_str(), "click" | "type" | "select") {
        return Err(format!("Unsupported action: {action}"));
    }
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let args = ActArgs {
        ref_id,
        action: &action,
        value: value.as_deref(),
    };
    let args_json =
        serde_json::to_string(&args).map_err(|e| format!("Serialize args failed: {e}"))?;
    let js = act_js(&args_json, MAX_SNAPSHOT_ELEMENTS);
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_offsets_are_valid_utf8_boundaries() {
        let mut content = "a".repeat(MAX_CONTENT_CHARS - 501);
        content.push('€');
        content.push_str(&"b".repeat(497));
        content.push('€');
        content.push('c');

        assert!(truncate_page_content(&mut content));
        assert!(content.is_char_boundary(content.len()));
        assert_eq!(
            content,
            format!(
                "{}\n\n[content truncated]",
                "a".repeat(MAX_CONTENT_CHARS - 501) + "€" + &"b".repeat(497)
            )
        );
    }

    #[test]
    fn short_content_is_unchanged() {
        let original = "Short page with valid UTF-8: 🪿".to_string();
        let mut content = original.clone();

        assert!(!truncate_page_content(&mut content));
        assert_eq!(content, original);
    }
}
