use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::webview::PageLoadEvent;
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

/// A MAIN-FRAME page-load transition. `url` is read from the WKWebView itself
/// (`WKWebView.URL`), so it is the main frame's URL by construction — a
/// subframe can never appear here.
///
/// This replaced an `on_navigation` emit, which was the wrong source: that hook
/// wraps `decidePolicyForNavigationAction`, which fires for EVERY frame and
/// hands the callback only a URL — no way to tell an ad iframe from the page.
/// CBC embeds Google's `/api2/aframe`, so the last iframe to load renamed the
/// tab `google.com` and put the ad frame's path in the address bar (reported
/// 2026-08-04). `on_page_load` maps to `didCommitNavigation` /
/// `didFinishNavigation`, which WebKit only calls for the main frame.
#[derive(Clone, Serialize)]
struct BrowserPageLoadPayload {
    webview_id: String,
    url: String,
    /// `true` at commit (the page's identity is now this URL), `false` at
    /// finish. Drives the reload spinner as well as the address bar.
    loading: bool,
}

#[derive(Clone, Serialize)]
struct BrowserTitleChangedPayload {
    webview_id: String,
    title: String,
}

/// `target=_blank` / `window.open` from a page: the popup is DENIED at the
/// WKWebView layer and re-routed to the tab strip via this event. The UI
/// listener predates this emitter's return — the original emitter (#240,
/// 9c856568e) lived in the old command-center shell and was deleted with it
/// (#709), leaving every popup link silently dead. Field name is
/// `source_webview_id` because that is what Browser.tsx already reads.
#[derive(Clone, Serialize)]
struct BrowserNewWindowPayload {
    source_webview_id: String,
    url: String,
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
    let popup_id = label.clone();
    let popup_app = app.clone();
    let popup_owner = owner.clone();
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
        .on_page_load(move |_webview, payload| {
            let _ = nav_app.emit(
                "browser_page_load",
                BrowserPageLoadPayload {
                    webview_id: nav_id.clone(),
                    url: payload.url().to_string(),
                    loading: matches!(payload.event(), PageLoadEvent::Started),
                },
            );
        })
        .on_document_title_changed(move |_webview, title| {
            let _ = title_app.emit(
                "browser_title_changed",
                BrowserTitleChangedPayload {
                    webview_id: title_id.clone(),
                    title,
                },
            );
        })
        // Popups become tabs: deny the native window, hand the URL to the tab
        // strip. Scoped to the OWNING window (`emit_to`) — a global emit would
        // open the link once per live Browser instance (BuildView + any
        // detached pane both run the listener).
        .on_new_window(move |url, _features| {
            let _ = popup_app.emit_to(
                popup_owner.as_str(),
                "browser_new_window_request",
                BrowserNewWindowPayload {
                    source_webview_id: popup_id.clone(),
                    url: url.to_string(),
                },
            );
            tauri::webview::NewWindowResponse::Deny
        });

    let position = tauri::Position::Logical(tauri::LogicalPosition::new(x, y));
    let size = tauri::Size::Logical(tauri::LogicalSize::new(width, height));

    let child = window
        .add_child(builder, position, size)
        .map_err(|e| format!("Failed to create webview: {e}"))?;

    // Media capture (getUserMedia) for the in-app browser.
    //
    // Without this a Zoom / Google Meet / Teams call cannot run here at all:
    // WKWebView hides navigator.mediaDevices unless the private
    // `_mediaCaptureEnabled` preference is set. The main and chat WINDOWS get
    // it by invoking `enable_media_capture_cmd` from their own JS on mount —
    // a route unavailable to this webview, whose JS belongs to the remote
    // page and has no Tauri bridge. So it must be applied here, at creation,
    // from the Rust side. Best-effort and non-fatal, exactly like the window
    // path: a failure means no camera/mic in the browser, not a dead tab.
    #[cfg(target_os = "macos")]
    {
        let _ = child.with_webview(|w| crate::apply_media_capture(&w));
    }

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
        .map_err(|e| format!("Reparent failed: {e}"))?;

