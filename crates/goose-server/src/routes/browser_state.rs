//! Built-in browser bookmarks + saved tab sets (#790).
//!
//! Small daemon-persisted UI state for the Build-tab browser, mirroring the
//! dashboard-layout persistence pattern (JSON file in the data dir, atomic
//! tmp-write + rename, defaults on missing/malformed). Full-replace PUT keeps
//! the wire contract trivial — the frontend owns list edits (star toggle,
//! chip removal, set save/delete) and persists the whole list each time.
//!
//! Endpoints:
//!   GET /api/browser/bookmarks — persisted bookmarks (empty list on first run)
//!   PUT /api/browser/bookmarks — replace the bookmark list (validated)
//!   GET /api/browser/tab-sets  — persisted named tab sets
//!   PUT /api/browser/tab-sets  — replace the tab-set list (validated)

use axum::{routing::get, Json, Router};
use permagent::config::paths::Paths;
use serde::{Deserialize, Serialize};

use crate::routes::errors::ErrorResponse;

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabSet {
    pub name: String,
    pub tabs: Vec<SavedTab>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarksFile {
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabSetsFile {
    pub tab_sets: Vec<TabSet>,
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Bounds keep a corrupted/hostile client from growing the state file without
/// limit; they are far above any real usage.
const MAX_BOOKMARKS: usize = 500;
const MAX_TAB_SETS: usize = 100;
const MAX_TABS_PER_SET: usize = 100;
const MAX_URL_LEN: usize = 2048;
const MAX_TITLE_LEN: usize = 512;
const MAX_NAME_LEN: usize = 128;

fn validate_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("url must not be empty".to_string());
    }
    if url.len() > MAX_URL_LEN {
        return Err(format!("url exceeds {} characters", MAX_URL_LEN));
    }
    // The built-in browser only navigates web schemes; anything else persisted
    // here would be dead weight at best and a javascript:/file: foothold at
    // worst.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("url must be http(s): {}", url));
    }
    Ok(())
}

fn validate_bookmarks(bookmarks: &[Bookmark]) -> Result<(), String> {
    if bookmarks.len() > MAX_BOOKMARKS {
        return Err(format!("too many bookmarks (max {})", MAX_BOOKMARKS));
    }
    let mut seen = std::collections::HashSet::new();
    for b in bookmarks {
        validate_url(&b.url)?;
        if b.title.len() > MAX_TITLE_LEN {
            return Err(format!("title exceeds {} characters", MAX_TITLE_LEN));
        }
        if !seen.insert(b.url.as_str()) {
            return Err(format!("duplicate bookmark url: {}", b.url));
        }
    }
    Ok(())
}

fn validate_tab_sets(tab_sets: &[TabSet]) -> Result<(), String> {
    if tab_sets.len() > MAX_TAB_SETS {
        return Err(format!("too many tab sets (max {})", MAX_TAB_SETS));
    }
    let mut seen = std::collections::HashSet::new();
    for set in tab_sets {
        let name = set.name.trim();
        if name.is_empty() {
            return Err("tab set name must not be empty".to_string());
        }
        if name.len() > MAX_NAME_LEN {
            return Err(format!("tab set name exceeds {} characters", MAX_NAME_LEN));
        }
        if !seen.insert(name.to_string()) {
            return Err(format!("duplicate tab set name: {}", name));
        }
        if set.tabs.is_empty() {
            return Err(format!("tab set {:?} has no tabs", name));
        }
        if set.tabs.len() > MAX_TABS_PER_SET {
            return Err(format!(
                "tab set {:?} exceeds {} tabs",
                name, MAX_TABS_PER_SET
            ));
        }
        for tab in &set.tabs {
            validate_url(&tab.url)?;
            if tab.title.len() > MAX_TITLE_LEN {
                return Err(format!("tab title exceeds {} characters", MAX_TITLE_LEN));
            }
        }
    }
    Ok(())
}

