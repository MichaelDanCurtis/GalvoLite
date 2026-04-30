use galvolite::db::{Db, NewProject, NewSession};
use tempfile::tempdir;
use uuid::Uuid;

fn setup_db() -> (Db, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("test.db")).unwrap();
    (db, dir)
}

#[test]
fn delete_project_cascade_removes_project_sessions() {
    let (db, _dir) = setup_db();

    let project = db
        .insert_project(NewProject {
            name: "proj".into(),
            path: "/tmp/proj".into(),
            remote_url: None,
        })
        .unwrap();

    let _session = db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: "/tmp/proj".into(),
            parent_session_id: None,
            task_summary: "test".into(),
        })
        .unwrap();

    db.delete_project_cascade(project.id).unwrap();

    assert!(db.get_project(project.id).is_err(), "project should be gone");
    assert_eq!(db.list_sessions().unwrap().len(), 0, "sessions purged");
}

#[test]
fn delete_project_cascade_returns_not_found_for_unknown_id() {
    let (db, _dir) = setup_db();
    let res = db.delete_project_cascade(Uuid::new_v4());
    assert!(matches!(
        res,
        Err(galvolite::error::Error::ProjectNotFound(_))
    ));
}
