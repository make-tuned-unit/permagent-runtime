//! Federation identity layer — Slice 1 of the federation build (#784).
//!
//! Implements §3.1–§3.4 of `docs/design/federation-security-spec.md`: the
//! per-hub Ed25519 identity + X25519 encryption keypairs, the `author_id`
//! wire format that binds to Spectral's OR-Set, the enc-key-cert binding the
//! encryption key to the identity, the TOFU-pinned peer registry, and
//! Signal-style safety numbers for out-of-band verification.
//!
//! Layering (§3.4): `middleware/auth.rs` (bearer `daemon_token`) answers
//! "may this client drive this hub" — device pairing, untouched by this
//! module. This module answers the orthogonal "who authored this shared
//! object / whose key may a realm key be wrapped to". Two independent
//! questions, two mechanisms.
//!
//! Key-at-rest posture (§5.7): private keys live in the secret store
//! (`Config` → OS keyring, with the existing file fallback), never a plain
//! flat file. Unlike the `daemon_token` precedent, a present-but-unreadable
//! identity is a HARD error, never silently regenerated: a fresh identity is
//! a fresh `author_id`, which orphans every object the old identity authored
//! and drops all realm memberships (RT-8 / OD-6).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use permagent::config::{Config, ConfigError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

/// The 32-byte author id Spectral stores as opaque bytes (`Option<[u8; 32]>`).
pub type AuthorId = [u8; 32];

/// Secret-store key holding [`StoredIdentity`].
const FEDERATION_IDENTITY_SECRET_KEY: &str = "federation_identity";

/// Domain-separation context for the enc-key-cert signature. Distinct from
/// every other Ed25519 message this identity will ever sign (pack manifests,
/// admin-chain links), so signatures can never be confused across purposes.
const ENC_KEY_CERT_CONTEXT: &[u8] = b"permagent.federation.enc-key-cert.v1";

/// Domain-separation context for safety numbers.
const SAFETY_NUMBER_CONTEXT: &[u8] = b"permagent.federation.safety-number.v1";

/// Validity window issued for a fresh enc-key-cert (§3.1: medium-lived).
const ENC_KEY_CERT_VALIDITY_SECS: i64 = 365 * 24 * 60 * 60;

/// Re-issue the cert on load once it is within this window of expiry, so a
/// regularly-started daemon never presents an expired cert.
const ENC_KEY_CERT_RENEWAL_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;

const STORED_IDENTITY_VERSION: u32 = 1;
const PEER_REGISTRY_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error(
        "federation identity exists but cannot be read: {0} — refusing to regenerate; a new \
         identity would orphan every authored object and drop realm membership (RT-8)"
    )]
    IdentityUnreadable(String),
    #[error("failed to persist federation identity: {0}")]
    IdentityStore(String),
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("enc-key-cert signature invalid")]
    CertSignatureInvalid,
    #[error("enc-key-cert not valid at {now}: window [{not_before}, {not_after}]")]
    CertOutsideValidity {
        now: i64,
        not_before: i64,
        not_after: i64,
    },
    #[error(
        "peer registry at {0} exists but cannot be read: {1} — refusing to start from an empty \
         registry; pinned trust state must never be silently discarded"
    )]
    RegistryUnreadable(String, String),
    #[error("failed to persist peer registry: {0}")]
    RegistryStore(String),
    #[error(
        "pin conflict for peer {0}: presented keys do not match the pinned identity (possible \
         MITM or rollback) — refusing to overwrite the pin"
    )]
    PinConflict(String),
    #[error("peer {0} is not pinned")]
    NotPinned(String),
}

// ---------------------------------------------------------------------------
// author_id — the G2 seam
// ---------------------------------------------------------------------------

/// The single point where an identity key becomes a Spectral-facing
/// `author_id`: the RAW Ed25519 public key bytes (RFC 8032 encoding,
/// `VerifyingKey::to_bytes`). Ratified with Spectral 2026-07-19 (§3.2) — the
/// author bytes ARE the signature-verification key, so the §4 authorship
/// check is byte equality. If the wire format ever changes, this function is
/// the one line that changes.
pub fn author_id_from_verifying_key(key: &VerifyingKey) -> AuthorId {
    key.to_bytes()
}

