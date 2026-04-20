//! Bearer-token resolver — constant-time, revocable.

use galvolite::auth;
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::error::Error;
use tempfile::TempDir;
use uuid::Uuid;

fn fresh_db_with_session() -> (TempDir, Db, Uuid) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let proj_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let project = db
        .insert_project(NewProject {
            name: "p".into(),
            path: proj_dir.to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    (tmp, db, session.id)
}

#[test]
fn valid_token_resolves_to_session_id() {
    let (_tmp, db, sid) = fresh_db_with_session();
    db.set_api_token(sid, "token-xyz").unwrap();
    assert_eq!(auth::resolve_token(&db, "token-xyz").unwrap(), sid);
}

#[test]
fn unknown_token_returns_unauthorized() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let err = auth::resolve_token(&db, "no-such-token").unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
}

#[test]
fn empty_token_returns_unauthorized() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let err = auth::resolve_token(&db, "").unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
}

#[test]
fn revoked_token_no_longer_resolves() {
    let (_tmp, db, sid) = fresh_db_with_session();
    db.set_api_token(sid, "tok-1").unwrap();
    assert!(auth::resolve_token(&db, "tok-1").is_ok());

    db.clear_api_token(sid).unwrap();
    let err = auth::resolve_token(&db, "tok-1").unwrap_err();
    assert!(matches!(err, Error::Unauthorized));
}

#[test]
fn distinct_tokens_resolve_to_distinct_sessions() {
    let (_tmp, db, sid_a) = fresh_db_with_session();
    // Add a second session in the same DB.
    let sid_b = {
        let project = db.insert_project(NewProject {
            name: "p2".into(),
            path: _tmp.path().join("proj2").to_string_lossy().into_owned(),
            remote_url: None,
        });
        std::fs::create_dir_all(_tmp.path().join("proj2")).unwrap();
        let project = project.unwrap();
        db.insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap()
        .id
    };
    db.set_api_token(sid_a, "tok-a").unwrap();
    db.set_api_token(sid_b, "tok-b").unwrap();

    assert_eq!(auth::resolve_token(&db, "tok-a").unwrap(), sid_a);
    assert_eq!(auth::resolve_token(&db, "tok-b").unwrap(), sid_b);
}
