// tests/pending_rejoin_endpoint.rs
mod common;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;
use galvolite::db::NewProject;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git").args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test]
async fn pending_rejoin_returns_markdown_and_clears_atomically() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();
    app.db.pool().get().unwrap().execute(
        "UPDATE sessions SET pending_rejoin = 'Children resolved. Resume.' WHERE id = ?1",
        [parent.id.to_string()],
    ).unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 4096).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(parsed["markdown"].as_str().unwrap().contains("Children resolved"));

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pending_rejoin_unauthorized_without_token() {
    let app = common::test_app();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", uuid::Uuid::new_v4()))
        .body(Body::empty()).unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pending_rejoin_forbidden_when_token_resolves_to_different_session() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let s_a = app.mgr.spawn_session(
        proj.id, "a".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let s_b = app.mgr.spawn_session(
        proj.id, "b".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token_a = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [s_a.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();

    // Token belongs to s_a but URL path targets s_b → 403.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", s_b.id))
        .header("Authorization", format!("Bearer {token_a}"))
        .body(Body::empty()).unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
