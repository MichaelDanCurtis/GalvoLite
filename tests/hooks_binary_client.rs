use galvolite::hooks::{
    client,
    types::{HookAction, HookIngestResponse, HookKind},
};
use serde_json::json;

#[test]
fn run_posts_envelope_and_returns_response_inject_text() {
    let mut mock = mockito::Server::new();
    let url = mock.url();

    let m = mock
        .mock("POST", "/hooks/ingest")
        .match_header("x-galvolite-token", "tok123")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::to_string(&HookIngestResponse {
                action: HookAction::Proceed,
                inject_text: Some("hello model".into()),
                note: None,
            })
            .unwrap(),
        )
        .create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok123".into(),
        timeout_secs: 5,
    };

    let outcome = client::run(env, HookKind::SessionStart, json!({"is_resume": false})).unwrap();
    assert_eq!(outcome.inject_text.as_deref(), Some("hello model"));
    m.assert();
}

#[test]
fn run_returns_no_inject_text_on_500() {
    let mut mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock
        .mock("POST", "/hooks/ingest")
        .with_status(500)
        .create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok".into(),
        timeout_secs: 5,
    };
    let outcome = client::run(env, HookKind::Stop, json!({})).unwrap();
    assert!(outcome.inject_text.is_none());
}

#[test]
fn run_times_out_within_bound() {
    use std::time::Instant;
    let mut mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock
        .mock("POST", "/hooks/ingest")
        .with_chunked_body(|w| {
            std::thread::sleep(std::time::Duration::from_secs(10));
            w.write_all(b"")
        })
        .create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok".into(),
        timeout_secs: 1,
    };
    let start = Instant::now();
    let _ = client::run(env, HookKind::Stop, serde_json::json!({}));
    assert!(start.elapsed().as_secs() < 3, "should bail within timeout");
}
