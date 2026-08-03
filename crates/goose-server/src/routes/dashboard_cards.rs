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
    routing::{get, put},
    Json, Router,
};
use permagent::config::paths::Paths;
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
}

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
            description: "CPU, memory, disk and uptime at a glance".to_string(),
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
            description: "Today's events from your Mac's Calendar".to_string(),
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
            description: "Current conditions for your location".to_string(),
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
fn cpu_load_percent(loadavg1: f64, ncpu: u32) -> u32 {
    if ncpu == 0 {
        return 0;
    }
    ((loadavg1 / ncpu as f64) * 100.0).round().min(100.0) as u32
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
            value: format!("{}%", cpu_load_percent(load, ncpu)),
            accent: true,
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
            value: fmt_gb_ratio(vm_used_bytes(&vm_stat), total),
            ..Default::default()
        });
    }

    // Disk capacity on the primary volume.
    if let Some(cap) = run_cmd("df", &["-k", "/"])
        .await
        .and_then(|s| parse_df_capacity(&s))
    {
        cells.push(CardCell {
            label: "Disk".to_string(),
            value: format!("{} used", cap),
            ..Default::default()
        });
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

/// Parse the AppleScript output into event cells, chronological by clock time.
fn parse_calendar_output(out: &str) -> Vec<CardCell> {
    let mut cells: Vec<CardCell> = out.lines().filter_map(parse_calendar_line).collect();
    cells.sort_by_key(|c| c.sub.clone());
    cells
}

async fn get_calendar() -> Json<CardData> {
    if !cfg!(target_os = "macos") {
        return Json(CardData::note("Calendar is available on macOS"));
    }
    let fut = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(CALENDAR_APPLESCRIPT)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(8), fut).await {
        Ok(Ok(o)) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        // Non-zero exit is almost always "Calendar access not granted".
        Ok(Ok(_)) => return Json(CardData::note("Grant Calendar access to see events")),
        Ok(Err(_)) | Err(_) => return Json(CardData::note("Calendar is unavailable right now")),
    };
    let cells = parse_calendar_output(&output);
    if cells.is_empty() {
        return Json(CardData::note("No events today"));
    }
    Json(CardData {
        cells,
        note: None,
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
        ..Default::default()
    }];
    if let (Some(hi), Some(lo)) = (hi, lo) {
        cells.push(CardCell {
            label: "High / Low".to_string(),
            value: format!("{}° / {}°", hi.round() as i64, lo.round() as i64),
            ..Default::default()
        });
    }
    if let Some(h) = humidity {
        cells.push(CardCell {
            label: "Humidity".to_string(),
            value: format!("{}%", h.round() as i64),
            ..Default::default()
        });
    }
    cells
}

async fn fetch_weather(loc: &WeatherLocation) -> Option<CardData> {
    let client = http_client()?;
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,weather_code&daily=temperature_2m_max,temperature_2m_min&forecast_days=1&timezone=auto",
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
                matches!(m.layout.as_str(), "stat-grid" | "list" | "key-value"),
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

    #[test]
    fn cpu_load_percent_normalizes_and_caps() {
        assert_eq!(cpu_load_percent(2.0, 8), 25);
        assert_eq!(cpu_load_percent(8.0, 8), 100);
        assert_eq!(cpu_load_percent(20.0, 8), 100); // capped
        assert_eq!(cpu_load_percent(1.0, 0), 0); // guard div-by-zero
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
