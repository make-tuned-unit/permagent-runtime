use super::http::{parse_response, WireQuestions, WireRequest, MAX_RESPONSE_BYTES};
use super::types::INJECTION_RAIL;
use super::*;
use crate::config::GooseMode;
use crate::session::{SessionManager, SessionType};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../docs/research/jev-evidence/jev-live-smoke-2026-09-21.json"
    ))
    .unwrap()
}

fn state(class: StateClass) -> DecisionState {
    DecisionState {
        text: fixture()["request"]["state"].as_str().unwrap().into(),
        class,
    }
}

fn questions() -> Vec<Question> {
    vec![
        Question::noul(
            "is_urgent",
            "Does this convey urgency?",
            "Explicitly time-sensitive",
            "No urgency expressed",
        )
        .unwrap(),
        Question::choice("department", "Which team should handle this?")
            .option("billing", "Payments, invoicing, refunds")
            .option("technical", "Bugs, outages, integrations")
            .option("sales", "Pricing, upgrades, new accounts")
            .exhaustive()
            .build()
            .unwrap(),
        Question::score(
            "frustration",
            "How frustrated is the customer?",
            vec!["Calm".into(), "Frustrated".into(), "Very angry".into()],
        )
        .unwrap(),
    ]
}

fn backend(endpoint: String) -> SystemOneHttpBackend {
    backend_with_key(endpoint, "test-secret")
}

fn backend_with_key(endpoint: String, api_key: &str) -> SystemOneHttpBackend {
    SystemOneHttpBackend::new(
        endpoint,
        "jev-latest".into(),
        api_key.into(),
        HttpTimeouts::default(),
    )
    .unwrap()
    .with_billing(JudgeBilling::LocalFree)
}

fn noul_questions(count: usize) -> Vec<Question> {
    (0..count)
        .map(|index| Question::noul(format!("q{index}"), "Is it true?", "Yes", "No").unwrap())
        .collect()
}

fn fallback() -> DeterministicBackend {
    DeterministicBackend::new(BTreeMap::from([("q0".into(), Answer::Noul { noul: 0.25 })]))
}

fn single_noul_response_with_padding(padding_len: usize) -> String {
    json!({
        "model": "test",
        "answers": { "q0": { "type": "noul", "noul": 0.75 } },
        "usage": { "input_tokens": 1, "output_tokens": 1 },
        "padding": "x".repeat(padding_len),
    })
    .to_string()
}

#[test]
fn verbatim_fixture_request_and_builder_rail() {
    let fixture = fixture();
    let mut questions = questions();
    let state = state(StateClass::Public);
    let guarded = serde_json::to_value(WireRequest {
        state: &state.text,
        model: "jev-latest",
        questions: WireQuestions(&questions),
    })
    .unwrap();
    let mut expected = fixture["request"].clone();
    for question in expected["questions"].as_object_mut().unwrap().values_mut() {
        question["instructions"] = json!(format!(
            "{} {INJECTION_RAIL}",
            question["instructions"].as_str().unwrap()
        ));
    }
    assert_eq!(guarded, expected);
    // The historical fixture predates the rail. Only this private wire test
    // restores its raw instructions; public construction cannot bypass it.
    for question in &mut questions {
        question.instructions = fixture["request"]["questions"][question.id()]["instructions"]
            .as_str()
            .unwrap()
            .into();
    }
    let raw = serde_json::to_value(WireRequest {
        state: &state.text,
        model: "jev-latest",
        questions: WireQuestions(&questions),
    })
    .unwrap();
    assert_eq!(raw, fixture["request"]);
}

#[test]
fn live_fixture_answers_are_exact() {
    assert_eq!(
        parse_response(&fixture()["response"].to_string(), &questions()).unwrap(),
        vec![
            Answer::Noul { noul: 0.95 },
            Answer::Choice {
                choice: "billing".into(),
                confidence: 0.81,
                probabilities: BTreeMap::from([
                    ("billing".into(), 0.88),
                    ("technical".into(), 0.12),
                    ("sales".into(), 0.0)
                ]),
            },
            Answer::Score {
                score: 1.04,
                confidence: 0.94,
                legend: BTreeMap::from([
                    ("0".into(), "Calm".into()),
                    ("1".into(), "Frustrated".into()),
                    ("2".into(), "Very angry".into())
                ]),
                probabilities: BTreeMap::from([
                    ("0".into(), 0.0),
                    ("1".into(), 0.96),
                    ("2".into(), 0.04)
                ]),
            },
        ]
    );
}

struct StubBackend {
    locality: DataLocality,
    result: Result<Vec<Answer>, DecisionError>,
    calls: AtomicUsize,
}

#[async_trait]
impl DecisionBackend for StubBackend {
    fn billing(&self) -> JudgeBilling {
        JudgeBilling::LocalFree
    }
    fn name(&self) -> &str {
        "stub"
    }
    fn data_locality(&self) -> DataLocality {
        self.locality
    }
    async fn decide_with_usage(
        &self,
        _: &DecisionState,
        _: &[Question],
    ) -> Result<DecisionResponse, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone().map(|answers| DecisionResponse {
            answers,
            usage: DecisionUsage { input_tokens: None },
        })
    }
}

