mod common;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_git(dir: &std::path::Path) {
    Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap();
    std::fs::write(dir.join("README"), "x").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn spawn_child_creates_worktree_and_db_row_with_parent() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());

    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(),
        "/bin/cat".into(), vec![],
    ).await.expect("parent spawn");

    let child = app.mgr.spawn_child(parent.id, "implementation", "child task")
        .await.expect("spawn_child");

    assert_eq!(child.parent_session_id, Some(parent.id));
    assert!(std::path::Path::new(&child.worktree_path).exists());
    assert!(child.branch.as_deref().unwrap().starts_with("galvolite/"));
    assert_ne!(child.worktree_path, parent.worktree_path);
}

#[tokio::test]
async fn set_waiting_on_persists_json_array() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();

    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let waiting: String = conn.query_row(
        "SELECT waiting_on FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0)
    ).unwrap();
    let parsed: Vec<String> = serde_json::from_str(&waiting).unwrap();
    assert_eq!(parsed.len(), 2);
    assert!(parsed.contains(&c1.id.to_string()));
    assert!(parsed.contains(&c2.id.to_string()));
}

#[tokio::test]
async fn build_rejoin_markdown_lists_each_child() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    let conn = app.db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET terminal_status = 'complete', handoff_path = '/tmp/h.md' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    drop(conn);

    let md = app.mgr.build_rejoin_markdown(parent.id, &[c.id]).await.unwrap();
    assert!(md.starts_with("Children resolved. Resume."));
    assert!(md.contains(&c.id.simple().to_string()[..8]));
    assert!(md.contains("status=complete"));
    assert!(md.contains("handoff=/tmp/h.md"));
}

#[tokio::test]
async fn on_child_terminal_clears_one_of_two_children() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    app.mgr.on_child_terminal(c1.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending): (Option<String>, Option<String>) = conn.query_row(
        "SELECT waiting_on, pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    let arr: Vec<String> = serde_json::from_str(waiting.as_deref().unwrap()).unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr.contains(&c2.id.to_string()));
    assert!(pending.is_none(), "no rejoin yet — c2 still pending");
}

#[tokio::test]
async fn on_child_terminal_writes_pending_rejoin_when_last() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();

    app.mgr.on_child_terminal(c.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending, term_status): (Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT s.waiting_on, s.pending_rejoin, c.terminal_status \
             FROM sessions s JOIN sessions c ON c.id = ?2 WHERE s.id = ?1",
            [parent.id.to_string(), c.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
    assert!(waiting.is_none(), "waiting_on cleared");
    assert!(pending.as_deref().unwrap().contains("Children resolved"));
    assert_eq!(term_status.as_deref(), Some("complete"));
}

#[tokio::test]
async fn status_report_complete_triggers_on_child_terminal_via_state() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();

    // Simulate the child reporting complete via the same path mcp tools use.
    galvolite::state::apply_event_with_bus(
        &app.db, c.id, app.mgr.event_bus(),
        galvolite::state::EventInput {
            source: galvolite::state::Source::Mcp,
            kind: "mcp.status_report".into(),
            payload: serde_json::json!({"kind":"complete"}),
            dedup_key: None,
        }
    ).unwrap();

    // Allow the spawned task in apply_event_with_bus to run.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let conn = app.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}

#[tokio::test]
async fn mcp_spawn_session_returns_child_descriptor() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"spawn_session","arguments":{"role":"implementation","task":"refactor auth"}}
    });
    let resp = common::rpc(app.app.clone(), &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(inner["session_id"].as_str().is_some());
    assert!(inner["worktree_path"].as_str().unwrap().contains(".galvolite/worktrees"));
    assert!(inner["branch"].as_str().unwrap().starts_with("galvolite/"));
}

#[tokio::test]
async fn mcp_spawn_session_rejects_unknown_role() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"spawn_session","arguments":{"role":"president","task":"x"}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn mcp_wait_children_records_waiting_on() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![],
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    let token = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();

    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "wait_children", "arguments": {"children": [c.id.to_string()]}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["ok"], true);
    assert_eq!(inner["immediate_rejoin"], false);

    let conn = app.db.pool().get().unwrap();
    let waiting: String = conn.query_row(
        "SELECT waiting_on FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0),
    ).unwrap();
    assert!(waiting.contains(&c.id.to_string()));
}

#[tokio::test]
async fn mcp_wait_children_short_circuits_on_already_terminal() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![],
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.db.pool().get().unwrap().execute(
        "UPDATE sessions SET terminal_status = 'complete' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    let token = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get::<_, String>(0),
    ).unwrap();

    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "wait_children", "arguments": {"children": [c.id.to_string()]}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["immediate_rejoin"], true);

    let conn = app.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0),
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}

/// Regression: smoke bug 2 — `Session` struct missing `waiting_on` field meant
/// `/api/sessions` and `/api/sessions/:id` silently elided the column even
/// though the DB held it. DB-level round-trip tests passed; the wire shape
/// did not. This locks the wire contract: HTTP responses must include
/// `waiting_on` as a JSON array of UUID strings.
#[tokio::test]
async fn api_get_and_list_sessions_serialize_waiting_on_as_array() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![],
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    // GET /api/sessions/:id — single-session shape
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}", parent.id))
        .body(Body::empty()).unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v["waiting_on"].as_array()
        .expect("GET /api/sessions/:id must serialize waiting_on as JSON array");
    let ids: Vec<&str> = arr.iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&c1.id.to_string().as_str()));
    assert!(ids.contains(&c2.id.to_string().as_str()));

    // GET /api/sessions — list shape
    let req = Request::builder()
        .method("GET")
        .uri("/api/sessions")
        .body(Body::empty()).unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let list = v.as_array().unwrap();
    let pid = parent.id.to_string();
    let parent_obj = list.iter().find(|s| s["id"] == pid)
        .expect("list must include parent session");
    let arr = parent_obj["waiting_on"].as_array()
        .expect("GET /api/sessions must serialize waiting_on as JSON array");
    assert_eq!(arr.len(), 2);
}
