//! Realm genesis + admin-chain — the per-realm root of trust (spec §3.5, RT-1).
//!
//! A realm's id is self-certifying: `realm_id = "realm:" || base32(SHA-256(
//! founder_pub || realm_nonce))`, so no attacker can craft a different genesis
//! hashing to the same id. Membership and adminship changes are an
//! append-only, hash-linked, signed log rooted at the genesis; every member
//! independently replays the chain to derive the current member/admin sets.
//! A link is valid only if its author is an admin as of the previous link;
//! the founding admin is the sole admin (and member) at genesis.
//!
//! Slice 2 scope: the in-memory objects, canonical byte encodings, signing,
//! and replay validation, fully property-tested. Replication of these control
//! objects (gate G1: a Permagent-owned parallel set beside the Spectral pack)
//! and the keyring that hangs off the derived admin set land in Slice 3+.
//!
//! Two deliberate additions over the spec's §3.5 struct sketch, flagged for
//! ratification in the Slice 2 PR:
//! - `Add` links carry a `role` (Member | Admin): §3.5 says adminship is
//!   "extended by signed chain links" and §3.3/§5.4 make member add/remove
//!   chain links too, so links need to say which of the two they grant.
//! - A chain may never remove its last admin: §5.6 flags the offline/absent
//!   admin as an availability hazard; a zero-admin realm is unrecoverable by
//!   construction (no key rotation, no adds), so the replay rejects it.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::auth::{author_id_from_verifying_key, AuthorId, EncKeyCert};

/// Domain-separation contexts (same discipline as `crate::auth`).
const GENESIS_CONTEXT: &[u8] = b"permagent.federation.genesis.v1";
const LINK_CONTEXT: &[u8] = b"permagent.federation.admin-chain-link.v1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RealmError {
    #[error("genesis signature invalid")]
    GenesisSignatureInvalid,
    #[error("realm id does not match the genesis it claims to certify")]
    RealmIdMismatch,
    #[error("link {seq}: prev hash does not match the chain head (tamper or fork)")]
    BrokenHashChain { seq: u64 },
    #[error("link {seq}: expected seq {expected}")]
    BadSeq { seq: u64, expected: u64 },
    #[error("link {seq}: author is not an admin as of the previous link")]
    NonAdminSigner { seq: u64 },
    #[error("link {seq}: signature invalid")]
    LinkSignatureInvalid { seq: u64 },
    #[error("link {seq}: author id is not a valid Ed25519 key")]
    InvalidAuthorKey { seq: u64 },
    #[error("link {seq}: subject is already a member")]
    DuplicateAdd { seq: u64 },
    #[error("link {seq}: subject is not a member")]
    RemoveAbsent { seq: u64 },
    #[error("link {seq}: removing the last admin would orphan the realm")]
    WouldOrphanRealm { seq: u64 },
    #[error("link {seq}: subject_keys vouch does not verify against the subject identity")]
    BadVouch { seq: u64 },
}

/// 32-byte SHA-256 content hash of a canonical object encoding.
pub type ObjectHash = [u8; 32];

// ---------------------------------------------------------------------------
// Genesis (§3.5)
// ---------------------------------------------------------------------------

/// `genesis := { founding_admin, realm_nonce, created_meta }`, signed by the
/// founder. The founder's `author_id` IS their Ed25519 key (§3.2), so a
/// single 32-byte field carries both.
#[derive(Clone)]
pub struct Genesis {
    pub founder: AuthorId,
    pub realm_nonce: [u8; 32],
    pub created_at: i64,
    pub sig: Signature,
}

/// Self-certifying realm id: `"realm:" || base32(SHA-256(founder_pub || nonce))`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RealmId(String);

impl RealmId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn derive(founder: &AuthorId, realm_nonce: &[u8; 32]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(founder);
        hasher.update(realm_nonce);
        RealmId(format!(
            "realm:{}",
            data_encoding::BASE32_NOPAD.encode(&hasher.finalize())
        ))
    }
}

