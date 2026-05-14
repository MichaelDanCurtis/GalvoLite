//! Plan 10e Task 2 — webhook handler integration tests.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use galvolite::api::events::DomainEvent;
use galvolite::db::NewProject;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tower::ServiceExt;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn sign(body: &[u8], secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn webhook_503_when_secret_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_WEBHOOK_SECRET");

    let setup = common::test_app();
    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .body(Body::from("{}")).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn webhook_401_on_signature_mismatch() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", "sha256=0000000000000000000000000000000000000000000000000000000000000000")
            .header("x-github-event", "ping")
            .body(Body::from("{}")).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_pong_for_ping_with_valid_signature() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    let body = b"{}";
    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", sign(body, "topsecret"))
            .header("x-github-event", "ping")
            .body(Body::from(body.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_noop_for_unmatched_repository() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    let body = br#"{"repository":{"full_name":"unknown/repo"}}"#;
    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", sign(body, "topsecret"))
            .header("x-github-event", "pull_request")
            .body(Body::from(body.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Confirm response body says "noop"
    let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["status"], "noop");

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_publishes_event_for_pr_merged() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    // Subscribe BEFORE add_project so we don't miss the ProjectAdded event we
    // intend to drain past.
    let mut rx = setup.mgr.event_bus().subscribe();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    // Drain ProjectAdded with a timeout so a missed event can't hang the test.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;

    let body = br#"{
        "action": "closed",
        "pull_request": {
            "html_url": "https://github.com/octocat/hello/pull/42",
            "number": 42,
            "merged": true
        },
        "repository": { "full_name": "octocat/hello" }
    }"#;

    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", sign(body, "topsecret"))
            .header("x-github-event", "pull_request")
            .body(Body::from(body.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Receive the bus event.
    let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await.expect("timeout").unwrap();
    match evt {
        DomainEvent::PullRequestMerged { project_id, pr_url, pr_number } => {
            assert_eq!(project_id, project.id);
            assert_eq!(pr_url, "https://github.com/octocat/hello/pull/42");
            assert_eq!(pr_number, 42);
        }
        other => panic!("expected PullRequestMerged, got {other:?}"),
    }

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_publishes_event_for_issue_opened() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    // Subscribe BEFORE add_project so the ProjectAdded event reaches us.
    let mut rx = setup.mgr.event_bus().subscribe();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();

    // Drain ProjectAdded with a timeout so a missed event can't hang the test.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;

    let body = br#"{
        "action": "opened",
        "issue": {
            "html_url": "https://github.com/octocat/hello/issues/7",
            "number": 7,
            "title": "Bug: thing broken"
        },
        "repository": { "full_name": "octocat/hello" }
    }"#;

    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", sign(body, "topsecret"))
            .header("x-github-event", "issues")
            .body(Body::from(body.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Wait for the IssueOpened event (skip past any other intermediate events).
    let mut found = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(DomainEvent::IssueOpened { project_id, issue_url, issue_number, title })) => {
                assert_eq!(project_id, project.id);
                assert_eq!(issue_url, "https://github.com/octocat/hello/issues/7");
                assert_eq!(issue_number, 7);
                assert_eq!(title, "Bug: thing broken");
                found = true;
                break;
            }
            Ok(Ok(_)) => continue, // unrelated event; keep looking
            _ => break,
        }
    }
    assert!(found, "IssueOpened event was not published");

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

#[tokio::test]
async fn webhook_noop_for_unhandled_event_type() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

    let setup = common::test_app();
    let body = br#"{"repository":{"full_name":"x/y"}}"#;
    let resp = setup.app.oneshot(
        Request::builder()
            .uri("/api/github/webhook")
            .method(Method::POST)
            .header("x-hub-signature-256", sign(body, "topsecret"))
            .header("x-github-event", "push")
            .body(Body::from(body.to_vec())).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}
