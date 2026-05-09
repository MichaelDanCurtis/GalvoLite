//! Plan 10a Task 2 — github::Client tests against a mockito-backed
//! impersonation of api.github.com.
//!
//! NOTE: octocrab does strict deserialization with non-exhaustive structs
//! whose fields are mostly required. We hand-roll JSON payloads that
//! include every required field (especially every URL field on Author).
//! When in doubt about whether a field is required, look at
//! ~/.cargo/registry/src/index.crates.io-*/octocrab-*/src/models{,/issues,/pulls}.rs.

use galvolite::github::Client;

fn author_json(login: &str, id: u64) -> String {
    format!(
        r#"{{
            "login": "{login}",
            "id": {id},
            "node_id": "MDQ6VXNlcjE=",
            "avatar_url": "https://avatars.githubusercontent.com/u/{id}",
            "gravatar_id": "",
            "url": "https://api.github.com/users/{login}",
            "html_url": "https://github.com/{login}",
            "followers_url": "https://api.github.com/users/{login}/followers",
            "following_url": "https://api.github.com/users/{login}/following{{/other_user}}",
            "gists_url": "https://api.github.com/users/{login}/gists{{/gist_id}}",
            "starred_url": "https://api.github.com/users/{login}/starred{{/owner}}{{/repo}}",
            "subscriptions_url": "https://api.github.com/users/{login}/subscriptions",
            "organizations_url": "https://api.github.com/users/{login}/orgs",
            "repos_url": "https://api.github.com/users/{login}/repos",
            "events_url": "https://api.github.com/users/{login}/events{{/privacy}}",
            "received_events_url": "https://api.github.com/users/{login}/received_events",
            "type": "User",
            "site_admin": false
        }}"#
    )
}

fn mock_user_payload() -> String {
    author_json("octocat", 1)
}

#[tokio::test]
async fn validate_returns_user_on_200() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/user")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_user_payload())
        .create_async()
        .await;

    let client = Client::with_token("test-token", Some(server.url())).unwrap();
    let user = client.validate().await.expect("validate");
    assert_eq!(user.login, "octocat");
    assert!(user.avatar_url.contains("avatars.githubusercontent.com"));
    m.assert_async().await;
}

#[tokio::test]
async fn validate_returns_err_on_401() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/user")
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"Bad credentials"}"#)
        .create_async()
        .await;

    let client = Client::with_token("bad", Some(server.url())).unwrap();
    assert!(client.validate().await.is_err());
}

fn mock_prs_payload() -> String {
    let alice = author_json("alice", 2);
    format!(
        r#"[
            {{
                "url": "https://api.github.com/repos/octocat/hello/pulls/1",
                "id": 1,
                "node_id": "x",
                "html_url": "https://github.com/octocat/hello/pull/1",
                "diff_url": "https://github.com/octocat/hello/pull/1.diff",
                "patch_url": "https://github.com/octocat/hello/pull/1.patch",
                "issue_url": "https://api.github.com/repos/octocat/hello/issues/1",
                "number": 1,
                "state": "open",
                "locked": false,
                "title": "First PR",
                "user": {alice},
                "body": null,
                "created_at": "2026-05-01T00:00:00Z",
                "updated_at": "2026-05-02T00:00:00Z",
                "closed_at": null,
                "merged_at": null,
                "merge_commit_sha": null,
                "draft": false,
                "head": {{"label":"alice:topic","ref":"topic","sha":"abc","user":null,"repo":null}},
                "base": {{"label":"octocat:main","ref":"main","sha":"def","user":null,"repo":null}},
                "_links": {{}},
                "author_association": "CONTRIBUTOR"
            }}
        ]"#
    )
}

#[tokio::test]
async fn list_open_prs_returns_summaries() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello/pulls")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("state".into(), "open".into()),
            mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_prs_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let prs = client.list_open_prs("octocat", "hello").await.unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].number, 1);
    assert_eq!(prs[0].title, "First PR");
    assert_eq!(prs[0].author, "alice");
    assert!(!prs[0].draft);
    assert_eq!(prs[0].url, "https://github.com/octocat/hello/pull/1");
}

