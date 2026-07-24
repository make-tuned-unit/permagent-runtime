//! Integration tests for the mesh pool engine (`mesh::pool`) against mocked
//! peer endpoints: health probing, scheduling, timeout fallback, peer death
//! mid-request, ladder exhaustion, and the workload/trust invariants.
//!
//! Every engine here is constructed directly with explicit peers and tuning
//! (including an injected `assume_trusted` and a mock `local_endpoint`), so no
//! test touches process env — env-mutating tests flake under parallel
//! `cargo test`.

use std::time::Duration;

use permagent::cost_router::{mesh_gate, MeshRoute, Tier};
use permagent::mesh::pool::{GenerateRequest, PeerConfig, PoolEngine, ServedBy, Tuning};
use permagent::mesh::Workload;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Tuning for tests: no background re-probing interference (huge intervals),
/// a short request budget where a test wants timeouts, trust injected, and
/// the "local" rung pointed at a mock.
fn tuning(local_endpoint: String, request_timeout: Duration, trusted: bool) -> Tuning {
    Tuning {
        probe_interval: Duration::from_secs(3600),
        probe_timeout: Duration::from_secs(2),
        request_timeout,
        stale_after: Duration::from_secs(3600),
        max_inflight: 1,
        local_endpoint,
        assume_trusted: Some(trusted),
        // Deterministic: no background prober races with request-time health
        // updates; each test drives `probe_now()` explicitly.
        autostart_prober: false,
    }
}

fn batch_request(model: &str) -> GenerateRequest {
    GenerateRequest {
        session_id: None,
        model: model.to_string(),
        prompt: "hello".to_string(),
        system: None,
        options: None,
        keep_alive: None,
        timeout: None,
        workload: Workload::Batch,
    }
}

/// Mount a healthy Ollama-shaped peer: `/api/tags` lists `model`,
/// `/api/generate` answers `reply`.
async fn mount_ollama_peer(server: &MockServer, model: &str, reply: &str) {
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "models": [{ "name": model }] })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": reply })))
        .mount(server)
        .await;
}

/// Mount an unreachable-shaped peer that answers both probe endpoints with 500.
async fn mount_sick_peer(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(500))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(500))
        .mount(server)
        .await;
}

/// An endpoint that refuses connections (bind, take the port, drop).
fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

// ── Health probing ──────────────────────────────────────────────────────────

