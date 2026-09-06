//! Daemon control-plane auth audit — who presented a credential, for what, when.
//!
//! ## Why this exists
//!
//! `permagentd` binds `127.0.0.1:3001` behind a bearer token at
//! `~/.permagent/secrets/daemon_token.json`. That file is `0600` inside a `0700`
//! directory, which protects it from OTHER USERS. It does not — and on macOS
//! cannot — protect it from OTHER PROCESSES RUNNING AS THE SAME USER. Unix file
//! permissions have no sub-user granularity, so every process with this user's
//! uid can read the token and then hold exactly the authority the desktop app
//! holds.
//!
//! The daemon therefore CANNOT prevent same-user misuse today. What it can do is
//! make misuse **visible after the fact**: record which credential was admitted,
//! for which route, in which consequence class, with what result. This module is
//! that record. See `docs/design/daemon-trust-boundary.md` for the full honest
//! statement of what the boundary is and is not.
//!
//! ## What this is NOT
//!
//! This is detection, never prevention. An attacker who can run code as this
//! user can also read this table, and can delete or corrupt the database file
//! that holds it. The `BEFORE UPDATE` / `BEFORE DELETE` triggers make the rows
//! append-only **to SQL**, which stops a careless or a naive-SQL tamperer; it
//! does not stop `rm`. Treat the presence of a row as evidence and the absence
//! of a row as no evidence either way.
//!
//! ## Shape
//!
//! Follows the `sovereignty::record_egress` pattern exactly: an owned write
//! struct, a camelCase wire struct, a pure `_row` writer taking an explicit
//! pool (testable), and a global convenience wrapper that resolves the shared
//! Spectral pool. Table + append-only triggers live in
//! `crate::session::spectral_schema::apply_daemon_auth_audit_schema` (v43).

use sqlx::{Pool, Row, Sqlite};

/// What a route can do to the user if a caller reaches it. Recorded on every
/// audited event so the log can answer "did anything execute code / touch
/// secrets / spend money", not merely "how many requests were there".
///
/// Ordering is by blast radius, most severe first, and is the order the
/// classifier's rule table is evaluated in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteClass {
    /// Runs code, dispatches tools, drives the desktop/browser, or starts or
    /// stops the agent. The most severe class: reaching it is arbitrary local
    /// execution under the user's account.
    Execute,
    /// Reads, writes, or mints credential material — provider API keys, secret
    /// sources, OAuth flows, device pairing tokens, stream tokens.
    Secrets,
    /// Causes a paid provider call or a model/network download; costs the user
    /// money or bandwidth.
    Spend,
    /// Writes user data (projects, sessions, memories, config, schedules).
    Mutate,
    /// Reads user data (the Brain, sessions, people, projects, audit logs).
    Read,
    /// Liveness and build metadata. No user data.
    Status,
}

impl RouteClass {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteClass::Execute => "execute",
            RouteClass::Secrets => "secrets",
            RouteClass::Spend => "spend",
            RouteClass::Mutate => "mutate",
            RouteClass::Read => "read",
            RouteClass::Status => "status",
        }
    }

    /// Whether an *admitted* request of this class is worth a row.
    ///
    /// Reads and status polls are deliberately excluded: the desktop app polls
    /// `/api/henry/status` every second, and an append-only table with no
    /// retention story (the same gap `docs/design/soc2-scoping.md` names for
    /// `egress_audit`) must not grow by a row per poll. Denials are recorded
    /// regardless of class — see [`AuthOutcome::Denied`].
    pub fn is_audited_on_success(self) -> bool {
        matches!(
            self,
            RouteClass::Execute | RouteClass::Secrets | RouteClass::Spend | RouteClass::Mutate
        )
    }
}