impl std::fmt::Display for RealmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn genesis_message(founder: &AuthorId, realm_nonce: &[u8; 32], created_at: i64) -> Vec<u8> {
    let mut msg = Vec::with_capacity(GENESIS_CONTEXT.len() + 32 + 32 + 8);
    msg.extend_from_slice(GENESIS_CONTEXT);
    msg.extend_from_slice(founder);
    msg.extend_from_slice(realm_nonce);
    msg.extend_from_slice(&created_at.to_be_bytes());
    msg
}

impl Genesis {
    /// Found a new realm. The nonce must be fresh random (caller supplies it
    /// so this layer stays deterministic and testable).
    pub fn create(founder: &SigningKey, realm_nonce: [u8; 32], created_at: i64) -> Self {
        let founder_id = author_id_from_verifying_key(&founder.verifying_key());
        let sig = founder.sign(&genesis_message(&founder_id, &realm_nonce, created_at));
        Self {
            founder: founder_id,
            realm_nonce,
            created_at,
            sig,
        }
    }

    pub fn realm_id(&self) -> RealmId {
        RealmId::derive(&self.founder, &self.realm_nonce)
    }

    /// Verify the founder's signature and that `claimed` is really this
    /// genesis's realm id (the self-certification check: a joiner trusts the
    /// OOB-received realm id, and the genesis must prove it hashes to it).
    pub fn verify(&self, claimed: &RealmId) -> Result<(), RealmError> {
        let founder_key = VerifyingKey::from_bytes(&self.founder)
            .map_err(|_| RealmError::GenesisSignatureInvalid)?;
        founder_key
            .verify(
                &genesis_message(&self.founder, &self.realm_nonce, self.created_at),
                &self.sig,
            )
            .map_err(|_| RealmError::GenesisSignatureInvalid)?;
        if &self.realm_id() != claimed {
            return Err(RealmError::RealmIdMismatch);
        }
        Ok(())
    }