#[tokio::test]
async fn probe_marks_ollama_peer_healthy_and_collects_models() {
    let up = MockServer::start().await;
    mount_ollama_peer(&up, "qwen2.5:7b", "ok").await;
    let down = MockServer::start().await;
    mount_sick_peer(&down).await;

    let engine = PoolEngine::with_config(
        vec![
            PeerConfig::new(up.uri(), "up", true),
            PeerConfig::new(down.uri(), "down", true),
        ],
        tuning("http://127.0.0.1:9".into(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    let statuses = engine.peer_statuses();
    assert!(statuses[0].healthy, "reachable peer with models is healthy");
    assert_eq!(statuses[0].models, vec!["qwen2.5:7b".to_string()]);
    assert!(!statuses[1].healthy, "a 500-ing peer is unhealthy");

    // The #717 gate now runs on live inputs: a healthy trusted pool admits batch.
    assert_eq!(
        mesh_gate(engine.gate_inputs(Workload::Batch)),
        MeshRoute::UseMesh,
        "live gate inputs from the engine activate the cost-router mesh tier"
    );
}

#[tokio::test]
async fn probe_falls_back_to_openai_style_model_listing() {
    // A llama.cpp-server-shaped peer: no /api/tags, but /v1/models answers.
    let peer = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&peer)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "qwen2.5:7b" }] })),
        )
        .mount(&peer)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(peer.uri(), "llamacpp", true)],
        tuning("http://127.0.0.1:9".into(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    let statuses = engine.peer_statuses();
    assert!(
        statuses[0].healthy,
        "OpenAI-compatible peers are first-class"
    );
    assert_eq!(statuses[0].models, vec!["qwen2.5:7b".to_string()]);
}

// ── Dispatch + scheduling ───────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_lands_on_the_healthy_trusted_peer() {
    let healthy = MockServer::start().await;
    mount_ollama_peer(&healthy, "qwen2.5:7b", "from-pool").await;
    let sick = MockServer::start().await;
    mount_sick_peer(&sick).await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "never" })))
        .expect(0)
        .mount(&sick)
        .await;

    let engine = PoolEngine::with_config(
        vec![
            PeerConfig::new(sick.uri(), "sick", true),
            PeerConfig::new(healthy.uri(), "healthy", true),
        ],
        tuning("http://127.0.0.1:9".into(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("dispatch succeeds via the healthy peer");
    assert_eq!(resp.text, "from-pool");
    assert_eq!(
        resp.served_by,
        ServedBy::PoolPeer {
            label: "healthy".into()
        }
    );
    sick.verify().await;
}

/// The scheduled-Librarian regression (coordinator finding 1): a configured,
/// trusted, healthy pool with NO local model server must still serve batch —
/// nothing may insist on localhost first.
#[tokio::test]
async fn pool_up_local_down_batch_still_dispatches_to_pool() {
    let pool = MockServer::start().await;
    mount_ollama_peer(&pool, "qwen2.5:7b", "pool-served").await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(pool.uri(), "mini", true)],
        // The local rung is a dead port — this machine runs no Ollama.
        tuning(dead_endpoint(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    // A warm-load-shaped request (keep_alive + generous budget) rides the
    // same ladder the batch will: it must land on the pool, not abort on
    // the dead localhost.
    let mut warm = batch_request("qwen2.5:7b");
    warm.keep_alive = Some("1800s".into());
    warm.timeout = Some(Duration::from_secs(10));
    let resp = engine
        .generate_with(&warm)
        .await
        .expect("warm reaches the pool");
    assert_eq!(resp.text, "pool-served");
    assert!(matches!(resp.served_by, ServedBy::PoolPeer { .. }));
}

#[tokio::test]
async fn busy_peer_is_routed_around_and_capacity_is_released() {
    let a = MockServer::start().await;
    mount_ollama_peer(&a, "qwen2.5:7b", "from-a").await;
    let b = MockServer::start().await;
    mount_ollama_peer(&b, "qwen2.5:7b", "from-b").await;

    let engine = PoolEngine::with_config(
        vec![
            PeerConfig::new(a.uri(), "a", true),
            PeerConfig::new(b.uri(), "b", true),
        ],
        tuning("http://127.0.0.1:9".into(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    // Occupy one peer (whichever the scheduler picks first) at its cap of 1.
    let lease = engine.lease_for(Some("qwen2.5:7b"));
    assert!(lease.is_pool_peer());
    let occupied = lease.endpoint().to_string();
    let expected_other = if occupied == a.uri() {
        "from-b"
    } else {
        "from-a"
    };

    // The next unit must land on the OTHER peer — least-loaded, busy routed around.
    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("free peer serves");
    assert_eq!(resp.text, expected_other);

    lease.succeed();

    // With one peer and it busy, the pool contributes nothing: local serves.
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "local" })))
        .mount(&local)
        .await;
    let single = PoolEngine::with_config(
        vec![PeerConfig::new(a.uri(), "a", true)],
        tuning(local.uri(), Duration::from_secs(5), true),
    );
    single.probe_now().await;
    let held = single.lease_for(None);
    assert!(held.is_pool_peer());
    let resp = single
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("local rung serves while the only peer is busy");
    assert_eq!(resp.served_by, ServedBy::Local);
    held.succeed();
}

// ── Fault tolerance: timeout, death mid-request, ladder exhaustion ─────────

#[tokio::test]
async fn timeout_falls_back_to_local_and_quarantines_the_peer() {
    let slow = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "models": [{ "name": "qwen2.5:7b" }] })),
        )
        .mount(&slow)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "response": "too-late" }))
                .set_delay(Duration::from_millis(800)),
        )
        .mount(&slow)
        .await;
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "response": "local-answer" })),
        )
        .mount(&local)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(slow.uri(), "slow", true)],
        tuning(local.uri(), Duration::from_millis(150), true),
    );
    engine.probe_now().await;

    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("the ladder must absorb a slow peer");
    assert_eq!(resp.text, "local-answer");
    assert_eq!(resp.served_by, ServedBy::Local);
    assert!(
        !engine.peer_statuses()[0].healthy,
        "a timed-out peer is quarantined immediately, not on the next probe"
    );
    assert_eq!(
        engine.peer_statuses()[0].inflight,
        0,
        "the failed request released its capacity slot"
    );
}