    // RE-PARK after reparenting, in the same call.
    //
    // The caller hides the webview before handing it over, but `hide_browser`
    // parks it at (-10000,-10000) in the OLD window's coordinate space. Once
    // `reparent` re-hosts the WKWebView under a new parent that offset no
    // longer means "offscreen", so the webview paints — at the PANE window's
    // size — until the receiving window's bounds pump corrects it a frame or
    // two later. That is the full-screen flash seen when closing a popped-out
    // browser (reported 2026-08-04, intermittent because it depends on whether
    // a compositor frame lands in the gap).
    //
    // Re-parking here closes the gap entirely: there is no JS round-trip
    // between the reparent and the park, so nothing can be painted in
    // between. The receiving side reveals it by setting real bounds, which is
    // already what `syncBounds` does on arrival.
    webview
        .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            -10000.0, -10000.0,
        )))
        .map_err(|e| format!("Re-park after reparent failed: {e}"))?;
    Ok(())
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

/// Close every browser webview the UI no longer knows about.
///
/// Why this exists: `BrowserSessions` and the native child webviews live for
/// the lifetime of the PROCESS, but the React shell's memory of which
/// `webview_id`s exist dies with the page. Reload the shell (or hot-reload in
/// dev) and every open browser child keeps compositing above the DOM — native
/// child webviews always render over HTML — while the only code that could
/// address them has forgotten their ids. The result is a UI buried under
/// webviews nothing can close, and the only way out is force-quitting the app.
///
/// The shell calls this once on mount with the ids it still believes in
/// (normally none, right after a reload). Anything labelled `browser-*` that
/// is not in `keep` is orphaned by definition and gets closed.
///
/// Safe as a prefix sweep because these labels come from a single
/// process-static counter (`WEBVIEW_COUNTER`) and no other webview in the app
/// uses the `browser-` prefix. Returns the number reaped so a caller — or a
/// human reading the log — can tell the difference between "nothing was
/// orphaned" and "the sweep never ran".
#[tauri::command]
pub async fn reap_orphan_browsers(app: AppHandle, keep: Vec<String>) -> Result<usize, String> {
    // Take the full label set first, then close outside the lock: closing a
    // webview can re-enter, and holding the mutex across that is how the pane
    // teardown deadlocked before.
    let orphans: Vec<String> = {
        let sessions = app.state::<BrowserSessions>();
        let map = sessions.0.lock().unwrap();
        map.keys()
            .filter(|label| !keep.contains(*label))
            .cloned()
            .collect()
    };

    let mut reaped = 0usize;
    for label in &orphans {
        if let Some(webview) = app.get_webview(label) {
            // Best-effort: a webview that is already gone is exactly the
            // outcome we want, so a close error must not abort the sweep and
            // strand the remaining orphans.
            if let Err(e) = webview.close() {
                eprintln!("[permagent-app] reap: close {label} failed: {e}");
                continue;
            }
        }
        app.state::<BrowserSessions>()
            .0
            .lock()
            .unwrap()
            .remove(label);
        reaped += 1;
    }

    if reaped > 0 {
        eprintln!(
            "[permagent-app] reap: closed {reaped} orphaned browser webview(s), kept {}",
            keep.len()
        );
    }
    Ok(reaped)
}

/// Tear down a detached pane window and its child browser webviews in ONE
/// native operation.
///
/// Why this exists: the pane window's JS close handler used to await a
/// `close_browser` invoke per tab and only then destroy the window. Closing a
/// native child webview while the window is servicing its own close-requested
/// event can stall that IPC round-trip, and a stalled await meant `destroy()`
/// was never reached — the window simply refused to close (the chat and
/// terminal panes, which don't close child webviews, were unaffected). Doing
/// the whole teardown Rust-side makes it atomic: once this command is
/// dispatched, the window dies even if the calling JS context never hears the
/// reply.
///
/// Order matters: child webviews are closed first (releases their
/// BrowserSessions entries), then the window. Every step is best-effort — a
/// missing webview or an already-closing window must never keep the window on
/// screen.
#[tauri::command]
pub async fn destroy_pane_window(
    app: AppHandle,
    window_label: String,
    webview_ids: Vec<String>,
) -> Result<(), String> {
    {
        let state = app.state::<BrowserSessions>();
        let mut sessions = state.0.lock().unwrap();
        for id in &webview_ids {
            sessions.remove(id);
        }
    }
    for id in &webview_ids {
        if let Some(webview) = app.get_webview(id) {
            if let Err(e) = webview.close() {
                eprintln!("[permagent-app] destroy_pane_window: close webview {id} failed: {e}");
            }
        }
    }
    if let Some(window) = app.get_window(&window_label) {
        window
            .destroy()
            .map_err(|e| format!("Destroy window failed: {e}"))?;
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
    /// The generation these refs were stamped in. An act must present it back,
    /// so a snapshot taken by another session invalidates these refs (#939).
    #[serde(default)]
    pub generation: String,
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

/// A fresh snapshot generation token. Refs are only valid within the generation
/// they were stamped in, so every snapshot mints a new one (#939).
fn new_generation() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn snapshot_js(cap: usize, generation_json: &str) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nreturn JSON.stringify(__permagentSnapshot({cap}, {generation_json}));\n}})()"
    )
}

