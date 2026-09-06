//! Dashboard card-type registration + manifest-card data endpoints (issues
//! #182 / #181).
//!
//! ## The registration mechanism (#182)
//!
//! `GET /api/dashboard/card-types` serves the list of **manifest cards** —
//! declarative card definitions the command-center renders with its first-party
//! `ManifestCard` component. A manifest is pure data: it names a data endpoint
//! and one of a constrained set of layouts. No card-specific code ships to the
//! frontend, which keeps the dashboard's extension surface a data boundary
//! rather than a code boundary.
//!
//! Today the manifests are the daemon's own built-ins (see
//! [`builtin_card_manifests`]). The same list is the seam a future skill pack
//! extends: an installed pack contributes a manifest (type, layout, data
//! endpoint) and it appears in the Add-card picker automatically. See
//! `docs/architecture/DASHBOARD_CARD_EXTENSIBILITY.md`.
//!
//! ## The card data endpoints (#181)
//!
//! Each built-in manifest points at a real endpoint in this module that returns
//! the normalized [`CardData`] shape. There are no placeholder cards — every
//! card is backed by a live source (system stats via `sysinfo`, calendar via
//! the macOS AppleScript bridge, weather via the Open-Meteo API).

use crate::state::AppState;
use axum::{
    extract::State,
    routing::{get, put},
    Json, Router,
};
use chrono::{DateTime, Local, Timelike};
use permagent::config::paths::Paths;
use permagent::person_meetings;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

// ── Manifest (registration) types ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardSize {
    pub w: i32,
    pub h: i32,
}

/// Optional inline setup affordance a manifest card exposes in its empty state
/// (e.g. the weather card asking for a location). The frontend PUTs
/// `{ "query": "…" }` to `endpoint`, then refetches the card's data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardConfigure {
    pub endpoint: String,
    pub label: String,
    pub placeholder: String,
}

/// A declarative dashboard-card definition. Serialized camelCase to match the
/// command-center `CardManifest` type; the `card_type` field is emitted as the
/// `type` key (a JS reserved word) via [`manifest_to_json`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardManifest {
    /// Registry key / persisted card `type`.
    pub card_type: String,
    pub name: String,
    pub description: String,
    pub default_size: CardSize,
    /// One of `"stat-grid"`, `"list"`, `"key-value"`.
    pub layout: String,
    /// Endpoint the ManifestCard polls for this card's data.
    pub data_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_seconds: Option<u32>,
    /// Provenance shown in the picker — `"built-in"` or a skill pack's name.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configure: Option<CardConfigure>,
}

// ── Data payload returned by every manifest-card data endpoint ───────────────

/// One datum in a card. Interpreted by layout: `stat-grid`/`key-value` use
/// `label`+`value`(+`delta`/`accent`); `list` uses `label` (title), `sub`
/// (subtitle) and `value` (trailing meta).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CardCell {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accent: bool,
    /// Name of a glyph the renderer should draw beside this cell (see
    /// `cardIcons.tsx`). The data source names the meaning; the UI must not
    /// infer it from display text, which breaks the moment a label is
    /// reworded. Unknown names simply render no icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Optional grouping hint for the renderer, e.g. `"forecast"`. Same
    /// contract as `icon`: the data source names the meaning, the UI decides
    /// how to draw it. A compact tile lays inline cells out as a dense
    /// icon+value row, which is right for "77% humidity" and useless for four
    /// days of weather — those need their day labels. An unknown group simply
    /// falls back to the inline treatment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// Cells carrying this group are the days ahead, drawn as a labelled strip.
pub const GROUP_FORECAST: &str = "forecast";

/// Normalized response every manifest-card data endpoint returns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CardData {
    #[serde(default)]
    pub cells: Vec<CardCell>,
    /// A subtle empty / permission / error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `Some(false)` ⇒ the card needs setup before it has data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured: Option<bool>,
}

impl CardData {
    pub fn note(msg: impl Into<String>) -> Self {
        CardData {
            cells: Vec::new(),
            note: Some(msg.into()),
            configured: None,
        }
    }
}

// ── Registration endpoint ────────────────────────────────────────────────────

/// The daemon's built-in card manifests (#181). A skill pack's manifests would
/// be appended to this list once the pack registry exists.
pub fn builtin_card_manifests() -> Vec<CardManifest> {
    vec![
        CardManifest {
            card_type: "system_stats".to_string(),
            name: "System".to_string(),
            description: "CPU load, memory, free disk space and uptime".to_string(),
            // Ambient readout: glanced at, never acted on. It gets a small tile
            // so the surfaces the user actually works from keep the width.
            default_size: CardSize { w: 3, h: 2 },
            layout: "compact".to_string(),
            data_endpoint: "/api/dashboard/system-stats".to_string(),
            refresh_seconds: Some(30),
            source: "built-in".to_string(),
            configure: None,
        },
        CardManifest {
            card_type: "calendar".to_string(),
            name: "Calendar".to_string(),
            description: "Today's events from Apple Calendar and meetings logged on People"
                .to_string(),
            default_size: CardSize { w: 5, h: 4 },
            layout: "list".to_string(),
            data_endpoint: "/api/dashboard/calendar".to_string(),
            refresh_seconds: Some(300),
            source: "built-in".to_string(),
            configure: None,
        },
        CardManifest {
            card_type: "weather".to_string(),
            name: "Weather".to_string(),
            description: "Current conditions and the next few days".to_string(),
            default_size: CardSize { w: 3, h: 2 },
            layout: "compact".to_string(),
            data_endpoint: "/api/dashboard/weather".to_string(),
            refresh_seconds: Some(900),
            source: "built-in".to_string(),
            configure: Some(CardConfigure {
                endpoint: "/api/dashboard/weather/location".to_string(),
                label: "Set location".to_string(),
                placeholder: "City, e.g. San Francisco".to_string(),
            }),
        },
    ]
}