/// Ordered classification rules: the first prefix that matches wins, so more
/// specific paths must appear before the prefixes that contain them.
///
/// This table is deliberately explicit rather than derived from the router.
/// Axum gives no consequence metadata, and a classifier that silently inherited
/// "unknown" for a new route would understate exactly the routes most worth
/// recording. The fallback below is conservative for the same reason.
const RULES: &[(&str, RouteClass)] = &[
    // ── Execute: code, tools, the desktop, the agent lifecycle ──
    ("/agent/call_tool", RouteClass::Execute),
    ("/agent/add_extension", RouteClass::Execute),
    ("/agent/remove_extension", RouteClass::Execute),
    ("/agent/set_container", RouteClass::Execute),
    ("/agent/start", RouteClass::Execute),
    ("/agent/stop", RouteClass::Execute),
    ("/agent/restart", RouteClass::Execute),
    ("/agent/resume", RouteClass::Execute),
    ("/agent/import_app", RouteClass::Execute),
    ("/terminal/", RouteClass::Execute),
    ("/api/desktop/launch", RouteClass::Execute),
    ("/api/browser/act", RouteClass::Execute),
    ("/api/browser/navigate", RouteClass::Execute),
    ("/recipes/create", RouteClass::Execute),
    ("/recipes/schedule", RouteClass::Execute),
    ("/schedule/create", RouteClass::Execute),
    ("/config/extensions", RouteClass::Execute),
    // ── Secrets: credential material in either direction ──
    ("/config/secret-sources", RouteClass::Secrets),
    ("/config/read", RouteClass::Secrets),
    ("/config/upsert", RouteClass::Secrets),
    ("/config/remove", RouteClass::Secrets),
    ("/config/providers", RouteClass::Secrets),
    ("/config/custom-providers", RouteClass::Secrets),
    ("/config/check_provider", RouteClass::Secrets),
    ("/config/set_provider", RouteClass::Secrets),
    ("/config/model-route", RouteClass::Secrets),
    ("/api/devices/pair", RouteClass::Secrets),
    ("/pair/claim", RouteClass::Secrets),
    ("/sse-token", RouteClass::Secrets),
    ("/integrations", RouteClass::Secrets),
    ("/handle_openrouter", RouteClass::Secrets),
    ("/handle_nanogpt", RouteClass::Secrets),
    ("/handle_tetrate", RouteClass::Secrets),
    ("/gateway/pair", RouteClass::Secrets),
    // ── Spend: paid provider calls and model/network downloads ──
    ("/reply", RouteClass::Spend),
    ("/sampling", RouteClass::Spend),
    ("/api/ollama/pull", RouteClass::Spend),
    ("/local-inference/download", RouteClass::Spend),
    ("/voice/synthesize", RouteClass::Spend),
    ("/voice/models", RouteClass::Spend),
    ("/voice/wake/models", RouteClass::Spend),
    ("/api/dictation/provision", RouteClass::Spend),
    ("/api/brain/search", RouteClass::Spend),
    ("/api/council/convene", RouteClass::Spend),
    // ── Status: liveness and build metadata only ──
    ("/status", RouteClass::Status),
    ("/api/version", RouteClass::Status),
    ("/api/henry/status", RouteClass::Status),
    ("/features", RouteClass::Status),
];

/// Classify a request by its method and path.
///
/// Unmatched paths fall back on the HTTP method: `GET`/`HEAD`/`OPTIONS` are
/// [`RouteClass::Read`], everything else is [`RouteClass::Mutate`]. The fallback
/// errs toward recording: a newly added POST route is audited from the day it
/// lands, without anyone remembering to update this table.
pub fn classify(method: &str, path: &str) -> RouteClass {
    for (prefix, class) in RULES {
        if path.starts_with(prefix) {
            return *class;
        }
    }
    // Some paths carry the id in the MIDDLE (`/sessions/{id}/reply`,
    // `/api/agents/{id}/secrets`), so a prefix rule cannot catch them and a
    // prefix broad enough to try would swallow sibling reads — `/api/agents/`
    // would drag the roster listing in with the credential endpoints.
    // Suffix-match those instead.
    if path.ends_with("/reply") {
        return RouteClass::Spend;
    }
    if path.ends_with("/secrets") || path.ends_with("/grants") {
        return RouteClass::Secrets;
    }
    match method {
        "GET" | "HEAD" | "OPTIONS" => RouteClass::Read,
        _ => RouteClass::Mutate,
    }
}