/// Human-facing rendering (`"ed25519:" || base32`). Display only — never a
/// wire format (§3.2).
pub fn display_author_id(id: &AuthorId) -> String {
    format!("ed25519:{}", data_encoding::BASE32_NOPAD.encode(id))
}

// ---------------------------------------------------------------------------
// enc-key-cert (§3.1)
// ---------------------------------------------------------------------------

/// `Ed25519_sign(id_sk, x25519_pub || not_before || not_after)` — binds a
/// medium-lived X25519 encryption key to the long-lived Ed25519 identity, so
/// the encryption key can rotate without re-establishing identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncKeyCert {
    /// Hex-encoded 32-byte X25519 public key.
    pub x25519_pub: String,
    /// Unix seconds, inclusive.
    pub not_before: i64,
    /// Unix seconds, inclusive.
    pub not_after: i64,
    /// Hex-encoded 64-byte Ed25519 signature.
    pub sig: String,
}

fn enc_key_cert_message(x25519_pub: &[u8; 32], not_before: i64, not_after: i64) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ENC_KEY_CERT_CONTEXT.len() + x25519_pub.len() + 8 + 8);
    msg.extend_from_slice(ENC_KEY_CERT_CONTEXT);
    msg.extend_from_slice(x25519_pub);
    msg.extend_from_slice(&not_before.to_be_bytes());
    msg.extend_from_slice(&not_after.to_be_bytes());
    msg
}

/// Issue a cert over `x25519_pub` with the given validity window, signed by
/// the Ed25519 identity key.
pub fn issue_enc_key_cert(
    identity: &SigningKey,
    x25519_pub: &[u8; 32],
    not_before: i64,
    not_after: i64,
) -> EncKeyCert {
    let sig = identity.sign(&enc_key_cert_message(x25519_pub, not_before, not_after));
    EncKeyCert {
        x25519_pub: hex::encode(x25519_pub),
        not_before,
        not_after,
        sig: hex::encode(sig.to_bytes()),
    }
}

impl EncKeyCert {
    pub fn x25519_pub_bytes(&self) -> Result<[u8; 32], AuthError> {
        parse_key32(&self.x25519_pub, "enc-key-cert x25519_pub")
    }

    /// Verify the signature against `identity` and check `now` falls inside
    /// the validity window. Signature is checked first: a cert that was never
    /// validly signed is [`AuthError::CertSignatureInvalid`] regardless of
    /// its claimed window.
    pub fn verify(&self, identity: &VerifyingKey, now: i64) -> Result<(), AuthError> {
        let x25519_pub = self.x25519_pub_bytes()?;
        let sig_bytes: [u8; 64] = hex::decode(&self.sig)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or(AuthError::CertSignatureInvalid)?;
        let sig = Signature::from_bytes(&sig_bytes);
        identity
            .verify(
                &enc_key_cert_message(&x25519_pub, self.not_before, self.not_after),
                &sig,
            )
            .map_err(|_| AuthError::CertSignatureInvalid)?;
        if now < self.not_before || now > self.not_after {
            return Err(AuthError::CertOutsideValidity {
                now,
                not_before: self.not_before,
                not_after: self.not_after,
            });
        }
        Ok(())
    }
}

fn parse_key32(hex_str: &str, what: &str) -> Result<[u8; 32], AuthError> {
    hex::decode(hex_str)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| AuthError::InvalidKey(format!("{what}: not 32 hex-encoded bytes")))
}

// ---------------------------------------------------------------------------
// The hub's own identity (§3.1)
// ---------------------------------------------------------------------------

/// Secret-store representation. Only the two 32-byte secrets and the cert
/// window are persisted — public keys and the cert signature re-derive
/// (Ed25519 signing is deterministic, RFC 8032).
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    /// Hex-encoded 32-byte Ed25519 secret key.
    ed25519_sk: String,
    /// Hex-encoded 32-byte X25519 static secret.
    x25519_sk: String,
    cert_not_before: i64,
    cert_not_after: i64,
    created_at: i64,
}

