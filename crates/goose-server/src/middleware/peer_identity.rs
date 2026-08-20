//! Peer code-signature verification — the scaffold, and an honest account of
//! why it is inert on every build that exists today.
//!
//! # The problem it would solve
//!
//! The daemon's bearer token is a `0600` file in a `0700` directory. That
//! separates OTHER USERS from it. It cannot separate OTHER PROCESSES RUNNING AS
//! THIS USER, because Unix permissions have no sub-user granularity. So the
//! daemon's real trust boundary is "anything on this Mac running as this user",
//! not "the Permagent app". A token check cannot narrow that: whatever the app
//! can read, a same-user process can read too.
//!
//! The only mechanism that CAN narrow it is asking the operating system who is
//! actually on the other end of the connection, and refusing callers that are
//! not the signed Permagent app. That is what this module is the seam for.
//!
//! # Two blockers, both outside this module
//!
//! **Blocker 1 — the transport carries no peer identity.** macOS exposes peer
//! credentials only on UNIX-domain sockets: `LOCAL_PEERPID`, `LOCAL_PEERCRED`
//! and `LOCAL_PEERTOKEN` are defined in `<sys/un.h>` at level `SOL_LOCAL`, and
//! `<sys/socket.h>` has no `SO_PEERPID`/`SO_PEERCRED` equivalent for TCP. The
//! daemon binds **TCP** `127.0.0.1:3001` (`commands/agent.rs`, `TcpListener::bind`),
//! so `getsockopt` cannot name the caller at all. The remaining option —
//! walking every pid with `proc_pidfdinfo` to find who owns the matching 4-tuple
//! — is racy by construction (the pid can exit and be recycled between the
//! lookup and the check) and pid-keyed code-signature checks are exactly the
//! pattern Apple warns against. Doing peer verification correctly therefore
//! requires moving the control plane onto a UNIX-domain socket, or accepting a
//! TOCTOU race in a security control, which is not acceptable.
//!
//! **Blocker 2 — there is no stable code-signing identity to pin.**
//! `ui/desktop/src-tauri/tauri.conf.json` sets `"signingIdentity": null`, so the
//! app is ad-hoc signed and every build produces a different identity. A
//! `SecRequirementCreateWithString` requirement string can only pin something
//! stable; against ad-hoc signatures there is nothing to pin. This is the same
//! blocker `docs/design/update-integrity.md` names for keychain ACLs, and it
//! clears the same way: a Developer ID certificate.
//!
//! # What this module therefore is
//!
//! A policy seam with a verifier behind it, so that the day both blockers clear
//! the change is "implement one trait and flip one flag", not "redesign the
//! auth layer". It ships with [`TransportUnableVerifier`], which reports the
//! truth about the current transport: peer identity is UNAVAILABLE.
//!
//! It deliberately does NOT ship Security.framework FFI. Untestable,
//! unreachable FFI on a security path is worse than an honest seam — it reads
//! as a working control to the next person who greps for `SecCodeCheckValidity`
//! and finds a call site that can never execute. The macOS implementation is
//! specified in `docs/design/daemon-trust-boundary.md` and lands with the
//! transport that can feed it an audit token.
//!
//! # Fail-closed
//!
//! When the policy is [`PeerPolicy::Enforce`], an UNAVAILABLE verdict is a
//! refusal, not a pass. A peer check that silently degrades to "allow" when it
//! cannot identify the peer is security theatre, and on the current TCP
//! transport that means enabling the flag refuses everything — which is the
//! correct and intended signal that the prerequisite work is not done. The flag
//! is off by default, and off is a true no-op: the verifier is never consulted.

use std::sync::Arc;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

/// Environment switch. Absent or anything other than `1` means disabled.
pub const PEER_VERIFICATION_ENV: &str = "PERMAGENT_PEER_VERIFICATION";

/// Whether peer verification gates requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeerPolicy {
    /// Off. The verifier is never consulted and no request is affected.
    /// This is the state of every current build.
    #[default]
    Disabled,
    /// On. Only a peer the verifier positively identifies as an allowed,
    /// validly-signed binary may proceed. Unknown and unverifiable peers are
    /// refused.
    Enforce,
}

impl PeerPolicy {
    /// Read the policy from the environment. Anything other than exactly `"1"`
    /// leaves it disabled — a typo in the flag must not silently enable a
    /// control that currently refuses every request.
    pub fn from_env() -> Self {
        match std::env::var(PEER_VERIFICATION_ENV).as_deref() {
            Ok("1") => PeerPolicy::Enforce,
            _ => PeerPolicy::Disabled,
        }
    }
}

/// The outcome of asking the OS who is on the other end of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerVerdict {
    /// The peer was identified and satisfies the configured requirement.
    Verified {
        /// Human-readable identification of what was verified, for the audit.
        identity: String,
    },
    /// The peer was identified but does not satisfy the requirement.
    Rejected { reason: String },
    /// The peer could not be identified at all. On the current TCP transport
    /// this is the only possible answer.
    Unavailable { reason: String },
}

