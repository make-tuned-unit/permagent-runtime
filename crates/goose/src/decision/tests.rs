use super::http::{parse_response, WireQuestions, WireRequest, MAX_RESPONSE_BYTES};
use super::types::INJECTION_RAIL;
use super::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    SystemOneHttpBackend::new(
        endpoint,
        "jev-latest".into(),
        "test-secret".into(),
        HttpTimeouts::default(),
    )
    .unwrap()
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
    fn name(&self) -> &str {
        "stub"
    }
    fn data_locality(&self) -> DataLocality {
        self.locality
    }
    async fn decide(
        &self,
        _: &DecisionState,
        _: &[Question],
    ) -> Result<Vec<Answer>, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
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
    .unwrap();
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