#[tokio::test]
async fn death_mid_request_falls_back_without_poisoning_the_caller() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // A raw TCP peer: healthy on /api/tags (Connection: close), then dies
    // mid-body on /api/generate — the llama.cpp-RPC failure mode the engine
    // exists to absorb.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                if head.contains("/api/tags") {
                    let body = r#"{"models":[{"name":"qwen2.5:7b"}]}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                } else {
                    // Promise a long body, deliver a fragment, die.
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100000\r\n\r\n{\"response\":\"par",
                        )
                        .await;
                    let _ = sock.flush().await;
                }
            });
        }
    });

    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "rescued" })))
        .mount(&local)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(format!("http://{addr}"), "dying", true)],
        tuning(local.uri(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;
    assert!(
        engine.peer_statuses()[0].healthy,
        "the peer probes healthy first"
    );

    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("a peer dying mid-body must not surface as caller failure");
    assert_eq!(resp.text, "rescued");
    assert_eq!(resp.served_by, ServedBy::Local);
    assert!(
        !engine.peer_statuses()[0].healthy,
        "the dead peer is quarantined"
    );
}

#[tokio::test]
async fn ladder_exhaustion_returns_the_cheap_cloud_escalation() {
    // Peer unreachable AND local unreachable: the engine gives a structured
    // error carrying the #717 handoff — never a hang, never an abort.
    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(dead_endpoint(), "gone", true)],
        tuning(dead_endpoint(), Duration::from_secs(2), true),
    );
    engine.probe_now().await;

    let err = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect_err("both rungs down is a terminal engine failure");
    assert!(!err.message.is_empty());
    assert_eq!(
        err.escalate_to,
        Some(Tier::CheapCloud),
        "batch exhaustion hands off to the cost-router's cheap-cloud tier"
    );
}

// ── The invariants: workload wall + trusted-only ────────────────────────────

#[tokio::test]
async fn interactive_requests_never_touch_the_pool() {
    let peer = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "models": [{ "name": "qwen2.5:7b" }] })),
        )
        .mount(&peer)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "never" })))
        .expect(0)
        .mount(&peer)
        .await;
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "local" })))
        .mount(&local)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(peer.uri(), "mini", true)],
        tuning(local.uri(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    let mut req = batch_request("qwen2.5:7b");
    req.workload = Workload::Interactive;
    let resp = engine
        .generate_with(&req)
        .await
        .expect("interactive serves locally");
    assert_eq!(resp.served_by, ServedBy::Local);
    // Even a healthy, trusted, idle pool received zero interactive requests.
    peer.verify().await;
}

#[tokio::test]
async fn untrusted_peers_never_receive_requests() {
    // Per-peer untrusted (healthy, idle) — never dispatched.
    let peer = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "models": [{ "name": "qwen2.5:7b" }] })),
        )
        .mount(&peer)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "never" })))
        .expect(0)
        .mount(&peer)
        .await;
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "local" })))
        .mount(&local)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(peer.uri(), "stranger", false)],
        tuning(local.uri(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;
    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("local serves");
    assert_eq!(resp.served_by, ServedBy::Local);

    // Global trust off — a trusted peer is still never used.
    let engine_off = PoolEngine::with_config(
        vec![PeerConfig::new(peer.uri(), "mini", true)],
        tuning(local.uri(), Duration::from_secs(5), false),
    );
    engine_off.probe_now().await;
    let resp = engine_off
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("local serves");
    assert_eq!(resp.served_by, ServedBy::Local);

    peer.verify().await;
}

#[tokio::test]
async fn model_not_served_by_pool_stays_local() {
    let peer = MockServer::start().await;
    mount_ollama_peer(&peer, "llama3:8b", "wrong-model").await;
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "response": "local" })))
        .mount(&local)
        .await;

    let engine = PoolEngine::with_config(
        vec![PeerConfig::new(peer.uri(), "mini", true)],
        tuning(local.uri(), Duration::from_secs(5), true),
    );
    engine.probe_now().await;

    let resp = engine
        .generate_with(&batch_request("qwen2.5:7b"))
        .await
        .expect("local serves the model the pool lacks");
    assert_eq!(resp.served_by, ServedBy::Local);
}
