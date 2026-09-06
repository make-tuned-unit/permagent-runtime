# B5.3 receipt — CLI spend announcement compatibility

Date: 2026-09-05 (America/Halifax)

Status: implemented; promotion gate pending focused runtime test capacity.

## Contract delivered

The CLI spend announcement remains notification-only: the harness's durable
ledger row is still the authority and the daemon recomputes the canonical
`budget-projection.v1` projection from Spectral. The spend response and
`session_spend_changed` event retain the existing scalar compatibility fields
and now carry the projection when the daemon has one. The event serializer
omits the optional projection key for legacy emitters, preserving their wire
shape; daemon announcements always provide it.

Projection/query failures, invalid bound projections, and unavailable settled
spend return service-unavailable before response/event construction. A genuine
authoritative zero remains a numeric zero. Unbound task scope remains explicit
in the projection, while pending and unknown holds remain visible in their
separate projection fields and unknown band rather than being flattened into
zero. The CLI checks HTTP response status with `error_for_status`, so bounded
announcement failures are observable in the existing warning path without
blocking the coding turn indefinitely.

## Verification

```text
CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --lib
passed

CARGO_INCREMENTAL=0 cargo check -p permagent-cli --lib
passed

git diff --check -- crates/goose-cli/src/session/spend_announce.rs \
  crates/goose-server/src/routes/coding_session.rs \
  crates/goose/src/events/mod.rs
passed
```

Deterministic route/helper coverage includes authoritative zero, unavailable
projection rejection, unbound task partial semantics, pending/unknown hold
visibility, and legacy/projection event serialization compatibility. The
focused event test and daemon route runtime test were not promoted as passing
evidence in this run because Cargo test processes retained the shared build
lock while compiling; no provider call was made. The prior B5.2 daemon
receipt's host SIGKILL startup limitation remains applicable, and no repeated
runtime retry was treated as evidence.

No provider calls, UI/projection-internal edits, new persistence, or ledger
writes were added by B5.3.