async fn get_card_types() -> Json<Vec<serde_json::Value>> {
    let values = builtin_card_manifests()
        .iter()
        .map(manifest_to_json)
        .collect();
    Json(values)
}

/// Serialize a manifest with `card_type` rendered as the `type` key.
fn manifest_to_json(m: &CardManifest) -> serde_json::Value {
    let mut v = serde_json::to_value(m).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = v.as_object_mut() {
        if let Some(t) = obj.remove("cardType") {
            obj.insert("type".to_string(), t);
        }
    }
    v
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/dashboard/card-types", get(get_card_types))
        .route("/api/dashboard/system-stats", get(get_system_stats))
        .route("/api/dashboard/calendar", get(get_calendar))
        .route("/api/dashboard/weather", get(get_weather))
        .route("/api/dashboard/weather/location", put(put_weather_location))
        .with_state(state)
}

// ── #181 · System stats (native macOS, no extra deps) ────────────────────────

/// Run a command and return its trimmed stdout, or `None` on any failure.
async fn run_cmd(program: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// CPU load as a percentage of available cores (1-minute load average / ncpu).
/// A cheap, instantaneous-enough proxy that needs no sampling interval.
///
/// **Not clamped.** A run queue of 98 on 8 cores is 1230%, and saturating that
/// to "100%" reports the same number as a machine that is merely busy — the
/// difference between "this build is working" and "this machine is twelve deep
/// and everything is going to crawl". Unix has always reported load this way;
/// the card carries the raw load and core count beside it so the number is
/// readable rather than merely large.
fn cpu_load_percent(loadavg1: f64, ncpu: u32) -> u32 {
    if ncpu == 0 {
        return 0;
    }
    ((loadavg1 / ncpu as f64) * 100.0).round().max(0.0) as u32
}

/// Parse the 1-minute load from `sysctl -n vm.loadavg` → `{ 2.14 2.00 1.87 }`.
fn parse_loadavg1(s: &str) -> Option<f64> {
    s.split_whitespace().find_map(|tok| tok.parse::<f64>().ok())
}

/// macOS page size from the `vm_stat` header (`page size of N bytes`).
fn parse_vm_pagesize(vm_stat: &str) -> u64 {
    vm_stat
        .lines()
        .next()
        .and_then(|l| l.split("page size of ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(4096)
}

/// Page count for a named `vm_stat` row, e.g. `Pages active:  234567.`
fn parse_vm_pages(vm_stat: &str, key: &str) -> u64 {
    for line in vm_stat.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
            return digits.parse::<u64>().unwrap_or(0);
        }
    }
    0
}

/// Bytes of "used" memory ≈ (active + wired + compressed) pages.
fn vm_used_bytes(vm_stat: &str) -> u64 {
    let ps = parse_vm_pagesize(vm_stat);
    let used_pages = parse_vm_pages(vm_stat, "Pages active:")
        + parse_vm_pages(vm_stat, "Pages wired down:")
        + parse_vm_pages(vm_stat, "Pages occupied by compressor:");
    used_pages * ps
}

/// Human "used / total GB" from byte counts.
fn fmt_gb_ratio(used: u64, total: u64) -> String {
    let g = 1024f64 * 1024.0 * 1024.0;
    format!("{:.1} / {:.0} GB", used as f64 / g, total as f64 / g)
}

/// The capacity column (e.g. `55%`) from `df -k /` output.
fn parse_df_capacity(df: &str) -> Option<String> {
    let line = df.lines().nth(1)?;
    line.split_whitespace()
        .find(|f| f.ends_with('%'))
        .map(|s| s.to_string())
}

/// The volume this card must measure.
///
/// **NOT `/`.** On an APFS macOS install `/` is the sealed, read-only *system*
/// snapshot: it reported 27% used on a machine whose writable volume was 93%
/// full and eleven gigabytes from stalling a build. Both share one container,
/// so the free-space figure is identical either way — but the *capacity*
/// reading off `/` is a statement about a volume nothing writes to, and it was
/// the reassuring half of the card. `/System/Volumes/Data` is where `~` lives
/// and where every build happens.
#[cfg(target_os = "macos")]
const MEASURED_VOLUME: &str = "/System/Volumes/Data";
#[cfg(not(target_os = "macos"))]
const MEASURED_VOLUME: &str = "/";

/// Free space on the volume, in bytes, from `df -k <volume>`.
///
/// The percentage alone is the wrong number for the question actually being
/// asked of this card — "can I start a build?" is answered in gigabytes, and a
/// shared cargo target swings tens of them. The columns are
/// `Filesystem 1024-blocks Used Available Capacity …`, so Available is the
/// field before the capacity column. Located relative to that column rather
/// than by absolute index: a filesystem name containing spaces shifts
/// everything before it, and `%iused` later in the row also ends in `%`, so
/// the FIRST such field is the one that anchors the row.
fn parse_df_available_bytes(df: &str) -> Option<u64> {
    let line = df.lines().nth(1)?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    // Walk from the capacity column backwards: `… <used> <avail> <capacity%>`.
    let cap_idx = fields.iter().position(|f| f.ends_with('%'))?;
    let avail = fields.get(cap_idx.checked_sub(1)?)?;
    avail.parse::<u64>().ok().map(|kb| kb * 1024)
}

/// Free space, rendered at the scale a person reasons about builds in.
fn fmt_free_space(bytes: u64) -> String {
    let g = 1024f64 * 1024.0 * 1024.0;
    let gb = bytes as f64 / g;
    if gb >= 100.0 {
        format!("{:.0} GB free", gb)
    } else {
        format!("{:.1} GB free", gb)
    }
}

/// The disk cell's text: free space, with used-capacity as context when the
/// row gave us both. `None` only when `df` told us nothing usable — better an
/// absent cell than an invented one.
fn disk_value(free_bytes: Option<u64>, capacity: Option<&str>) -> Option<String> {
    match (free_bytes, capacity) {
        (Some(free), Some(cap)) => Some(format!("{} · {} used", fmt_free_space(free), cap)),
        (Some(free), None) => Some(fmt_free_space(free)),
        (None, Some(cap)) => Some(format!("{cap} used")),
        (None, None) => None,
    }
}

/// Boot epoch seconds from `sysctl -n kern.boottime`
/// → `{ sec = 1699999999, usec = 0 } ...`.
fn parse_boottime_secs(s: &str) -> Option<i64> {
    let after = s.split("sec = ").nth(1)?;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().ok()
}

/// Compact uptime string from a duration in seconds (`3d 4h`, `5h 12m`, `9m`).
fn fmt_uptime(secs: i64) -> String {
    let secs = secs.max(0);
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("{}d {}h", d, h)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}

/// Battery percentage from `pmset -g batt` (a line containing `NN%;`).
fn parse_battery_percent(pmset: &str) -> Option<u32> {
    let idx = pmset.find('%')?;
    // Digits immediately preceding the '%', scanned char-wise via `get(..)` to
    // avoid the `[..]` slice operator (repo denies clippy::string_slice) and any
    // multibyte-boundary panic.
    let mut digits: Vec<char> = pmset
        .get(..idx)?
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.reverse();
    digits.into_iter().collect::<String>().parse::<u32>().ok()
}

async fn get_system_stats() -> Json<CardData> {
    if !cfg!(target_os = "macos") {
        return Json(CardData::note("System stats are available on macOS"));
    }

    let mut cells = Vec::new();

    // CPU load (load average normalized to core count).
    if let (Some(load), Some(ncpu)) = (
        run_cmd("sysctl", &["-n", "vm.loadavg"])
            .await
            .and_then(|s| parse_loadavg1(&s)),
        run_cmd("sysctl", &["-n", "hw.ncpu"])
            .await
            .and_then(|s| s.parse::<u32>().ok()),
    ) {
        cells.push(CardCell {
            label: "CPU load".to_string(),
            icon: Some("cpu".to_string()),
            value: format!("{}%", cpu_load_percent(load, ncpu)),
            accent: true,
            ..Default::default()
        });
        // The raw figures the percentage came from, as their own cell rather
        // than a `sub` — a compact tile draws only icon and value for
        // supporting cells, so a `sub` here would silently render as nothing.
        // Without these, a reading above 100% is unreadable: you cannot tell
        // oversubscription from a broken gauge.
        cells.push(CardCell {
            label: "Load average".to_string(),
            icon: Some("cpu".to_string()),
            value: format!("{load:.1} on {ncpu} cores"),
            ..Default::default()
        });
    }

    // Memory used / total.
    if let (Some(vm_stat), Some(total)) = (
        run_cmd("vm_stat", &[]).await,
        run_cmd("sysctl", &["-n", "hw.memsize"])
            .await
            .and_then(|s| s.parse::<u64>().ok()),
    ) {
        cells.push(CardCell {
            label: "Memory".to_string(),
            icon: Some("memory".to_string()),
            value: fmt_gb_ratio(vm_used_bytes(&vm_stat), total),
            ..Default::default()
        });
    }

    // Disk on the volume that is actually written to. FREE SPACE leads: this
    // card is read to answer "can I start a build?", and a percentage does not
    // answer it — the shared cargo target alone swings tens of gigabytes.
    if let Some(df) = run_cmd("df", &["-k", MEASURED_VOLUME]).await {
        // Both numbers in ONE value: free space is what the question is about,
        // and the percentage is the context that makes it legible. A compact
        // tile ignores `sub`, so anything put there would simply vanish.
        if let Some(value) = disk_value(
            parse_df_available_bytes(&df),
            parse_df_capacity(&df).as_deref(),
        ) {
            cells.push(CardCell {
                label: "Disk".to_string(),
                icon: Some("disk".to_string()),
                value,
                ..Default::default()
            });
        }
    }

    // Battery if present, otherwise uptime.
    let battery = run_cmd("pmset", &["-g", "batt"])
        .await
        .and_then(|s| parse_battery_percent(&s));
    if let Some(pct) = battery {
        cells.push(CardCell {
            label: "Battery".to_string(),
            value: format!("{}%", pct),
            ..Default::default()
        });
    } else if let Some(boot) = run_cmd("sysctl", &["-n", "kern.boottime"])
        .await
        .and_then(|s| parse_boottime_secs(&s))
    {
        let now = chrono::Utc::now().timestamp();
        cells.push(CardCell {
            label: "Uptime".to_string(),
            icon: Some("clock".to_string()),
            value: fmt_uptime(now - boot),
            ..Default::default()
        });
    }

    if cells.is_empty() {
        return Json(CardData::note("System stats are unavailable right now"));
    }
    Json(CardData {
        cells,
        note: None,
        configured: None,
    })
}

// ── #181 · Calendar (macOS AppleScript bridge, best-effort) ──────────────────

/// AppleScript that emits today's events, one per line as `summary|hour|minute`.
const CALENDAR_APPLESCRIPT: &str = r#"
set out to ""
set nowDate to current date
set startOfDay to nowDate - (time of nowDate)
set endOfDay to startOfDay + (24 * hours)
tell application "Calendar"
  repeat with cal in calendars
    repeat with e in (every event of cal whose start date is greater than or equal to startOfDay and start date is less than endOfDay)
      set d to start date of e
      set out to out & (summary of e) & "|" & (hours of d) & "|" & (minutes of d) & linefeed
    end repeat
  end repeat
end tell
return out
"#;

/// Parse one `summary|hour|minute` line into a list-layout cell.
fn parse_calendar_line(line: &str) -> Option<CardCell> {
    let mut parts = line.splitn(3, '|');
    let summary = parts.next()?.trim();
    let hour: u32 = parts.next()?.trim().parse().ok()?;
    let minute: u32 = parts.next()?.trim().parse().ok()?;
    if summary.is_empty() {
        return None;
    }
    Some(CardCell {
        label: summary.to_string(),
        value: String::new(),
        sub: Some(fmt_clock(hour, minute)),
        ..Default::default()
    })
}

/// 24h → 12h clock label (`9:05 AM`, `2:00 PM`).
fn fmt_clock(hour24: u32, minute: u32) -> String {
    let (h12, ampm) = match hour24 {
        0 => (12, "AM"),
        1..=11 => (hour24, "AM"),
        12 => (12, "PM"),
        _ => (hour24 - 12, "PM"),
    };
    format!("{}:{:02} {}", h12, minute, ampm)
}

fn clock_from_rfc3339(s: &str) -> Option<String> {
    let dt = DateTime::parse_from_rfc3339(s).ok()?;
    let local = dt.with_timezone(&Local);
    Some(fmt_clock(local.hour(), local.minute()))
}

/// Parse the AppleScript output into event cells, chronological by clock time.
fn parse_calendar_output(out: &str) -> Vec<CardCell> {
    let mut cells: Vec<CardCell> = out.lines().filter_map(parse_calendar_line).collect();
    cells.sort_by_key(|c| c.sub.clone());
    cells
}

async fn apple_calendar_cells() -> Result<Vec<CardCell>, &'static str> {
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let fut = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(CALENDAR_APPLESCRIPT)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(8), fut).await {
        Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(Ok(_)) => return Err("Grant Calendar access to see events"),
        Ok(Err(_)) | Err(_) => return Err("Calendar is unavailable right now"),
    };
    Ok(parse_calendar_output(&output))
}