#[tokio::test]
async fn locality_refuses_before_backend_call() {
    let backend = StubBackend {
        locality: DataLocality::Cloud,
        result: Ok(vec![Answer::Noul { noul: 0.75 }]),
        calls: AtomicUsize::new(0),
    };
    assert_eq!(
        decide_guarded(&backend, &state(StateClass::Private), &noul_questions(1)).await,
        Err(DecisionError::RefusedByLocality)
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        decide_guarded(&backend, &state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn endpoint_locality_fails_closed() {
    for endpoint in [
        "http://127.0.0.1:8000",
        "http://127.42.1.2",
        "http://[::1]:8000",
        "http://localhost",
    ] {
        assert_eq!(
            endpoint_locality(endpoint),
            DataLocality::Local,
            "{endpoint}"
        );
    }
    for endpoint in [
        "not a URL",
        "https://api.typesafe.ai",
        "http://192.168.1.2",
        "http://localhost.example",
        "http://[::ffff:127.0.0.1]",
        "file://localhost/tmp",
    ] {
        assert_eq!(
            endpoint_locality(endpoint),
            DataLocality::Cloud,
            "{endpoint}"
        );
    }
}

#[test]
fn request_url_accepts_base_and_full_systemone_paths() {
    let cases = [
        (
            "https://api.typesafe.ai",
            "https://api.typesafe.ai/v1/systemone",
        ),
        (
            "https://api.typesafe.ai/v1/systemone",
            "https://api.typesafe.ai/v1/systemone",
        ),
        (
            "https://api.typesafe.ai/v1/systemone/",
            "https://api.typesafe.ai/v1/systemone",
        ),
        (
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/v1/systemone",
        ),
        (
            "http://127.0.0.1:8080/proxy",
            "http://127.0.0.1:8080/proxy/v1/systemone",
        ),
    ];
    for (endpoint, expected) in cases {
        assert_eq!(
            backend(endpoint.into()).request_url().unwrap().as_str(),
            expected
        );
    }
}

#[tokio::test]
async fn private_loopback_posts_fixture_and_cloud_direct_call_refuses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("Authorization", "Bearer test-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&fixture()["response"]))
        .expect(1)
        .mount(&server)
        .await;
    let answers = decide_guarded(
        &backend(server.uri()),
        &state(StateClass::Private),
        &questions(),
    )
    .await
    .unwrap();
    assert_eq!(
        answers,
        parse_response(&fixture()["response"].to_string(), &questions()).unwrap()
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(request["state"], fixture()["request"]["state"]);
    for question in request["questions"].as_object().unwrap().values() {
        assert_eq!(
            question["instructions"]
                .as_str()
                .unwrap()
                .matches(INJECTION_RAIL)
                .count(),
            1
        );
    }
    let invalid = backend("not a URL".into());
    assert_eq!(
        invalid
            .decide(&state(StateClass::Private), &questions())
            .await,
        Err(DecisionError::RefusedByLocality)
    );
    assert_eq!(
        backend("https://api.typesafe.ai".into())
            .decide(&state(StateClass::Private), &questions())
            .await,
        Err(DecisionError::RefusedByLocality)
    );
}

#[tokio::test]
async fn empty_state_is_rejected_before_backend_call() {
    let backend = StubBackend {
        locality: DataLocality::Local,
        result: Ok(vec![Answer::Noul { noul: 0.75 }]),
        calls: AtomicUsize::new(0),
    };
    let empty = DecisionState {
        text: "  \n\t".into(),
        class: StateClass::Public,
    };
    assert_eq!(
        decide_guarded(&backend, &empty, &noul_questions(1)).await,
        Err(DecisionError::Validation("empty state".into()))
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn twenty_five_questions_make_twenty_and_five_in_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/systemone"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let answers: serde_json::Map<String, Value> = body["questions"].as_object().unwrap().keys().map(|id| {
                let index: usize = id.strip_prefix('q').unwrap().parse().unwrap();
                (id.clone(), json!({"type": "noul", "noul": index as f64 / 25.0}))
            }).collect();
            ResponseTemplate::new(200).set_body_json(json!({"model": "test", "answers": answers, "usage": {"input_tokens": 1, "output_tokens": 1}}))
        }).expect(2).mount(&server).await;
    let answers = decide_guarded(
        &backend(server.uri()),
        &state(StateClass::Private),
        &noul_questions(25),
    )
    .await
    .unwrap();
    assert_eq!(
        answers,
        (0..25)
            .map(|index| Answer::Noul {
                noul: index as f64 / 25.0
            })
            .collect::<Vec<_>>()
    );
    let requests = server.received_requests().await.unwrap();
    let bodies: Vec<Value> = requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(
        bodies
            .iter()
            .map(|b| b["questions"].as_object().unwrap().len())
            .collect::<Vec<_>>(),
        vec![20, 5]
    );
    for body in &bodies {
        assert_eq!(body["state"], fixture()["request"]["state"]);
    }
    for index in 0..25 {
        assert!(bodies[usize::from(index >= 20)]["questions"]
            .get(format!("q{index}"))
            .is_some());
    }
}

#[test]
fn missing_or_invalid_answers_are_malformed() {
    let mut missing = fixture()["response"].clone();
    missing["answers"]
        .as_object_mut()
        .unwrap()
        .remove("department");
    assert_eq!(
        parse_response(&missing.to_string(), &questions()),
        Err(DecisionError::MalformedResponse)
    );
    for (field, value) in [("noul", json!(1.2)), ("type", json!("choice"))] {
        let mut response = fixture()["response"].clone();
        response["answers"]["is_urgent"][field] = value;
        assert_eq!(
            parse_response(&response.to_string(), &questions()),
            Err(DecisionError::MalformedResponse)
        );
    }
    let mut response = fixture()["response"].clone();
    response["answers"]["department"]["probabilities"]["billing"] = json!(0.5);
    assert_eq!(
        parse_response(&response.to_string(), &questions()),
        Err(DecisionError::MalformedResponse)
    );
    assert_eq!(
        parse_response("not JSON", &questions()),
        Err(DecisionError::MalformedResponse)
    );
}

#[tokio::test]
async fn http_status_mapping_and_secret_redaction() {
    for (status, expected) in [
        (401, DecisionError::Auth),
        (
            422,
            DecisionError::Validation("remote rejected the request (HTTP 422)".into()),
        ),
        (429, DecisionError::RateLimited),
        (529, DecisionError::Unavailable),
        (500, DecisionError::Unavailable),
        (503, DecisionError::Unavailable),
        (400, DecisionError::MalformedResponse),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"detail": "bad field test-secret"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = backend(server.uri());
        assert!(!format!("{client:?}").contains("test-secret"));
        let error = client
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("test-secret"));
        assert!(!format!("{error:?}").contains("test-secret"));
        assert_eq!(error, expected);
    }
}

