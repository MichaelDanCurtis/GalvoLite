use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::api::events::DomainEvent;
use galvolite::api::router;
use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

const SAMPLE: &str = r#"# Directive: Sub-plan 7

Plan: ./plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
|----|-------|----------|------------|
| P1 | 1     | full     | —          |
| P2 | 2, 3  | review   | P1         |
| P3 | 4     | manual   | P1, P2     |

## Escalation rules

- On schema change → ask_operator

## Notes

ctx
"#;

async fn setup() -> (axum::Router, Arc<SessionManager>, tempfile::TempDir, tempfile::TempDir) {
    let data_dir = tempdir().unwrap();
    let project_dir = tempdir().unwrap();
    let db = Db::open(data_dir.path().join("t.db")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = Arc::new(SessionManager::with_event_bus(db, data_dir.path().to_path_buf(), bus));
    let app = router(mgr.clone());
    (app, mgr, data_dir, project_dir)
}

async fn create_project(mgr: &Arc<SessionManager>, project_dir: &std::path::Path) -> uuid::Uuid {
    mgr.add_project(NewProject {
        name: "p".into(),
        path: project_dir.to_string_lossy().into_owned(),
        remote_url: None,
    })
    .unwrap()
    .id
}

#[tokio::test]
async fn put_then_get_directive() {
    let (app, mgr, _data, project_dir) = setup().await;
    let pid = create_project(&mgr, project_dir.path()).await;

    let put_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/projects/{pid}/directive"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"markdown": SAMPLE}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::OK);

    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/projects/{pid}/directive"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(get_resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["name"], "Sub-plan 7");
    assert_eq!(body["phases"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn put_directive_with_invalid_markdown_returns_400() {
    let (app, mgr, _data, project_dir) = setup().await;
    let pid = create_project(&mgr, project_dir.path()).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/projects/{pid}/directive"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"markdown": "not a directive"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_missing_directive_returns_404() {
    let (app, mgr, _data, project_dir) = setup().await;
    let pid = create_project(&mgr, project_dir.path()).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/projects/{pid}/directive"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_directive_publishes_directive_changed_event() {
    let (app, mgr, _data, project_dir) = setup().await;
    let pid = create_project(&mgr, project_dir.path()).await;
    let mut rx = mgr.subscribe_events();

    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/projects/{pid}/directive"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"markdown": SAMPLE}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("no event")
        .unwrap();
    assert!(matches!(evt, DomainEvent::DirectiveChanged { project_id } if project_id == pid));
}