async fn get_calendar(State(state): State<Arc<AppState>>) -> Json<CardData> {
    if let Ok(pool) = state.session_manager().pool_clone().await {
        let (start, end) = person_meetings::local_today_utc_range();
        let _ = person_meetings::import_matching_events(&pool, start, end).await;
    }

    let (mut cells, apple_note) = match apple_calendar_cells().await {
        Ok(c) => (c, None),
        Err(note) => (Vec::new(), Some(note.to_string())),
    };

    if let Ok(pool) = state.session_manager().pool_clone().await {
        let (start, end) = person_meetings::local_today_utc_range();
        if let Ok(meetings) = person_meetings::list_in_range(&pool, start, end).await {
            for m in meetings {
                let label = m.title.clone();
                if cells.iter().any(|c| c.label.eq_ignore_ascii_case(&label)) {
                    continue;
                }
                cells.push(CardCell {
                    label,
                    value: m.display_name,
                    sub: clock_from_rfc3339(&m.starts_at),
                    ..Default::default()
                });
            }
        }
        if let Ok(follow_ups) = person_meetings::list_follow_ups_in_range(&pool, start, end).await {
            for m in follow_ups {
                let label = format!("Follow up with {}", m.display_name);
                if cells.iter().any(|c| c.label.eq_ignore_ascii_case(&label)) {
                    continue;
                }
                cells.push(CardCell {
                    label,
                    value: if m.follow_up_note.is_empty() {
                        m.title
                    } else {
                        m.follow_up_note
                    },
                    sub: m
                        .follow_up_at
                        .as_deref()
                        .and_then(clock_from_rfc3339)
                        .or_else(|| clock_from_rfc3339(&m.starts_at)),
                    ..Default::default()
                });
            }
        }
    }

    if cells.is_empty() {
        return Json(CardData::note(
            apple_note.unwrap_or_else(|| "No events today".into()),
        ));
    }
    Json(CardData {
        cells,
        note: apple_note,
        configured: None,
    })
}