#[tokio::test]
async fn authorization_header_is_absent_for_empty_key_and_exact_for_secret_key() {
    let no_auth_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(single_noul_response_with_padding(0)),
        )
        .expect(1)
        .mount(&no_auth_server)
        .await;
    assert_eq!(
        backend_with_key(no_auth_server.uri(), "")
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );
    let no_auth_requests = no_auth_server.received_requests().await.unwrap();
    assert_eq!(no_auth_requests.len(), 1);
    assert!(no_auth_requests[0].headers.get("authorization").is_none());

    let auth_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(single_noul_response_with_padding(0)),
        )
        .expect(1)
        .mount(&auth_server)
        .await;
    assert_eq!(
        backend_with_key(auth_server.uri(), "test-secret")
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );
    let auth_requests = auth_server.received_requests().await.unwrap();
    assert_eq!(auth_requests.len(), 1);
    assert_eq!(
        auth_requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer test-secret"
    );
}

#[tokio::test]
async fn http_timeout_is_bounded_and_falls_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_json(&fixture()["response"]),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::new(
        server.uri(),
        "jev-latest".into(),
        "test-secret".into(),
        HttpTimeouts {
            connect: Duration::from_secs(1),
            total: Duration::from_millis(50),
        },
    )
    .unwrap()
    .with_billing(JudgeBilling::LocalFree);
    let outcome = decide_or_fallback(
        Some(&client),
        &fallback(),
        &state(StateClass::Public),
        &noul_questions(1),
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
    assert_eq!(
        outcome.fallback_reason,
        Some(FallbackReason::PrimaryFailed(DecisionError::Timeout))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn http_response_body_size_is_capped_at_one_megabyte() {
    let valid_base = single_noul_response_with_padding(0);
    let under_padding = MAX_RESPONSE_BYTES.saturating_sub(valid_base.len());
    let over_padding = under_padding + 1;

    let small_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(single_noul_response_with_padding(under_padding)),
        )
        .expect(1)
        .mount(&small_server)
        .await;
    assert_eq!(
        backend(small_server.uri())
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );

    let large_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(single_noul_response_with_padding(over_padding)),
        )
        .expect(1)
        .mount(&large_server)
        .await;
    assert_eq!(
        backend(large_server.uri())
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await,
        Err(DecisionError::MalformedResponse)
    );
}

