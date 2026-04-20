use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{self, EventInput, SessionState, Source};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn starting_to_idle_on_session_started_per_spec_8_6() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();

    let outcome = state::apply_event(
        &db,
        s.id,
        EventInput {
            source: Source::System,
            kind: "system.session_started".into(),
            payload: json!({}),
            dedup_key: Some(format!("session-started:{}", s.id)),
        },
    )
    .unwrap();

    assert_eq!(
        outcome.next_state,
        SessionState::Idle,
        "spec §8.6: system.session_started leaves session Idle, then UserPromptSubmit moves it to Working"
    );
}
