//! Federation — Permagent's half of "git for federated brains" (#784).
//!
//! Design: `docs/design/federation-security-spec.md`. Slice map:
//! - Slice 1 (identity, TOFU pinning, safety numbers) lives in [`crate::auth`].
//! - Slice 2 ([`realm`]): realm genesis + admin-chain — the per-realm root of
//!   trust (§3.5). In-memory logic + validation; replication of these control
//!   objects (the Permagent-owned parallel set beside Spectral packs, gate G1)
//!   arrives with Slice 3.
//! - Slices 3–6 (E2E seal/open, key management, transport, UI) follow.

pub mod realm;