#[tokio::test]
async fn fallback_reports_every_failure_and_preserves_success() {
    let state = state(StateClass::Private);
    let questions = noul_questions(1);
    let fallback = fallback();
    let absent = decide_or_fallback(None, &fallback, &state, &questions)
        .await
        .unwrap();
    assert_eq!(
        absent,
        DecisionOutcome {
            answers: vec![Answer::Noul { noul: 0.25 }],
            fallback_reason: Some(FallbackReason::PrimaryAbsent)
        }
    );
    for (error, configuration_error) in [
        (DecisionError::RefusedByLocality, true),
        (DecisionError::Timeout, false),
        (DecisionError::Unavailable, false),
        (DecisionError::MalformedResponse, false),
        (DecisionError::MissingRule, false),
    ] {
        let primary = StubBackend {
            locality: DataLocality::Local,
            result: Err(error.clone()),
            calls: AtomicUsize::new(0),
        };
        let outcome = decide_or_fallback(Some(&primary), &fallback, &state, &questions)
            .await
            .unwrap();
        assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
        assert_eq!(
            outcome
                .fallback_reason
                .as_ref()
                .unwrap()
                .is_configuration_error(),
            configuration_error
        );
        assert_eq!(
            outcome.fallback_reason,
            Some(FallbackReason::PrimaryFailed(error))
        );
        assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
    }
    let cloud = StubBackend {
        locality: DataLocality::Cloud,
        result: Ok(vec![]),
        calls: AtomicUsize::new(0),
    };
    assert_eq!(
        decide_or_fallback(Some(&cloud), &fallback, &state, &questions)
            .await
            .unwrap()
            .fallback_reason,
        Some(FallbackReason::PrimaryFailed(
            DecisionError::RefusedByLocality
        ))
    );
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 0);
    let primary = StubBackend {
        locality: DataLocality::Local,
        result: Ok(vec![Answer::Noul { noul: 0.75 }]),
        calls: AtomicUsize::new(0),
    };
    // An empty fallback would error if called, so success proves it is untouched.
    assert_eq!(
        decide_or_fallback(
            Some(&primary),
            &DeterministicBackend::new(BTreeMap::new()),
            &state,
            &questions
        )
        .await
        .unwrap(),
        DecisionOutcome {
            answers: vec![Answer::Noul { noul: 0.75 }],
            fallback_reason: None
        }
    );
    assert_eq!(primary.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fallback_reports_configuration_errors_and_rate_limits_with_redaction() {
    for (status, reason, configuration_error) in [
        (401, FallbackReason::Auth, true),
        (
            422,
            FallbackReason::Validation("remote rejected the request (HTTP 422)".into()),
            true,
        ),
        (429, FallbackReason::RateLimited, false),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"detail": "bad field test-secret"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let outcome = decide_or_fallback(
            Some(&backend(server.uri())),
            &fallback(),
            &state(StateClass::Public),
            &noul_questions(1),
        )
        .await
        .unwrap();
        assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
        assert_eq!(
            outcome
                .fallback_reason
                .as_ref()
                .unwrap()
                .is_configuration_error(),
            configuration_error
        );
        assert_eq!(outcome.fallback_reason, Some(reason));
        assert!(!format!("{outcome:?}").contains("test-secret"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn fallback_requires_present_and_valid_rules() {
    assert_eq!(
        decide_or_fallback(
            None,
            &DeterministicBackend::new(BTreeMap::new()),
            &state(StateClass::Private),
            &noul_questions(1)
        )
        .await,
        Err(DecisionError::MissingRule)
    );
    let primary = StubBackend {
        locality: DataLocality::Local,
        result: Err(DecisionError::Auth),
        calls: AtomicUsize::new(0),
    };
    assert_eq!(
        decide_or_fallback(
            Some(&primary),
            &DeterministicBackend::new(BTreeMap::new()),
            &state(StateClass::Public),
            &noul_questions(1)
        )
        .await,
        Err(DecisionError::MissingRule)
    );
    let invalid =
        DeterministicBackend::new(BTreeMap::from([("q0".into(), Answer::Noul { noul: 2.0 })]));
    assert_eq!(
        decide_or_fallback(
            None,
            &invalid,
            &state(StateClass::Private),
            &noul_questions(1)
        )
        .await,
        Err(DecisionError::MalformedResponse)
    );
    let question = noul_questions(1).remove(0);
    assert_eq!(
        decide_or_fallback(
            None,
            &fallback(),
            &state(StateClass::Private),
            &[question.clone(), question]
        )
        .await,
        Err(DecisionError::Validation("duplicate question id".into()))
    );
}

#[test]
fn choice_requires_explicit_coverage() {
    assert!(matches!(
        Question::choice("q", "Pick").option("one", "One").build(),
        Err(DecisionError::Validation(_))
    ));
    assert!(Question::choice("q", "Pick")
        .option("one", "One")
        .exhaustive()
        .build()
        .is_ok());
    let question = Question::choice("q", "Pick")
        .option("one", "One")
        .none_of_these("none")
        .build()
        .unwrap();
    let value = serde_json::to_value(WireQuestions(&[question])).unwrap();
    assert_eq!(
        value["q"]["criteria"]["none"],
        "None of these options applies."
    );
    assert!(Question::choice("q", "Pick")
        .option("one", "One")
        .none_of_these("one")
        .build()
        .is_err());
}

#[test]
fn injection_rail_is_idempotent_for_every_kind() {
    for instructions in [
        "Judge".to_owned(),
        format!("Judge {INJECTION_RAIL}"),
        format!("Judge {INJECTION_RAIL} {INJECTION_RAIL}"),
    ] {
        let built = [
            Question::noul("n", &instructions, "Yes", "No").unwrap(),
            Question::score("s", &instructions, vec!["Low".into(), "High".into()]).unwrap(),
            Question::choice("c", &instructions)
                .none_of_these("none")
                .build()
                .unwrap(),
        ];
        for question in built {
            assert_eq!(question.instructions(), format!("Judge {INJECTION_RAIL}"));
            assert_eq!(question.instructions().matches(INJECTION_RAIL).count(), 1);
        }
    }
}

#[test]
fn score_level_bounds_and_choice_order() {
    for count in [0, 1, 11] {
        assert!(Question::score("s", "Rate", vec!["Level".into(); count]).is_err());
    }
    for count in [2, 10] {
        assert!(Question::score("s", "Rate", vec!["Level".into(); count]).is_ok());
    }
    let questions = [Question::choice("c", "Pick")
        .option("z", "First")
        .option("a", "Second")
        .exhaustive()
        .build()
        .unwrap()];
    let wire = serde_json::to_string(&WireQuestions(&questions)).unwrap();
    assert!(wire.find("\"z\"").unwrap() < wire.find("\"a\"").unwrap());
}

#[tokio::test]
async fn duplicate_ids_and_empty_batches_never_send() {
    let server = MockServer::start().await;
    let client = backend(server.uri());
    let question = noul_questions(1).remove(0);
    assert!(matches!(
        client
            .decide(&state(StateClass::Public), &[question.clone(), question])
            .await,
        Err(DecisionError::Validation(_))
    ));
    assert_eq!(
        client
            .decide(&state(StateClass::Public), &[])
            .await
            .unwrap(),
        vec![]
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn redirects_cannot_egress_private_state() {
    let target = MockServer::start().await;
    let source = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/v1/systemone", target.uri())),
        )
        .expect(1)
        .mount(&source)
        .await;
    assert_eq!(
        backend(source.uri())
            .decide(&state(StateClass::Private), &noul_questions(1))
            .await,
        Err(DecisionError::MalformedResponse)
    );
    assert_eq!(source.received_requests().await.unwrap().len(), 1);
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn http_422_never_echoes_remote_body_details() {
    let secret = "test-secret";
    let encoded_secret = "test%2Dsecret";
    let sentinel_state = "private-state-sentinel";
    for body in [
        format!("key leaked: {secret}"),
        format!("encoded key leaked: {encoded_secret}"),
        format!("state leaked: {sentinel_state}"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(422).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = backend(server.uri());
        let state = DecisionState {
            text: sentinel_state.into(),
            class: StateClass::Public,
        };
        let error = client.decide(&state, &noul_questions(1)).await.unwrap_err();
        assert_eq!(
            error,
            DecisionError::Validation("remote rejected the request (HTTP 422)".into())
        );
        let display = error.to_string();
        let debug = format!("{error:?}");
        for forbidden in [secret, encoded_secret, sentinel_state] {
            assert!(!display.contains(forbidden));
            assert!(!debug.contains(forbidden));
        }
    }
}

#[tokio::test]
async fn transport_errors_and_debug_are_redacted() {
    let secret = "test-secret";
    let client = backend("http://127.0.0.1:1".into());
    assert!(!format!("{client:?}").contains(secret));
    assert_eq!(
        client
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await,
        Err(DecisionError::Unavailable)
    );
    let invalid = backend("not a URL test-secret".into());
    assert_eq!(
        invalid
            .decide(&state(StateClass::Public), &noul_questions(1))
            .await,
        Err(DecisionError::Unavailable)
    );
    assert!(!format!("{invalid:?}").contains("test-secret"));
    for error in [
        DecisionError::RefusedByLocality,
        DecisionError::Auth,
        DecisionError::RateLimited,
        DecisionError::Unavailable,
        DecisionError::Timeout,
        DecisionError::MalformedResponse,
        DecisionError::MissingRule,
    ] {
        assert!(!error.to_string().contains("test-secret"));
    }
    let private_state = DecisionState {
        text: "private-state-sentinel".into(),
        class: StateClass::Private,
    };
    assert!(!format!("{private_state:?}").contains("private-state-sentinel"));
}

#[tokio::test]
async fn malformed_http_response_reaches_deterministic_fallback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model": "test", "answers": {}, "usage": {"input_tokens": 0, "output_tokens": 0}})))
        .expect(1).mount(&server).await;
    let outcome = decide_or_fallback(
        Some(&backend(server.uri())),
        &fallback(),
        &state(StateClass::Private),
        &noul_questions(1),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        DecisionOutcome {
            answers: vec![Answer::Noul { noul: 0.25 }],
            fallback_reason: Some(FallbackReason::PrimaryFailed(
                DecisionError::MalformedResponse
            )),
        }
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[derive(Debug)]
struct TestClock {
    seconds: Mutex<f64>,
}

impl TestClock {
    fn new() -> Self {
        Self {
            seconds: Mutex::new(0.0),
        }
    }

    fn advance_seconds(&self, seconds: f64) {
        let mut guard = self.seconds.lock().expect("test clock mutex poisoned");
        *guard += seconds;
    }
}

impl DecisionClock for TestClock {
    fn now_seconds(&self) -> f64 {
        *self.seconds.lock().expect("test clock mutex poisoned")
    }
}

fn test_state_ref() -> StateRef {
    StateRef::from_id("decision-state-id-1").unwrap()
}

async fn test_manager() -> (tempfile::TempDir, Arc<SessionManager>, String) {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let session = manager
        .create_session(
            PathBuf::from("."),
            "decision-tests".to_string(),
            SessionType::User,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    (dir, manager, session.id)
}

#[tokio::test]
async fn journal_row_stores_state_ref_without_state_text() {
    let (_dir, manager, session_id) = test_manager().await;
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let sentinel_state = "SENTINEL_PRIVATE_STATE_123";
    let state = DecisionState {
        text: sentinel_state.to_string(),
        class: StateClass::Private,
    };
    let questions = noul_questions(1);
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        None,
        &fallback(),
        &state,
        &questions,
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
    let rows = journal.recent("watcher.candidate_pick", 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state_ref, "decision-state-id-1");
    assert_eq!(rows[0].backend, "fallback:primary_absent");
    assert!(
        !rows[0].distribution_json.contains(sentinel_state),
        "distribution_json must never include state text"
    );
}

#[tokio::test]
async fn local_rate_limit_records_rate_limited_and_refills_after_clock_advance() {
    let (_dir, manager, session_id) = test_manager().await;
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool);
    let clock = Arc::new(TestClock::new());
    let limiter = LocalDecisionRateLimiter::new_with_clock(10.0, 10.0, clock.clone());
    let state = state(StateClass::Private);
    let questions = noul_questions(1);
    for _ in 0..100 {
        let _ = decide_journaled(
            DecisionSite {
                site: "watcher.candidate_pick",
                question_version: "v1",
                state_ref: &test_state_ref(),
            },
            None,
            &fallback(),
            &state,
            &questions,
            JudgeRuntime {
                journal: &journal,
                rate_limiter: &limiter,
                session_manager: &manager,
            },
            &DecisionContext {
                session_id: session_id.clone(),
                is_headless: false,
                acted: false,
            },
        )
        .await
        .unwrap();
    }
    let rows = journal.recent("watcher.candidate_pick", 200).await.unwrap();
    let limited = rows
        .iter()
        .filter(|row| row.fallback_reason.as_deref() == Some("rate_limited_locally"))
        .count();
    assert!(limited > 0, "expected rate-limited rows after capacity");

    clock.advance_seconds(60.0);
    let unavailable_primary = backend("http://127.0.0.1:1".to_string());
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&unavailable_primary),
        &fallback(),
        &state,
        &questions,
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        outcome.fallback_reason,
        Some(FallbackReason::PrimaryFailed(DecisionError::Unavailable))
    );
}

#[tokio::test]
async fn local_ledger_rows_record_systemone_and_deterministic_separately() {
    let (_dir, manager, session_id) = test_manager().await;
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "test",
            "answers": { "q0": { "type": "noul", "noul": 0.75 } },
            "usage": { "input_tokens": 9, "output_tokens": 1 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = state(StateClass::Private);
    let questions = noul_questions(1);
    let local_primary = resolved_systemone(&server.uri(), Some("local_free"));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&local_primary),
        &fallback(),
        &state,
        &questions,
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id: session_id.clone(),
            is_headless: true,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.fallback_reason, None);
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.75 }]);
    let _ = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        None,
        &fallback(),
        &state,
        &questions,
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id: session_id.clone(),
            is_headless: true,
            acted: false,
        },
    )
    .await
    .unwrap();

    let rows: Vec<(String, i64, f64)> = sqlx::query_as(
        "SELECT provider, is_chargeable, cost_usd
         FROM cost_ledger
         WHERE session_id = ?
         ORDER BY ts ASC, call_id ASC",
    )
    .bind(session_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(rows.iter().any(|row| row.0 == "systemone"));
    assert!(rows.iter().any(|row| row.0 == "deterministic"));
    assert!(rows.iter().all(|row| row.1 == 0));
    assert!(rows.iter().all(|row| row.2 == 0.0));
    assert_eq!(rows.len(), 2);
    let reservations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reservations, 0);
}

#[tokio::test]
async fn journal_write_failure_does_not_change_answer() {
    let (_dir, manager, session_id) = test_manager().await;
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    sqlx::query("DROP TABLE decision_journal")
        .execute(&pool)
        .await
        .unwrap();
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        None,
        &fallback(),
        &state(StateClass::Private),
        &noul_questions(1),
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
}

#[derive(Debug, PartialEq)]
enum RecordedFinish {
    Usage {
        input_tokens: u64,
        is_estimated: bool,
    },
    NotSent,
    UsageUnknown,
}

struct RecordingAccounting {
    real: SessionMeteredAccounting,
    refuse: bool,
    fail_finish_once: std::sync::atomic::AtomicBool,
    requests: Mutex<Vec<MeteredCallRequest>>,
    finishes: Mutex<Vec<RecordedFinish>>,
}

impl RecordingAccounting {
    async fn new(manager: Arc<SessionManager>, session_id: &str, refuse: bool) -> Arc<Self> {
        manager.begin_budget_task(session_id).await.unwrap();
        let session = manager.get_session(session_id, false).await.unwrap();
        Arc::new(Self {
            real: SessionMeteredAccounting::new(manager, session),
            refuse,
            fail_finish_once: std::sync::atomic::AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
            finishes: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl MeteredAccounting for RecordingAccounting {
    async fn reserve(
        &self,
        request: MeteredCallRequest,
    ) -> Result<MeteredCallContext, MeteredCallError> {
        // The recording fake can refuse, while granted requests exercise real storage.
        let result = if self.refuse {
            Err(MeteredCallError::CeilingExhausted {
                scope: crate::cost_router::budget::BudgetScope::Task,
                spent_usd: 1.0,
                held_usd: 0.25,
                requested_usd: 0.125,
                ceiling_usd: 1.0,
                approval_required: true,
            })
        } else {
            self.real
                .reserve(MeteredCallRequest {
                    call_id: request.call_id.clone(),
                    kind: request.kind,
                    backend: request.backend.clone(),
                    model: request.model.clone(),
                    estimated_input_tokens: request.estimated_input_tokens,
                    input_token_upper_bound: request.input_token_upper_bound,
                    input_usd_per_mtok: request.input_usd_per_mtok,
                })
                .await
        };
        self.requests.lock().unwrap().push(request);
        result
    }

    async fn finish(
        &self,
        call: &MeteredCallContext,
        finish: MeteredCallFinish,
    ) -> Result<(), MeteredCallError> {
        let recorded = match &finish {
            MeteredCallFinish::Usage {
                input_tokens,
                is_estimated,
            } => RecordedFinish::Usage {
                input_tokens: *input_tokens,
                is_estimated: *is_estimated,
            },
            MeteredCallFinish::NotSent => RecordedFinish::NotSent,
            MeteredCallFinish::UsageUnknown => RecordedFinish::UsageUnknown,
        };
        self.finishes.lock().unwrap().push(recorded);
        if self.fail_finish_once.swap(false, Ordering::SeqCst) {
            return Err(MeteredCallError::Storage(anyhow::anyhow!(
                "test settlement failure"
            )));
        }
        self.real.finish(call, finish).await
    }
}

// Preserve the injected fake when exercising the journal's generic backend seam.
struct InjectedBackend(SystemOneHttpBackend);

#[async_trait]
impl DecisionBackend for InjectedBackend {
    fn billing(&self) -> JudgeBilling {
        self.0.billing()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn model_name(&self) -> Option<&str> {
        self.0.model_name()
    }
    fn data_locality(&self) -> DataLocality {
        self.0.data_locality()
    }
    async fn decide_with_accounting(
        &self,
        state: &DecisionState,
        questions: &[Question],
        _accounting: Arc<dyn MeteredAccounting>,
    ) -> Result<DecisionResponse, DecisionError> {
        self.0.decide_with_usage(state, questions).await
    }
    async fn decide_with_usage(
        &self,
        state: &DecisionState,
        questions: &[Question],
    ) -> Result<DecisionResponse, DecisionError> {
        self.0.decide_with_usage(state, questions).await
    }
}

fn batch_response(request: &Request, with_usage: bool) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let answers: serde_json::Map<String, Value> = body["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|id| (id.clone(), json!({"type": "noul", "noul": 0.75})))
        .collect();
    let mut body = json!({"model": "test", "answers": answers});
    if with_usage {
        body["usage"] = json!({"input_tokens": 9, "output_tokens": 1});
    }
    ResponseTemplate::new(200).set_body_json(body)
}

#[tokio::test]
async fn hosted_batches_reserve_and_finish_each_physical_post() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| batch_response(request, true))
        .expect(2)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::hosted_test_backend(*server.address())
        .with_accounting(accounting.clone());
    let answers = decide_guarded(&client, &state(StateClass::Public), &noul_questions(25))
        .await
        .unwrap();
    assert_eq!(answers, vec![Answer::Noul { noul: 0.75 }; 25]);
    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    assert_eq!(
        bodies
            .iter()
            .map(|body| body["questions"].as_object().unwrap().len())
            .collect::<Vec<_>>(),
        vec![20, 5]
    );
    {
        let requests = accounting.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0].call_id, requests[1].call_id);
        for (request, body) in requests.iter().zip(&bodies) {
            let chars = body["state"].as_str().unwrap().chars().count()
                + serde_json::to_string(&body["questions"])
                    .unwrap()
                    .chars()
                    .count();
            assert_eq!(request.estimated_input_tokens, chars.div_ceil(4) as u64);
            assert!(request.input_token_upper_bound >= request.estimated_input_tokens);
            assert_eq!(
                request.input_usd_per_mtok,
                super::role::judge_input_usd_per_mtok()
            );
        }
    }
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![
            RecordedFinish::Usage {
                input_tokens: 9,
                is_estimated: false
            },
            RecordedFinish::Usage {
                input_tokens: 9,
                is_estimated: false
            },
        ]
    );
    let pool = manager.pool_clone().await.unwrap();
    let reservations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reservations, 2);
    let rows: Vec<(String, i64, i64, f64)> =
        sqlx::query_as("SELECT provider, input_tokens, is_chargeable, cost_usd FROM cost_ledger")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    for (provider, tokens, chargeable, cost) in rows {
        assert_eq!(provider, "systemone");
        assert_eq!(tokens, 9);
        assert_eq!(chargeable, 1);
        assert!((cost - 9.0 / 1_000_000.0 * super::role::judge_input_usd_per_mtok()).abs() < 1e-12);
    }
}

