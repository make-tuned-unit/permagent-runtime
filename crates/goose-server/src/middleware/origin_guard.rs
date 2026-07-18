//! Cross-origin request guard for the daemon (C1/C2 launch audit).
//!
//! A remote web page loaded in the user's browser can issue `fetch`/`XHR`/
//! `WebSocket`/`EventSource` requests at `http://127.0.0.1:3001` (or the
//! tailnet address). The loopback guard does NOT stop this — the user's own
//! browser IS a loopback peer — and CORS alone does not prevent the request
//! from *executing* server-side. This middleware rejects any browser request
//! whose `Origin` is not one of ours, closing the cross-site surface for
//! every route (including the deliberately unauthenticated browser bridge).
//!
//! Allowed origins:
//! - the Tauri webview: `tauri://localhost` (macOS/Linux), `https://tauri.localhost`
//!   and `http://tauri.localhost` (Windows / `useHttpsScheme` variants)
//! - loopback dev/served origins: `http(s)://localhost|127.x.x.x|[::1]` on any
//!   port (vite dev server, daemon-served `/ui`)
//! - same-origin requests: the `Origin`'s authority equals the request `Host`
//!   (a companion device that loaded `/ui` from `http://<tailnet-ip>:3001`
//!   talks back to that same authority)
//! - operator escape hatch: exact-match entries from the
//!   `PERMAGENT_ALLOWED_ORIGINS` env var (comma-separated), so a surprise
//!   webview origin can be unblocked without a rebuild.
//!
//! Requests WITHOUT an `Origin` header pass: native clients (CLI, the Tauri
//! Rust process, curl, server-to-server) send none, and same-origin GETs
//! usually omit it. `Origin: null` (sandboxed iframes, `file://` pages) is
//! rejected — it is attacker-reachable and never sent by our webview.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};

/// Extra allowed origins from the environment (comma-separated, exact match).
pub fn env_allowed_origins() -> Option<String> {
    std::env::var("PERMAGENT_ALLOWED_ORIGINS").ok()
}

/// Pure allowlist decision. `request_host` is the request's `Host` header (or
/// URI authority); `extra` is a comma-separated exact-match list (from
/// `PERMAGENT_ALLOWED_ORIGINS`). Kept env-free so tests need no env mutation.
pub fn origin_allowed(origin: &str, request_host: Option<&str>, extra: Option<&str>) -> bool {
    // Operator escape hatch: exact string match on the raw Origin value.
    if let Some(extra) = extra {
        if extra
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|allowed| allowed == origin)
        {
            return true;
        }
    }

    // Tauri webview origins (prod bundles).
    if matches!(
        origin,
        "tauri://localhost" | "https://tauri.localhost" | "http://tauri.localhost"
    ) {
        return true;
    }

    // scheme://authority — only http(s) beyond this point.
    let authority = match origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    {
        Some(rest) => rest,
        None => return false,
    };
    // An authority never contains '/', '?' or '#'; reject anything malformed
    // rather than allowlisting a crafted value like `http://localhost.evil.com`
    // (handled below by exact host parsing anyway) or `http://localhost/x`.
    if authority.is_empty() || authority.contains(['/', '?', '#']) {
        return false;
    }

    // Loopback origins on any port: localhost, 127.0.0.0/8, [::1].
    let host = authority_host(authority);
    if host.eq_ignore_ascii_case("localhost") || host == "[::1]" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        if ip.is_loopback() {
            return true;
        }
    }

    // Same-origin: the Origin's authority equals the request's Host header
    // (case-insensitive). Covers companion devices reaching the daemon-served
    // UI over the tailnet at whatever address the daemon is bound to.
    if let Some(request_host) = request_host {
        if authority.eq_ignore_ascii_case(request_host.trim()) {
            return true;
        }
    }

    false
}

/// Split `host[:port]` → host, tolerating a bracketed IPv6 literal.
fn authority_host(authority: &str) -> &str {
    if authority.starts_with('[') {
        if let Some(end) = authority.find(']') {
            return &authority[..=end];
        }
    }
    authority.split(':').next().unwrap_or(authority)
}