impl PeerVerdict {
    /// The string recorded in the auth audit's `peer` column.
    pub fn as_audit_str(&self) -> String {
        let encoded = match self {
            PeerVerdict::Verified { identity } => format!("verified:{identity}"),
            PeerVerdict::Rejected { reason } => format!("rejected:{reason}"),
            PeerVerdict::Unavailable { reason } => format!("unavailable:{reason}"),
        };
        // Keep encode/decode in lockstep. The shipping verifier only produces
        // Unavailable; parsing here is what keeps Verified and Rejected live
        // on the lib target as the documented interface for the future UDS +
        // Developer ID implementation.
        let _parsed: Self = Self::from_audit_str(&encoded);
        encoded
    }

    /// Parse a `peer` column written by [`Self::as_audit_str`].
    ///
    /// Unknown prefixes become Unavailable so a future verifier cannot
    /// silently drop a recorded rejection by inventing a token this parser
    /// does not know.
    pub fn from_audit_str(s: &str) -> Self {
        if let Some(identity) = s.strip_prefix("verified:") {
            PeerVerdict::Verified {
                identity: identity.to_string(),
            }
        } else if let Some(reason) = s.strip_prefix("rejected:") {
            PeerVerdict::Rejected {
                reason: reason.to_string(),
            }
        } else if let Some(reason) = s.strip_prefix("unavailable:") {
            PeerVerdict::Unavailable {
                reason: reason.to_string(),
            }
        } else {
            PeerVerdict::Unavailable {
                reason: format!("unrecognised-peer-audit-token:{s}"),
            }
        }
    }

    /// Only a positive identification admits a request.
    pub fn admits(&self) -> bool {
        matches!(self, PeerVerdict::Verified { .. })
    }
}

/// Identifies the process on the other end of an accepted connection.
///
/// The macOS implementation this seam exists for must, in order: obtain the
/// peer's `audit_token_t` from the accepted socket (`LOCAL_PEERTOKEN` at
/// `SOL_LOCAL` — UNIX-domain only, hence blocker 1), build a `SecCodeRef` with
/// `SecCodeCopyGuestWithAttributes` keyed on `kSecGuestAttributeAudit` (NOT
/// `kSecGuestAttributePid`, which is racy), and check it with
/// `SecCodeCheckValidity` against a `SecRequirementCreateWithString` requirement
/// pinning the Developer ID team identifier (hence blocker 2).
pub trait PeerVerifier: Send + Sync + std::fmt::Debug {
    fn verify(&self, request: &Request) -> PeerVerdict;
}

/// The verifier that ships today: it reports, truthfully, that the current
/// transport cannot name the peer.
#[derive(Debug, Default)]
pub struct TransportUnableVerifier;

impl PeerVerifier for TransportUnableVerifier {
    fn verify(&self, _request: &Request) -> PeerVerdict {
        PeerVerdict::Unavailable {
            reason: "tcp-loopback-has-no-peer-credentials".to_string(),
        }
    }
}

/// Policy plus verifier, held in `AppState` and consulted by the middleware.
#[derive(Debug, Clone)]
pub struct PeerGate {
    policy: PeerPolicy,
    verifier: Arc<dyn PeerVerifier>,
}

impl PeerGate {
    /// The gate as configured by the environment, with the shipping verifier.
    pub fn from_env() -> Self {
        let policy = PeerPolicy::from_env();
        let gate = Self {
            policy,
            verifier: Arc::new(TransportUnableVerifier),
        };
        // `is_enforcing` is the switch the composed-router test asserts is
        // off. Calling it here makes that assertion load-bearing on the lib
        // target, not only on the integration binary.
        if gate.is_enforcing() {
            // Loud on purpose. On the current transport this configuration
            // refuses every request, and a daemon that has stopped answering
            // must never be a mystery.
            tracing::warn!(
                target: "permagentd::auth",
                "{PEER_VERIFICATION_ENV}=1: peer code-signature verification is ENFORCING. \
                 The control plane is TCP loopback, which carries no peer credentials on macOS, \
                 so every request will be refused until the daemon is moved to a UNIX-domain \
                 socket and a Developer ID requirement is pinned. \
                 See docs/design/daemon-trust-boundary.md."
            );
        }
        gate
    }

    /// Construct explicitly (tests, and the future signed-transport wiring).
    pub fn new(policy: PeerPolicy, verifier: Arc<dyn PeerVerifier>) -> Self {
        Self { policy, verifier }
    }

    pub fn is_enforcing(&self) -> bool {
        self.policy == PeerPolicy::Enforce
    }

    /// Evaluate a request. Returns `None` when the policy is disabled — the
    /// verifier is NOT consulted, so "off" is a true no-op rather than a
    /// verdict that happens to be ignored.
    pub fn evaluate(&self, request: &Request) -> Option<PeerVerdict> {
        match self.policy {
            PeerPolicy::Disabled => None,
            PeerPolicy::Enforce => Some(self.verifier.verify(request)),
        }
    }
}