/// Whether the request was admitted or refused by the auth layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    /// A valid credential was presented and the handler ran.
    Admitted,
    /// No credential, an unrecognised one, or a fail-closed refusal (503 when
    /// the daemon holds no token at all). ALWAYS recorded, whatever the class:
    /// a process probing the daemon with a guessed token is the single most
    /// interesting thing this log can show.
    Denied,
}

impl AuthOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthOutcome::Admitted => "admitted",
            AuthOutcome::Denied => "denied",
        }
    }
}

/// Which class of credential the caller presented. Distinct from `principal`:
/// this says *what kind* of key opened the door, the principal says *which one*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// The master `daemon_token` — the hub's own app, the CLI, the in-process
    /// MCP tools, and anything else on this machine that read the file.
    Master,
    /// A per-device pairing token from the device registry.
    Device,
    /// A short-lived stream-scoped token minted by `/sse-token`.
    Stream,
    /// No credential was presented at all.
    None,
    /// A credential was presented but matched nothing. The interesting one:
    /// a same-user process probing the daemon with a guessed or stale token
    /// produces these, and nothing else on the machine does.
    Unrecognised,
}

impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CredentialKind::Master => "master",
            CredentialKind::Device => "device",
            CredentialKind::Stream => "stream",
            CredentialKind::None => "none",
            CredentialKind::Unrecognised => "unrecognised",
        }
    }
}

/// One auth event, as written.
#[derive(Debug, Clone)]
pub struct AuthEventRecord {
    pub outcome: AuthOutcome,
    /// `"master"`, a device id, or `None` when nothing was admitted.
    pub principal: Option<String>,
    pub credential: CredentialKind,
    pub class: RouteClass,
    pub method: String,
    /// Request path only. NEVER the query string: long-lived tokens ride
    /// `?token=` on the WebSocket and SSE rails (see `middleware/access_log.rs`,
    /// which strips it for the same reason), and an audit that leaked the
    /// credential it is auditing would be worse than no audit.
    pub path: String,
    /// HTTP status the caller received, when the request ran.
    pub status: Option<u16>,
    /// Peer-verification verdict, when peer verification is enabled. `None`
    /// while the feature is off — which is its state on every current build.
    /// See `crates/goose-server/src/middleware/peer_identity.rs`.
    pub peer: Option<String>,
}

/// One auth event, as read back over the wire.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthEventEntry {
    pub id: String,
    pub ts: String,
    pub outcome: String,
    pub principal: Option<String>,
    pub credential: String,
    pub class: String,
    pub method: String,
    pub path: String,
    pub status: Option<i64>,
    pub peer: Option<String>,
}

