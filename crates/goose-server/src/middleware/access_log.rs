//! HTTP access log middleware.
//!
//! Emits one line per request — method, path, status, latency — under the
//! `permagentd::http` target. This is the daemon's HTTP access log; without
//! it, mutations leave no trace at all (#568 was invisible for exactly this
//! reason).
//!
//! Never logs headers or query strings: the Bearer token rides the
//! Authorization header and the voice WebSocket token rides the query
//! string, so only `uri.path()` is emitted. WebSocket routes log the
//! upgrade request once (101); frames are not HTTP requests and never
//! appear here.

use axum::{extract::Request, http::Method, middleware::Next, response::Response};
use std::time::Instant;

/// High-frequency read-only polls (the 1s henry-status poll, liveness
/// probes) log at DEBUG so they don't drown the access log. Failures on
/// these paths (>= 400) are never demoted. Everything else is INFO.
/// Surface with `RUST_LOG=permagentd::http=debug`.
const NOISY_GET_PATHS: &[&str] = &["/api/henry/status", "/status", "/api/version"];

pub async fn http_access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();

    let response = next.run(request).await;

    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let noisy = method == Method::GET && NOISY_GET_PATHS.contains(&path.as_str());

    if noisy && status < 400 {
        tracing::debug!(target: "permagentd::http", %method, path, status, latency_ms, "request");
    } else {
        tracing::info!(target: "permagentd::http", %method, path, status, latency_ms, "request");
    }

    response
}
