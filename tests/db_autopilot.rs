use galvolite::db::{Db, NewProject, NewSession};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn set_autopilot_flips_flag_and_persists() {
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
            // NO id field — struct doesn't expose one
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: "/tmp/p".into(),
            parent_session_id: None,
            task_summary: "t".into(),
        })
        .unwrap();

    assert!(!session.autopilot, "default should be false");

    db.set_session_autopilot(session.id, true).unwrap();
    assert!(db.get_session(session.id).unwrap().autopilot);

    db.set_session_autopilot(session.id, false).unwrap();
    assert!(!db.get_session(session.id).unwrap().autopilot);
}

#[test]
fn set_autopilot_unknown_session_errors() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let res = db.set_session_autopilot(Uuid::new_v4(), true);
    assert!(res.is_err());
}