#[tokio::test]
async fn refused_reservation_journals_typed_ceiling_and_never_posts() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, true).await;
    let server = MockServer::start().await;
    let client = InjectedBackend(
        SystemOneHttpBackend::hosted_test_backend(*server.address())
            .with_accounting(accounting.clone()),
    );
    let journal = DecisionJournal::new(manager.pool_clone().await.unwrap());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&client),
        &fallback(),
        &state(StateClass::Public),
        &noul_questions(1),
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
    let expected = FallbackReason::BudgetRefused(BudgetRefusal::CeilingExhausted {
        scope: crate::cost_router::budget::BudgetScope::Task,
        spent_usd: 1.0,
        held_usd: 0.25,
        requested_usd: 0.125,
        ceiling_usd: 1.0,
        approval_required: true,
    });
    assert_eq!(outcome.fallback_reason, Some(expected.clone()));
    let rows = journal.recent("watcher.candidate_pick", 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].fallback_reason,
        Some("budget_refused:ceiling_exhausted".to_string())
    );
    let reason = rows[0].fallback_reason.as_ref().unwrap();
    assert!(!reason
        .as_bytes()
        .windows(3)
        .any(|w| w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_digit()));
    assert_eq!(accounting.requests.lock().unwrap().len(), 1);
    assert!(accounting.finishes.lock().unwrap().is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn hosted_connection_failure_finishes_not_sent() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client =
        SystemOneHttpBackend::hosted_test_backend(address).with_accounting(accounting.clone());
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Public), &noul_questions(1)).await,
        Err(DecisionError::Unavailable)
    );
    assert_eq!(accounting.requests.lock().unwrap().len(), 1);
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![RecordedFinish::NotSent]
    );
    let pool = manager.pool_clone().await.unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM cost_reservations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "released");
}

