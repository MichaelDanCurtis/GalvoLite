use galvolite::db::{Db, NewProject, NewSession};
use galvolite::error::Error;
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn set_session_role_updates_row() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();

    let project = db
        .insert_project(NewProject {
            name: "p".into(),
            path: "/tmp/p".into(),
            remote_url: None,
        })
        .unwrap();

    let session = db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: "/tmp/p".into(),
            parent_session_id: None,
            task_summary: "t".into(),
        })
        .unwrap();

    db.set_session_role(session.id, "brainstorm").unwrap();

    let updated = db.get_session(session.id).unwrap();
    assert_eq!(updated.role, "brainstorm");
}

#[test]
fn set_session_role_session_not_found() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let res = db.set_session_role(Uuid::new_v4(), "x");
    assert!(matches!(res, Err(Error::SessionNotFound(_))));
}