fn mock_issues_payload() -> String {
    // Two items: one real issue, one that is actually a PR (has
    // pull_request field) — the second must be filtered out.
    let bob = author_json("bob", 3);
    let carol = author_json("carol", 4);
    format!(
        r#"[
            {{
                "url": "https://api.github.com/repos/octocat/hello/issues/2",
                "repository_url": "https://api.github.com/repos/octocat/hello",
                "labels_url": "https://api.github.com/repos/octocat/hello/issues/2/labels{{/name}}",
                "comments_url": "https://api.github.com/repos/octocat/hello/issues/2/comments",
                "events_url": "https://api.github.com/repos/octocat/hello/issues/2/events",
                "id": 10,
                "node_id": "i",
                "html_url": "https://github.com/octocat/hello/issues/2",
                "number": 2,
                "state": "open",
                "state_reason": null,
                "title": "Real bug",
                "body": null,
                "user": {bob},
                "labels": [{{"id":1,"node_id":"l","url":"https://api.github.com/repos/octocat/hello/labels/bug","name":"bug","color":"f00","default":true,"description":null}}],
                "assignee": null,
                "assignees": [],
                "milestone": null,
                "locked": false,
                "comments": 0,
                "closed_at": null,
                "created_at": "2026-05-01T00:00:00Z",
                "updated_at": "2026-05-02T00:00:00Z",
                "author_association": "CONTRIBUTOR"
            }},
            {{
                "url": "https://api.github.com/repos/octocat/hello/issues/3",
                "repository_url": "https://api.github.com/repos/octocat/hello",
                "labels_url": "https://api.github.com/repos/octocat/hello/issues/3/labels{{/name}}",
                "comments_url": "https://api.github.com/repos/octocat/hello/issues/3/comments",
                "events_url": "https://api.github.com/repos/octocat/hello/issues/3/events",
                "id": 11,
                "node_id": "j",
                "html_url": "https://github.com/octocat/hello/pull/3",
                "number": 3,
                "state": "open",
                "state_reason": null,
                "title": "This is a PR",
                "body": null,
                "user": {carol},
                "labels": [],
                "assignee": null,
                "assignees": [],
                "milestone": null,
                "locked": false,
                "comments": 0,
                "closed_at": null,
                "created_at": "2026-05-01T00:00:00Z",
                "updated_at": "2026-05-02T00:00:00Z",
                "author_association": "CONTRIBUTOR",
                "pull_request": {{
                    "url":"https://api.github.com/repos/octocat/hello/pulls/3",
                    "html_url":"https://github.com/octocat/hello/pull/3",
                    "diff_url":"https://github.com/octocat/hello/pull/3.diff",
                    "patch_url":"https://github.com/octocat/hello/pull/3.patch"
                }}
            }}
        ]"#
    )
}

#[tokio::test]
async fn list_open_issues_filters_out_prs() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello/issues")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_issues_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let issues = client.list_open_issues("octocat", "hello").await.unwrap();
    assert_eq!(issues.len(), 1, "the PR-shaped issue should be filtered");
    assert_eq!(issues[0].number, 2);
    assert_eq!(issues[0].title, "Real bug");
    assert_eq!(issues[0].labels, vec!["bug".to_string()]);
}

fn mock_repo_payload() -> String {
    let owner = author_json("octocat", 1);
    format!(
        r#"{{
            "id": 1,
            "node_id": "x",
            "name": "hello",
            "full_name": "octocat/hello",
            "private": false,
            "owner": {owner},
            "html_url": "https://github.com/octocat/hello",
            "description": null,
            "fork": false,
            "url": "https://api.github.com/repos/octocat/hello",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "pushed_at": "2026-01-01T00:00:00Z",
            "clone_url": "https://github.com/octocat/hello.git",
            "size": 0,
            "stargazers_count": 0,
            "watchers_count": 0,
            "forks_count": 0,
            "open_issues_count": 0,
            "default_branch": "main"
        }}"#
    )
}

#[tokio::test]
async fn validate_repo_ok_on_200() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_repo_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    assert!(client.validate_repo("octocat", "hello").await.is_ok());
}

#[tokio::test]
async fn validate_repo_err_on_404() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/missing")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"Not Found"}"#)
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    assert!(client.validate_repo("octocat", "missing").await.is_err());
}
