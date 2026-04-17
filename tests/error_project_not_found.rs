use galvolite::error::Error;
use uuid::Uuid;

#[test]
fn project_not_found_has_clear_display() {
    let id = Uuid::new_v4();
    let e = Error::ProjectNotFound(id);
    let s = e.to_string();
    assert!(s.contains("project"), "display should mention project: {s}");
    assert!(s.contains(&id.to_string()), "display should include id: {s}");
}