/// Insert one auth event (pure — explicit pool; used by the wrapper and tests).
pub async fn record_auth_event_row(
    pool: &Pool<Sqlite>,
    rec: &AuthEventRecord,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::now_v7().to_string();
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query(
        "INSERT INTO daemon_auth_audit \
         (id, ts, outcome, principal, credential, class, method, path, status, peer) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(ts)
    .bind(rec.outcome.as_str())
    .bind(rec.principal.as_deref())
    .bind(rec.credential.as_str())
    .bind(rec.class.as_str())
    .bind(rec.method.as_str())
    .bind(rec.path.as_str())
    .bind(rec.status.map(i64::from))
    .bind(rec.peer.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

/// Record an auth event against the shared Spectral pool.
///
/// Best-effort by design, and this is the one place it differs from
/// `sovereignty::record_egress`: an egress-audit failure may refuse the cloud
/// call, because an unlogged egress breaks the sovereignty promise. There is no
/// equivalent promise here that is worth locking the user out of their own
/// daemon to keep — a hard-failing auth audit would turn a full disk into a
/// total loss of access. The failure is logged loudly at `error` instead, so a
/// silently-not-auditing daemon is visible in the daemon log.
pub async fn record_auth_event(rec: AuthEventRecord) {
    let result = match crate::session::SessionManager::instance()
        .pool_clone()
        .await
    {
        Ok(pool) => record_auth_event_row(&pool, &rec).await,
        Err(e) => Err(e.context("auth audit unavailable (no db pool)")),
    };
    if let Err(e) = result {
        tracing::error!(
            target: "permagentd::authaudit",
            outcome = rec.outcome.as_str(),
            class = rec.class.as_str(),
            method = %rec.method,
            path = %rec.path,
            error = %format!("{e:#}"),
            "failed to write daemon auth audit row — this request is NOT in the auth audit log"
        );
    }
}

/// Read the most recent auth events, newest first (pure — explicit pool).
pub async fn recent_auth_event_rows(
    pool: &Pool<Sqlite>,
    limit: i64,
) -> anyhow::Result<Vec<AuthEventEntry>> {
    let rows = sqlx::query(
        "SELECT id, ts, outcome, principal, credential, class, method, path, status, peer \
         FROM daemon_auth_audit ORDER BY ts DESC, id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| AuthEventEntry {
            id: r.get("id"),
            ts: r.get("ts"),
            outcome: r.get("outcome"),
            principal: r.get("principal"),
            credential: r.get("credential"),
            class: r.get("class"),
            method: r.get("method"),
            path: r.get("path"),
            status: r.get("status"),
            peer: r.get("peer"),
        })
        .collect())
}

/// Read the most recent auth events against the shared Spectral pool.
pub async fn recent_auth_events(limit: i64) -> anyhow::Result<Vec<AuthEventEntry>> {
    let pool = crate::session::SessionManager::instance()
        .pool_clone()
        .await?;
    recent_auth_event_rows(&pool, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_dispatch_is_execute() {
        // The single most consequential route behind the token: it dispatches a
        // tool straight through `ExtensionManager::dispatch_tool_call`, which
        // does not consult the confirmation router that gates model-initiated
        // calls.
        assert_eq!(classify("POST", "/agent/call_tool"), RouteClass::Execute);
        assert_eq!(
            classify("POST", "/terminal/supervised/output"),
            RouteClass::Execute
        );
        assert_eq!(classify("POST", "/api/desktop/launch"), RouteClass::Execute);
    }

    #[test]
    fn credential_bearing_routes_are_secrets() {
        assert_eq!(classify("POST", "/config/read"), RouteClass::Secrets);
        assert_eq!(classify("POST", "/config/upsert"), RouteClass::Secrets);
        assert_eq!(
            classify("GET", "/config/secret-sources"),
            RouteClass::Secrets
        );
        assert_eq!(classify("POST", "/api/devices/pair"), RouteClass::Secrets);
        assert_eq!(classify("POST", "/sse-token"), RouteClass::Secrets);
    }

    #[test]
    fn provider_invoking_routes_are_spend() {
        assert_eq!(classify("POST", "/reply"), RouteClass::Spend);
        // The session id sits in the middle, so this exercises the suffix rule.
        assert_eq!(
            classify("POST", "/sessions/abc-123/reply"),
            RouteClass::Spend
        );
        assert_eq!(classify("POST", "/api/ollama/pull"), RouteClass::Spend);
    }

    #[test]
    fn agent_credential_endpoints_are_secrets_but_the_roster_is_not() {
        // A prefix rule on `/api/agents/` would have classified the roster
        // listing as `secrets` and put a row in the audit on every load.
        assert_eq!(
            classify("GET", "/api/agents/abc/secrets"),
            RouteClass::Secrets
        );
        assert_eq!(
            classify("GET", "/api/agents/abc/grants"),
            RouteClass::Secrets
        );
        assert_eq!(classify("GET", "/api/agents/roster"), RouteClass::Read);
        assert!(!classify("GET", "/api/agents/roster").is_audited_on_success());
    }

    #[test]
    fn unknown_routes_fall_back_on_method() {
        assert_eq!(classify("GET", "/api/projects"), RouteClass::Read);
        assert_eq!(classify("POST", "/api/projects"), RouteClass::Mutate);
        assert_eq!(classify("DELETE", "/api/projects/p1"), RouteClass::Mutate);
        assert_eq!(classify("PATCH", "/api/workspaces/w1"), RouteClass::Mutate);
    }

    #[test]
    fn status_polls_are_status_and_are_not_audited_on_success() {
        // The desktop app polls these; a row per poll would bury the log.
        for path in ["/status", "/api/version", "/api/henry/status"] {
            let class = classify("GET", path);
            assert_eq!(class, RouteClass::Status, "{path}");
            assert!(!class.is_audited_on_success(), "{path}");
        }
        assert!(!RouteClass::Read.is_audited_on_success());
    }

    #[test]
    fn every_consequential_class_is_audited_on_success() {
        for class in [
            RouteClass::Execute,
            RouteClass::Secrets,
            RouteClass::Spend,
            RouteClass::Mutate,
        ] {
            assert!(class.is_audited_on_success(), "{class:?}");
        }
    }

    async fn memory_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::apply_daemon_auth_audit_schema(&pool)
            .await
            .unwrap();
        pool
    }

    fn denial(path: &str) -> AuthEventRecord {
        AuthEventRecord {
            outcome: AuthOutcome::Denied,
            principal: None,
            credential: CredentialKind::None,
            class: classify("POST", path),
            method: "POST".to_string(),
            path: path.to_string(),
            status: Some(401),
            peer: None,
        }
    }

    #[tokio::test]
    async fn the_audit_records_exactly_what_it_claims_to() {
        let pool = memory_pool().await;

        record_auth_event_row(
            &pool,
            &AuthEventRecord {
                outcome: AuthOutcome::Admitted,
                principal: Some("device-7".to_string()),
                credential: CredentialKind::Device,
                class: classify("POST", "/agent/call_tool"),
                method: "POST".to_string(),
                path: "/agent/call_tool".to_string(),
                status: Some(200),
                peer: None,
            },
        )
        .await
        .unwrap();
        record_auth_event_row(&pool, &denial("/config/read"))
            .await
            .unwrap();

        let rows = recent_auth_event_rows(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2);

        // Newest first.
        let denied = &rows[0];
        assert_eq!(denied.outcome, "denied");
        assert_eq!(denied.principal, None);
        assert_eq!(denied.credential, "none");
        assert_eq!(denied.class, "secrets");
        assert_eq!(denied.path, "/config/read");
        assert_eq!(denied.status, Some(401));

        let admitted = &rows[1];
        assert_eq!(admitted.outcome, "admitted");
        assert_eq!(admitted.principal.as_deref(), Some("device-7"));
        assert_eq!(admitted.credential, "device");
        assert_eq!(admitted.class, "execute");
        assert_eq!(admitted.method, "POST");
        assert_eq!(admitted.status, Some(200));
        assert_eq!(admitted.peer, None);
    }

    #[tokio::test]
    async fn rows_are_append_only_at_the_database() {
        let pool = memory_pool().await;
        record_auth_event_row(&pool, &denial("/agent/call_tool"))
            .await
            .unwrap();

        assert!(
            sqlx::query("UPDATE daemon_auth_audit SET outcome = 'admitted'")
                .execute(&pool)
                .await
                .is_err(),
            "a rewritable auth audit is a lying auth audit"
        );
        assert!(
            sqlx::query("DELETE FROM daemon_auth_audit")
                .execute(&pool)
                .await
                .is_err(),
            "the audit must not be erasable through SQL"
        );

        // The row survived both attempts.
        assert_eq!(recent_auth_event_rows(&pool, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn schema_application_is_idempotent() {
        let pool = memory_pool().await;
        crate::session::spectral_schema::apply_daemon_auth_audit_schema(&pool)
            .await
            .unwrap();
        record_auth_event_row(&pool, &denial("/config/read"))
            .await
            .unwrap();
        assert_eq!(recent_auth_event_rows(&pool, 10).await.unwrap().len(), 1);
    }
}
