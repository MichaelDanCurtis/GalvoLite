use galvolite::db::Db;
use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::overseer::observer::ContextSnapshot;
use galvolite::overseer::recommender::{review, OverseerState};
use galvolite::recommendations::{self, RecStatus};
use uuid::Uuid;

#[tokio::test]
async fn happy_path_inserts_and_emits() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let mut sub = bus.subscribe();
    let pid = Uuid::new_v4();

    let mut state = OverseerState::new(100_000);
    let kimi_text = r#"{"recommendations":[
        {"kind":"stuck","severity":"warn","title":"T","body":"B","rationale":"R","evidence_refs":[]}
    ]}"#;
    let snap = ContextSnapshot::default();
    review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
        Ok::<_, galvolite::overseer::kimi::KimiError>((kimi_text.to_string(), 50u64))
    }).await.unwrap();

    let recs = recommendations::list_by_status(&db, pid, RecStatus::Pending).unwrap();
    assert_eq!(recs.len(), 1);
    let ev = sub.try_recv().unwrap();
    assert!(matches!(ev, DomainEvent::RecommendationCreated { .. }));
    assert_eq!(state.tokens_used, 50);
}

#[tokio::test]
async fn five_failures_in_a_row_emit_degraded() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let mut sub = bus.subscribe();
    let pid = Uuid::new_v4();
    let mut state = OverseerState::new(100_000);
    let snap = ContextSnapshot::default();

    for _ in 0..5 {
        let _ = review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
            Err::<(String, u64), _>(galvolite::overseer::kimi::KimiError::Transient("nope".into()))
        }).await;
    }
    let mut saw_degraded = false;
    while let Ok(ev) = sub.try_recv() {
        if matches!(ev, DomainEvent::OverseerDegraded { .. }) { saw_degraded = true; }
    }
    assert!(saw_degraded);
}

#[tokio::test]
async fn token_budget_blocks_call() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let pid = Uuid::new_v4();
    let mut state = OverseerState::new(10);
    state.tokens_used = 50;
    let snap = ContextSnapshot::default();
    let res = review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
        panic!("should not be called");
    }).await;
    assert!(res.is_err());
}

#[test]
fn pack_prompt_includes_pty_tails_when_present() {
    use galvolite::overseer::observer::ContextSnapshot;
    let mut snap = ContextSnapshot::default();
    let sid = uuid::Uuid::new_v4();
    snap.tail_per_session.insert(sid, "claude> running tests\nFAIL: 3 tests".into());

    let (_, user) = galvolite::overseer::recommender::pack_prompt(&snap);
    assert!(user.contains("# Recent PTY tails"), "expected PTY tail header");
    assert!(user.contains("FAIL: 3 tests"), "expected tail body");
    assert!(user.contains(&sid.to_string()), "expected session id in output");
}

#[test]
fn pack_prompt_omits_pty_section_when_map_empty() {
    use galvolite::overseer::observer::ContextSnapshot;
    let snap = ContextSnapshot::default();
    let (_, user) = galvolite::overseer::recommender::pack_prompt(&snap);
    assert!(!user.contains("# Recent PTY tails"));
}

#[test]
fn budget_resets_at_utc_date_rollover() {
    use galvolite::overseer::recommender::OverseerState;

    // Construct a state with a very small budget, exhaust it, then
    // simulate a date rollover by mutating the test-only setter.
    let mut s = OverseerState::new(100);
    s.tokens_used = 100;
    assert!(s.is_budget_exhausted());

    // Plan 9 Task 5 — public test-mode helper to simulate yesterday.
    s.set_last_reset_for_test(chrono::Utc::now().date_naive() - chrono::Duration::days(1));

    // The reset is performed inside the budget check; force a check.
    s.maybe_reset_for_today();
    assert_eq!(s.tokens_used, 0, "tokens should reset after date rollover");
    assert!(!s.is_budget_exhausted());
}
