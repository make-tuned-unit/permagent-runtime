//! Per-device pairing tokens (#628) — the device registry.
//!
//! Follow-up to #625's hub-and-spoke v1, where every companion shared the ONE
//! `daemon_token`. This registry gives each paired client its own bearer token
//! so the daemon can tell devices apart (name, last-seen) and revoke a single
//! lost device without rotating the master token and re-pairing everything.
//!
//! Storage: `~/.permagent/secrets/device_tokens.json` — the same axis as
//! `daemon_token.json` (0600 secure_fs file, atomic writes), deliberately NOT
//! a Spectral table: this is credential material and belongs in `secrets/`,
//! and a JSON file avoids schema-migration-class work for a handful of rows.
//! Only SHA-256 hashes of tokens are persisted; the raw token value exists
//! exactly once — in the pairing response shown to the new device.
//!
//! Pairing flow (one-time claim codes): the hub mints a short-lived, single-use
//! claim code (`POST /api/devices/pair`, bearer-protected). The pairing URL
//! carries `#claim=<code>`; on first load the companion exchanges it
//! (`POST /pair/claim`, public — the device has no credential yet) for a fresh
//! device token. The URL therefore stops being a forever-secret: after one
//! claim (or expiry) it is inert.
//!
//! Last-seen: updated in memory on every authenticated device request, but
//! persisted at most once per `LAST_SEEN_PERSIST_INTERVAL` per device so the
//! auth hot path never turns into a write-per-request.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a minted claim code stays exchangeable.
pub const CLAIM_CODE_TTL: Duration = Duration::from_secs(10 * 60);

/// Minimum interval between persisted last-seen writes for one device.
const LAST_SEEN_PERSIST_INTERVAL: Duration = Duration::from_secs(60);

/// A persisted device record. `token_hash` is the SHA-256 (hex) of the bearer
/// token — the raw token is never stored anywhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    /// RFC 3339 creation timestamp.
    pub created: String,
    /// RFC 3339 timestamp of the most recent authenticated request, if any.
    pub last_seen: Option<String>,
    pub revoked: bool,
}

/// The wire-safe projection of a device: everything EXCEPT the token hash.
/// List endpoints must never echo credential material (not even hashes).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub created: String,
    pub last_seen: Option<String>,
    pub revoked: bool,
}

