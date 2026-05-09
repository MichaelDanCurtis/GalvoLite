//! Plan 10a Task 3+ — API tests for the GitHub endpoints.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tower::ServiceExt;

// Tests share env vars; serialize with a process-wide async mutex so the
// guard can be held across `.await` points without tripping
// clippy::await_holding_lock.
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[tokio::test]
async fn status_returns_not_connected_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");

    let setup = common::test_app();
    let resp = setup.app.oneshot(
        Request::builder().uri("/api/github/status").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["connected"], false);
    assert_eq!(v["reason"], "no token");
    assert!(v["user"].is_null());
}

#[tokio::test]
async fn status_returns_token_invalid_when_var_set_but_user_uncached() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "fake-for-test");

    let setup = common::test_app();
    let resp = setup.app.oneshot(
        Request::builder().uri("/api/github/status").body(Body::empty()).unwrap()
    ).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["connected"], false);
    assert_eq!(v["reason"], "token invalid");

    std::env::remove_var("GITHUB_TOKEN");
}

use axum::http::Method;
use galvolite::db::NewProject;

#[tokio::test]
async fn link_repo_rejects_bad_format() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "doesnt-matter");

    let setup = common::test_app();
    let p = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let req = Request::builder()
        .uri(format!("/api/projects/{}/github/repo", p.id))
        .method(Method::PUT)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"repo":"not a valid format"}"#))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn link_repo_503_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");

    let setup = common::test_app();
    let p = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let req = Request::builder()
        .uri(format!("/api/projects/{}/github/repo", p.id))
        .method(Method::PUT)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"repo":"octocat/hello"}"#))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// Note: the "happy path link succeeds" case requires hitting api.github.com
// (or a mock), which we can't easily do through the axum integration test
// because the handler constructs a Client from env. Coverage for the
// GitHub-side validation is in tests/github_client.rs::validate_repo_*.
// We separately verify the DB write path via tests/db_crud.rs::project_github_repo_round_trip.

#[tokio::test]
async fn list_prs_404_when_no_linked_repo() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let p = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/github/prs", p.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn list_issues_404_when_project_missing() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let bogus = uuid::Uuid::new_v4();
    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/github/issues", bogus))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}