/// The hub's federation identity: long-lived Ed25519 signing identity +
/// medium-lived X25519 encryption key, certified by the former.
///
/// Deliberately does not implement `Debug`/`Serialize` — secret material
/// must not leak through logging or accidental serialization.
pub struct FederationIdentity {
    signing: SigningKey,
    encryption: StaticSecret,
    enc_key_cert: EncKeyCert,
}

impl FederationIdentity {
    /// Load the identity from the secret store, creating and persisting a
    /// fresh one only if none exists (`ConfigError::NotFound`). Any other
    /// read failure is a hard error — see module docs.
    pub fn load_or_create(config: &Config) -> Result<Self, AuthError> {
        Self::load_or_create_at(config, chrono::Utc::now().timestamp())
    }

    /// Clock-injected variant of [`Self::load_or_create`] for tests.
    pub fn load_or_create_at(config: &Config, now: i64) -> Result<Self, AuthError> {
        match config.get_secret::<StoredIdentity>(FEDERATION_IDENTITY_SECRET_KEY) {
            Ok(stored) => Self::from_stored(config, stored, now),
            Err(ConfigError::NotFound(_)) => Self::generate(config, now),
            Err(e) => Err(AuthError::IdentityUnreadable(e.to_string())),
        }
    }

    fn generate(config: &Config, now: i64) -> Result<Self, AuthError> {
        let ed25519_sk: [u8; 32] = rand::random();
        let x25519_sk: [u8; 32] = rand::random();
        let stored = StoredIdentity {
            version: STORED_IDENTITY_VERSION,
            ed25519_sk: hex::encode(ed25519_sk),
            x25519_sk: hex::encode(x25519_sk),
            cert_not_before: now,
            cert_not_after: now + ENC_KEY_CERT_VALIDITY_SECS,
            created_at: now,
        };
        config
            .set_secret(FEDERATION_IDENTITY_SECRET_KEY, &stored)
            .map_err(|e| AuthError::IdentityStore(e.to_string()))?;
        tracing::info!(
            target: "permagentd::auth",
            "Generated new federation identity {}",
            display_author_id(&author_id_from_verifying_key(
                &SigningKey::from_bytes(&ed25519_sk).verifying_key()
            ))
        );
        Self::from_stored(config, stored, now)
    }

    fn from_stored(config: &Config, stored: StoredIdentity, now: i64) -> Result<Self, AuthError> {
        if stored.version != STORED_IDENTITY_VERSION {
            return Err(AuthError::IdentityUnreadable(format!(
                "stored identity version {} is not supported by this build (expected {})",
                stored.version, STORED_IDENTITY_VERSION
            )));
        }
        let ed_sk = parse_key32(&stored.ed25519_sk, "stored ed25519_sk")
            .map_err(|e| AuthError::IdentityUnreadable(e.to_string()))?;
        let x_sk = parse_key32(&stored.x25519_sk, "stored x25519_sk")
            .map_err(|e| AuthError::IdentityUnreadable(e.to_string()))?;
        let signing = SigningKey::from_bytes(&ed_sk);
        let encryption = StaticSecret::from(x_sk);

        // Re-issue the cert when expired or near expiry; the identity (and
        // the encryption key) are unchanged, only the window moves.
        let (not_before, not_after) =
            if now >= stored.cert_not_after - ENC_KEY_CERT_RENEWAL_WINDOW_SECS {
                let refreshed = StoredIdentity {
                    cert_not_before: now,
                    cert_not_after: now + ENC_KEY_CERT_VALIDITY_SECS,
                    ..stored
                };
                config
                    .set_secret(FEDERATION_IDENTITY_SECRET_KEY, &refreshed)
                    .map_err(|e| AuthError::IdentityStore(e.to_string()))?;
                (refreshed.cert_not_before, refreshed.cert_not_after)
            } else {
                (stored.cert_not_before, stored.cert_not_after)
            };

        let x25519_pub = X25519PublicKey::from(&encryption).to_bytes();
        let enc_key_cert = issue_enc_key_cert(&signing, &x25519_pub, not_before, not_after);
        Ok(Self {
            signing,
            encryption,
            enc_key_cert,
        })
    }