// ── Persistence (dashboard.rs layout pattern: read-or-default, atomic write) ─

fn bookmarks_path() -> std::path::PathBuf {
    Paths::in_data_dir("browser_bookmarks.json")
}

fn tab_sets_path() -> std::path::PathBuf {
    Paths::in_data_dir("browser_tab_sets.json")
}

async fn read_state<T: Default + serde::de::DeserializeOwned>(path: &std::path::Path) -> T {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => serde_json::from_str::<T>(&contents).unwrap_or_default(),
        Err(_) => T::default(),
    }
}

async fn write_state<T: Serialize>(path: &std::path::Path, state: &T) -> Result<(), ErrorResponse> {
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let json =
        serde_json::to_string_pretty(state).map_err(|e| ErrorResponse::internal(e.to_string()))?;
    // Atomic write: write to tmp then rename.
    let tmp_path = path.with_extension("json.tmp");
    tokio::fs::write(&tmp_path, json.as_bytes())
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|e| ErrorResponse::internal(e.to_string()))?;
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn get_bookmarks() -> Json<BookmarksFile> {
    Json(read_state(&bookmarks_path()).await)
}

async fn put_bookmarks(
    Json(body): Json<BookmarksFile>,
) -> Result<Json<BookmarksFile>, ErrorResponse> {
    validate_bookmarks(&body.bookmarks).map_err(ErrorResponse::bad_request)?;
    write_state(&bookmarks_path(), &body).await?;
    Ok(Json(body))
}

async fn get_tab_sets() -> Json<TabSetsFile> {
    Json(read_state(&tab_sets_path()).await)
}

async fn put_tab_sets(Json(body): Json<TabSetsFile>) -> Result<Json<TabSetsFile>, ErrorResponse> {
    validate_tab_sets(&body.tab_sets).map_err(ErrorResponse::bad_request)?;
    write_state(&tab_sets_path(), &body).await?;
    Ok(Json(body))
}

