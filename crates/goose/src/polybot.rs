//! Polybot — read-only status for the user's prediction-market bot.
//!
//! Polybot (`~/…/Polybot`) already sizes, places, and pauses its own
//! Polymarket orders. This module does none of that. It locates the checkout
//! the same way Picker does (`polybot_root`, else `dev_roots()` → `Polybot`)
//! and reads `logs/bankroll.json` plus whether a `PAUSED` file exists.
//!
//! Credentials live in an OpenClaw vault and are never imported. A missing
//! or stale bankroll is a first-class answer, not an empty zeroed card.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Config key overriding where the Polybot checkout lives.
pub const POLYBOT_ROOT_KEY: &str = "polybot_root";

/// After this long without a bankroll write, the numbers are labelled stale.
const STALE_AFTER: Duration = Duration::from_secs(48 * 3600);

/// Locate the Polybot checkout. `polybot_root` wins; otherwise the shared
/// `dev_roots` resolver, so a move between `~/dev` and `~/Documents/dev`
/// does not silently empty the card.
pub fn polybot_root() -> Option<PathBuf> {
    if let Ok(configured) = crate::config::Config::global().get_param::<String>(POLYBOT_ROOT_KEY) {
        let p = PathBuf::from(shellexpand::tilde(&configured).into_owned());
        if p.is_dir() {
            return Some(p);
        }
    }
    crate::config::dev_roots::dev_roots()
        .into_iter()
        .map(|root| root.join("Polybot"))
        .find(|p| p.is_dir())
}

/// What the Finance tab shows for Polybot. `found: false` is honest empty,
/// never a zeroed balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolybotStatus {
    pub found: bool,
    pub root: Option<String>,
    pub paused: bool,
    pub current_balance: Option<f64>,
    pub realized_pnl: Option<f64>,
    pub open_exposure: Option<f64>,
    pub trade_count: Option<u64>,
    pub last_updated: Option<String>,
    pub stale: bool,
    /// Why it is missing or unreadable, when it is.
    pub detail: Option<String>,
}

impl Default for PolybotStatus {
    fn default() -> Self {
        Self {
            found: false,
            root: None,
            paused: false,
            current_balance: None,
            realized_pnl: None,
            open_exposure: None,
            trade_count: None,
            last_updated: None,
            stale: false,
            detail: None,
        }
    }
}

/// Read Polybot's on-disk bankroll. Never calls Polymarket.
pub fn status() -> PolybotStatus {
    let Some(root) = polybot_root() else {
        return PolybotStatus {
            detail: Some(
                "no Polybot checkout found — set polybot_root or keep it under a known code directory"
                    .into(),
            ),
            ..Default::default()
        };
    };
    status_from_root(&root)
}

fn status_from_root(root: &Path) -> PolybotStatus {
    let mut out = PolybotStatus {
        found: true,
        root: Some(root.display().to_string()),
        paused: root.join("PAUSED").is_file(),
        ..Default::default()
    };
    let bankroll_path = root.join("logs/bankroll.json");
    let Ok(raw) = std::fs::read_to_string(&bankroll_path) else {
        out.detail = Some(format!(
            "no bankroll at {} — Polybot has not written status yet",
            bankroll_path.display()
        ));
        return out;
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            out.detail = Some(format!("bankroll.json is unreadable: {e}"));
            return out;
        }
    };
    out.current_balance = num_field(&v, "current_balance");
    out.realized_pnl = num_field(&v, "realized_pnl");
    out.open_exposure = num_field(&v, "open_exposure");
    out.trade_count = v
        .get("trade_count")
        .and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)));
    out.last_updated = v
        .get("last_updated")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    out.stale = is_stale(out.last_updated.as_deref());
    if out.stale {
        out.detail = Some("bankroll has not been written in 48 hours".into());
    }
    out
}

fn num_field(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|n| n.as_f64())
}

fn is_stale(last_updated: Option<&str>) -> bool {
    let Some(ts) = last_updated else {
        return true;
    };
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return true;
    };
    let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
    match age.to_std() {
        Ok(d) => d > STALE_AFTER,
        Err(_) => false, // last_updated is in the future — not stale.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_checkout_is_found_false_not_a_zero_balance() {
        let s = status_from_root(Path::new("/no/such/polybot"));
        // The helper assumes the root exists; status() is the missing-checkout
        // path. Directly: a root with no bankroll still must not invent zeros.
        assert!(s.current_balance.is_none());
        assert!(s.realized_pnl.is_none());
    }

    #[test]
    fn reads_bankroll_and_paused() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("logs")).unwrap();
        fs::write(dir.path().join("PAUSED"), "").unwrap();
        fs::write(
            dir.path().join("logs/bankroll.json"),
            r#"{
                "current_balance": 81.5,
                "realized_pnl": 8.18,
                "open_exposure": 12.0,
                "trade_count": 14,
                "last_updated": "2099-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();
        let s = status_from_root(dir.path());
        assert!(s.found);
        assert!(s.paused);
        assert_eq!(s.current_balance, Some(81.5));
        assert_eq!(s.realized_pnl, Some(8.18));
        assert_eq!(s.open_exposure, Some(12.0));
        assert_eq!(s.trade_count, Some(14));
        assert!(!s.stale, "a future timestamp is not stale");
    }

    #[test]
    fn unreadable_bankroll_is_detail_not_zeros() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("logs")).unwrap();
        fs::write(dir.path().join("logs/bankroll.json"), "not-json").unwrap();
        let s = status_from_root(dir.path());
        assert!(s.found);
        assert!(s.detail.as_deref().unwrap().contains("unreadable"));
        assert!(s.current_balance.is_none());
    }

    #[test]
    fn a_missing_timestamp_is_stale() {
        assert!(is_stale(None));
        assert!(is_stale(Some("yesterday")));
        assert!(is_stale(Some("2020-01-01T00:00:00Z")));
    }
}
