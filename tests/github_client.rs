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

// ===== Plan 10b Task 2 — default_branch + create_pull_request =====

fn mock_repo_payload_with_default_branch(default_branch: &str) -> String {
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
            "default_branch": "{default_branch}"
        }}"#
    )
}

#[tokio::test]
async fn default_branch_returns_value_from_response() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_repo_payload_with_default_branch("trunk"))
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let branch = client.default_branch("octocat", "hello").await.unwrap();
    assert_eq!(branch, "trunk");
}

fn mock_pr_create_payload() -> String {
    let octocat = author_json("octocat", 1);
    format!(
        r#"{{
            "url": "https://api.github.com/repos/octocat/hello/pulls/42",
            "id": 999,
            "node_id": "x",
            "html_url": "https://github.com/octocat/hello/pull/42",
            "diff_url": "https://github.com/octocat/hello/pull/42.diff",
            "patch_url": "https://github.com/octocat/hello/pull/42.patch",
            "issue_url": "https://api.github.com/repos/octocat/hello/issues/42",
            "number": 42,
            "state": "open",
            "locked": false,
            "title": "Add token-budget reset",
            "user": {octocat},
            "body": "...",
            "created_at": "2026-05-09T00:00:00Z",
            "updated_at": "2026-05-09T00:00:00Z",
            "closed_at": null,
            "merged_at": null,
            "merge_commit_sha": null,
            "draft": false,
            "head": {{"label":"octocat:galvolite/abc12345","ref":"galvolite/abc12345","sha":"abc","user":null,"repo":null}},
            "base": {{"label":"octocat:main","ref":"main","sha":"def","user":null,"repo":null}},
            "_links": {{}},
            "author_association": "OWNER"
        }}"#
    )
}

#[tokio::test]
async fn create_pull_request_returns_pr_info_on_201() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/repos/octocat/hello/pulls")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(mock_pr_create_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let info = client
        .create_pull_request(
            "octocat",
            "hello",
            "galvolite/abc12345",
            "main",
            "Add token-budget reset",
            "## Task\n...",
            false,
        )
        .await
        .unwrap();
    assert_eq!(info.number, 42);
    assert_eq!(info.html_url, "https://github.com/octocat/hello/pull/42");
}

#[tokio::test]
async fn create_pull_request_err_on_422() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/repos/octocat/hello/pulls")
        .with_status(422)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","code":"custom","message":"A pull request already exists for octocat:galvolite/abc12345."}]}"#)
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let err = client
        .create_pull_request(
            "octocat",
            "hello",
            "galvolite/abc12345",
            "main",
            "x",
            "y",
            false,
        )
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("422") || format!("{err}").contains("Validation"));
}

#[tokio::test]
async fn default_branch_err_on_404() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/missing")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"message":"Not Found"}"#)
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    assert!(client.default_branch("octocat", "missing").await.is_err());
}

// ===== Plan 10b Task 4 — token_scopes (x-oauth-scopes header parsing) =====

#[tokio::test]
async fn token_scopes_parses_x_oauth_scopes_header() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/user")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_header("x-oauth-scopes", "repo, read:org, workflow")
        .with_body(mock_user_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let scopes = client.token_scopes().await.unwrap();
    assert!(scopes.contains(&"repo".to_string()));
    assert!(scopes.contains(&"read:org".to_string()));
    assert!(scopes.contains(&"workflow".to_string()));
}

#[tokio::test]
async fn token_scopes_returns_empty_when_header_absent() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/user")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_user_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let scopes = client.token_scopes().await.unwrap();
    assert!(scopes.is_empty());
}

// ===== Plan 10c — list_pr_reviews (reviews + inline comments) =====

fn mock_review_payload() -> String {
    let alice = author_json("alice", 5);
    format!(
        r#"[{{
            "id": 100,
            "node_id": "x",
            "user": {alice},
            "body": "LGTM",
            "state": "APPROVED",
            "html_url": "https://github.com/octocat/hello/pull/42#pullrequestreview-100",
            "pull_request_url": "https://api.github.com/repos/octocat/hello/pulls/42",
            "_links": {{}},
            "submitted_at": "2026-05-01T00:00:00Z",
            "commit_id": "abc",
            "author_association": "OWNER"
        }}]"#
    )
}

fn mock_review_comment_payload() -> String {
    let bob = author_json("bob", 6);
    format!(
        r#"[{{
            "url": "https://api.github.com/repos/octocat/hello/pulls/comments/200",
            "pull_request_review_id": 100,
            "id": 200,
            "node_id": "x",
            "diff_hunk": "@@ -1,1 +1,1 @@",
            "path": "src/main.rs",
            "commit_id": "abc",
            "original_commit_id": "abc",
            "user": {bob},
            "body": "consider extracting this",
            "created_at": "2026-05-01T00:00:00Z",
            "updated_at": "2026-05-01T00:00:00Z",
            "html_url": "https://github.com/octocat/hello/pull/42#discussion_r200",
            "pull_request_url": "https://api.github.com/repos/octocat/hello/pulls/42",
            "author_association": "CONTRIBUTOR",
            "_links": {{}},
            "line": 42,
            "original_line": 42
        }}]"#
    )
}

#[tokio::test]
async fn list_pr_reviews_returns_bundle_on_success() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello/pulls/42/reviews")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_review_payload())
        .create_async()
        .await;
    server
        .mock("GET", "/repos/octocat/hello/pulls/42/comments")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(mock_review_comment_payload())
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    let bundle = client
        .list_pr_reviews("octocat", "hello", 42)
        .await
        .expect("list_pr_reviews");
    assert_eq!(bundle.reviews.len(), 1);
    assert_eq!(bundle.reviews[0].author, "alice");
    assert!(bundle.reviews[0].state.contains("APPROVED"));
    assert_eq!(bundle.reviews[0].body, "LGTM");
    assert_eq!(bundle.comments.len(), 1);
    assert_eq!(bundle.comments[0].author, "bob");
    assert_eq!(bundle.comments[0].path, "src/main.rs");
    assert_eq!(bundle.comments[0].line, Some(42));
    assert!(bundle.comments[0]
        .url
        .contains("discussion_r200"));
}

#[tokio::test]
async fn list_pr_reviews_err_on_404() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/repos/octocat/hello/pulls/9999/reviews")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .with_body(r#"{"message":"Not Found"}"#)
        .create_async()
        .await;
    // Comments mock not strictly required since tokio::join! returns
    // both errors; one suffices, but providing it makes the test stable
    // regardless of which leg races first.
    server
        .mock("GET", "/repos/octocat/hello/pulls/9999/comments")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .with_body(r#"{"message":"Not Found"}"#)
        .create_async()
        .await;

    let client = Client::with_token("t", Some(server.url())).unwrap();
    assert!(client
        .list_pr_reviews("octocat", "hello", 9999)
        .await
        .is_err());
}
