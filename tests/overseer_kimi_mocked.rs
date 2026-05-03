use galvolite::overseer::kimi::{KimiClient, KimiError};

#[tokio::test]
async fn happy_path_returns_text() {
    let mut server = mockito::Server::new_async().await;
    let m = server.mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer test-key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"choices":[{"message":{"content":"hello"}}],
                       "usage":{"total_tokens":42}}"#)
        .create_async().await;

    let c = KimiClient::new("test-key", &server.url());
    let (text, tokens) = c.chat("system", "user").await.unwrap();
    assert_eq!(text, "hello");
    assert_eq!(tokens, 42);
    m.assert_async().await;
}

#[tokio::test]
async fn http_5xx_returns_transient_error() {
    let mut server = mockito::Server::new_async().await;
    server.mock("POST", "/chat/completions").with_status(503).create_async().await;
    let c = KimiClient::new("k", &server.url());
    let err = c.chat("s", "u").await.unwrap_err();
    assert!(matches!(err, KimiError::Transient(_)));
}
