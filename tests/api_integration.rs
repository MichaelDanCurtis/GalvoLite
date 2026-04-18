//! End-to-end lifecycle test: POST a project, POST a session, GET session
//! detail, DELETE the session — all through the axum router.
//!
//! Uses `echo` as the claude-cmd so the PTY child exits immediately; we
//! only care that the handlers return the right shape + status codes.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

struct Setup {
    app: axum::Router,
    project_path: String,
}

fn setup() -> Setup {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();

    // SessionManager's spawn_session writes into `{project}/.galvolite`, so
    // the project path must be a real, writable directory that survives for
    // the life of the test.
    let project_dir = dir.path().join("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    let project_path = project_dir.to_string_lossy().into_owned();

    // Leak the tempdir guard so the paths stay valid through the async test.
    let _ = Box::leak(Box::new(dir));

    let mgr = Arc::new(SessionManager::new(db, data_dir));
    let app = galvolite::api::router(mgr);
    Setup { app, project_path }
}

#[tokio::test]
async fn full_api_lifecycle() {
    let Setup { app, project_path } = setup();

    // 1. Create project
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "TestProject",
                        "path": project_path,
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let project_id = project["id"].as_str().unwrap().to_owned();

    // 2. Session list starts empty
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let sessions: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(sessions.is_empty());

    // 3. Create a session backed by `echo` — child exits immediately, which
    // is fine: we're checking the HTTP contract, not the ritual loop.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "project_id": project_id,
                        "task": "integration test",
                        "claude_cmd": "echo",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = session["id"].as_str().unwrap().to_owned();
    assert_eq!(session["project_id"], project_id);

    // 4. Fetch session detail
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. List sessions — the new session must appear regardless of whether
    // the echo child has exited yet.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let sessions: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["id"], session_id);

    // 6. Give the echo child a moment to exit on its own before we DELETE —
    // otherwise kill_session may race with the natural EOF that marks the
    // session Complete. Either outcome is an acceptable response (404 after
    // natural end, or 200 via kill path), so we accept both.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(
            resp.status(),
            StatusCode::OK
                | StatusCode::NO_CONTENT
                | StatusCode::NOT_FOUND
                | StatusCode::CONFLICT
        ),
        "unexpected DELETE status: {}",
        resp.status()
    );
}

#[tokio::test]
async fn fs_list_returns_dir_entries() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("alpha")).unwrap();
    std::fs::create_dir(root.join("beta")).unwrap();
    std::fs::write(root.join("file.txt"), b"x").unwrap();

    let Setup { app, .. } = setup();
    let path = urlencoding::encode(root.to_str().unwrap());
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/fs/list?path={}", path))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["path"], root.to_str().unwrap());
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"file.txt"));
    let alpha = entries.iter().find(|e| e["name"] == "alpha").unwrap();
    assert_eq!(alpha["is_dir"], true);
    let f = entries.iter().find(|e| e["name"] == "file.txt").unwrap();
    assert_eq!(f["is_dir"], false);
}

#[tokio::test]
async fn fs_list_rejects_relative_path() {
    let Setup { app, .. } = setup();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/fs/list?path=relative/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn fs_list_rejects_missing_path() {
    let Setup { app, .. } = setup();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/fs/list?path=/no/such/dir/anywhere/here")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