fn act_js(
    args_json: &str,
    expected_url_json: &str,
    expected_generation_json: &str,
    next_generation_json: &str,
    cap: usize,
) -> String {
    format!(
        "(function(){{\n{GROUNDING_JS}\nif (String(window.location.href) !== {expected_url_json}) return JSON.stringify({{ok:false,error:'The page changed since the snapshot. Take a fresh snapshot before acting.'}});\nreturn JSON.stringify(__permagentAct({args_json}, {cap}, {expected_generation_json}, {next_generation_json}));\n}})()"
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
    let generation_json = serde_json::to_string(&new_generation())
        .map_err(|e| format!("Serialize generation failed: {e}"))?;
    let js = snapshot_js(MAX_SNAPSHOT_ELEMENTS, &generation_json);
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(5))
}

/// Act on a ref from `get_page_snapshot`: click / type / select (#649). Returns
/// success plus a fresh snapshot. The action set is validated here (defense in
/// depth — the daemon route validates too). The in-page scheme guard in
/// `__permagentAct` refuses non-http(s) pages.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn act_on_ref(
    app: AppHandle,
    webview_id: String,
    ref_id: u32,
    action: String,
    value: Option<String>,
    expected_url: String,
    expected_generation: Option<String>,
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
    let expected_url_json = serde_json::to_string(&expected_url)
        .map_err(|e| format!("Serialize expected URL failed: {e}"))?;
    let expected_generation_json = serde_json::to_string(&expected_generation.unwrap_or_default())
        .map_err(|e| format!("Serialize expected generation failed: {e}"))?;
    let next_generation_json = serde_json::to_string(&new_generation())
        .map_err(|e| format!("Serialize generation failed: {e}"))?;
    let js = act_js(
        &args_json,
        &expected_url_json,
        &expected_generation_json,
        &next_generation_json,
        MAX_SNAPSHOT_ELEMENTS,
    );
    eval_returning_json(&webview, &js, std::time::Duration::from_secs(8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_script_rejects_a_changed_page_before_acting() {
        let js = act_js(
            r#"{"ref":1,"action":"click"}"#,
            r#""https://example.com/form""#,
            r#""gen-a""#,
            r#""gen-b""#,
            MAX_SNAPSHOT_ELEMENTS,
        );
        let guard = js.find("window.location.href").unwrap();
        let act = js.find("__permagentAct({").unwrap();
        assert!(guard < act);
        assert!(js.contains("The page changed since the snapshot"));
    }

    /// #939: the act carries the generation its refs were stamped in, plus a
    /// fresh one for the post-action snapshot. Without this a second session
    /// snapshotting the same tab restamps every ref from 0 and the first
    /// session's "ref 3" silently resolves to a different element.
    #[test]
    fn act_script_passes_the_expected_and_next_generations() {
        let js = act_js(
            r#"{"ref":3,"action":"click"}"#,
            r#""https://example.com/form""#,
            r#""gen-expected""#,
            r#""gen-next""#,
            MAX_SNAPSHOT_ELEMENTS,
        );
        assert!(
            js.contains("gen-expected"),
            "expected generation is injected"
        );
        assert!(js.contains("gen-next"), "next generation is injected");
        // Order matters: __permagentAct(args, cap, expectedGen, nextGen).
        let call = js.find("__permagentAct({").unwrap();
        let expected_at = js[call..].find("gen-expected").unwrap();
        let next_at = js[call..].find("gen-next").unwrap();
        assert!(expected_at < next_at);
    }

    /// Every snapshot mints a distinct generation — the whole point is that a
    /// later snapshot supersedes an earlier one's refs.
    #[test]
    fn each_snapshot_generation_is_unique() {
        assert_ne!(new_generation(), new_generation());
    }

    #[test]
    fn snapshot_script_stamps_the_generation() {
        let js = snapshot_js(MAX_SNAPSHOT_ELEMENTS, r#""gen-1""#);
        assert!(js.contains("__permagentSnapshot(150, \"gen-1\")"));
    }

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

    // ── Tab identity may only come from a MAIN-FRAME source ─────────────────
    //
    // These are source guards, not behavioural tests, because the thing being
    // protected is a choice of WebKit callback: `on_navigation` wraps
    // `decidePolicyForNavigationAction` (fires for EVERY frame, and hands the
    // callback nothing but a URL), while `on_page_load` wraps
    // didCommit/didFinish (main frame only, URL read from the webview itself).
    // No test that can run in CI has a WKWebView to prove the difference, and
    // the wrong choice is invisible until a page with ad iframes relabels its
    // own tab — which is exactly what shipped: CBC embeds Google's
    // `/api2/aframe`, and the last iframe to load renamed the tab `google.com`
    // and rewrote the address bar (reported 2026-08-04).
    //
    // The needles are assembled with `concat!` so they do not appear literally
    // in this file — otherwise every one of these assertions would match its
    // own source text and pass no matter what the wiring above actually says.

    #[test]
    fn identity_events_come_from_the_main_frame_hook() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!(".on_", "page_load(")),
            "create_browser_webview must wire the main-frame page-load hook. \
             Tab identity and the address bar are fed from it; it is the only \
             navigation callback WebKit restricts to the main frame."
        );
    }

    #[test]
    fn the_all_frames_navigation_hook_is_not_wired() {
        let src = include_str!("browser.rs");
        assert!(
            !src.contains(concat!(".on_", "navigation(")),
            "This hook fires for SUBFRAMES too, so anything derived from it can \
             be an ad iframe's URL rather than the page's. It was removed on \
             purpose. If you need to intercept or block navigations, do that \
             here — but never emit tab identity from it, and update this test \
             deliberately rather than deleting it."
        );
    }

    /// The event name is a contract with Browser.tsx, which has a matching
    /// guard. Renaming one side silently stops every tab updating.
    #[test]
    fn page_load_event_name_and_shape_are_pinned() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!("\"browser_", "page_load\"")),
            "The frontend listens for this exact event name (see \
             browserRegressions.test.ts). Rename both sides together."
        );
        assert!(
            src.contains("loading: matches!("),
            "The payload carries the commit/finish distinction: the label is \
             only re-derived on a commit onto a new page, so that a real page \
             title is not clobbered when the load finishes."
        );
    }

    /// Popup links must be denied-and-rerouted, never dropped. The UI half of
    /// this contract (Browser.tsx's `browser_new_window_request` listener)
    /// outlived the original emitter once already — #240's emitter lived in
    /// the old command-center shell and died with it in #709, leaving Gmail
    /// meeting links dead for a month. This pins the re-added emitter.
    #[test]
    fn popup_links_are_denied_and_rerouted_to_the_tab_strip() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!(".on_", "new_window(")),
            "create_browser_webview must wire the new-window hook — without it \
             WKWebView silently drops every target=_blank / window.open link."
        );
        assert!(
            src.contains(concat!("\"browser_", "new_window_request\"")),
            "The frontend listens for this exact event name (Browser.tsx). \
             Rename both sides together."
        );
        assert!(
            src.contains(concat!("source_", "webview_id")),
            "Browser.tsx reads `source_webview_id` — every other payload uses \
             `webview_id`, so this mismatch would fail silently."
        );
        assert!(
            src.contains("NewWindowResponse::Deny"),
            "The native popup window must be denied; the tab strip owns the URL."
        );
        assert!(
            src.contains(concat!("emit_", "to(")),
            "The event must be scoped to the owning window — a global emit \
             opens the link once per live Browser instance (BuildView + any \
             detached pane)."
        );
    }

    /// The pull channel for same-document navigation must exist — WebKit fires
    /// no callback for `pushState`, a hash change or an SPA route.
    #[test]
    fn same_document_pull_channel_exists() {
        let src = include_str!("browser.rs");
        assert!(
            src.contains(concat!("fn browser_", "current_url")),
            "Without this the address bar goes stale on every SPA route change, \
             and the only alternative anyone reaches for is a poll."
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

/// Main-frame URL plus whether the webview can move through its own history.
#[derive(Clone, Serialize)]
pub struct BrowserNavState {
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// Read navigation state from the platform webview.
///
/// The back/forward buttons were removed in the 2026-07 wiring audit because
/// they were permanently disabled with no handler behind them — a control that
/// cannot act is worse than no control. Restoring them needs the REAL history
/// stack, and WKWebView owns it: `canGoBack`/`canGoForward` are the only honest
/// source for whether the buttons should be live. Guessing from a URL list
/// would desync the moment a page redirects or a fragment changes.
#[tauri::command]
pub fn browser_nav_state(app: AppHandle, webview_id: String) -> Result<BrowserNavState, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    let url = webview.url().map(|u| u.to_string()).unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        let flags = std::sync::Arc::new(std::sync::Mutex::new((false, false)));
        let out = flags.clone();
        let _ = webview.with_webview(move |w| {
            // Best-effort and exception-guarded, like apply_media_capture: a
            // failure here must grey the buttons out, never kill the app.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    let wk: *mut AnyObject = w.inner() as *mut _ as *mut AnyObject;
                    if wk.is_null() {
                        return;
                    }
                    let back: bool = msg_send![wk, canGoBack];
                    let fwd: bool = msg_send![wk, canGoForward];
                    if let Ok(mut g) = out.lock() {
                        *g = (back, fwd);
                    }
                }))
            }));
        });
        let (can_go_back, can_go_forward) = *flags.lock().unwrap();
        Ok(BrowserNavState {
            url,
            can_go_back,
            can_go_forward,
        })
    }

    #[cfg(not(target_os = "macos"))]
    Ok(BrowserNavState {
        url,
        can_go_back: false,
        can_go_forward: false,
    })
}

