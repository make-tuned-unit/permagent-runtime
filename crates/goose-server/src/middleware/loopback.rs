//! Loopback-only guard for routes that must never be reachable off-box (#630).
//!
//! The browser bridge (`/api/browser/navigate` + `/api/browser/content/*`) is
//! unauthenticated by design: it's driven by the in-process MCP tool, which has
//! no daemon token. That is safe while the daemon binds `127.0.0.1` — every peer
//! is loopback. But the multi-device / tailnet story binds a *routable* address,
//! and at that point an unauthenticated `navigate` lets a remote peer drive the
//! user's browser, and `content/read` lets it exfiltrate the current tab. This
//! guard closes that surface without token plumbing: it rejects any request
//! whose peer is not a loopback address, so the bridge stays strictly local
//! regardless of bind host.
//!
//! In-process and same-box webview callers (`127.0.0.1` / `::1`) pass; network
//! peers get `403`. It reads the peer via `ConnectInfo<SocketAddr>`, so the
//! server MUST be served with `into_make_service_with_connect_info::<SocketAddr>()`
//! — if the peer address is absent the extractor rejects and the guard fails
//! closed (denies), which is the safe default for a security control.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

pub async fn require_loopback(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if peer.ip().is_loopback() {
        Ok(next.run(request).await)
    } else {
        tracing::warn!(
            target: "permagentd::browser",
            %peer,
            path = %request.uri().path(),
            "rejected non-loopback request to the loopback-only browser bridge"
        );
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(require_loopback))
    }

    async fn status_for(ip: IpAddr) -> StatusCode {
        // Mirror how axum's ConnectInfo make-service supplies the peer: the
        // extractor reads `ConnectInfo<SocketAddr>` from request extensions.
        let request = HttpRequest::builder()
            .uri("/x")
            .extension(ConnectInfo(SocketAddr::new(ip, 40000)))
            .body(Body::empty())
            .unwrap();
        app().oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn loopback_peers_pass() {
        assert_eq!(
            status_for(IpAddr::V4(Ipv4Addr::LOCALHOST)).await,
            StatusCode::OK
        );
        assert_eq!(
            status_for(IpAddr::V6(Ipv6Addr::LOCALHOST)).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn off_box_peers_are_forbidden() {
        assert_eq!(
            status_for(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_for(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 4))).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn missing_peer_fails_closed() {
        // No ConnectInfo extension at all → the extractor rejects → the guard
        // denies rather than serving the bridge unguarded.
        let request = HttpRequest::builder()
            .uri("/x")
            .body(Body::empty())
            .unwrap();
        let status = app().oneshot(request).await.unwrap().status();
        assert_ne!(status, StatusCode::OK);
    }
}
