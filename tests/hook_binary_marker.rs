use galvolite::hooks::client::{fetch_pending_rejoin, ClientEnv};

#[test]
fn fetch_pending_rejoin_returns_markdown_on_200() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock(
            "GET",
            "/api/sessions/00000000-0000-0000-0000-000000000001/pending-rejoin",
        )
        .match_header("authorization", "Bearer tok-x")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"markdown":"Children resolved. Resume."}"#)
        .create();

    let env = ClientEnv {
        api_base: server.url(),
        session_id: "00000000-0000-0000-0000-000000000001".into(),
        token: "tok-x".into(),
        timeout_secs: 5,
    };
    let md = fetch_pending_rejoin(&env).unwrap();
    assert_eq!(md.as_deref(), Some("Children resolved. Resume."));
}

#[test]
fn fetch_pending_rejoin_returns_none_on_404() {
    let mut server = mockito::Server::new();
    let _m = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(404)
        .create();
    let env = ClientEnv {
        api_base: server.url(),
        session_id: "00000000-0000-0000-0000-000000000001".into(),
        token: "tok-x".into(),
        timeout_secs: 5,
    };
    let md = fetch_pending_rejoin(&env).unwrap();
    assert!(md.is_none());
}