/// Step the webview's own history. `forward` selects the direction.
///
/// Uses WKWebView's goBack/goForward rather than `history.back()` in the page:
/// the JS call is same-document only and a cross-origin page can refuse it,
/// which is exactly when a user reaches for the button.
#[tauri::command]
pub fn browser_go(app: AppHandle, webview_id: String, forward: bool) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(move |w| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
                    use objc2::msg_send;
                    use objc2::runtime::AnyObject;
                    let wk: *mut AnyObject = w.inner() as *mut _ as *mut AnyObject;
                    if wk.is_null() {
                        return;
                    }
                    if forward {
                        let _: *mut AnyObject = msg_send![wk, goForward];
                    } else {
                        let _: *mut AnyObject = msg_send![wk, goBack];
                    }
                }))
            }));
        });
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let js = if forward {
            "history.forward()"
        } else {
            "history.back()"
        };
        webview.eval(js).map_err(|e| format!("eval failed: {e}"))
    }
}

/// The webview's current main-frame URL, on demand.
///
/// `browser_page_load` is the push channel and covers every real navigation,
/// but WebKit fires no navigation callback for a SAME-DOCUMENT change —
/// `history.pushState`, a hash change, an SPA route. `WKWebView.URL` tracks
/// those, so the frontend re-reads it when a signal implies the page moved
/// (a title change) or when a tab reattaches after being detached from the
/// event stream. Pull, not poll: this is called on an event, never on a timer.
#[tauri::command]
pub fn browser_current_url(app: AppHandle, webview_id: String) -> Result<String, String> {
    let webview = app
        .get_webview(&webview_id)
        .ok_or_else(|| "Webview not found".to_string())?;
    webview
        .url()
        .map(|u| u.to_string())
        .map_err(|e| format!("Read url failed: {e}"))
}
