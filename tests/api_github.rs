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

// ---------------------------------------------------------------------------
// Plan 10b Task 5 — `GET /api/sessions/:id/pr-draft` error-path coverage.
// Happy-path body shaping (orchestrator vs child) is left to integration
// smoke since it requires a real worktree + git history; here we exercise
// the routing/state branches that are most likely to regress.
// ---------------------------------------------------------------------------

use galvolite::db::NewSession;

#[tokio::test]
async fn pr_draft_404s_on_missing_session() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let bogus = uuid::Uuid::new_v4();
    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-draft", bogus))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn pr_draft_404s_on_no_linked_repo() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc12345".into()),
        worktree_path: setup._dir.path().join("wt").to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-draft", session.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn pr_draft_404s_on_session_with_no_branch() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    // Note: schema CHECK currently allows roles {implementation, brainstorm,
    // scout, fix} — `orchestrator` would be rejected at INSERT. We're
    // exercising the no-branch error path here, which is role-agnostic.
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "orch".into(),
        parent_session_id: None,
    }).unwrap();

    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-draft", session.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn pr_draft_503_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc12345".into()),
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-draft", session.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Plan 10b Task 6 — `POST /api/sessions/:id/pr` error-path coverage.
// Happy path requires a real GitHub call + a real worktree to push from, so
// we cover routing/state branches here. The GitHub call itself is exercised
// in tests/github_client.rs::create_pull_request_returns_pr_info_on_201, and
// the push helper in tests/github_push.rs::push_fails_with_redacted_token_*.
// ---------------------------------------------------------------------------

use axum::http::Method as HttpMethod;

#[tokio::test]
async fn open_pr_422_when_pr_url_already_set() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc12345".into()),
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();
    // Pre-populate pr_url to simulate a PR that already exists.
    setup.db.set_session_pr_url(session.id, Some("https://github.com/octocat/hello/pull/1")).unwrap();

    let req = Request::builder()
        .uri(format!("/api/sessions/{}/pr", session.id))
        .method(HttpMethod::POST)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"t","body":"b","base":"main"}"#))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["pr_url"], "https://github.com/octocat/hello/pull/1");

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn open_pr_503_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc12345".into()),
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let req = Request::builder()
        .uri(format!("/api/sessions/{}/pr", session.id))
        .method(HttpMethod::POST)
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"t","body":"b","base":"main"}"#))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Plan 10c — `GET /api/sessions/:id/pr-reviews` error-path coverage.
// Happy path requires hitting api.github.com (or a mock) — that's covered by
// tests/github_client.rs::list_pr_reviews_*. Here we exercise the routing/
// state branches that gate the GitHub call.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pr_reviews_404_when_no_pr_url() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc".into()),
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();
    // No pr_url set.
    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-reviews", session.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn pr_reviews_503_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");

    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    let session = setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: Some("galvolite/abc".into()),
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();
    setup.db.set_session_pr_url(session.id, Some("https://github.com/octocat/hello/pull/42")).unwrap();
    let resp = setup.app.oneshot(
        Request::builder()
            .uri(format!("/api/sessions/{}/pr-reviews", session.id))
            .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Plan 10d Task 3 — `POST /api/projects/:id/github/issues/:number/spawn`
// error-path coverage. Happy path requires hitting api.github.com (or a
// mock) plus a real worktree to spawn into; here we exercise the routing/
// state branches that gate the GitHub call + spawn.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawn_from_issue_404_when_no_linked_repo() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");
    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let req = Request::builder()
        .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
        .method(HttpMethod::POST)
        .body(Body::empty()).unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    std::env::remove_var("GITHUB_TOKEN");
}

#[tokio::test]
async fn spawn_from_issue_503_when_token_unset() {
    let _g = env_lock().lock().await;
    std::env::remove_var("GITHUB_TOKEN");
    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    // Insert a "fake orchestrator" so we get past the 409 check.
    setup.db.insert_session(NewSession {
        project_id: project.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: setup._dir.path().to_string_lossy().into_owned(),
        task_summary: "orch".into(),
        parent_session_id: None,
    }).unwrap();
    let req = Request::builder()
        .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
        .method(HttpMethod::POST)
        .body(Body::empty()).unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn spawn_from_issue_409_when_no_orchestrator() {
    let _g = env_lock().lock().await;
    std::env::set_var("GITHUB_TOKEN", "x");
    let setup = common::test_app();
    let project = setup.mgr.add_project(NewProject {
        name: "p".into(),
        path: setup._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
    // No session inserted = no orchestrator.
    let req = Request::builder()
        .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
        .method(HttpMethod::POST)
        .body(Body::empty()).unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    std::env::remove_var("GITHUB_TOKEN");
}