    /// Content hash the first chain link points at.
    pub fn hash(&self) -> ObjectHash {
        let mut hasher = Sha256::new();
        hasher.update(genesis_message(
            &self.founder,
            &self.realm_nonce,
            self.created_at,
        ));
        hasher.update(self.sig.to_bytes());
        hasher.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Admin-chain links (§3.5)
// ---------------------------------------------------------------------------

/// What an `Add` link grants. Spec §3.5 addition (see module docs): the chain
/// carries both member adds (§3.3 vouch, §5.4 removal) and admin anointment
/// ("extended by signed chain links"), so the link states which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Member,
    Admin,
}

/// RT-5 in-band vouch: the adding admin binds the new member's encryption
/// key + cert into the signed link (`author_id` is the `subject` itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectKeys {
    pub x25519_pub: [u8; 32],
    pub cert: EncKeyCert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOp {
    Add {
        subject: AuthorId,
        role: Role,
        /// Present when the adding admin vouches the subject's keys (RT-5).
        subject_keys: Option<SubjectKeys>,
    },
    Remove {
        subject: AuthorId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminChainLink {
    /// Hash of the previous link (or of the genesis for the first link).
    pub prev: ObjectHash,
    /// 1-based position in the chain.
    pub seq: u64,
    pub op: LinkOp,
    /// The admin who authored this link (`author_id` == their Ed25519 key).
    pub by: AuthorId,
    pub at: i64,
    pub sig: Signature,
}

fn link_message(prev: &ObjectHash, seq: u64, op: &LinkOp, by: &AuthorId, at: i64) -> Vec<u8> {
    let mut msg = Vec::with_capacity(160);
    msg.extend_from_slice(LINK_CONTEXT);
    msg.extend_from_slice(prev);
    msg.extend_from_slice(&seq.to_be_bytes());
    match op {
        LinkOp::Add {
            subject,
            role,
            subject_keys,
        } => {
            msg.push(1);
            msg.extend_from_slice(subject);
            msg.push(match role {
                Role::Member => 0,
                Role::Admin => 1,
            });
            match subject_keys {
                None => msg.push(0),
                Some(keys) => {
                    msg.push(1);
                    msg.extend_from_slice(&keys.x25519_pub);
                    msg.extend_from_slice(&keys.cert.not_before.to_be_bytes());
                    msg.extend_from_slice(&keys.cert.not_after.to_be_bytes());
                    // The cert's own signature bytes pin the exact cert.
                    msg.extend_from_slice(keys.cert.sig.as_bytes());
                }
            }
        }
        LinkOp::Remove { subject } => {
            msg.push(2);
            msg.extend_from_slice(subject);
        }
    }
    msg.extend_from_slice(by);
    msg.extend_from_slice(&at.to_be_bytes());
    msg
}

impl AdminChainLink {
    /// Author + sign a link. `signer` must be the admin whose `author_id`
    /// goes in `by` (enforced by construction here; replay re-checks).
    pub fn create(signer: &SigningKey, prev: ObjectHash, seq: u64, op: LinkOp, at: i64) -> Self {
        let by = author_id_from_verifying_key(&signer.verifying_key());
        let sig = signer.sign(&link_message(&prev, seq, &op, &by, at));
        Self {
            prev,
            seq,
            op,
            by,
            at,
            sig,
        }
    }

    pub fn hash(&self) -> ObjectHash {
        let mut hasher = Sha256::new();
        hasher.update(link_message(
            &self.prev, self.seq, &self.op, &self.by, self.at,
        ));
        hasher.update(self.sig.to_bytes());
        hasher.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Replay — derive the member/admin sets (§3.5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub role: Role,
    /// The RT-5 vouch carried by the Add link, if any.
    pub vouched_keys: Option<SubjectKeys>,
    /// Seq of the link that added this member (0 = genesis founder).
    pub added_at_seq: u64,
}

/// The state a replica derives by replaying the chain from genesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmState {
    pub realm_id: RealmId,
    members: BTreeMap<AuthorId, MemberInfo>,
    head: ObjectHash,
    next_seq: u64,
}

impl RealmState {
    /// Validate the genesis against the OOB-trusted realm id and start the
    /// state: the founder is the sole admin (and member).
    pub fn from_genesis(genesis: &Genesis, trusted_realm_id: &RealmId) -> Result<Self, RealmError> {
        genesis.verify(trusted_realm_id)?;
        let mut members = BTreeMap::new();
        members.insert(
            genesis.founder,
            MemberInfo {
                role: Role::Admin,
                vouched_keys: None,
                added_at_seq: 0,
            },
        );
        Ok(Self {
            realm_id: genesis.realm_id(),
            members,
            head: genesis.hash(),
            next_seq: 1,
        })
    }

    /// Replay a full chain. Fails on the first invalid link; a valid prefix
    /// never survives an invalid suffix (all-or-nothing, like git fsck).
    pub fn replay(
        genesis: &Genesis,
        trusted_realm_id: &RealmId,
        links: &[AdminChainLink],
    ) -> Result<Self, RealmError> {
        let mut state = Self::from_genesis(genesis, trusted_realm_id)?;
        for link in links {
            state.apply(link)?;
        }
        Ok(state)
    }

    /// Validate + apply one link.
    pub fn apply(&mut self, link: &AdminChainLink) -> Result<(), RealmError> {
        let seq = link.seq;
        if seq != self.next_seq {
            return Err(RealmError::BadSeq {
                seq,
                expected: self.next_seq,
            });
        }
        if link.prev != self.head {
            return Err(RealmError::BrokenHashChain { seq });
        }
        // Author must be an admin as of the previous link.
        match self.members.get(&link.by) {
            Some(info) if info.role == Role::Admin => {}
            _ => return Err(RealmError::NonAdminSigner { seq }),
        }
        let by_key =
            VerifyingKey::from_bytes(&link.by).map_err(|_| RealmError::InvalidAuthorKey { seq })?;
        by_key
            .verify(
                &link_message(&link.prev, link.seq, &link.op, &link.by, link.at),
                &link.sig,
            )
            .map_err(|_| RealmError::LinkSignatureInvalid { seq })?;

        match &link.op {
            LinkOp::Add {
                subject,
                role,
                subject_keys,
            } => {
                if self.members.contains_key(subject) {
                    return Err(RealmError::DuplicateAdd { seq });
                }
                if let Some(keys) = subject_keys {
                    // The vouch must actually bind: the cert must verify
                    // against the SUBJECT's identity key at the link's time,
                    // and name the same x25519 key the vouch carries.
                    let subject_key = VerifyingKey::from_bytes(subject)
                        .map_err(|_| RealmError::InvalidAuthorKey { seq })?;
                    let cert_matches = keys.cert.x25519_pub_bytes().ok() == Some(keys.x25519_pub);
                    if !cert_matches || keys.cert.verify(&subject_key, link.at).is_err() {
                        return Err(RealmError::BadVouch { seq });
                    }
                }
                self.members.insert(
                    *subject,
                    MemberInfo {
                        role: *role,
                        vouched_keys: subject_keys.clone(),
                        added_at_seq: seq,
                    },
                );
            }
            LinkOp::Remove { subject } => {
                let Some(info) = self.members.get(subject) else {
                    return Err(RealmError::RemoveAbsent { seq });
                };
                if info.role == Role::Admin && self.admins().count() == 1 {
                    return Err(RealmError::WouldOrphanRealm { seq });
                }
                self.members.remove(subject);
            }
        }
        self.head = link.hash();
        self.next_seq += 1;
        Ok(())
    }

    /// Hash the next link's `prev` must carry.
    pub fn head_hash(&self) -> ObjectHash {
        self.head
    }

    /// Seq the next link must carry.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn is_member(&self, id: &AuthorId) -> bool {
        self.members.contains_key(id)
    }

    /// §5.2/§4: is this author currently an admin (keyring-signer check)?
    pub fn is_admin(&self, id: &AuthorId) -> bool {
        matches!(self.members.get(id), Some(info) if info.role == Role::Admin)
    }

    pub fn admins(&self) -> impl Iterator<Item = &AuthorId> {
        self.members
            .iter()
            .filter(|(_, info)| info.role == Role::Admin)
            .map(|(id, _)| id)
    }

    pub fn members(&self) -> impl Iterator<Item = (&AuthorId, &MemberInfo)> {
        self.members.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::issue_enc_key_cert;
    use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

    const NOW: i64 = 1_800_000_000;
    const NONCE: [u8; 32] = [0xAB; 32];

    fn identity(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn author(seed: u8) -> AuthorId {
        author_id_from_verifying_key(&identity(seed).verifying_key())
    }

    fn vouch_for(seed: u8) -> SubjectKeys {
        let signing = identity(seed);
        let x_pub =
            X25519PublicKey::from(&StaticSecret::from([seed.wrapping_add(9); 32])).to_bytes();
        SubjectKeys {
            x25519_pub: x_pub,
            cert: issue_enc_key_cert(&signing, &x_pub, NOW - 10, NOW + 10_000),
        }
    }

    fn add(
        state: &RealmState,
        signer: &SigningKey,
        subject: AuthorId,
        role: Role,
    ) -> AdminChainLink {
        AdminChainLink::create(
            signer,
            state.head_hash(),
            state.next_seq(),
            LinkOp::Add {
                subject,
                role,
                subject_keys: None,
            },
            NOW,
        )
    }

    fn remove(state: &RealmState, signer: &SigningKey, subject: AuthorId) -> AdminChainLink {
        AdminChainLink::create(
            signer,
            state.head_hash(),
            state.next_seq(),
            LinkOp::Remove { subject },
            NOW,
        )
    }

    // -- genesis self-certification -----------------------------------------

    #[test]
    fn genesis_self_certifies_and_rejects_mismatch() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let realm_id = genesis.realm_id();
        assert!(realm_id.as_str().starts_with("realm:"));
        genesis.verify(&realm_id).unwrap();

        // A different nonce yields a different realm id — the genesis cannot
        // certify a realm id it does not hash to.
        let other = Genesis::create(&founder, [0xCD; 32], NOW);
        assert_eq!(
            genesis.verify(&other.realm_id()),
            Err(RealmError::RealmIdMismatch)
        );

        // Tampered founder field breaks the signature.
        let mut forged = genesis.clone();
        forged.founder = author(2);
        assert_eq!(
            forged.verify(&realm_id),
            Err(RealmError::GenesisSignatureInvalid)
        );
    }

    // -- chain replay --------------------------------------------------------

    #[test]
    fn replay_derives_member_and_admin_sets() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let realm_id = genesis.realm_id();
        let mut state = RealmState::from_genesis(&genesis, &realm_id).unwrap();

        let l1 = add(&state, &founder, author(2), Role::Member);
        state.apply(&l1).unwrap();
        let l2 = add(&state, &founder, author(3), Role::Admin);
        state.apply(&l2).unwrap();
        // Cross-author: the NEW admin (3) removes member 2 — accepted.
        let l3 = remove(&state, &identity(3), author(2));
        state.apply(&l3).unwrap();

        // Full replay from scratch converges to the same state.
        let replayed = RealmState::replay(&genesis, &realm_id, &[l1, l2, l3]).unwrap();
        assert_eq!(replayed, state);
        assert!(state.is_admin(&author(1)));
        assert!(state.is_admin(&author(3)));
        assert!(!state.is_member(&author(2)));
        assert_eq!(state.admins().count(), 2);
    }

    #[test]
    fn add_with_valid_vouch_is_recorded_and_bad_vouch_rejected() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();

        let good = AdminChainLink::create(
            &founder,
            state.head_hash(),
            state.next_seq(),
            LinkOp::Add {
                subject: author(2),
                role: Role::Member,
                subject_keys: Some(vouch_for(2)),
            },
            NOW,
        );
        state.apply(&good).unwrap();
        assert!(state
            .members()
            .find(|(id, _)| **id == author(2))
            .unwrap()
            .1
            .vouched_keys
            .is_some());

        // Vouch whose cert was signed by the WRONG identity (relay swapped
        // the subject) must be rejected.
        let bad = AdminChainLink::create(
            &founder,
            state.head_hash(),
            state.next_seq(),
            LinkOp::Add {
                subject: author(3),
                role: Role::Member,
                subject_keys: Some(vouch_for(4)), // cert chains to 4, not 3
            },
            NOW,
        );
        assert_eq!(state.apply(&bad), Err(RealmError::BadVouch { seq: 2 }));
    }

    // -- forged-admin rejection ---------------------------------------------

    #[test]
    fn non_admin_cannot_author_links() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();
        let l1 = add(&state, &founder, author(2), Role::Member);
        state.apply(&l1).unwrap();

        // Member 2 (not admin) tries to add someone.
        let forged = add(&state, &identity(2), author(3), Role::Member);
        assert_eq!(
            state.apply(&forged),
            Err(RealmError::NonAdminSigner { seq: 2 })
        );

        // A total outsider asserting adminship is equally rejected.
        let outsider = add(&state, &identity(9), author(3), Role::Admin);
        assert_eq!(
            state.apply(&outsider),
            Err(RealmError::NonAdminSigner { seq: 2 })
        );
    }

    #[test]
    fn link_claiming_admin_by_but_signed_by_other_key_is_rejected() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();

        // Signed by 9 but claiming `by` = founder.
        let mut forged = add(&state, &identity(9), author(2), Role::Member);
        forged.by = author(1);
        assert_eq!(
            state.apply(&forged),
            Err(RealmError::LinkSignatureInvalid { seq: 1 })
        );
    }