pub fn routes() -> Router {
    Router::new()
        .route(
            "/api/browser/bookmarks",
            get(get_bookmarks).put(put_bookmarks),
        )
        .route("/api/browser/tab-sets", get(get_tab_sets).put(put_tab_sets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(url: &str) -> Bookmark {
        Bookmark {
            url: url.to_string(),
            title: "Example".to_string(),
            created_at: "2026-07-20T00:00:00Z".to_string(),
        }
    }

    fn tab_set(name: &str, urls: &[&str]) -> TabSet {
        TabSet {
            name: name.to_string(),
            tabs: urls
                .iter()
                .map(|u| SavedTab {
                    url: u.to_string(),
                    title: String::new(),
                })
                .collect(),
            created_at: "2026-07-20T00:00:00Z".to_string(),
        }
    }

    // ── Wire shape ──

    #[test]
    fn bookmarks_serialize_camel_case() {
        let file = BookmarksFile {
            bookmarks: vec![bookmark("https://example.com")],
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""createdAt""#));
        assert!(!json.contains("created_at"));
        let parsed: BookmarksFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, file);
    }

    #[test]
    fn tab_sets_serialize_camel_case() {
        let file = TabSetsFile {
            tab_sets: vec![tab_set("Research", &["https://example.com"])],
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""tabSets""#));
        assert!(!json.contains("tab_sets"));
        let parsed: TabSetsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, file);
    }

    #[test]
    fn malformed_json_reads_as_default() {
        let parsed =
            serde_json::from_str::<BookmarksFile>("{ not valid json }").unwrap_or_default();
        assert_eq!(parsed, BookmarksFile::default());
    }

    // ── Validation ──

    #[test]
    fn valid_bookmarks_pass() {
        let list = vec![bookmark("https://example.com"), bookmark("http://a.dev")];
        assert!(validate_bookmarks(&list).is_ok());
    }

    #[test]
    fn duplicate_bookmark_url_rejected() {
        let list = vec![
            bookmark("https://example.com"),
            bookmark("https://example.com"),
        ];
        let err = validate_bookmarks(&list).unwrap_err();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn non_web_scheme_rejected() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "",
            "example.com",
        ] {
            assert!(
                validate_bookmarks(&[bookmark(url)]).is_err(),
                "should reject {url:?}"
            );
        }
    }

    #[test]
    fn oversized_bookmark_fields_rejected() {
        let mut b = bookmark("https://example.com");
        b.title = "t".repeat(MAX_TITLE_LEN + 1);
        assert!(validate_bookmarks(&[b]).is_err());

        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(validate_bookmarks(&[bookmark(&long_url)]).is_err());

        let too_many: Vec<Bookmark> = (0..=MAX_BOOKMARKS)
            .map(|i| bookmark(&format!("https://example.com/{i}")))
            .collect();
        assert!(validate_bookmarks(&too_many).is_err());
    }

    #[test]
    fn valid_tab_sets_pass() {
        let sets = vec![
            tab_set("Research", &["https://example.com", "https://a.dev"]),
            tab_set("Work", &["https://b.dev"]),
        ];
        assert!(validate_tab_sets(&sets).is_ok());
    }

    #[test]
    fn empty_or_duplicate_set_name_rejected() {
        assert!(validate_tab_sets(&[tab_set("  ", &["https://a.dev"])]).is_err());
        let dup = vec![
            tab_set("Research", &["https://a.dev"]),
            tab_set("Research", &["https://b.dev"]),
        ];
        let err = validate_tab_sets(&dup).unwrap_err();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn empty_tab_set_rejected() {
        assert!(validate_tab_sets(&[tab_set("Empty", &[])]).is_err());
    }

    #[test]
    fn tab_set_with_bad_url_rejected() {
        assert!(validate_tab_sets(&[tab_set("Bad", &["javascript:alert(1)"])]).is_err());
    }

    // ── Persistence ──
    //
    // The file helpers take explicit paths, so these tests run against a
    // tempdir directly — no PERMAGENT_PATH_ROOT mutation, no #[serial], and
    // immune to any sibling test in the crate redirecting the path root
    // mid-test (the parallel-test race that flaked CI on macos-15).

    #[tokio::test]
    async fn bookmarks_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("browser_bookmarks.json");

        let file = BookmarksFile {
            bookmarks: vec![bookmark("https://example.com")],
        };
        write_state(&path, &file).await.unwrap();
        let read_back: BookmarksFile = read_state(&path).await;
        assert_eq!(read_back, file);
        // Atomic write leaves no tmp file behind.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[tokio::test]
    async fn tab_sets_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("browser_tab_sets.json");

        let file = TabSetsFile {
            tab_sets: vec![tab_set("Research", &["https://example.com"])],
        };
        write_state(&path, &file).await.unwrap();
        let read_back: TabSetsFile = read_state(&path).await;
        assert_eq!(read_back, file);
    }

    #[tokio::test]
    async fn missing_files_read_as_empty_defaults() {
        let tmp = tempfile::tempdir().unwrap();

        let bookmarks: BookmarksFile = read_state(&tmp.path().join("browser_bookmarks.json")).await;
        assert!(bookmarks.bookmarks.is_empty());
        let sets: TabSetsFile = read_state(&tmp.path().join("browser_tab_sets.json")).await;
        assert!(sets.tab_sets.is_empty());
    }

    /// The handlers' real paths resolve inside the data dir under the expected
    /// file names. Read-only (no env mutation): whatever the ambient path root
    /// is, only the leaf names are asserted.
    #[test]
    fn state_paths_use_expected_file_names() {
        assert_eq!(
            bookmarks_path().file_name().unwrap(),
            "browser_bookmarks.json"
        );
        assert_eq!(
            tab_sets_path().file_name().unwrap(),
            "browser_tab_sets.json"
        );
    }
}