#[tokio::test]
async fn hosted_missing_usage_settles_estimated_input() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| batch_response(request, false))
        .expect(1)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::hosted_test_backend(*server.address())
        .with_accounting(accounting.clone());
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );
    let estimate = accounting.requests.lock().unwrap()[0].estimated_input_tokens;
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![RecordedFinish::Usage {
            input_tokens: estimate,
            is_estimated: true
        },]
    );
    let pool = manager.pool_clone().await.unwrap();
    let row: (i64, bool) = sqlx::query_as("SELECT input_tokens, is_estimated FROM cost_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row, (estimate as i64, true));
}

#[tokio::test]
async fn hosted_retry_after_5xx_uses_fresh_reservation() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    let server = MockServer::start().await;
    let count = AtomicUsize::new(0);
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
            } else {
                batch_response(request, true)
            }
        })
        .expect(2)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::hosted_test_backend(*server.address())
        .with_accounting(accounting.clone());
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Public), &noul_questions(1)).await,
        Err(DecisionError::Unavailable)
    );
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Public), &noul_questions(1))
            .await
            .unwrap(),
        vec![Answer::Noul { noul: 0.75 }]
    );
    {
        let requests = accounting.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0].call_id, requests[1].call_id);
    }
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![
            RecordedFinish::UsageUnknown,
            RecordedFinish::Usage {
                input_tokens: 9,
                is_estimated: false
            },
        ]
    );
    let pool = manager.pool_clone().await.unwrap();
    let mut states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM cost_reservations ORDER BY state")
            .fetch_all(&pool)
            .await
            .unwrap();
    states.sort();
    // The client marked the first hold UsageUnknown (asserted above through the
    // fake). The real metered layer then reconciles a stale unknown hold when
    // the next reservation in the same session is taken: with no ledger spend
    // recorded for it, it becomes `released`; the retry's hold settles.
    assert_eq!(states.len(), 2);
    assert!(states.contains(&"settled".to_string()), "{states:?}");
    assert!(
        states.iter().any(|s| s == "released" || s == "unknown"),
        "{states:?}"
    );
}