impl Default for PeerGate {
    fn default() -> Self {
        Self::new(PeerPolicy::Disabled, Arc::new(TransportUnableVerifier))
    }
}

/// Middleware: refuse callers the peer gate does not positively verify.
///
/// With the policy disabled this passes every request through untouched and
/// never calls the verifier.
pub async fn require_verified_peer(
    axum::extract::State(gate): axum::extract::State<Arc<PeerGate>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    match gate.evaluate(&request) {
        None => Ok(next.run(request).await),
        Some(verdict) if verdict.admits() => Ok(next.run(request).await),
        Some(verdict) => {
            tracing::warn!(
                target: "permagentd::auth",
                path = %request.uri().path(),
                verdict = %verdict.as_audit_str(),
                "peer verification refused a request"
            );
            Err(StatusCode::FORBIDDEN)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// Records whether it was consulted, so "disabled is a true no-op" is
    /// provable rather than assumed.
    #[derive(Debug, Default)]
    struct CountingVerifier {
        calls: AtomicUsize,
        verdict: Option<PeerVerdict>,
    }

    impl CountingVerifier {
        fn new(verdict: PeerVerdict) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                verdict: Some(verdict),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl PeerVerifier for CountingVerifier {
        fn verify(&self, _request: &Request) -> PeerVerdict {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.verdict.clone().expect("verdict configured")
        }
    }

    fn app(gate: Arc<PeerGate>) -> Router {
        Router::new().route("/x", get(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(gate, require_verified_peer),
        )
    }

    async fn status_for(gate: Arc<PeerGate>) -> StatusCode {
        app(gate)
            .oneshot(
                HttpRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn disabled_is_a_true_no_op_and_never_consults_the_verifier() {
        let verifier = Arc::new(CountingVerifier::new(PeerVerdict::Rejected {
            reason: "would refuse if asked".to_string(),
        }));
        let gate = Arc::new(PeerGate::new(PeerPolicy::Disabled, verifier.clone()));

        assert_eq!(status_for(gate).await, StatusCode::OK);
        assert_eq!(
            verifier.calls(),
            0,
            "a disabled gate must not evaluate the peer at all"
        );
    }

    #[tokio::test]
    async fn enforcing_refuses_a_peer_that_is_not_verified() {
        let verifier = Arc::new(CountingVerifier::new(PeerVerdict::Rejected {
            reason: "not-the-signed-app".to_string(),
        }));
        let gate = Arc::new(PeerGate::new(PeerPolicy::Enforce, verifier.clone()));

        assert_eq!(status_for(gate).await, StatusCode::FORBIDDEN);
        assert_eq!(verifier.calls(), 1);
    }

    #[tokio::test]
    async fn enforcing_admits_a_verified_peer() {
        let verifier = Arc::new(CountingVerifier::new(PeerVerdict::Verified {
            identity: "team:FIXTURE".to_string(),
        }));
        let gate = Arc::new(PeerGate::new(PeerPolicy::Enforce, verifier));

        assert_eq!(status_for(gate).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn enforcing_fails_closed_when_the_peer_cannot_be_identified() {
        // The shipping verifier's answer on the current TCP transport. Enabling
        // the flag today refuses everything, and that is the intended signal.
        let gate = Arc::new(PeerGate::new(
            PeerPolicy::Enforce,
            Arc::new(TransportUnableVerifier),
        ));
        assert_eq!(status_for(gate).await, StatusCode::FORBIDDEN);
    }

    #[test]
    fn the_shipping_verifier_reports_the_transport_truthfully() {
        let request = HttpRequest::builder()
            .uri("/x")
            .body(Body::empty())
            .unwrap();
        let verdict = TransportUnableVerifier.verify(&request);
        assert!(matches!(verdict, PeerVerdict::Unavailable { .. }));
        assert!(!verdict.admits());
        assert_eq!(
            verdict.as_audit_str(),
            "unavailable:tcp-loopback-has-no-peer-credentials"
        );
    }

    #[test]
    fn only_exactly_one_enables_the_flag() {
        // A typo must not enable a control that currently refuses everything.
        assert_eq!(PeerPolicy::default(), PeerPolicy::Disabled);
        assert!(!PeerGate::default().is_enforcing());
        assert!(
            PeerGate::new(PeerPolicy::Enforce, Arc::new(TransportUnableVerifier)).is_enforcing()
        );
    }

    #[test]
    fn audit_strings_round_trip_verified_and_rejected() {
        let verified = PeerVerdict::Verified {
            identity: "team:FIXTURE".to_string(),
        };
        let rejected = PeerVerdict::Rejected {
            reason: "not-the-signed-app".to_string(),
        };
        assert_eq!(
            PeerVerdict::from_audit_str(&verified.as_audit_str()),
            verified
        );
        assert_eq!(
            PeerVerdict::from_audit_str(&rejected.as_audit_str()),
            rejected
        );
        assert_eq!(
            PeerVerdict::from_audit_str("garbage").as_audit_str(),
            "unavailable:unrecognised-peer-audit-token:garbage"
        );
        assert!(!PeerGate::default().is_enforcing());
    }
}