impl From<&DeviceRecord> for DeviceView {
    fn from(r: &DeviceRecord) -> Self {
        DeviceView {
            id: r.id.clone(),
            name: r.name.clone(),
            created: r.created.clone(),
            last_seen: r.last_seen.clone(),
            revoked: r.revoked,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredRegistry {
    devices: Vec<DeviceRecord>,
}

/// A minted-but-unclaimed pairing code. In-memory only: a daemon restart voids
/// pending codes, which is the safe failure mode.
struct PendingClaim {
    code: String,
    name: String,
    expires_at: Instant,
}

pub struct DeviceRegistry {
    path: PathBuf,
    /// std (not tokio) lock: every critical section is short and sync, and the
    /// verify path is called from sync contexts (WS upgrades) too.
    devices: Mutex<Vec<DeviceRecord>>,
    pending: Mutex<Vec<PendingClaim>>,
    /// Per-device throttle clock for persisted last-seen writes.
    last_persist: Mutex<HashMap<String, Instant>>,
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// N random bytes, hex-encoded (same primitive as `load_or_create_daemon_token`).
fn random_hex<const N: usize>() -> String {
    let buf: [u8; N] = rand::random();
    hex::encode(buf)
}

impl DeviceRegistry {
    /// Load the registry from `path`. Missing file → empty registry (fresh
    /// install / pre-#628 daemon). Malformed file → warn and start empty
    /// (fail-safe: an unreadable registry must never fail the daemon open or
    /// closed for the master token, which lives elsewhere).
    pub fn load(path: PathBuf) -> Self {
        let devices = match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<StoredRegistry>(&contents) {
                Ok(stored) => stored.devices,
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::auth",
                        "device_tokens.json is malformed ({e}); starting with an empty device registry"
                    );
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        tracing::info!(
            target: "permagentd::auth",
            "Device registry loaded: {} device(s) from {}",
            devices.len(),
            path.display()
        );
        Self {
            path,
            devices: Mutex::new(devices),
            pending: Mutex::new(Vec::new()),
            last_persist: Mutex::new(HashMap::new()),
        }
    }

    /// The default on-disk location, beside `daemon_token.json`.
    pub fn default_path() -> PathBuf {
        permagent::config::paths::Paths::data_dir()
            .join("secrets")
            .join("device_tokens.json")
    }

    /// Persist the given snapshot (0600, atomic). Best-effort: a failed write
    /// is logged, and in-memory state stays authoritative for this process.
    fn persist(&self, devices: &[DeviceRecord]) {
        let stored = StoredRegistry {
            devices: devices.to_vec(),
        };
        let json = match serde_json::to_string_pretty(&stored) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(target: "permagentd::auth", "serialize device registry: {e}");
                return;
            }
        };
        if let Some(parent) = self.path.parent() {
            if let Err(e) = permagent::config::secure_fs::ensure_private_dir(parent) {
                tracing::error!(target: "permagentd::auth", "create secrets dir: {e}");
                return;
            }
        }
        if let Err(e) =
            permagent::config::secure_fs::write_private_file(&self.path, json.as_bytes())
        {
            tracing::error!(
                target: "permagentd::auth",
                "write device_tokens.json: {e} — device registry change not persisted"
            );
        }
    }

    /// Mint a one-time claim code for a device named `name`. The code (not a
    /// token) rides the pairing URL; it expires after [`CLAIM_CODE_TTL`] and
    /// is consumed on first exchange.
    pub fn create_claim(&self, name: &str) -> (String, chrono::DateTime<chrono::Utc>) {
        let code = random_hex::<16>(); // 128-bit
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|c| c.expires_at > now);
        pending.push(PendingClaim {
            code: code.clone(),
            name: name.to_string(),
            expires_at: now + CLAIM_CODE_TTL,
        });
        (code, chrono::Utc::now() + CLAIM_CODE_TTL)
    }

    /// Exchange a claim code for a fresh device token. Single-use: the code is
    /// removed whether or not it turned out to be expired. Returns the ONE
    /// place the raw token ever exists.
    pub fn claim(&self, code: &str) -> Option<(String, DeviceView)> {
        let name = {
            let mut pending = self.pending.lock().unwrap();
            // Constant-time scan: compare against every pending code without
            // early exit, then act on the found index.
            let mut found: Option<usize> = None;
            for (i, c) in pending.iter().enumerate() {
                let eq: bool =
                    subtle::ConstantTimeEq::ct_eq(c.code.as_bytes(), code.as_bytes()).into();
                if eq && found.is_none() {
                    found = Some(i);
                }
            }
            let idx = found?;
            let entry = pending.remove(idx);
            if entry.expires_at <= Instant::now() {
                return None;
            }
            entry.name
        };
        Some(self.pair(&name))
    }

    /// Register a new device and mint its token directly (the claim path calls
    /// this; tests may too). Returns `(raw_token, view)` — the raw token is
    /// shown once and never stored.
    pub fn pair(&self, name: &str) -> (String, DeviceView) {
        let token = random_hex::<32>(); // same 256-bit strength as the daemon token
        let record = DeviceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            token_hash: hash_token(&token),
            created: chrono::Utc::now().to_rfc3339(),
            last_seen: None,
            revoked: false,
        };
        let view = DeviceView::from(&record);
        let mut devices = self.devices.lock().unwrap();
        devices.push(record);
        self.persist(&devices);
        (token, view)
    }

    /// All devices (revoked included — the UI shows them as revoked), newest
    /// first. Never contains token material.
    pub fn list(&self) -> Vec<DeviceView> {
        let mut views: Vec<DeviceView> = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .map(DeviceView::from)
            .collect();
        views.sort_by(|a, b| b.created.cmp(&a.created));
        views
    }