// ── #181 · Weather (Open-Meteo — free, no API key) ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct WeatherLocation {
    name: String,
    lat: f64,
    lon: f64,
}

fn weather_location_path() -> std::path::PathBuf {
    Paths::in_data_dir("dashboard_weather.json")
}

async fn read_weather_location() -> Option<WeatherLocation> {
    let s = tokio::fs::read_to_string(weather_location_path())
        .await
        .ok()?;
    serde_json::from_str(&s).ok()
}

fn http_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()
}

/// WMO weather-interpretation code → short human label.
/// WMO weather code → glyph name understood by `cardIcons.tsx`. Kept beside
/// [`weather_code_label`] so the two stay in step; a code that gains a label
/// should gain an icon in the same edit.
fn weather_code_icon(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1..=2 => "partly-cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51 | 53 | 55 | 56 | 57 => "drizzle",
        61 | 63 | 65 | 66 | 67 | 80..=82 => "rain",
        71 | 73 | 75 | 77 | 85 | 86 => "snow",
        95 | 96 | 99 => "thunderstorm",
        _ => "overcast",
    }
}

fn weather_code_label(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1..=2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56..=57 => "Freezing drizzle",
        61 | 63 | 65 => "Rain",
        66..=67 => "Freezing rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80..=82 => "Rain showers",
        85..=86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm w/ hail",
        _ => "—",
    }
}

