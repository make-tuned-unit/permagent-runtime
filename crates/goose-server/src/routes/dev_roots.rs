//! Where this user keeps their code — the setup-time question.
//!
//! [`permagent::config::dev_roots`] explains why guessing this is a bug: four
//! features independently assumed `~/dev`, and on a machine whose repos live in
//! `~/Documents/dev` all four failed by finding NOTHING, which is
//! indistinguishable from a genuinely clean machine.
//!
//! The resolver alone does not close that hole. Discovery is a fallback, and a
//! fallback that runs silently is still a guess — it just guesses better. This
//! endpoint exists so onboarding can put the answer in front of the user and
//! have them confirm it, which is the only step that turns a guess into a fact.
//!
//! Read-only: the UI writes the confirmed list through the ordinary config
//! endpoint under [`permagent::config::dev_roots::DEV_ROOTS_KEY`].

use axum::{routing::get, Json, Router};
use permagent::config::dev_roots::{self, DEV_ROOTS_KEY};
use serde::Serialize;

#[derive(Serialize)]
pub struct DevRootsResponse {
    /// Directories the user has already confirmed, if any. Non-empty means
    /// onboarding is re-entry, not first run, and we must not silently
    /// overwrite an earlier answer with a fresh guess.
    confirmed: Vec<String>,
    /// Directories where a `.git` was actually found. Proposals, not answers.
    discovered: Vec<String>,
    /// Where to look if nothing was found — shown so an empty result reads as
    /// "I looked here and found none", not as a blank panel.
    home: String,
}

async fn get_dev_roots() -> Json<DevRootsResponse> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));

    let confirmed = permagent::config::Config::global()
        .get_param::<Vec<String>>(DEV_ROOTS_KEY)
        .unwrap_or_default();

    // Discovery runs even when a confirmed list exists: a user who added a
    // second checkout directory since setup should be offered it rather than
    // having to remember the exact path.
    let discovered = dev_roots::discover_dev_roots(&home)
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    Json(DevRootsResponse {
        confirmed,
        discovered,
        home: home.to_string_lossy().into_owned(),
    })
}

#[derive(serde::Deserialize)]
pub struct CheckQuery {
    path: String,
}

#[derive(Serialize)]
pub struct CheckResponse {
    /// Expanded form of what the user typed, so `~/dev` echoes back as a real
    /// path and a tilde that didn't expand is visible rather than mysterious.
    resolved: String,
    exists: bool,
    /// A `.git` was found within the probe depth. False here is not a refusal —
    /// a user may be pointing at a directory they are about to fill — but it
    /// must be SAID, because the alternative is a confirmed root that silently
    /// contributes nothing forever.
    has_repositories: bool,
}

/// Validate a hand-typed directory before it becomes a confirmed root.
///
/// `dev_roots()` filters out paths that are not directories, so a typo
/// confirmed here would be dropped at read time and every consumer would go
/// back to finding nothing — the original bug, re-entered through the fix.
async fn check_dev_root(
    axum::extract::Query(q): axum::extract::Query<CheckQuery>,
) -> Json<CheckResponse> {
    // Expand through the resolver's own helper: checking one path and storing
    // another is a silent failure wearing a confirmation's clothes.
    let path = dev_roots::expand(&q.path);
    let exists = path.is_dir();

    Json(CheckResponse {
        resolved: path.to_string_lossy().into_owned(),
        exists,
        has_repositories: exists && dev_roots::contains_repo_at(&path),
    })
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/dev-roots", get(get_dev_roots))
        .route("/api/dev-roots/check", get(check_dev_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The response must distinguish "you already told me" from "here's my
    /// guess". Collapsing them into one list is how a re-entered wizard
    /// overwrites a deliberate answer with discovery output.
    #[test]
    fn confirmed_and_discovered_are_separate_fields() {
        let json = serde_json::to_value(DevRootsResponse {
            confirmed: vec!["/Users/x/code".into()],
            discovered: vec!["/Users/x/Documents/dev".into()],
            home: "/Users/x".into(),
        })
        .unwrap();

        assert_eq!(json["confirmed"][0], "/Users/x/code");
        assert_eq!(json["discovered"][0], "/Users/x/Documents/dev");
        assert!(
            json.get("home").is_some(),
            "empty state needs somewhere to point"
        );
    }

    /// Serialising an empty discovery must produce `[]`, not `null` — the UI
    /// branches on length, and `null` would render as a crash rather than as
    /// the honest "I couldn't find any" message.
    #[test]
    fn an_empty_discovery_serialises_as_an_empty_list() {
        let json = serde_json::to_value(DevRootsResponse {
            confirmed: vec![],
            discovered: vec![],
            home: "/Users/x".into(),
        })
        .unwrap();

        assert_eq!(json["discovered"], serde_json::json!([]));
        assert_eq!(json["confirmed"], serde_json::json!([]));
    }
}
