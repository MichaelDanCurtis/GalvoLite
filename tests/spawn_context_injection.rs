//! Task 12: spawn-time context injection helper.

use chrono::Utc;
use galvolite::claude_md::build_prior_context;
use galvolite::decisions::{record as record_decision, Decision};
use galvolite::postmortem::{write as write_postmortem, Outcome, Postmortem};
use uuid::Uuid;

#[test]
fn returns_empty_when_no_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let out = build_prior_context(dir.path(), Some(5));
    assert_eq!(out, "");
}

#[test]
fn injects_postmortem_for_matching_part() {
    let dir = tempfile::tempdir().unwrap();
    write_postmortem(dir.path(), &Postmortem {
        plan_part: 5,
        session_id: Uuid::new_v4(),
        outcome: Outcome::Stuck,
        cycles: 4,
        duration_seconds: 3600,
        written_at: Utc::now(),
        what_worked: String::new(),
        gotchas: "DB lock contention with parallel migrations".into(),
        follow_ups: String::new(),
    }).unwrap();
    let out = build_prior_context(dir.path(), Some(5));
    assert!(out.contains("## Prior context"));
    assert!(out.contains("DB lock contention"));
    assert!(out.contains("part 5"));
}

#[test]
fn caps_at_2kib_and_drops_oldest() {
    let dir = tempfile::tempdir().unwrap();
    // Write 10 postmortems for part=7, each ~500 bytes of gotchas.
    let big_gotcha = "x".repeat(500);
    for i in 0..10u32 {
        write_postmortem(dir.path(), &Postmortem {
            plan_part: 7,
            session_id: Uuid::new_v4(),
            outcome: Outcome::Shipped,
            cycles: i,
            duration_seconds: 60,
            // Stagger written_at so list_for_part can sort deterministically.
            written_at: Utc::now() + chrono::Duration::seconds(i as i64),
            what_worked: String::new(),
            gotchas: big_gotcha.clone(),
            follow_ups: String::new(),
        }).unwrap();
        // Sleep 1ms between writes to ensure unique fs timestamps if any code path uses them.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let out = build_prior_context(dir.path(), Some(7));
    assert!(out.len() <= 2048, "out.len() = {} > 2048", out.len());
    // Newest 3 are taken (cycles 9, 8, 7), but cap may stop earlier.
    // At minimum the newest one (cycle 9) must be present.
    assert!(out.contains("9 cycles"), "newest postmortem missing");
}

#[test]
fn injects_recent_decisions() {
    let dir = tempfile::tempdir().unwrap();
    record_decision(dir.path(), &Decision {
        session_id: Uuid::new_v4(),
        asked_at: Utc::now(),
        question: "Use Postgres or SQLite?".into(),
        answer: "SQLite for now".into(),
        choices: None,
    }).unwrap();
    let out = build_prior_context(dir.path(), None);
    assert!(out.contains("## Prior context"));
    assert!(out.contains("Use Postgres or SQLite?"));
    assert!(out.contains("SQLite for now"));
}