    /// Look up a device by id. Used by the voice socket to name the caller
    /// ("iPhone") after auth returns only the id.
    pub fn get(&self, id: &str) -> Option<DeviceView> {
        let devices = self.devices.lock().unwrap();
        devices.iter().find(|d| d.id == id).map(DeviceView::from)
    }

    /// Rename a device. Returns the updated view, or None if unknown.
    pub fn rename(&self, id: &str, new_name: &str) -> Option<DeviceView> {
        let mut devices = self.devices.lock().unwrap();
        let record = devices.iter_mut().find(|d| d.id == id)?;
        record.name = new_name.to_string();
        let view = DeviceView::from(&*record);
        self.persist(&devices);
        Some(view)
    }

    /// Revoke a device's token. Persisted immediately (revocation must survive
    /// a crash). Idempotent. Returns the updated view, or None if unknown.
    pub fn revoke(&self, id: &str) -> Option<DeviceView> {
        let mut devices = self.devices.lock().unwrap();
        let record = devices.iter_mut().find(|d| d.id == id)?;
        record.revoked = true;
        let view = DeviceView::from(&*record);
        self.persist(&devices);
        Some(view)
    }

    /// Check `provided` against every non-revoked device token. Constant-time:
    /// the provided value is hashed once (fixed-length digest), then compared
    /// with `subtle::ct_eq` against EVERY candidate — no early exit — so
    /// neither token bytes nor the matching position leak through timing.
    /// Returns the matching device id.
    pub fn verify(&self, provided: &str) -> Option<String> {
        let provided_hash = hash_token(provided);
        let devices = self.devices.lock().unwrap();
        let mut matched: Option<String> = None;
        for d in devices.iter() {
            let eq: bool =
                subtle::ConstantTimeEq::ct_eq(d.token_hash.as_bytes(), provided_hash.as_bytes())
                    .into();
            if eq && !d.revoked && matched.is_none() {
                matched = Some(d.id.clone());
            }
        }
        matched
    }