    pub fn author_id(&self) -> AuthorId {
        author_id_from_verifying_key(&self.signing.verifying_key())
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    pub fn x25519_public(&self) -> [u8; 32] {
        X25519PublicKey::from(&self.encryption).to_bytes()
    }

    pub fn enc_key_cert(&self) -> &EncKeyCert {
        &self.enc_key_cert
    }

    /// Sign with the Ed25519 identity key. Callers own domain separation:
    /// every message class gets its own context prefix (the enc-key-cert and
    /// safety-number contexts above are the pattern).
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing.sign(message)
    }

    /// X25519 Diffie-Hellman with a peer's encryption key (the HPKE/wrap
    /// primitive Slice 3 builds on).
    pub fn diffie_hellman(&self, peer_x25519_pub: &[u8; 32]) -> [u8; 32] {
        self.encryption
            .diffie_hellman(&X25519PublicKey::from(*peer_x25519_pub))
            .to_bytes()
    }

    /// The bundle this hub presents on the invite/response leg (§3.3).
    pub fn key_bundle(&self) -> PeerKeyBundle {
        PeerKeyBundle {
            author_id: hex::encode(self.author_id()),
            x25519_pub: self.enc_key_cert.x25519_pub.clone(),
            enc_key_cert: self.enc_key_cert.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Peer registry — TOFU pinning (§3.3, RT-5)
// ---------------------------------------------------------------------------

/// The signed key bundle a peer presents: identity key (== `author_id`),
/// encryption key, and the cert binding them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerKeyBundle {
    /// Hex-encoded 32-byte raw Ed25519 public key.
    pub author_id: String,
    /// Hex-encoded 32-byte X25519 public key. Must match the cert's.
    pub x25519_pub: String,
    pub enc_key_cert: EncKeyCert,
}

impl PeerKeyBundle {
    /// Structural + cryptographic validation: the author id parses as a real
    /// Ed25519 point, the bundle's encryption key matches the cert's, and
    /// the cert verifies against the bundle's own identity key at `now`.
    pub fn validate(&self, now: i64) -> Result<VerifyingKey, AuthError> {
        let id_bytes = parse_key32(&self.author_id, "peer author_id")?;
        let identity = VerifyingKey::from_bytes(&id_bytes)
            .map_err(|_| AuthError::InvalidKey("peer author_id: not a valid Ed25519 key".into()))?;
        if self.x25519_pub != self.enc_key_cert.x25519_pub {
            return Err(AuthError::InvalidKey(
                "peer bundle x25519_pub does not match its enc-key-cert".into(),
            ));
        }
        self.enc_key_cert.verify(&identity, now)?;
        Ok(identity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    /// Hex-encoded 32-byte X25519 public key currently bound to this peer.
    pub x25519_pub: String,
    pub enc_key_cert: EncKeyCert,
    /// RT-5: `true` only after the human safety-number comparison succeeded
    /// (or, later, an admin-chain vouch — Slice 2). Pinning alone never sets
    /// this, and an unverified peer is never a realm-key wrap target.
    pub verified: bool,
    pub pinned_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinOutcome {
    /// First contact: identity pinned, unverified.
    NewlyPinned,
    /// Identical to the pinned state — no-op.
    AlreadyPinned,
    /// Same pinned identity presented a NEW encryption key/cert, validly
    /// signed by that identity with a non-rewound window (§3.1 enc-key
    /// rotation). Identity — and therefore `verified` — is unchanged.
    EncKeyRotated,
}

#[derive(Serialize, Deserialize)]
struct StoredRegistry {
    version: u32,
    /// Keyed by hex-encoded `author_id`.
    peers: BTreeMap<String, PeerRecord>,
}

/// Persisted `author_id → PeerRecord` map with TOFU pinning. Contents are
/// public keys (not secrets), but the file is still written atomically at
/// 0600 via `secure_fs` — it is trust state, and guarantee C (disk/OS
/// protection) is its integrity backstop.
pub struct PeerRegistry {
    path: PathBuf,
    peers: BTreeMap<String, PeerRecord>,
}

/// Default on-disk location, beside the other daemon-owned state.
pub fn default_peer_registry_path() -> PathBuf {
    permagent::config::paths::Paths::data_dir()
        .join("federation")
        .join("peers.json")
}

impl PeerRegistry {
    /// Load the registry at `path`. A missing file is an empty registry; a
    /// present-but-unreadable file is a hard error (pinned trust state must
    /// never be silently discarded — that would reopen first-contact MITM).
    pub fn load(path: &Path) -> Result<Self, AuthError> {
        if !path.exists() {
            return Ok(Self {
                path: path.to_path_buf(),
                peers: BTreeMap::new(),
            });
        }
        let unreadable = |e: String| AuthError::RegistryUnreadable(path.display().to_string(), e);
        let content = std::fs::read_to_string(path).map_err(|e| unreadable(e.to_string()))?;
        let stored: StoredRegistry =
            serde_json::from_str(&content).map_err(|e| unreadable(e.to_string()))?;
        if stored.version != PEER_REGISTRY_VERSION {
            return Err(unreadable(format!(
                "registry version {} is not supported by this build (expected {})",
                stored.version, PEER_REGISTRY_VERSION
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
            peers: stored.peers,
        })
    }

    fn save(&self) -> Result<(), AuthError> {
        let stored = StoredRegistry {
            version: PEER_REGISTRY_VERSION,
            peers: self.peers.clone(),
        };
        let json = serde_json::to_string_pretty(&stored)
            .map_err(|e| AuthError::RegistryStore(e.to_string()))?;
        if let Some(parent) = self.path.parent() {
            permagent::config::secure_fs::ensure_private_dir(parent)
                .map_err(|e| AuthError::RegistryStore(e.to_string()))?;
        }
        permagent::config::secure_fs::write_private_file(&self.path, json.as_bytes())
            .map_err(|e| AuthError::RegistryStore(e.to_string()))
    }

    /// TOFU pin (§3.3). First contact pins the identity (unverified). A
    /// re-presented identical bundle is a no-op. The same identity may
    /// rotate its encryption key with a new validly-signed cert whose
    /// `not_before` does not rewind (replaying an older bundle to roll back
    /// to a compromised encryption key is a [`AuthError::PinConflict`]).
    pub fn pin(&mut self, bundle: &PeerKeyBundle, now: i64) -> Result<PinOutcome, AuthError> {
        bundle.validate(now)?;
        let key = bundle.author_id.to_lowercase();
        match self.peers.get(&key) {
            None => {
                self.peers.insert(
                    key,
                    PeerRecord {
                        x25519_pub: bundle.x25519_pub.clone(),
                        enc_key_cert: bundle.enc_key_cert.clone(),
                        verified: false,
                        pinned_at: now,
                    },
                );
                self.save()?;
                Ok(PinOutcome::NewlyPinned)
            }
            Some(existing) => {
                if existing.x25519_pub == bundle.x25519_pub
                    && existing.enc_key_cert == bundle.enc_key_cert
                {
                    return Ok(PinOutcome::AlreadyPinned);
                }
                if bundle.enc_key_cert.not_before < existing.enc_key_cert.not_before {
                    return Err(AuthError::PinConflict(display_author_id(&parse_key32(
                        &bundle.author_id,
                        "peer author_id",
                    )?)));
                }
                let record = self.peers.get_mut(&key).expect("checked present above");
                record.x25519_pub = bundle.x25519_pub.clone();
                record.enc_key_cert = bundle.enc_key_cert.clone();
                self.save()?;
                Ok(PinOutcome::EncKeyRotated)
            }
        }
    }

    /// Record that the human safety-number comparison for this peer
    /// succeeded (§3.3 step 3a).
    pub fn mark_verified(&mut self, author_id: &AuthorId) -> Result<(), AuthError> {
        let key = hex::encode(author_id);
        let record = self
            .peers
            .get_mut(&key)
            .ok_or_else(|| AuthError::NotPinned(display_author_id(author_id)))?;
        record.verified = true;
        self.save()
    }

    pub fn get(&self, author_id: &AuthorId) -> Option<&PeerRecord> {
        self.peers.get(&hex::encode(author_id))
    }

    /// RT-5: may a realm key be wrapped to this peer's encryption key?
    /// Requires the peer to be pinned, VERIFIED (safety number or — later —
    /// admin-chain vouch), and holding a currently-valid enc-key-cert.
    /// Pinning-for-display alone must never be sufficient.
    pub fn is_verified_wrap_target(&self, author_id: &AuthorId, now: i64) -> bool {
        let Some(record) = self.get(author_id) else {
            return false;
        };
        if !record.verified {
            return false;
        }
        let Ok(identity) = VerifyingKey::from_bytes(author_id) else {
            return false;
        };
        record.enc_key_cert.verify(&identity, now).is_ok()
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Safety numbers (§3.3)
// ---------------------------------------------------------------------------

/// Signal-style safety number over the two identity keys: 60 decimal digits
/// in 12 groups of 5, symmetric in its arguments (both sides render the same
/// string), for humans to compare over an existing trusted channel.
pub fn safety_number(a: &AuthorId, b: &AuthorId) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha512::new();
    hasher.update(SAFETY_NUMBER_CONTEXT);
    hasher.update(lo);
    hasher.update(hi);
    let digest = hasher.finalize();
    digest
        .chunks(5)
        .take(12)
        .map(|chunk| {
            let mut v: u64 = 0;
            for &byte in chunk {
                v = (v << 8) | u64::from(byte);
            }
            format!("{:05}", v % 100_000)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn test_config(dir: &Path) -> Config {
        Config::new_with_file_secrets(dir.join("config.yaml"), dir.join("secrets.json"))
            .expect("test config")
    }

    fn test_identity(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// A valid bundle for a synthetic peer whose keys derive from `seed`.
    fn test_bundle(seed: u8, not_before: i64, not_after: i64) -> PeerKeyBundle {
        let signing = test_identity(seed);
        let x_sk = StaticSecret::from([seed.wrapping_add(1); 32]);
        let x_pub = X25519PublicKey::from(&x_sk).to_bytes();
        let cert = issue_enc_key_cert(&signing, &x_pub, not_before, not_after);
        PeerKeyBundle {
            author_id: hex::encode(author_id_from_verifying_key(&signing.verifying_key())),
            x25519_pub: hex::encode(x_pub),
            enc_key_cert: cert,
        }
    }

    // -- keygen / load ------------------------------------------------------

    #[test]
    fn load_or_create_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let first = FederationIdentity::load_or_create_at(&config, NOW).unwrap();
        let second = FederationIdentity::load_or_create_at(&config, NOW).unwrap();
        assert_eq!(first.author_id(), second.author_id());
        assert_eq!(first.x25519_public(), second.x25519_public());
        assert_eq!(first.enc_key_cert(), second.enc_key_cert());
    }

    #[test]
    fn malformed_identity_is_a_hard_error_never_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        config
            .set_secret(FEDERATION_IDENTITY_SECRET_KEY, &"not an identity blob")
            .unwrap();
        let err = FederationIdentity::load_or_create_at(&config, NOW).unwrap_err();
        assert!(
            matches!(err, AuthError::IdentityUnreadable(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn near_expiry_cert_is_reissued_on_load_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let first = FederationIdentity::load_or_create_at(&config, NOW).unwrap();
        let late = NOW + ENC_KEY_CERT_VALIDITY_SECS - 1; // inside renewal window
        let second = FederationIdentity::load_or_create_at(&config, late).unwrap();
        assert_eq!(first.author_id(), second.author_id());
        assert_eq!(first.x25519_public(), second.x25519_public());
        assert_eq!(second.enc_key_cert().not_before, late);
        second
            .enc_key_cert()
            .verify(&second.verifying_key(), late)
            .unwrap();
    }

    // -- author_id ----------------------------------------------------------

    #[test]
    fn author_id_is_the_raw_verifying_key_and_stable() {
        let signing = test_identity(7);
        let id = author_id_from_verifying_key(&signing.verifying_key());
        // The ratified wire format: byte-for-byte the RFC 8032 public key.
        assert_eq!(id, signing.verifying_key().to_bytes());
        assert_eq!(id, author_id_from_verifying_key(&signing.verifying_key()));

        let display = display_author_id(&id);
        assert!(display.starts_with("ed25519:"), "got {display}");
        assert_eq!(
            data_encoding::BASE32_NOPAD
                .decode(display.strip_prefix("ed25519:").unwrap().as_bytes())
                .unwrap(),
            id.to_vec()
        );
    }

    // -- enc-key-cert -------------------------------------------------------

    #[test]
    fn cert_verifies_within_window_and_rejects_outside() {
        let signing = test_identity(1);
        let x_pub = [9u8; 32];
        let cert = issue_enc_key_cert(&signing, &x_pub, NOW, NOW + 100);
        cert.verify(&signing.verifying_key(), NOW).unwrap();
        cert.verify(&signing.verifying_key(), NOW + 100).unwrap();
        assert!(matches!(
            cert.verify(&signing.verifying_key(), NOW + 101),
            Err(AuthError::CertOutsideValidity { .. })
        ));
        assert!(matches!(
            cert.verify(&signing.verifying_key(), NOW - 1),
            Err(AuthError::CertOutsideValidity { .. })
        ));
    }

    #[test]
    fn tampered_or_wrong_signer_cert_is_rejected() {
        let signing = test_identity(1);
        let cert = issue_enc_key_cert(&signing, &[9u8; 32], NOW, NOW + 100);

        let mut tampered = cert.clone();
        tampered.x25519_pub = hex::encode([10u8; 32]);
        assert!(matches!(
            tampered.verify(&signing.verifying_key(), NOW),
            Err(AuthError::CertSignatureInvalid)
        ));

        let mut shifted = cert.clone();
        shifted.not_after += 1_000_000; // extend validity without re-signing
        assert!(matches!(
            shifted.verify(&signing.verifying_key(), NOW),
            Err(AuthError::CertSignatureInvalid)
        ));

        let other = test_identity(2);
        assert!(matches!(
            cert.verify(&other.verifying_key(), NOW),
            Err(AuthError::CertSignatureInvalid)
        ));
    }

    // -- peer registry / TOFU ----------------------------------------------

    #[test]
    fn tofu_pin_then_verify_transition() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federation").join("peers.json");
        let bundle = test_bundle(3, NOW, NOW + 1000);
        let author = parse_key32(&bundle.author_id, "test").unwrap();

        let mut registry = PeerRegistry::load(&path).unwrap();
        assert!(registry.is_empty());
        assert_eq!(registry.pin(&bundle, NOW).unwrap(), PinOutcome::NewlyPinned);
        assert_eq!(
            registry.pin(&bundle, NOW).unwrap(),
            PinOutcome::AlreadyPinned
        );

        // Pinned but unverified: NOT a wrap target (RT-5).
        assert!(!registry.is_verified_wrap_target(&author, NOW));
        registry.mark_verified(&author).unwrap();
        assert!(registry.is_verified_wrap_target(&author, NOW));

        // Persistence round-trip.
        let reloaded = PeerRegistry::load(&path).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert!(reloaded.get(&author).unwrap().verified);
        assert!(reloaded.is_verified_wrap_target(&author, NOW));
    }

    #[test]
    fn conflicting_bundle_is_rejected_and_pin_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let bundle = test_bundle(3, NOW, NOW + 1000);
        let author = parse_key32(&bundle.author_id, "test").unwrap();

        let mut registry = PeerRegistry::load(&path).unwrap();
        registry.pin(&bundle, NOW).unwrap();
        registry.mark_verified(&author).unwrap();

        // Same identity replays an OLDER bundle (rewound cert window) with a
        // different encryption key — rollback attempt, must be refused.
        let signing = test_identity(3);
        let old_x = X25519PublicKey::from(&StaticSecret::from([42u8; 32])).to_bytes();
        let rollback = PeerKeyBundle {
            author_id: bundle.author_id.clone(),
            x25519_pub: hex::encode(old_x),
            enc_key_cert: issue_enc_key_cert(&signing, &old_x, NOW - 500, NOW + 1000),
        };
        assert!(matches!(
            registry.pin(&rollback, NOW),
            Err(AuthError::PinConflict(_))
        ));
        assert_eq!(
            registry.get(&author).unwrap().x25519_pub,
            bundle.x25519_pub,
            "pinned state must be unchanged after a rejected pin"
        );
    }

    #[test]
    fn enc_key_rotation_by_same_identity_keeps_verified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let bundle = test_bundle(3, NOW, NOW + 1000);
        let author = parse_key32(&bundle.author_id, "test").unwrap();

        let mut registry = PeerRegistry::load(&path).unwrap();
        registry.pin(&bundle, NOW).unwrap();
        registry.mark_verified(&author).unwrap();

        // Legitimate rotation: fresh key, fresh cert, window moves forward.
        let signing = test_identity(3);
        let new_x = X25519PublicKey::from(&StaticSecret::from([43u8; 32])).to_bytes();
        let rotated = PeerKeyBundle {
            author_id: bundle.author_id.clone(),
            x25519_pub: hex::encode(new_x),
            enc_key_cert: issue_enc_key_cert(&signing, &new_x, NOW + 10, NOW + 2000),
        };
        assert_eq!(
            registry.pin(&rotated, NOW + 10).unwrap(),
            PinOutcome::EncKeyRotated
        );
        let record = registry.get(&author).unwrap();
        assert_eq!(record.x25519_pub, hex::encode(new_x));
        assert!(
            record.verified,
            "identity unchanged → verification survives"
        );
    }

    #[test]
    fn expired_cert_peer_is_not_a_wrap_target_even_if_verified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let bundle = test_bundle(5, NOW, NOW + 100);
        let author = parse_key32(&bundle.author_id, "test").unwrap();

        let mut registry = PeerRegistry::load(&path).unwrap();
        registry.pin(&bundle, NOW).unwrap();
        registry.mark_verified(&author).unwrap();
        assert!(registry.is_verified_wrap_target(&author, NOW));
        assert!(!registry.is_verified_wrap_target(&author, NOW + 101));
    }

    #[test]
    fn forged_bundle_never_pins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        let mut registry = PeerRegistry::load(&path).unwrap();

        // Cert signed by identity 3 but presented under identity 4's author
        // id — the relay substituting keys on the response leg (RT-5).
        let honest = test_bundle(3, NOW, NOW + 1000);
        let imposter = test_identity(4);
        let forged = PeerKeyBundle {
            author_id: hex::encode(author_id_from_verifying_key(&imposter.verifying_key())),
            ..honest
        };
        assert!(matches!(
            registry.pin(&forged, NOW),
            Err(AuthError::CertSignatureInvalid)
        ));
        assert!(registry.is_empty());
    }

    #[test]
    fn corrupt_registry_file_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peers.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(
            PeerRegistry::load(&path),
            Err(AuthError::RegistryUnreadable(_, _))
        ));
    }

    // -- safety numbers -----------------------------------------------------

    #[test]
    fn safety_number_is_symmetric_distinct_and_well_formed() {
        let a = author_id_from_verifying_key(&test_identity(1).verifying_key());
        let b = author_id_from_verifying_key(&test_identity(2).verifying_key());
        let c = author_id_from_verifying_key(&test_identity(3).verifying_key());

        let ab = safety_number(&a, &b);
        assert_eq!(ab, safety_number(&b, &a), "must be order-independent");
        assert_ne!(ab, safety_number(&a, &c));

        let groups: Vec<&str> = ab.split(' ').collect();
        assert_eq!(groups.len(), 12);
        assert!(groups
            .iter()
            .all(|g| g.len() == 5 && g.chars().all(|ch| ch.is_ascii_digit())));
    }

    // -- diffie-hellman sanity (the Slice 3 primitive) ----------------------

    #[test]
    fn diffie_hellman_agrees_between_two_identities() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let alice = FederationIdentity::load_or_create_at(&test_config(dir_a.path()), NOW).unwrap();
        let bob = FederationIdentity::load_or_create_at(&test_config(dir_b.path()), NOW).unwrap();
        assert_eq!(
            alice.diffie_hellman(&bob.x25519_public()),
            bob.diffie_hellman(&alice.x25519_public())
        );
        assert_ne!(alice.author_id(), bob.author_id());
    }
}
