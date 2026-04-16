use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn add_and_list_project() {
    let data_dir = TempDir::new().unwrap();
    let proj_dir = TempDir::new().unwrap();

    Command::cargo_bin("galvolite")
        .unwrap()
        .args(&[
            "--data-dir", &data_dir.path().to_string_lossy(),
            "add-project", &proj_dir.path().to_string_lossy(),
            "--name", "Test Project",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added project"));

    Command::cargo_bin("galvolite")
        .unwrap()
        .args(&["--data-dir", &data_dir.path().to_string_lossy(), "projects"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Project"));
}