/// Middleware: 403 any request whose `Origin` header is present but not ours.
pub async fn require_allowed_origin(request: Request, next: Next) -> Result<Response, StatusCode> {
    let origin = match request.headers().get(header::ORIGIN) {
        None => return Ok(next.run(request).await),
        Some(value) => match value.to_str() {
            Ok(s) => s.to_owned(),
            // Non-UTF8 Origin: no legitimate client sends this.
            Err(_) => return Err(StatusCode::FORBIDDEN),
        },
    };

    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| request.uri().authority().map(|a| a.as_str().to_owned()));

    if origin_allowed(&origin, host.as_deref(), env_allowed_origins().as_deref()) {
        Ok(next.run(request).await)
    } else {
        tracing::warn!(
            target: "permagentd::auth",
            origin = %origin,
            path = %request.uri().path(),
            "rejected cross-origin request from a non-allowlisted Origin \
             (set PERMAGENT_ALLOWED_ORIGINS to extend the allowlist)"
        );
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tauri_webview_origins_pass() {
        assert!(origin_allowed("tauri://localhost", None, None));
        assert!(origin_allowed("https://tauri.localhost", None, None));
        assert!(origin_allowed("http://tauri.localhost", None, None));
    }

    #[test]
    fn loopback_dev_origins_pass() {
        assert!(origin_allowed("http://localhost:5273", None, None)); // vite dev
        assert!(origin_allowed("http://localhost:1520", None, None));
        assert!(origin_allowed("http://127.0.0.1:3001", None, None)); // served /ui
        assert!(origin_allowed("https://127.0.0.1:3001", None, None));
        assert!(origin_allowed("http://[::1]:3001", None, None));
        assert!(origin_allowed("http://localhost", None, None));
    }

    #[test]
    fn same_origin_host_match_passes() {
        // Companion device loaded /ui from the tailnet address.
        assert!(origin_allowed(
            "http://100.101.102.103:3001",
            Some("100.101.102.103:3001"),
            None
        ));
        assert!(origin_allowed(
            "http://hub.tailnet.ts.net:3001",
            Some("hub.tailnet.ts.net:3001"),
            None
        ));
        // Case-insensitive.
        assert!(origin_allowed(
            "http://Hub.Tailnet.ts.net:3001",
            Some("hub.tailnet.ts.net:3001"),
            None
        ));
    }

    #[test]
    fn remote_web_pages_are_rejected() {
        assert!(!origin_allowed("https://evil.example", None, None));
        assert!(!origin_allowed(
            "https://evil.example",
            Some("127.0.0.1:3001"),
            None
        ));
        // Cross-origin from another tailnet host: Origin ≠ Host.
        assert!(!origin_allowed(
            "http://other.machine:9999",
            Some("100.101.102.103:3001"),
            None
        ));
        // Deceptive hostnames.
        assert!(!origin_allowed("http://localhost.evil.example", None, None));
        assert!(!origin_allowed("http://127.0.0.1.evil.example", None, None));
        assert!(!origin_allowed("tauri://evil", None, None));
        // Opaque origin (sandboxed iframe / file://).
        assert!(!origin_allowed("null", None, None));
        assert!(!origin_allowed("", None, None));
        // Non-http(s) schemes.
        assert!(!origin_allowed("ftp://localhost", None, None));
    }

    #[test]
    fn env_extra_origins_exact_match() {
        assert!(origin_allowed(
            "https://surprise.webview",
            None,
            Some("https://surprise.webview")
        ));
        assert!(origin_allowed(
            "https://b.example",
            None,
            Some("https://a.example, https://b.example")
        ));
        // Exact match only — no prefixing.
        assert!(!origin_allowed(
            "https://surprise.webview.evil",
            None,
            Some("https://surprise.webview")
        ));
        // Emergency: even an opaque origin can be allowlisted explicitly.
        assert!(origin_allowed("null", None, Some("null")));
    }

    #[tokio::test]
    async fn middleware_passes_no_origin_and_allowed_rejects_foreign() {
        use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
        use tower::ServiceExt;

        let app = || {
            Router::new()
                .route("/x", get(|| async { "ok" }))
                .layer(axum::middleware::from_fn(require_allowed_origin))
        };

        // No Origin header (native client) → pass.
        let res = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Tauri origin → pass.
        let res = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/x")
                    .header("origin", "tauri://localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Same-origin via Host match → pass.
        let res = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/x")
                    .header("origin", "http://100.64.0.7:3001")
                    .header("host", "100.64.0.7:3001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Foreign web page → 403.
        let res = app()
            .oneshot(
                HttpRequest::builder()
                    .uri("/x")
                    .header("origin", "https://evil.example")
                    .header("host", "127.0.0.1:3001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
}