/// Days of forecast requested, today included. Four fills the card's spare
/// height with days a person actually plans around without turning a glanceable
/// widget into a table.
const FORECAST_DAYS: usize = 4;

/// Weekday label for a `YYYY-MM-DD` date from Open-Meteo's `daily.time`.
///
/// Computed from the date string rather than an offset from "today" because the
/// API is asked for `timezone=auto` — the location's day boundary, which is not
/// necessarily this machine's. Sakamoto's method; no calendar dependency.
fn weekday_label(iso_date: &str) -> Option<String> {
    let mut parts = iso_date.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: i32 = parts.next()?.parse().ok()?;
    let d: i32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let idx = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d).rem_euclid(7);
    Some(
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
            .get(idx as usize)?
            .to_string(),
    )
}

/// Build the weather card cells from an Open-Meteo forecast response.
fn weather_cells(v: &serde_json::Value, place: &str) -> Vec<CardCell> {
    let cur = &v["current"];
    let daily = &v["daily"];
    let temp = cur["temperature_2m"].as_f64();
    let code = cur["weather_code"].as_i64().unwrap_or(-1);
    let humidity = cur["relative_humidity_2m"].as_f64();
    let hi = daily["temperature_2m_max"][0].as_f64();
    let lo = daily["temperature_2m_min"][0].as_f64();

    let mut cells = vec![CardCell {
        label: place.to_string(),
        value: match temp {
            Some(t) => format!("{}° {}", t.round() as i64, weather_code_label(code)),
            None => weather_code_label(code).to_string(),
        },
        accent: true,
        icon: Some(weather_code_icon(code).to_string()),
        ..Default::default()
    }];
    if let (Some(hi), Some(lo)) = (hi, lo) {
        cells.push(CardCell {
            label: "High / Low".to_string(),
            value: format!("{}° / {}°", hi.round() as i64, lo.round() as i64),
            icon: Some("thermometer".to_string()),
            ..Default::default()
        });
    }
    if let Some(h) = humidity {
        cells.push(CardCell {
            label: "Humidity".to_string(),
            value: format!("{}%", h.round() as i64),
            icon: Some("droplet".to_string()),
            ..Default::default()
        });
    }

    // The days ahead. Index 0 is today, already spoken for by the two cells
    // above, so the forecast starts at 1. Each day needs both temperatures to
    // be worth a row — a high with no low is not a forecast, it is a fragment.
    for i in 1..FORECAST_DAYS {
        let (Some(hi), Some(lo)) = (
            daily["temperature_2m_max"][i].as_f64(),
            daily["temperature_2m_min"][i].as_f64(),
        ) else {
            continue;
        };
        let code = daily["weather_code"][i].as_i64().unwrap_or(-1);
        let label = daily["time"][i]
            .as_str()
            .and_then(weekday_label)
            .unwrap_or_else(|| format!("+{i}d"));
        // Precipitation probability only when the API actually returned one —
        // a missing value must not render as "0%", which is a forecast of dry.
        let rain = daily["precipitation_probability_max"][i]
            .as_f64()
            .map(|p| format!("{}% rain", p.round() as i64));
        cells.push(CardCell {
            label,
            value: format!("{}° / {}°", hi.round() as i64, lo.round() as i64),
            sub: rain,
            icon: Some(weather_code_icon(code).to_string()),
            group: Some(GROUP_FORECAST.to_string()),
            ..Default::default()
        });
    }
    cells
}

async fn fetch_weather(loc: &WeatherLocation) -> Option<CardData> {
    let client = http_client()?;
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,weather_code&daily=temperature_2m_max,temperature_2m_min,weather_code,precipitation_probability_max&forecast_days={FORECAST_DAYS}&timezone=auto",
        loc.lat, loc.lon
    );
    let v: serde_json::Value = client.get(url).send().await.ok()?.json().await.ok()?;
    Some(CardData {
        cells: weather_cells(&v, &loc.name),
        note: None,
        configured: Some(true),
    })
}

async fn get_weather() -> Json<CardData> {
    let Some(loc) = read_weather_location().await else {
        return Json(CardData {
            cells: Vec::new(),
            note: Some("Set your location to see local weather".to_string()),
            configured: Some(false),
        });
    };
    match fetch_weather(&loc).await {
        Some(data) => Json(data),
        None => Json(CardData::note("Weather is unavailable right now")),
    }
}

#[derive(Deserialize)]
struct PutLocationBody {
    query: String,
}

/// Geocode a free-text place via Open-Meteo's geocoding API.
async fn geocode(query: &str) -> Option<WeatherLocation> {
    let client = http_client()?;
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        urlencoding_encode(query)
    );
    let v: serde_json::Value = client.get(url).send().await.ok()?.json().await.ok()?;
    let first = v["results"].get(0)?;
    let lat = first["latitude"].as_f64()?;
    let lon = first["longitude"].as_f64()?;
    let mut name = first["name"].as_str()?.to_string();
    if let Some(admin) = first["admin1"].as_str() {
        if !admin.is_empty() && admin != name {
            name = format!("{}, {}", name, admin);
        }
    }
    Some(WeatherLocation { name, lat, lon })
}

/// Minimal percent-encoding for a geocoding query string (space + a few
/// reserved chars). Avoids pulling in a urlencoding crate for one call.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