    /// Record an authenticated request from device `id`. In-memory always;
    /// persisted at most once per [`LAST_SEEN_PERSIST_INTERVAL`] per device so
    /// the auth hot path stays cheap.
    pub fn touch(&self, id: &str) {
        let now_ts = chrono::Utc::now().to_rfc3339();
        let should_persist = {
            let mut clock = self.last_persist.lock().unwrap();
            match clock.get(id) {
                Some(last) if last.elapsed() < LAST_SEEN_PERSIST_INTERVAL => false,
                _ => {
                    clock.insert(id.to_string(), Instant::now());
                    true
                }
            }
        };
        let mut devices = self.devices.lock().unwrap();
        if let Some(record) = devices.iter_mut().find(|d| d.id == id) {
            record.last_seen = Some(now_ts);
            if should_persist {
                self.persist(&devices);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry() -> (tempfile::TempDir, DeviceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device_tokens.json");
        (dir, DeviceRegistry::load(path))
    }

    #[test]
    fn pair_mints_token_and_stores_only_the_hash() {
        let (_dir, reg) = temp_registry();
        let (token, view) = reg.pair("iPhone");
        assert_eq!(token.len(), 64, "256-bit hex token");
        assert_eq!(view.name, "iPhone");
        assert!(!view.revoked);

        // The raw token must not appear anywhere on disk — only its hash.
        let on_disk = std::fs::read_to_string(&reg.path).unwrap();
        assert!(
            !on_disk.contains(&token),
            "raw token must never be persisted"
        );
        assert!(on_disk.contains(&hash_token(&token)));
    }

    #[test]
    fn verify_accepts_valid_rejects_unknown_and_revoked() {
        let (_dir, reg) = temp_registry();
        let (token, view) = reg.pair("iPad");
        assert_eq!(reg.verify(&token), Some(view.id.clone()));
        assert_eq!(reg.verify("not-a-token"), None);

        reg.revoke(&view.id).expect("device exists");
        assert_eq!(reg.verify(&token), None, "revoked token must not verify");
    }

    #[test]
    fn get_returns_the_paired_name() {
        let (_dir, reg) = temp_registry();
        let (_token, view) = reg.pair("iPhone");
        assert_eq!(
            reg.get(&view.id).map(|v| v.name),
            Some("iPhone".to_string())
        );
        assert!(reg.get("no-such-device").is_none());
    }

    #[test]
    fn list_never_exposes_token_material() {
        let (_dir, reg) = temp_registry();
        let (token, _) = reg.pair("Laptop");
        let listed = serde_json::to_string(&reg.list()).unwrap();
        assert!(!listed.contains(&token));
        assert!(!listed.contains(&hash_token(&token)));
        assert!(!listed.contains("token_hash"));
    }

    #[test]
    fn rename_updates_and_persists() {
        let (_dir, reg) = temp_registry();
        let (_, view) = reg.pair("Old Name");
        let renamed = reg.rename(&view.id, "New Name").unwrap();
        assert_eq!(renamed.name, "New Name");
        assert!(reg.rename("ghost", "x").is_none());

        // Reload from disk: rename survived.
        let reloaded = DeviceRegistry::load(reg.path.clone());
        assert_eq!(reloaded.list()[0].name, "New Name");
    }

    #[test]
    fn revocation_survives_reload() {
        let (_dir, reg) = temp_registry();
        let (token, view) = reg.pair("Lost Phone");
        reg.revoke(&view.id).unwrap();

        let reloaded = DeviceRegistry::load(reg.path.clone());
        assert_eq!(reloaded.verify(&token), None);
        assert!(reloaded.list()[0].revoked);
    }

    #[test]
    fn claim_code_is_single_use() {
        let (_dir, reg) = temp_registry();
        let (code, _expires) = reg.create_claim("Tablet");
        let (token, view) = reg.claim(&code).expect("first claim succeeds");
        assert_eq!(view.name, "Tablet");
        assert_eq!(reg.verify(&token), Some(view.id));
        assert!(reg.claim(&code).is_none(), "second claim must fail");
    }

    #[test]
    fn unknown_claim_code_is_rejected() {
        let (_dir, reg) = temp_registry();
        reg.create_claim("Real");
        assert!(reg.claim("0000000000000000").is_none());
    }

    #[test]
    fn touch_updates_last_seen_in_memory() {
        let (_dir, reg) = temp_registry();
        let (_, view) = reg.pair("Watch");
        assert!(reg.list()[0].last_seen.is_none());
        reg.touch(&view.id);
        assert!(reg.list()[0].last_seen.is_some());
    }

    #[test]
    fn touch_throttles_persisted_writes() {
        let (_dir, reg) = temp_registry();
        let (_, view) = reg.pair("Phone");
        reg.touch(&view.id); // first touch: persisted (no prior clock entry)
        let first = std::fs::read_to_string(&reg.path).unwrap();
        assert!(first.contains("last_seen"));
        let seen_after_first: StoredRegistry = serde_json::from_str(&first).unwrap();
        let persisted_first = seen_after_first.devices[0].last_seen.clone();

        // Immediate second touch: in-memory updates, disk does NOT.
        std::thread::sleep(std::time::Duration::from_millis(5));
        reg.touch(&view.id);
        let second = std::fs::read_to_string(&reg.path).unwrap();
        let seen_after_second: StoredRegistry = serde_json::from_str(&second).unwrap();
        assert_eq!(
            seen_after_second.devices[0].last_seen, persisted_first,
            "second touch within the throttle window must not rewrite the file"
        );
        assert_ne!(
            reg.list()[0].last_seen,
            persisted_first,
            "in-memory moved on"
        );
    }

    #[test]
    fn malformed_file_yields_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device_tokens.json");
        std::fs::write(&path, "{ not json }").unwrap();
        let reg = DeviceRegistry::load(path);
        assert!(reg.list().is_empty());
    }

    #[test]
    fn missing_file_yields_empty_registry() {
        let (_dir, reg) = temp_registry();
        assert!(reg.list().is_empty());
    }
}