    // -- tamper-changes-hash -------------------------------------------------

    #[test]
    fn tampered_mid_chain_link_breaks_replay() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let realm_id = genesis.realm_id();
        let mut state = RealmState::from_genesis(&genesis, &realm_id).unwrap();
        let l1 = add(&state, &founder, author(2), Role::Member);
        state.apply(&l1).unwrap();
        let l2 = add(&state, &founder, author(3), Role::Admin);
        state.apply(&l2).unwrap();

        // Tamper l1's subject after the fact: its own signature now fails.
        let mut t1 = l1.clone();
        if let LinkOp::Add { subject, .. } = &mut t1.op {
            *subject = author(7);
        }
        let err = RealmState::replay(&genesis, &realm_id, &[t1.clone(), l2.clone()]).unwrap_err();
        assert_eq!(err, RealmError::LinkSignatureInvalid { seq: 1 });

        // Re-sign the tampered link (attacker holds NO admin key, but suppose
        // they re-sign with their own): now l2's prev no longer matches — the
        // hash chain itself detects the splice.
        let resigned = AdminChainLink::create(&identity(9), l1.prev, 1, t1.op.clone(), l1.at);
        let err = RealmState::replay(&genesis, &realm_id, &[resigned, l2]).unwrap_err();
        // Rejected either as a non-admin signer (9 was never admin) — the
        // earliest check that fires — never silently accepted.
        assert_eq!(err, RealmError::NonAdminSigner { seq: 1 });
    }

    #[test]
    fn wrong_prev_or_seq_is_rejected() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();

        let bad_prev = AdminChainLink::create(
            &founder,
            [0u8; 32],
            1,
            LinkOp::Add {
                subject: author(2),
                role: Role::Member,
                subject_keys: None,
            },
            NOW,
        );
        assert_eq!(
            state.apply(&bad_prev),
            Err(RealmError::BrokenHashChain { seq: 1 })
        );

        let bad_seq = AdminChainLink::create(
            &founder,
            state.head_hash(),
            5,
            LinkOp::Add {
                subject: author(2),
                role: Role::Member,
                subject_keys: None,
            },
            NOW,
        );
        assert_eq!(
            state.apply(&bad_seq),
            Err(RealmError::BadSeq {
                seq: 5,
                expected: 1
            })
        );
    }

    // -- membership invariants ----------------------------------------------

    #[test]
    fn duplicate_add_and_remove_absent_are_rejected() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();
        let l1 = add(&state, &founder, author(2), Role::Member);
        state.apply(&l1).unwrap();

        let dup = add(&state, &founder, author(2), Role::Member);
        assert_eq!(state.apply(&dup), Err(RealmError::DuplicateAdd { seq: 2 }));

        let absent = remove(&state, &founder, author(5));
        assert_eq!(
            state.apply(&absent),
            Err(RealmError::RemoveAbsent { seq: 2 })
        );
    }

    #[test]
    fn last_admin_cannot_be_removed_but_readd_after_remove_works() {
        let founder = identity(1);
        let genesis = Genesis::create(&founder, NONCE, NOW);
        let mut state = RealmState::from_genesis(&genesis, &genesis.realm_id()).unwrap();

        // Sole admin removing themselves would orphan the realm.
        let orphan = remove(&state, &founder, author(1));
        assert_eq!(
            state.apply(&orphan),
            Err(RealmError::WouldOrphanRealm { seq: 1 })
        );

        // With a second admin, the founder CAN be rotated out...
        let l1 = add(&state, &founder, author(3), Role::Admin);
        state.apply(&l1).unwrap();
        let l2 = remove(&state, &identity(3), author(1));
        state.apply(&l2).unwrap();
        assert!(!state.is_member(&author(1)));

        // ...and re-added later (OD-6 re-attestation shape).
        let l3 = add(&state, &identity(3), author(1), Role::Member);
        state.apply(&l3).unwrap();
        assert!(state.is_member(&author(1)));
        assert!(!state.is_admin(&author(1)));
    }
}