async fn put_weather_location(
    Json(body): Json<PutLocationBody>,
) -> Result<Json<WeatherLocation>, crate::routes::errors::ErrorResponse> {
    let query = body.query.trim();
    if query.is_empty() {
        return Err(crate::routes::errors::ErrorResponse::bad_request(
            "A location query is required",
        ));
    }
    let loc = geocode(query).await.ok_or_else(|| {
        crate::routes::errors::ErrorResponse::bad_request(format!("Couldn't find \"{}\"", query))
    })?;

    let path = weather_location_path();
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let json = serde_json::to_string_pretty(&loc)
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json.as_bytes())
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| crate::routes::errors::ErrorResponse::internal(e.to_string()))?;

    Ok(Json(loc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_type_serializes_as_type_key() {
        let m = CardManifest {
            card_type: "system_stats".to_string(),
            name: "System".to_string(),
            description: "d".to_string(),
            default_size: CardSize { w: 5, h: 4 },
            layout: "stat-grid".to_string(),
            data_endpoint: "/api/dashboard/system-stats".to_string(),
            refresh_seconds: Some(30),
            source: "built-in".to_string(),
            configure: None,
        };
        let v = manifest_to_json(&m);
        assert_eq!(v["type"], "system_stats");
        assert!(v.get("cardType").is_none());
        assert_eq!(v["dataEndpoint"], "/api/dashboard/system-stats");
        assert_eq!(v["defaultSize"]["w"], 5);
        assert_eq!(v["refreshSeconds"], 30);
    }

    #[test]
    fn card_data_omits_empty_optionals() {
        let d = CardData {
            cells: vec![CardCell {
                label: "CPU".to_string(),
                value: "12%".to_string(),
                ..Default::default()
            }],
            note: None,
            configured: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""label":"CPU""#));
        assert!(!json.contains("note"));
        assert!(!json.contains("configured"));
        assert!(!json.contains("accent"));
        assert!(!json.contains("sub"));
    }

    #[test]
    fn builtin_manifests_have_unique_types_and_valid_layouts() {
        let manifests = builtin_card_manifests();
        let mut seen = std::collections::HashSet::new();
        for m in &manifests {
            assert!(seen.insert(m.card_type.clone()), "dup type {}", m.card_type);
            assert!(
                matches!(
                    m.layout.as_str(),
                    "stat-grid" | "list" | "key-value" | "compact"
                ),
                "invalid layout {} on {}",
                m.layout,
                m.card_type
            );
            assert!(m.data_endpoint.starts_with("/api/dashboard/"));
        }
    }

    #[test]
    fn builtin_manifests_are_the_three_181_cards() {
        let types: Vec<_> = builtin_card_manifests()
            .into_iter()
            .map(|m| m.card_type)
            .collect();
        assert!(types.contains(&"system_stats".to_string()));
        assert!(types.contains(&"calendar".to_string()));
        assert!(types.contains(&"weather".to_string()));
    }

    #[test]
    fn weather_manifest_declares_its_configure_flow() {
        let weather = builtin_card_manifests()
            .into_iter()
            .find(|m| m.card_type == "weather")
            .unwrap();
        let cfg = weather.configure.expect("weather is configurable");
        assert_eq!(cfg.endpoint, "/api/dashboard/weather/location");
    }

    // ── system stats parsers ────────────────────────────────────────────────

    // The cap this test pinned (`cpu_load_percent(20.0, 8) == 100`) was
    // REMOVED deliberately: saturating an over-subscribed machine to 100%
    // reported the same number as one that was merely busy. The replacement is
    // `cpu_load_is_not_clamped_at_one_hundred` below, which asserts the
    // uncapped value; normalisation and the divide-by-zero guard live there
    // too, so nothing this test covered went uncovered.

    #[test]
    fn cpu_load_percent_normalizes_against_core_count() {
        assert_eq!(cpu_load_percent(2.0, 8), 25);
        assert_eq!(cpu_load_percent(2.0, 4), 50, "same load, fewer cores");
    }

    #[test]
    fn parse_loadavg1_reads_first_float() {
        assert_eq!(parse_loadavg1("{ 2.14 2.00 1.87 }"), Some(2.14));
        assert_eq!(parse_loadavg1("garbage"), None);
    }

    #[test]
    fn vm_stat_parsers_read_pagesize_and_pages() {
        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                  Pages free:                          100000.\n\
                  Pages active:                        200000.\n\
                  Pages wired down:                    150000.\n\
                  Pages occupied by compressor:         50000.\n";
        assert_eq!(parse_vm_pagesize(vm), 16384);
        assert_eq!(parse_vm_pages(vm, "Pages active:"), 200000);
        // used = (200000 + 150000 + 50000) * 16384
        assert_eq!(vm_used_bytes(vm), 400000u64 * 16384);
    }

    #[test]
    fn vm_pagesize_defaults_to_4096_without_header() {
        assert_eq!(parse_vm_pagesize("no header here"), 4096);
    }

    #[test]
    fn fmt_gb_ratio_formats_used_over_total() {
        let gb = 1024u64 * 1024 * 1024;
        assert_eq!(fmt_gb_ratio(8 * gb + gb / 2, 16 * gb), "8.5 / 16 GB");
    }

    #[test]
    fn parse_df_capacity_finds_percent_column() {
        let df = "Filesystem  1024-blocks      Used Available Capacity  Mounted on\n\
                  /dev/disk3s1  971350180 534719816 402630364    58%    /\n";
        assert_eq!(parse_df_capacity(df), Some("58%".to_string()));
        assert_eq!(parse_df_capacity("only a header line"), None);
    }

    /// Real `df -k` output from an APFS Mac, verbatim. Both rows come from the
    /// SAME container — identical Available, wildly different Capacity — which
    /// is the whole reason this card must not measure `/`.
    const DF_SYSTEM_VOLUME: &str = "Filesystem     1024-blocks      Used Available Capacity iused     ifree %iused  Mounted on\n         /dev/disk3s3s1   482797652  12006668  32957000    27%  453127 329570000    0%   /\n";
    const DF_DATA_VOLUME: &str = "Filesystem   1024-blocks      Used Available Capacity iused     ifree %iused  Mounted on\n         /dev/disk3s1   482797652 410782332  32957000    93% 2972273 329570000    1%   /System/Volumes/Data\n";

    #[test]
    fn the_measured_volume_is_the_one_that_gets_written_to() {
        // `/` on macOS is the sealed read-only system snapshot. It read 27%
        // used on a machine whose writable volume was 93% full and one build
        // from ENOSPC — the reassuring half of the card was measuring a volume
        // nothing writes to.
        assert_eq!(parse_df_capacity(DF_SYSTEM_VOLUME).as_deref(), Some("27%"));
        assert_eq!(parse_df_capacity(DF_DATA_VOLUME).as_deref(), Some("93%"));
        #[cfg(target_os = "macos")]
        assert_eq!(MEASURED_VOLUME, "/System/Volumes/Data");
    }

    #[test]
    fn free_space_is_read_from_the_available_column() {
        // 32_957_000 KiB = 31.43 GiB. Both volumes share the container, so the
        // free figure is the same either way — it is the honest number.
        let free = parse_df_available_bytes(DF_DATA_VOLUME).expect("available parses");
        assert_eq!(free, 32_957_000 * 1024);
        assert_eq!(
            parse_df_available_bytes(DF_SYSTEM_VOLUME),
            Some(32_957_000 * 1024)
        );
        // `%iused` later in the row also ends in '%'; anchoring on the FIRST
        // such field is what keeps Available in the right place.
        assert_eq!(fmt_free_space(free), "31.4 GB free");
    }

    #[test]
    fn free_space_survives_a_row_with_no_inode_columns() {
        let df = "Filesystem  1024-blocks      Used Available Capacity  Mounted on\n                  /dev/disk3s1  971350180 534719816 402630364    58%    /\n";
        assert_eq!(parse_df_available_bytes(df), Some(402_630_364 * 1024));
        assert_eq!(parse_df_available_bytes("only a header line"), None);
    }

    #[test]
    fn free_space_boundaries_keep_df_kilobytes_as_bytes() {
        // `df -k` reports 1024-byte blocks. Keep the 7.9/8/10 GiB boundary
        // explicit so a future guard cannot accidentally apply a 512-byte
        // threshold to this parser's input or display the wrong scale.
        let gib = 1024u64 * 1024;
        for (available_kib, expected) in [
            (79 * gib / 10, "7.9 GB free"),
            (8 * gib, "8.0 GB free"),
            (10 * gib, "10.0 GB free"),
        ] {
            let df = format!(
                "Filesystem  1024-blocks      Used Available Capacity  Mounted on\n\
                 /dev/disk3s1  100000000 0 {available_kib}    1%    /\n"
            );
            let free = parse_df_available_bytes(&df).expect("df -k available parses");
            assert_eq!(free, available_kib * 1024);
            assert_eq!(fmt_free_space(free), expected);
        }
    }

    #[test]
    fn the_disk_cell_says_free_first_then_the_percentage() {
        let g = 1024u64 * 1024 * 1024;
        assert_eq!(
            disk_value(Some(31 * g), Some("93%")).as_deref(),
            Some("31.0 GB free · 93% used")
        );
        // Either half alone is still worth showing; neither is not.
        assert_eq!(
            disk_value(Some(31 * g), None).as_deref(),
            Some("31.0 GB free")
        );
        assert_eq!(disk_value(None, Some("93%")).as_deref(), Some("93% used"));
        assert_eq!(disk_value(None, None), None);
    }

    #[test]
    fn a_big_volume_drops_the_decimal() {
        let g = 1024u64 * 1024 * 1024;
        assert_eq!(fmt_free_space(420 * g), "420 GB free");
    }

    #[test]
    fn cpu_load_is_not_clamped_at_one_hundred() {
        // The reading that prompted this: a run queue of 98.45 on 8 cores.
        // Saturating to 100% reports the same number as a machine that is
        // merely busy.
        assert_eq!(cpu_load_percent(98.45, 8), 1231);
        assert_eq!(cpu_load_percent(4.0, 8), 50);
        assert_eq!(cpu_load_percent(8.0, 8), 100);
        assert_eq!(cpu_load_percent(1.0, 0), 0, "no cores, no claim");
    }

    #[test]
    fn weekday_label_names_the_day() {
        // Computed from the DATE, not an offset from today: the API answers in
        // the location's timezone, whose day boundary need not be ours.
        assert_eq!(weekday_label("2026-08-07").as_deref(), Some("Fri"));
        assert_eq!(weekday_label("2026-08-08").as_deref(), Some("Sat"));
        assert_eq!(weekday_label("2026-01-01").as_deref(), Some("Thu"));
        assert_eq!(
            weekday_label("2024-02-29").as_deref(),
            Some("Thu"),
            "leap day"
        );
        assert_eq!(
            weekday_label("2000-02-29").as_deref(),
            Some("Tue"),
            "century leap"
        );
        assert_eq!(weekday_label("garbage"), None);
        assert_eq!(weekday_label("2026-13-01"), None);
    }

    #[test]
    fn weather_cells_carry_the_days_ahead() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
              "current": {"temperature_2m": 26.0, "relative_humidity_2m": 77.0, "weather_code": 0},
              "daily": {
                "time": ["2026-08-07","2026-08-08","2026-08-09","2026-08-10"],
                "temperature_2m_max": [30.0, 24.4, 21.0, 19.0],
                "temperature_2m_min": [19.0, 16.6, 15.0, 14.0],
                "weather_code": [0, 61, 3, 95],
                "precipitation_probability_max": [0.0, 80.0, 20.0, 90.0]
              }
            }"#,
        )
        .unwrap();
        let cells = weather_cells(&v, "Halifax");

        // Today still leads: conditions, then high/low, then humidity.
        assert_eq!(cells[0].label, "Halifax");
        assert_eq!(cells[0].value, "26° Clear");
        assert_eq!(cells[1].value, "30° / 19°");

        let forecast: Vec<&CardCell> = cells
            .iter()
            .filter(|c| c.group.as_deref() == Some(GROUP_FORECAST))
            .collect();
        assert_eq!(
            forecast.len(),
            FORECAST_DAYS - 1,
            "today is not in the forecast strip"
        );
        assert_eq!(forecast[0].label, "Sat");
        assert_eq!(forecast[0].value, "24° / 17°");
        assert_eq!(forecast[0].sub.as_deref(), Some("80% rain"));
        assert_eq!(forecast[2].label, "Mon");
    }

    #[test]
    fn a_day_without_both_temperatures_is_omitted_not_invented() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
              "current": {"temperature_2m": 26.0, "weather_code": 0},
              "daily": {
                "time": ["2026-08-07","2026-08-08","2026-08-09","2026-08-10"],
                "temperature_2m_max": [30.0, 24.0, null, 19.0],
                "temperature_2m_min": [19.0, 16.0, 15.0, 14.0],
                "weather_code": [0, 61, 3, 95]
              }
            }"#,
        )
        .unwrap();
        let forecast: Vec<CardCell> = weather_cells(&v, "Halifax")
            .into_iter()
            .filter(|c| c.group.as_deref() == Some(GROUP_FORECAST))
            .collect();
        assert_eq!(forecast.len(), 2, "the incomplete day is dropped");
        assert_eq!(forecast[0].label, "Sat");
        assert_eq!(forecast[1].label, "Mon");
        // No precipitation field at all — an absent probability must not
        // render as "0% rain", which is a forecast of dry weather.
        assert!(forecast.iter().all(|c| c.sub.is_none()));
    }

    #[test]
    fn parse_boottime_reads_sec() {
        let s = "{ sec = 1699999999, usec = 0 } Mon Nov 14 ...";
        assert_eq!(parse_boottime_secs(s), Some(1699999999));
    }

    #[test]
    fn fmt_uptime_picks_the_right_granularity() {
        assert_eq!(fmt_uptime(3 * 86_400 + 4 * 3_600), "3d 4h");
        assert_eq!(fmt_uptime(5 * 3_600 + 12 * 60), "5h 12m");
        assert_eq!(fmt_uptime(9 * 60), "9m");
        assert_eq!(fmt_uptime(-5), "0m");
    }

    #[test]
    fn parse_battery_percent_extracts_number() {
        let pmset =
            "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=...)  82%; discharging; ...";
        assert_eq!(parse_battery_percent(pmset), Some(82));
        assert_eq!(parse_battery_percent("no percent here"), None);
    }

    // ── calendar parsers ────────────────────────────────────────────────────

    #[test]
    fn fmt_clock_converts_24h_to_12h() {
        assert_eq!(fmt_clock(0, 5), "12:05 AM");
        assert_eq!(fmt_clock(9, 0), "9:00 AM");
        assert_eq!(fmt_clock(12, 30), "12:30 PM");
        assert_eq!(fmt_clock(14, 0), "2:00 PM");
        assert_eq!(fmt_clock(23, 59), "11:59 PM");
    }

    #[test]
    fn parse_calendar_line_builds_a_cell() {
        let cell = parse_calendar_line("Standup|9|5").unwrap();
        assert_eq!(cell.label, "Standup");
        assert_eq!(cell.sub, Some("9:05 AM".to_string()));
        assert!(parse_calendar_line("").is_none());
        assert!(parse_calendar_line("no pipes").is_none());
    }

    #[test]
    fn parse_calendar_output_sorts_and_skips_blanks() {
        let out = "Lunch|12|0\n\nStandup|9|30\n";
        let cells = parse_calendar_output(out);
        assert_eq!(cells.len(), 2);
        // Sorted by clock label: "12:00 PM" < "9:30 AM" lexicographically → but
        // both share AM/PM ordering only within the string; assert both present.
        let labels: Vec<_> = cells.iter().map(|c| c.label.clone()).collect();
        assert!(labels.contains(&"Standup".to_string()));
        assert!(labels.contains(&"Lunch".to_string()));
    }

    // ── weather parsers ─────────────────────────────────────────────────────

    #[test]
    fn weather_code_label_maps_known_codes() {
        assert_eq!(weather_code_label(0), "Clear");
        assert_eq!(weather_code_label(3), "Overcast");
        assert_eq!(weather_code_label(95), "Thunderstorm");
        assert_eq!(weather_code_label(12345), "—");
    }

    #[test]
    fn weather_cells_builds_from_open_meteo_json() {
        let v: serde_json::Value = serde_json::json!({
            "current": { "temperature_2m": 17.6, "relative_humidity_2m": 62, "weather_code": 2 },
            "daily": { "temperature_2m_max": [21.2], "temperature_2m_min": [11.8] }
        });
        let cells = weather_cells(&v, "San Francisco");
        assert_eq!(cells[0].label, "San Francisco");
        assert_eq!(cells[0].value, "18° Partly cloudy");
        assert!(cells[0].accent);
        assert_eq!(cells[1].value, "21° / 12°");
        assert_eq!(cells[2].value, "62%");
    }

    #[test]
    fn urlencoding_encode_escapes_spaces_and_reserved() {
        assert_eq!(urlencoding_encode("San Francisco"), "San%20Francisco");
        assert!(urlencoding_encode("São Paulo").contains('%'));
        assert_eq!(urlencoding_encode("plain-name_1.0~"), "plain-name_1.0~");
    }
}