#[tokio::test]
async fn paid_loopback_without_budget_task_is_refused_before_wire_and_journaled() {
    let (_dir, manager, _) = test_manager().await;
    let session = manager
        .create_session(
            PathBuf::from("."),
            "headless-judge".into(),
            SessionType::Hidden,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    let server = MockServer::start().await;
    let client = resolved_systemone(&server.uri(), None);
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&client),
        &fallback(),
        &state(StateClass::Public),
        &noul_questions(1),
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id: session.id,
            is_headless: true,
            acted: false,
        },
    )
    .await
    .unwrap();
    let expected = FallbackReason::BudgetRefused(BudgetRefusal::NoTaskIdentity);
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.25 }]);
    assert_eq!(outcome.fallback_reason, Some(expected.clone()));
    let rows = journal.recent("watcher.candidate_pick", 10).await.unwrap();
    assert_eq!(
        rows[0].fallback_reason,
        Some("budget_refused:no_task_identity".to_string())
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let reservations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reservations, 0);
}

#[test]
fn metered_refusals_are_mapped_by_variant() {
    for error in [
        MeteredCallError::MissingTaskIdentity {
            session_id: "session".into(),
        },
        MeteredCallError::TaskIdentityMismatch {
            session_id: "session".into(),
        },
    ] {
        assert_eq!(
            DecisionError::from(error),
            DecisionError::BudgetRefused(BudgetRefusal::NoTaskIdentity)
        );
    }
    for error in [
        MeteredCallError::InvalidBound {
            reason: "test-secret".into(),
        },
        MeteredCallError::BudgetUnknown {
            reason: "test-secret".into(),
        },
        MeteredCallError::InvocationNotFresh {
            call_id: "test-secret".into(),
        },
        MeteredCallError::Storage(anyhow::anyhow!("test-secret")),
    ] {
        assert_eq!(DecisionError::from(error), DecisionError::Accounting);
    }
}

