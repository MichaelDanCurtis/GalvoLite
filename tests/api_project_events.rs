mod common;

use futures_util::StreamExt;
use galvolite::api::events::DomainEvent;
use tokio_tungstenite::tungstenite::Message;

/// Helper — drain text frames until the first one matching `predicate`,
/// or panic on timeout after `secs`.
async fn await_text_frame<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    secs: u64,
    predicate: F,
) -> serde_json::Value
where
    F: Fn(&serde_json::Value) -> bool,
{
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if predicate(&v) {
                        return v;
                    }
                }
                Some(Ok(_)) => continue, // non-text frame; skip
                Some(Err(e)) => panic!("ws error while waiting for matching frame: {e}"),
                None => panic!("ws stream closed before matching frame arrived"),
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(secs), fut)
        .await
        .expect("did not receive matching frame in time")
}

/// Helper — assert NO text frame matching `predicate` arrives within `ms`.
async fn assert_no_text_frame<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ms: u64,
    predicate: F,
) where
    F: Fn(&serde_json::Value) -> bool,
{
    let fut = async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if predicate(&v) {
                        panic!("unexpected matching frame arrived: {v}");
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("ws error during negative assertion: {e}"),
                None => panic!("ws closed during negative assertion — cannot conclude absence"),
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_millis(ms), fut).await;
    // timeout elapsing without panic is the success case
}

fn mk_project(app: &common::TestApp) -> uuid::Uuid {
    app.mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: app._dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap()
        .id
}

#[tokio::test]
async fn project_ws_delivers_issue_opened_for_matched_project() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::IssueOpened {
        project_id: pid,
        issue_url: "https://github.com/o/r/issues/1".into(),
        issue_number: 1,
        title: "smoke".into(),
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "github.issue_opened").await;
    assert_eq!(v["issue_number"], 1);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_filters_out_events_for_other_project() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);
    let other_pid = uuid::Uuid::new_v4();

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::IssueOpened {
        project_id: other_pid,
        issue_url: "https://github.com/o/r/issues/99".into(),
        issue_number: 99,
        title: "other".into(),
    });

    assert_no_text_frame(&mut ws, 300, |v| v["type"] == "github.issue_opened").await;
}

#[tokio::test]
async fn project_ws_filters_out_session_only_events() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // OperatorToast has only session_id, no project_id — must be dropped.
    app.mgr.event_bus().publish(DomainEvent::OperatorToast {
        session_id: uuid::Uuid::new_v4(),
        level: "info".into(),
        message: "should not arrive".into(),
    });

    assert_no_text_frame(&mut ws, 300, |v| v["type"] == "operator.toast").await;
}

#[tokio::test]
async fn project_ws_delivers_pull_request_merged() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::PullRequestMerged {
        project_id: pid,
        pr_url: "https://github.com/o/r/pull/42".into(),
        pr_number: 42,
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "github.pull_request_merged").await;
    assert_eq!(v["pr_number"], 42);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_delivers_session_pr_created_dual_scoped() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::SessionPrCreated {
        session_id: uuid::Uuid::new_v4(),
        project_id: pid,
        pr_url: "https://github.com/o/r/pull/7".into(),
        pr_number: 7,
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "session.pr_created").await;
    assert_eq!(v["pr_number"], 7);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_returns_404_for_unknown_project_before_upgrade() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;

    // Use a non-existent project id — server should refuse the WS upgrade with 404.
    let pid = uuid::Uuid::new_v4();
    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");

    let result = tokio_tungstenite::connect_async(url).await;
    assert!(result.is_err(), "expected upgrade failure for unknown project, got success");
    let err = result.unwrap_err();

    // Inspect the error variant directly to assert precisely on 404.
    use tokio_tungstenite::tungstenite::error::Error as WsError;
    use tokio_tungstenite::tungstenite::http::StatusCode;
    match err {
        WsError::Http(resp) => {
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "expected HTTP 404 NOT_FOUND, got: {}",
                resp.status()
            );
        }
        other => panic!("expected Http(404) error, got: {other:?}"),
    }
}