#[tokio::test]
async fn journaled_hosted_success_uses_session_accounting_without_duplicate_local_row() {
    let (_dir, manager, session_id) = test_manager().await;
    manager.begin_budget_task(&session_id).await.unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| batch_response(request, true))
        .expect(2)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::hosted_test_backend(*server.address());
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&client),
        &fallback(),
        &state(StateClass::Public),
        &noul_questions(25),
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.75 }; 25]);
    assert_eq!(outcome.fallback_reason, None);
    let reservations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE state = 'settled'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let ledger_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reservations, 2);
    assert_eq!(ledger_rows, 2);
    let rows = journal.recent("watcher.candidate_pick", 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].backend, "systemone");
    assert_eq!(rows[0].fallback_reason, None);
}

#[tokio::test]
async fn cancelled_hosted_send_keeps_unknown_hold() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    let client = SystemOneHttpBackend::hosted_test_backend(*server.address())
        .with_accounting(accounting.clone());
    let state = state(StateClass::Public);
    let questions = noul_questions(1);
    let mut call = Box::pin(decide_guarded(&client, &state, &questions));
    // Observe the physical request before cancelling, avoiding a timing assumption.
    tokio::select! {
        result = &mut call => panic!("request unexpectedly completed: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !server.received_requests().await.unwrap().is_empty() { break; }
                tokio::task::yield_now().await;
            }
        }) => result.unwrap(),
    }
    drop(call);
    let pool = manager.pool_clone().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let unknown: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM cost_reservations WHERE state = 'unknown'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if unknown == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![RecordedFinish::UsageUnknown]
    );
}

#[tokio::test]
async fn journaled_paid_loopback_reserves_without_duplicate_free_row() {
    let (_dir, manager, session_id) = test_manager().await;
    manager.begin_budget_task(&session_id).await.unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| batch_response(request, true))
        .expect(2)
        .mount(&server)
        .await;
    let client = resolved_systemone(&server.uri(), None);
    let pool = manager.pool_clone().await.unwrap();
    let journal = DecisionJournal::new(pool.clone());
    let limiter =
        LocalDecisionRateLimiter::new_with_clock(100.0, 100.0, Arc::new(TestClock::new()));
    let outcome = decide_journaled(
        DecisionSite {
            site: "watcher.candidate_pick",
            question_version: "v1",
            state_ref: &test_state_ref(),
        },
        Some(&client),
        &fallback(),
        &state(StateClass::Public),
        &noul_questions(25),
        JudgeRuntime {
            journal: &journal,
            rate_limiter: &limiter,
            session_manager: &manager,
        },
        &DecisionContext {
            session_id,
            is_headless: false,
            acted: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.answers, vec![Answer::Noul { noul: 0.75 }; 25]);
    assert_eq!(outcome.fallback_reason, None);
    let reservations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE state = 'settled'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let ledger_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reservations, 2);
    assert_eq!(ledger_rows, 2);
    let rows = journal.recent("watcher.candidate_pick", 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].backend, "systemone");
    assert_eq!(rows[0].fallback_reason, None);
}

fn resolved_systemone(endpoint: &str, billing: Option<&str>) -> SystemOneHttpBackend {
    let resolved = resolve_judge_with_billing(
        Some("systemone"),
        Some("local-judge"),
        Some(endpoint),
        None,
        false,
        billing,
    )
    .unwrap();
    let JudgeResolution::Configured(settings) = resolved else {
        panic!("expected configured systemone");
    };
    build_judge_backend(&settings, HttpTimeouts::default()).unwrap()
}

#[tokio::test]
async fn failed_finish_is_not_retried_on_drop() {
    let (_dir, manager, session_id) = test_manager().await;
    let accounting = RecordingAccounting::new(Arc::clone(&manager), &session_id, false).await;
    accounting.fail_finish_once.store(true, Ordering::SeqCst);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| batch_response(request, true))
        .expect(1)
        .mount(&server)
        .await;
    let client = resolved_systemone(&server.uri(), None).with_accounting(accounting.clone());
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Public), &noul_questions(1)).await,
        Err(DecisionError::Accounting)
    );
    // Let any erroneous drop-spawned finish run on this current-thread runtime.
    tokio::task::yield_now().await;
    assert_eq!(accounting.requests.lock().unwrap().len(), 1);
    assert_eq!(
        *accounting.finishes.lock().unwrap(),
        vec![RecordedFinish::Usage {
            input_tokens: 9,
            is_estimated: false
        },]
    );
}

#[tokio::test]
async fn local_free_cloud_endpoint_still_refuses_private_state() {
    let client = resolved_systemone("https://judge.example", Some("local_free"));
    assert_eq!(client.billing(), JudgeBilling::LocalFree);
    assert_eq!(client.data_locality(), DataLocality::Cloud);
    assert_eq!(
        decide_guarded(&client, &state(StateClass::Private), &noul_questions(1)).await,
        Err(DecisionError::RefusedByLocality)
    );
    assert_eq!(
        client
            .decide_with_usage(&state(StateClass::Private), &noul_questions(1))
            .await,
        Err(DecisionError::RefusedByLocality)
    );
}

#[test]
fn state_ref_rejects_free_text_and_hashes_it() {
    for invalid in [
        "",
        "a sentence containing state text",
        "line\nbreak",
        "tab\tseparated",
        "a\0b",
    ] {
        assert!(StateRef::from_id(invalid).is_err());
    }
    assert!(StateRef::from_id("x".repeat(65)).is_err());
    assert!(StateRef::from_id("x".repeat(64)).is_ok());
    assert_eq!(
        StateRef::from_id("decision-state-id-1").unwrap().as_str(),
        "decision-state-id-1"
    );
    let text = "a sentence containing state text";
    let hashed = StateRef::hashed(text);
    assert_eq!(hashed.as_str().len(), 64);
    assert!(hashed.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        hashed.as_str(),
        crate::sovereignty::hash_texts(&[text.to_string()])
    );
    assert_eq!(hashed, StateRef::hashed(text));
    assert_ne!(hashed, StateRef::hashed("different text"));
    assert!(!hashed.as_str().contains(text));
}
