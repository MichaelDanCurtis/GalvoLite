# Plan 10a — GitHub Read-Only Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the foundational GitHub integration layer for GalvoLite — PAT-based auth via env var, per-project repo linkage, and read-only browsing of the linked repo's open PRs and issues from the dashboard.

**Architecture:** One new Rust subsystem under `src/github/` (octocrab wrapper with PAT auth + connected-user cache), one new SQLite migration adding `projects.github_repo`, three new HTTP endpoint groups under `src/api/github.rs`, and three new React components (`GithubStatusBadge`, `LinkRepoModal`, `GithubPanel`). All GitHub calls go through the Rust backend so the UI never sees the token. No mutations to GitHub. No background polling.

**Tech Stack:** Rust 1.x (axum 0.7, tokio, rusqlite 0.31, chrono, tracing, serde, **new: octocrab**), SQLite migrations under `migrations/`, React 18 + TypeScript + Vitest + Tailwind.

**Spec:** [docs/superpowers/specs/2026-05-07-plan-10a-github-readonly-design.md](../specs/2026-05-07-plan-10a-github-readonly-design.md)

---

## Task ordering

Sequential — each task lands one commit (with optional fix commits if review surfaces issues).

| # | Task | Layer |
|---|---|---|
| 1 | Migration + `Project.github_repo` field + DB setter | DB |
| 2 | `github::Client` (env-based ctor + validate + list_open_prs + list_open_issues) | GitHub client |
| 3 | API: `GET /api/github/status` | API |
| 4 | API: `PUT /api/projects/:id/github/repo` (validates via GitHub before DB write) | API |
| 5 | API: `GET /api/projects/:id/github/{prs,issues}` | API |
| 6 | UI: types + `api.ts` client functions for the new endpoints | UI plumbing |
| 7 | UI: `GithubStatusBadge` mounted in Dashboard header | UI |
| 8 | UI: `LinkRepoModal` per-project | UI |
| 9 | UI: `GithubPanel` mounted from project card | UI |

---

## Task 1: Migration 0007 + `Project.github_repo` field + DB setter

**Files:**
- Create: `migrations/0007_projects_github_repo.sql`
- Modify: `src/db.rs` (MIGRATIONS list + Project struct + insert/get/list/setter)
- Modify: `tests/db_init.rs:19` (schema-version assertion bump 6 → 7)
- Test: `tests/db_crud.rs` (extend with github_repo round-trip)

- [ ] **Step 1: Create migration `migrations/0007_projects_github_repo.sql`**

  ```sql
  -- Plan 10a Task 1 — add per-project linked GitHub repo (owner/name).
  -- Nullable; an unset value means the project has no linked repo.
  -- No CHECK constraint (we validate the format/existence at the API layer).
  ALTER TABLE projects ADD COLUMN github_repo TEXT NULL;
  ```

- [ ] **Step 2: Register migration in `src/db.rs::MIGRATIONS`**

  Modify the SCHEMA constants block in `src/db.rs` (around line 16-21):

  ```rust
  const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
  const SCHEMA_V2: &str = include_str!("../migrations/0002_hooks_mcp_foundations.sql");
  const SCHEMA_V3: &str = include_str!("../migrations/0003_session_spawn.sql");
  const SCHEMA_V4: &str = include_str!("../migrations/0004_overseer_executor.sql");
  const SCHEMA_V5: &str = include_str!("../migrations/0005_pending_phases.sql");
  const SCHEMA_V6: &str = include_str!("../migrations/0006_rec_archived.sql");
  const SCHEMA_V7: &str = include_str!("../migrations/0007_projects_github_repo.sql");
  ```

  And the MIGRATIONS list:

  ```rust
  const MIGRATIONS: &[(i64, &str)] = &[
      (1, SCHEMA_V1),
      (2, SCHEMA_V2),
      (3, SCHEMA_V3),
      (4, SCHEMA_V4),
      (5, SCHEMA_V5),
      (6, SCHEMA_V6),
      (7, SCHEMA_V7),
  ];
  ```

- [ ] **Step 3: Update schema-version test expectation**

  Modify `tests/db_init.rs:19`:

  ```rust
      assert_eq!(version, 7);
  ```

- [ ] **Step 4: Add `github_repo` to `Project` struct**

  Modify `src/db.rs` `Project` struct (around lines 133-139):

  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct Project {
      pub id: Uuid,
      pub name: String,
      pub path: String,
      pub remote_url: Option<String>,
      pub created_at: DateTime<Utc>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub github_repo: Option<String>,
  }
  ```

  (`skip_serializing_if` keeps existing API consumers' JSON shape unchanged when the field is absent.)

- [ ] **Step 5: Update `insert_project`, `get_project`, `list_projects` to read/write the column**

  In `src/db.rs::insert_project` (around line 187), the new row is inserted with `github_repo` as `NULL` (the schema default is implicit since the column is nullable; we don't set it in INSERT). Update the constructed `Project` value at the bottom of the function:

  ```rust
      pub fn insert_project(&self, new: NewProject) -> Result<Project> {
          let id = Uuid::new_v4();
          let now = Utc::now();
          let conn = self.pool.get()?;
          conn.execute(
              "INSERT INTO projects (id, name, path, remote_url, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5)",
              params![
                  id.to_string(),
                  new.name,
                  new.path,
                  new.remote_url,
                  now.to_rfc3339()
              ],
          )?;
          Ok(Project {
              id,
              name: new.name,
              path: new.path,
              remote_url: new.remote_url,
              created_at: now,
              github_repo: None,
          })
      }
  ```

  Modify `get_project` (around line 211) to SELECT and populate the new column:

  ```rust
      pub fn get_project(&self, id: Uuid) -> Result<Project> {
          let conn = self.pool.get()?;
          let row = conn.query_row(
              "SELECT id, name, path, remote_url, created_at, github_repo
               FROM projects WHERE id = ?1",
              params![id.to_string()],
              |r| {
                  Ok(Project {
                      id: parse_uuid(&r.get::<_, String>(0)?)?,
                      name: r.get(1)?,
                      path: r.get(2)?,
                      remote_url: r.get(3)?,
                      created_at: parse_datetime(&r.get::<_, String>(4)?)?,
                      github_repo: r.get(5)?,
                  })
              },
          )?;
          Ok(row)
      }
  ```

  And `list_projects` (around line 230):

  ```rust
      pub fn list_projects(&self) -> Result<Vec<Project>> {
          let conn = self.pool.get()?;
          let mut stmt = conn.prepare(
              "SELECT id, name, path, remote_url, created_at, github_repo
               FROM projects ORDER BY created_at ASC",
          )?;
          let rows = stmt
              .query_map([], |r| {
                  Ok(Project {
                      id: parse_uuid(&r.get::<_, String>(0)?)?,
                      name: r.get(1)?,
                      path: r.get(2)?,
                      remote_url: r.get(3)?,
                      created_at: parse_datetime(&r.get::<_, String>(4)?)?,
                      github_repo: r.get(5)?,
                  })
              })?
              .collect::<std::result::Result<Vec<_>, _>>()?;
          Ok(rows)
      }
  ```

  Search the file for any other place that constructs a `Project { ... }` literal (there may be 1-2 more — `parse_uuid` helper or similar). Add `github_repo: r.get(...)?` or `github_repo: None` as appropriate. The compiler will tell you what's missing.

- [ ] **Step 6: Add `set_project_github_repo` setter on `Db`**

  Append to the `impl Db` block in `src/db.rs` (after `list_projects`):

  ```rust
      /// Plan 10a Task 1 — set or clear the linked GitHub repo for a project.
      /// Pass `None` to clear. The value is stored verbatim; format/existence
      /// validation happens at the API layer before this is called.
      pub fn set_project_github_repo(&self, id: Uuid, repo: Option<&str>) -> Result<()> {
          let conn = self.pool.get()?;
          let n = conn.execute(
              "UPDATE projects SET github_repo = ?1 WHERE id = ?2",
              params![repo, id.to_string()],
          )?;
          if n == 0 {
              return Err(Error::ProjectNotFound(id));
          }
          Ok(())
      }
  ```

- [ ] **Step 7: Write failing test for round-trip**

  Inspect `tests/db_crud.rs` first to match the existing helper pattern:

  Run: `grep -n "fn " tests/db_crud.rs | head -10`

  Append:

  ```rust
  #[test]
  fn project_github_repo_round_trip() {
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let p = db.insert_project(galvolite::db::NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      assert_eq!(p.github_repo, None, "fresh project has no linked repo");

      db.set_project_github_repo(p.id, Some("octocat/hello-world")).unwrap();
      let p2 = db.get_project(p.id).unwrap();
      assert_eq!(p2.github_repo.as_deref(), Some("octocat/hello-world"));

      db.set_project_github_repo(p.id, None).unwrap();
      let p3 = db.get_project(p.id).unwrap();
      assert_eq!(p3.github_repo, None);
  }

  #[test]
  fn set_github_repo_unknown_project_errors() {
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let bogus = uuid::Uuid::new_v4();
      let err = db.set_project_github_repo(bogus, Some("x/y")).unwrap_err();
      assert!(format!("{err:?}").contains("ProjectNotFound"));
  }
  ```

- [ ] **Step 8: Run tests to verify pass**

  Run: `cargo test --test db_init --test db_crud -- --test-threads=1`
  Expected: all PASS, including the new round-trip and the schema-version-7 assertion.

- [ ] **Step 9: Run full backend suite to confirm no regressions**

  Run: `cargo test -- --test-threads=1`
  Expected: all PASS. Schema-touching tests in particular shouldn't break.

- [ ] **Step 10: Commit**

  ```bash
  git add migrations/0007_projects_github_repo.sql src/db.rs \
          tests/db_init.rs tests/db_crud.rs
  git commit -m "feat(db): add projects.github_repo column + Project.github_repo + setter"
  ```

---

## Task 2: `github::Client` wrapper

**Files:**
- Create: `src/github/mod.rs`
- Create: `src/github/client.rs`
- Modify: `src/lib.rs` (`pub mod github;`)
- Modify: `Cargo.toml` (add `octocrab` to `[dependencies]`)
- Test: `tests/github_client.rs`

- [ ] **Step 1: Add `octocrab` to Cargo.toml**

  In `Cargo.toml` `[dependencies]` (after `reqwest`):

  ```toml
  octocrab = "0.42"
  ```

  If cargo resolves a higher minor version, accept it — octocrab follows semver and the surface we use (`/user`, `/repos/{owner}/{name}`, `pulls().list()`, `issues().list()`) is stable.

- [ ] **Step 2: Create `src/github/mod.rs`**

  ```rust
  //! Plan 10a — GitHub integration layer.
  //!
  //! Read-only PAT-based access via `octocrab`. The client is constructed once
  //! at server startup from `GITHUB_TOKEN`; the connected-user cache lives on
  //! the API state and is shared with all handlers.

  pub mod client;

  pub use client::{Client, IssueSummary, PrSummary, User};
  ```

- [ ] **Step 3: Create `src/github/client.rs` with `User` + `from_env` + `validate`**

  ```rust
  //! GitHub API client wrapper. Thin layer over `octocrab` to keep the
  //! handler-side code free of `octocrab` types and to centralize error
  //! mapping into our `thiserror` enum.

  use serde::Serialize;

  /// Connected GitHub user, cached after `validate()` returns Ok.
  #[derive(Debug, Clone, Serialize)]
  pub struct User {
      pub login: String,
      pub avatar_url: String,
  }

  /// Per-PR summary returned to the dashboard. Drops fields the panel
  /// doesn't render; the user clicks `url` to see full details on github.com.
  #[derive(Debug, Clone, Serialize)]
  pub struct PrSummary {
      pub number: u64,
      pub title: String,
      pub author: String,
      pub draft: bool,
      pub updated_at: String, // RFC 3339
      pub url: String,        // html_url
  }

  /// Per-issue summary; same rationale as `PrSummary`.
  #[derive(Debug, Clone, Serialize)]
  pub struct IssueSummary {
      pub number: u64,
      pub title: String,
      pub author: String,
      pub labels: Vec<String>,
      pub updated_at: String,
      pub url: String,
  }

  #[derive(Debug, thiserror::Error)]
  pub enum GithubError {
      #[error("github api: {0}")]
      Api(String),
      #[error("network: {0}")]
      Network(String),
  }

  /// Wraps an `octocrab::Octocrab` instance configured with PAT auth + an
  /// optional alternate base URL (used by mockito tests to redirect to a
  /// local mock server).
  pub struct Client {
      inner: octocrab::Octocrab,
  }

  impl Client {
      /// Build a client from `GITHUB_TOKEN` env var. Returns `None` if the
      /// var is unset or empty. Does NOT validate the token — call
      /// `validate()` for that.
      pub fn from_env() -> Option<Self> {
          let token = std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty())?;
          Self::with_token(&token, None).ok()
      }

      /// Construct a client with a specific token and optional base URL.
      /// The base URL override is the seam tests use to redirect to a
      /// mockito server; production callers should pass `None`.
      pub fn with_token(token: &str, base_url: Option<String>) -> Result<Self, GithubError> {
          let mut builder = octocrab::Octocrab::builder().personal_token(token.to_string());
          if let Some(url) = base_url {
              builder = builder
                  .base_uri(url)
                  .map_err(|e| GithubError::Api(format!("base_uri: {e}")))?;
          }
          let inner = builder.build()
              .map_err(|e| GithubError::Api(format!("build: {e}")))?;
          Ok(Self { inner })
      }

      /// Hit `GET /user` and return the connected `User`. Returns `Err` on
      /// any HTTP error (including 401 invalid-token). Caller decides whether
      /// to treat that as "not connected" vs surface the error verbatim.
      pub async fn validate(&self) -> Result<User, GithubError> {
          let u: octocrab::models::Author = self.inner
              .current()
              .user()
              .await
              .map_err(map_err)?;
          Ok(User {
              login: u.login,
              avatar_url: u.avatar_url.to_string(),
          })
      }
  }

  fn map_err(e: octocrab::Error) -> GithubError {
      // octocrab::Error is non-exhaustive; the Display impl is informative
      // enough for our purposes (UI surfaces it verbatim).
      let s = format!("{e}");
      if s.contains("connect") || s.contains("dns") || s.contains("timeout") {
          GithubError::Network(s)
      } else {
          GithubError::Api(s)
      }
  }
  ```

- [ ] **Step 4: Register the module in `src/lib.rs`**

  Add to `src/lib.rs` (alphabetical with the other `pub mod` lines):

  ```rust
  pub mod github;
  ```

- [ ] **Step 5: Write failing tests for `validate`**

  Create `tests/github_client.rs`:

  ```rust
  //! Plan 10a Task 2 — github::Client tests against a mockito-backed
  //! impersonation of api.github.com.

  use galvolite::github::Client;

  fn mock_user_payload() -> &'static str {
      r#"{
          "login": "octocat",
          "id": 1,
          "node_id": "MDQ6VXNlcjE=",
          "avatar_url": "https://avatars.githubusercontent.com/u/1",
          "gravatar_id": "",
          "url": "https://api.github.com/users/octocat",
          "html_url": "https://github.com/octocat",
          "type": "User",
          "site_admin": false
      }"#
  }

  #[tokio::test]
  async fn validate_returns_user_on_200() {
      let mut server = mockito::Server::new_async().await;
      let m = server.mock("GET", "/user")
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
      server.mock("GET", "/user")
          .with_status(401)
          .with_header("content-type", "application/json")
          .with_body(r#"{"message":"Bad credentials"}"#)
          .create_async()
          .await;

      let client = Client::with_token("bad", Some(server.url())).unwrap();
      assert!(client.validate().await.is_err());
  }
  ```

- [ ] **Step 6: Run validate tests**

  Run: `cargo test --test github_client validate -- --test-threads=1`
  Expected: PASS.

- [ ] **Step 7: Add `list_open_prs` and `list_open_issues` to client**

  Append to `impl Client` in `src/github/client.rs`:

  ```rust
      /// List open PRs for `owner/name`. Caps at 50 per the spec; for v1
      /// that's enough (users with more open PRs use github.com directly).
      pub async fn list_open_prs(&self, owner: &str, name: &str) -> Result<Vec<PrSummary>, GithubError> {
          let page = self.inner
              .pulls(owner, name)
              .list()
              .state(octocrab::params::State::Open)
              .per_page(50)
              .send()
              .await
              .map_err(map_err)?;
          Ok(page.items.into_iter().map(|p| PrSummary {
              number: p.number,
              title: p.title.unwrap_or_default(),
              author: p.user.map(|u| u.login).unwrap_or_default(),
              draft: p.draft.unwrap_or(false),
              updated_at: p.updated_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
              url: p.html_url.map(|u| u.to_string()).unwrap_or_default(),
          }).collect())
      }

      /// List open issues for `owner/name`. GitHub's API returns PRs in the
      /// issues list; we filter them out (every PR is an issue, but only
      /// non-PR issues belong here).
      pub async fn list_open_issues(&self, owner: &str, name: &str) -> Result<Vec<IssueSummary>, GithubError> {
          let page = self.inner
              .issues(owner, name)
              .list()
              .state(octocrab::params::State::Open)
              .per_page(50)
              .send()
              .await
              .map_err(map_err)?;
          Ok(page.items.into_iter()
              .filter(|i| i.pull_request.is_none())
              .map(|i| IssueSummary {
                  number: i.number,
                  title: i.title,
                  author: i.user.login,
                  labels: i.labels.into_iter().map(|l| l.name).collect(),
                  updated_at: i.updated_at.to_rfc3339(),
                  url: i.html_url.to_string(),
              }).collect())
      }

      /// Validate a repo exists (and the PAT can see it). Returns Ok on
      /// 200, Err on any other status. Used by the link-repo endpoint
      /// before we write to the DB.
      pub async fn validate_repo(&self, owner: &str, name: &str) -> Result<(), GithubError> {
          self.inner
              .repos(owner, name)
              .get()
              .await
              .map_err(map_err)?;
          Ok(())
      }
  ```

- [ ] **Step 8: Add tests for the list methods + repo validation**

  Append to `tests/github_client.rs`:

  ```rust
  fn mock_prs_payload() -> &'static str {
      r#"[
          {
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
              "user": {
                  "login": "alice", "id": 2, "node_id": "y",
                  "avatar_url": "https://avatars.githubusercontent.com/u/2",
                  "gravatar_id": "", "url": "x", "html_url": "x",
                  "type": "User", "site_admin": false
              },
              "body": null,
              "created_at": "2026-05-01T00:00:00Z",
              "updated_at": "2026-05-02T00:00:00Z",
              "closed_at": null,
              "merged_at": null,
              "merge_commit_sha": null,
              "draft": false,
              "head": {"label":"alice:topic","ref":"topic","sha":"abc","user":null,"repo":null},
              "base": {"label":"octocat:main","ref":"main","sha":"def","user":null,"repo":null},
              "_links": {},
              "author_association": "CONTRIBUTOR"
          }
      ]"#
  }

  #[tokio::test]
  async fn list_open_prs_returns_summaries() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello/pulls")
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

  fn mock_issues_payload() -> &'static str {
      // Two items: one real issue, one that is actually a PR (has
      // pull_request field) — the second must be filtered out.
      r#"[
          {
              "url": "https://api.github.com/repos/octocat/hello/issues/2",
              "id": 10,
              "node_id": "i",
              "html_url": "https://github.com/octocat/hello/issues/2",
              "number": 2,
              "state": "open",
              "title": "Real bug",
              "body": null,
              "user": {"login":"bob","id":3,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "labels": [{"id":1,"node_id":"l","url":"x","name":"bug","color":"f00","default":true,"description":null}],
              "assignee": null,
              "assignees": [],
              "milestone": null,
              "locked": false,
              "comments": 0,
              "closed_at": null,
              "created_at": "2026-05-01T00:00:00Z",
              "updated_at": "2026-05-02T00:00:00Z",
              "author_association": "CONTRIBUTOR"
          },
          {
              "url": "https://api.github.com/repos/octocat/hello/issues/3",
              "id": 11,
              "node_id": "j",
              "html_url": "https://github.com/octocat/hello/pull/3",
              "number": 3,
              "state": "open",
              "title": "This is a PR",
              "body": null,
              "user": {"login":"carol","id":4,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
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
              "pull_request": {"url":"x","html_url":"x","diff_url":"x","patch_url":"x"}
          }
      ]"#
  }

  #[tokio::test]
  async fn list_open_issues_filters_out_prs() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello/issues")
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

  #[tokio::test]
  async fn validate_repo_ok_on_200() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"{"id":1,"node_id":"x","name":"hello","full_name":"octocat/hello","private":false,"owner":{"login":"octocat","id":1,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},"html_url":"https://github.com/octocat/hello","description":null,"fork":false,"url":"x","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","pushed_at":"2026-01-01T00:00:00Z","clone_url":"x","size":0,"stargazers_count":0,"watchers_count":0,"forks_count":0,"open_issues_count":0,"default_branch":"main"}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      assert!(client.validate_repo("octocat", "hello").await.is_ok());
  }

  #[tokio::test]
  async fn validate_repo_err_on_404() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/missing")
          .with_status(404)
          .with_header("content-type", "application/json")
          .with_body(r#"{"message":"Not Found"}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      assert!(client.validate_repo("octocat", "missing").await.is_err());
  }
  ```

  Note: the JSON payloads above are intentionally complete because octocrab does strict deserialization. If the test fails with "missing field X", add the field with a sensible default — don't try to make octocrab tolerate partial responses.

- [ ] **Step 9: Run all client tests**

  Run: `cargo test --test github_client -- --test-threads=1`
  Expected: 5 tests PASS.

- [ ] **Step 10: Commit**

  ```bash
  git add Cargo.toml Cargo.lock src/lib.rs src/github/ tests/github_client.rs
  git commit -m "feat(github): add octocrab-backed Client with validate + list_open_{prs,issues} + validate_repo"
  ```

---

## Task 3: API `GET /api/github/status`

**Files:**
- Create: `src/api/github.rs`
- Modify: `src/api/mod.rs` (route registration)
- Modify: `src/session_manager.rs` (cache the connected user on construction)
- Modify: `src/main.rs` (validate + cache on startup)
- Test: `tests/api_github.rs`

The connected user is computed once at server startup and stored on `SessionManager` (or an adjacent shared `Arc<RwLock<Option<User>>>`). The status handler reads from the cache; never touches GitHub on the request path.

- [ ] **Step 1: Add a `connected_github_user` field to `SessionManager`**

  In `src/session_manager.rs`, find the `SessionManager` struct definition (around line 35-50). Add a new field:

  ```rust
  pub struct SessionManager {
      // ... existing fields ...
      connected_github_user: Arc<std::sync::RwLock<Option<crate::github::User>>>,
  }
  ```

  Initialize it in the `new` / `with_event_bus` constructor:

  ```rust
              connected_github_user: Arc::new(std::sync::RwLock::new(None)),
  ```

  Add public accessors:

  ```rust
      /// Plan 10a Task 3 — read the cached connected GitHub user. Returns
      /// `None` if no token was set or validation failed at startup.
      pub fn github_user(&self) -> Option<crate::github::User> {
          self.connected_github_user.read().unwrap().clone()
      }

      /// Plan 10a Task 3 — set the cached connected user. Called once at
      /// startup after `Client::validate()` succeeds.
      pub fn set_github_user(&self, user: Option<crate::github::User>) {
          *self.connected_github_user.write().unwrap() = user;
      }
  ```

- [ ] **Step 2: Validate token + cache user at startup**

  In `src/main.rs::cmd_serve` (around line 218 onwards), after `let mgr = ...` and before binding the listener, attempt validation:

  ```rust
      // Plan 10a Task 3 — validate GITHUB_TOKEN at startup; cache the
      // connected user on the manager. Best-effort: if no token or
      // validation fails, the cache stays None and the dashboard shows
      // "GitHub: not connected".
      if let Some(client) = galvolite::github::Client::from_env() {
          match client.validate().await {
              Ok(user) => {
                  tracing::info!(login = %user.login, "GitHub PAT validated");
                  mgr.set_github_user(Some(user));
              }
              Err(e) => {
                  tracing::warn!(error = ?e, "GitHub PAT validation failed; running disconnected");
              }
          }
      } else {
          tracing::info!("GITHUB_TOKEN not set; GitHub features disabled");
      }
  ```

- [ ] **Step 3: Create `src/api/github.rs` with the status handler**

  ```rust
  //! Plan 10a — GitHub HTTP endpoints.

  use axum::{
      extract::{Path, State},
      http::StatusCode,
      response::{IntoResponse, Response},
      Json,
  };
  use serde::{Deserialize, Serialize};
  use std::sync::Arc;
  use uuid::Uuid;

  use crate::session_manager::SessionManager;

  #[derive(Debug, Serialize)]
  pub struct StatusResponse {
      pub connected: bool,
      pub user: Option<crate::github::User>,
      pub reason: Option<String>,
  }

  /// `GET /api/github/status` — returns whether a token was present and
  /// validated at startup. Never touches GitHub on the request path.
  pub async fn status(State(mgr): State<Arc<SessionManager>>) -> Response {
      let token_present = std::env::var("GITHUB_TOKEN")
          .ok()
          .map(|s| !s.is_empty())
          .unwrap_or(false);
      let user = mgr.github_user();
      let body = match (token_present, user) {
          (false, _) => StatusResponse {
              connected: false,
              user: None,
              reason: Some("no token".into()),
          },
          (true, Some(u)) => StatusResponse {
              connected: true,
              user: Some(u),
              reason: None,
          },
          (true, None) => StatusResponse {
              connected: false,
              user: None,
              reason: Some("token invalid".into()),
          },
      };
      Json(body).into_response()
  }
  ```

- [ ] **Step 4: Register module + route in `src/api/mod.rs`**

  Add `pub mod github;` to the existing `pub mod` list at the top.

  Add the route inside `router()`, alongside the other `/api/projects/...` routes (anywhere that compiles):

  ```rust
          .route("/api/github/status", get(github::status))
  ```

- [ ] **Step 5: Write failing tests for the status endpoint**

  Create `tests/api_github.rs`:

  ```rust
  //! Plan 10a Task 3+ — API tests for the GitHub endpoints.
  //!
  //! Uses the existing test helpers in tests/common/mod.rs to spin up a
  //! router with a fresh DB. Token validation against api.github.com is
  //! avoided by NOT setting GITHUB_TOKEN in the test process.

  mod common;

  use axum::body::Body;
  use axum::http::{Request, StatusCode};
  use serde_json::Value;
  use tower::ServiceExt;

  // Use a process-wide mutex if needed because tests share env vars.
  // For status tests we only need to ensure the var is unset.
  use std::sync::Mutex;
  static ENV_LOCK: Mutex<()> = Mutex::new(());

  #[tokio::test]
  async fn status_returns_not_connected_when_token_unset() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::remove_var("GITHUB_TOKEN");

      let (_tmp, app) = common::test_app().await;
      let resp = app
          .oneshot(Request::builder().uri("/api/github/status").body(Body::empty()).unwrap())
          .await
          .unwrap();
      assert_eq!(resp.status(), StatusCode::OK);

      let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
      let v: Value = serde_json::from_slice(&bytes).unwrap();
      assert_eq!(v["connected"], false);
      assert_eq!(v["reason"], "no token");
      assert!(v["user"].is_null());
  }

  #[tokio::test]
  async fn status_returns_token_invalid_when_var_set_but_user_uncached() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GITHUB_TOKEN", "fake-for-test");

      let (_tmp, app) = common::test_app().await;
      let resp = app
          .oneshot(Request::builder().uri("/api/github/status").body(Body::empty()).unwrap())
          .await
          .unwrap();
      let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
      let v: Value = serde_json::from_slice(&bytes).unwrap();
      assert_eq!(v["connected"], false);
      assert_eq!(v["reason"], "token invalid");

      std::env::remove_var("GITHUB_TOKEN");
  }
  ```

  If `tests/common/mod.rs` doesn't already have a `test_app()` helper that returns `(TempDir, Router)`, look at how the existing api tests bootstrap a router — there's a pattern (likely `tests/common/mod.rs:218` per session memory). Use the same pattern. If you need to add a helper, name it `test_app()` and return the tempdir guard so it stays alive.

- [ ] **Step 6: Run tests**

  Run: `cargo test --test api_github -- --test-threads=1`
  Expected: PASS.

- [ ] **Step 7: Commit**

  ```bash
  git add src/api/github.rs src/api/mod.rs src/session_manager.rs \
          src/main.rs tests/api_github.rs
  git commit -m "feat(api): GET /api/github/status backed by startup-cached user"
  ```

---

## Task 4: API `PUT /api/projects/:id/github/repo`

**Files:**
- Modify: `src/api/github.rs` (add `link_repo` handler + parse helper)
- Modify: `src/api/mod.rs` (register route)
- Test: `tests/api_github.rs` (extend)

The handler validates `owner/name` format, attempts `GET /repos/{owner}/{name}` via the GitHub client, then writes to the DB.

For tests, we cannot easily inject a mock GitHub client through axum's State extractor without a major refactor. Strategy: extract the validation step into a standalone function that takes a `&Client` parameter, then test that function directly via `tests/github_client.rs` (already covered there as `validate_repo_*`). For the API endpoint, write a smaller test that covers the format-parse failure path (which doesn't hit GitHub) and a happy-path stub that inserts the row directly via DB. We accept that the "round-trip via GitHub" path is exercised at the `tests/github_client.rs` level, not at the api integration level.

- [ ] **Step 1: Add `link_repo` request/response types and handler stub**

  Append to `src/api/github.rs`:

  ```rust
  #[derive(Debug, Deserialize)]
  pub struct LinkRepoReq {
      pub repo: String, // "owner/name"
  }

  #[derive(Debug, Serialize)]
  pub struct LinkRepoResp {
      pub repo: String,
  }

  /// Parse "owner/name" into its parts. Returns None if format is invalid.
  fn parse_repo(s: &str) -> Option<(&str, &str)> {
      let s = s.trim();
      let mut parts = s.splitn(2, '/');
      let owner = parts.next()?;
      let name = parts.next()?;
      if owner.is_empty() || name.is_empty() || owner.contains(' ') || name.contains(' ') || name.contains('/') {
          return None;
      }
      Some((owner, name))
  }

  /// `PUT /api/projects/:id/github/repo` — body `{"repo": "owner/name"}`.
  ///
  /// Format-validates locally, then hits `GET /repos/{owner}/{name}` via
  /// our cached `Client` to confirm the repo exists and the PAT can see
  /// it, then writes `projects.github_repo`. Returns 200 on success, 422
  /// on bad format or unknown repo, 503 if no GitHub client is available
  /// (no token at startup).
  pub async fn link_repo(
      State(mgr): State<Arc<SessionManager>>,
      Path(project_id): Path<Uuid>,
      Json(req): Json<LinkRepoReq>,
  ) -> Response {
      let Some((owner, name)) = parse_repo(&req.repo) else {
          return (StatusCode::UNPROCESSABLE_ENTITY,
              Json(serde_json::json!({"error": "expected format 'owner/name'"})))
              .into_response();
      };

      // Construct a fresh Client for this request from env. Cheap; the
      // octocrab builder doesn't perform any I/O. Returning early when the
      // env var is missing surfaces a clearer error than a 502 from the
      // validate call.
      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})))
              .into_response();
      };

      if let Err(e) = client.validate_repo(owner, name).await {
          return (StatusCode::UNPROCESSABLE_ENTITY,
              Json(serde_json::json!({"error": format!("{e}")})))
              .into_response();
      }

      match mgr.db().set_project_github_repo(project_id, Some(&req.repo)) {
          Ok(()) => Json(LinkRepoResp { repo: req.repo }).into_response(),
          Err(e) => crate::api::err_to_response(e),
      }
  }
  ```

- [ ] **Step 2: Register route in `src/api/mod.rs`**

  ```rust
          .route("/api/projects/:id/github/repo", put(github::link_repo))
  ```

- [ ] **Step 3: Write failing tests**

  Append to `tests/api_github.rs`:

  ```rust
  use axum::http::Method;
  use galvolite::db::NewProject;

  #[tokio::test]
  async fn link_repo_rejects_bad_format() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GITHUB_TOKEN", "doesnt-matter");

      let (tmp, app) = common::test_app().await;
      // Add a project to link against.
      let mgr = common::mgr_from_tmp(&tmp);
      let p = mgr.add_project(NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();

      let req = Request::builder()
          .uri(format!("/api/projects/{}/github/repo", p.id))
          .method(Method::PUT)
          .header("content-type", "application/json")
          .body(Body::from(r#"{"repo":"not a valid format"}"#))
          .unwrap();
      let resp = app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn link_repo_503_when_token_unset() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::remove_var("GITHUB_TOKEN");

      let (tmp, app) = common::test_app().await;
      let mgr = common::mgr_from_tmp(&tmp);
      let p = mgr.add_project(NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();

      let req = Request::builder()
          .uri(format!("/api/projects/{}/github/repo", p.id))
          .method(Method::PUT)
          .header("content-type", "application/json")
          .body(Body::from(r#"{"repo":"octocat/hello"}"#))
          .unwrap();
      let resp = app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }

  // Note: the "happy path link succeeds" case requires hitting api.github.com
  // (or a mock), which we can't easily do through the axum integration test
  // because the handler constructs a Client from env. Coverage for the
  // GitHub-side validation is in tests/github_client.rs::validate_repo_*.
  // We separately verify the DB write path via tests/db_crud.rs::project_github_repo_round_trip.
  ```

  This test relies on `common::mgr_from_tmp(tmp)` existing — if it doesn't, you may need to add a tiny helper in `tests/common/mod.rs` that returns a `SessionManager` reusing the same DB the `test_app` opened. If that's awkward, restructure `test_app()` to return `(TempDir, Arc<SessionManager>, Router)` and update `status_*` tests to discard the manager.

- [ ] **Step 4: Run tests**

  Run: `cargo test --test api_github -- --test-threads=1`
  Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add src/api/github.rs src/api/mod.rs tests/api_github.rs
  git commit -m "feat(api): PUT /api/projects/:id/github/repo with validation"
  ```

---

## Task 5: API `GET /api/projects/:id/github/{prs,issues}`

**Files:**
- Modify: `src/api/github.rs` (add `list_prs` and `list_issues` handlers)
- Modify: `src/api/mod.rs` (register routes)
- Test: `tests/api_github.rs` (extend)

- [ ] **Step 1: Add the two handlers**

  Append to `src/api/github.rs`:

  ```rust
  /// `GET /api/projects/:id/github/prs` — list open PRs for the project's
  /// linked repo. Returns 404 if no repo is linked or project unknown,
  /// 503 if no token, 502 on GitHub error.
  pub async fn list_prs(
      State(mgr): State<Arc<SessionManager>>,
      Path(project_id): Path<Uuid>,
  ) -> Response {
      let (owner, name) = match resolve_linked_repo(&mgr, project_id) {
          Ok(parts) => parts,
          Err(resp) => return resp,
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };

      match client.list_open_prs(&owner, &name).await {
          Ok(prs) => Json(prs).into_response(),
          Err(e) => (StatusCode::BAD_GATEWAY,
              Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
      }
  }

  /// `GET /api/projects/:id/github/issues` — same shape as list_prs but
  /// for issues (PRs filtered out by the client).
  pub async fn list_issues(
      State(mgr): State<Arc<SessionManager>>,
      Path(project_id): Path<Uuid>,
  ) -> Response {
      let (owner, name) = match resolve_linked_repo(&mgr, project_id) {
          Ok(parts) => parts,
          Err(resp) => return resp,
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };

      match client.list_open_issues(&owner, &name).await {
          Ok(issues) => Json(issues).into_response(),
          Err(e) => (StatusCode::BAD_GATEWAY,
              Json(serde_json::json!({"error": format!("{e}")}))).into_response(),
      }
  }

  /// Helper: fetch the project, parse its linked repo, return owner+name.
  /// Folds all error cases (project not found, no linked repo, malformed
  /// stored repo) into pre-built `Response` values so the call sites stay
  /// flat.
  fn resolve_linked_repo(
      mgr: &Arc<SessionManager>,
      project_id: Uuid,
  ) -> Result<(String, String), Response> {
      let project = match mgr.db().get_project(project_id) {
          Ok(p) => p,
          Err(crate::error::Error::ProjectNotFound(_)) => {
              return Err((StatusCode::NOT_FOUND,
                  Json(serde_json::json!({"error": "project not found"}))).into_response());
          }
          Err(e) => return Err(crate::api::err_to_response(e)),
      };
      let stored = project.github_repo.ok_or_else(|| {
          (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no linked repo"}))).into_response()
      })?;
      match parse_repo(&stored) {
          Some((o, n)) => Ok((o.to_string(), n.to_string())),
          None => Err((StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "stored repo has bad format"}))).into_response()),
      }
  }
  ```

- [ ] **Step 2: Register routes in `src/api/mod.rs`**

  ```rust
          .route("/api/projects/:id/github/prs", get(github::list_prs))
          .route("/api/projects/:id/github/issues", get(github::list_issues))
  ```

- [ ] **Step 3: Write failing tests**

  Append to `tests/api_github.rs`:

  ```rust
  #[tokio::test]
  async fn list_prs_404_when_no_linked_repo() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GITHUB_TOKEN", "x");

      let (tmp, app) = common::test_app().await;
      let mgr = common::mgr_from_tmp(&tmp);
      let p = mgr.add_project(NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();

      let resp = app.oneshot(
          Request::builder()
              .uri(format!("/api/projects/{}/github/prs", p.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn list_issues_404_when_project_missing() {
      let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GITHUB_TOKEN", "x");

      let (_tmp, app) = common::test_app().await;
      let bogus = uuid::Uuid::new_v4();
      let resp = app.oneshot(
          Request::builder()
              .uri(format!("/api/projects/{}/github/issues", bogus))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);

      std::env::remove_var("GITHUB_TOKEN");
  }
  ```

  Note: as with task 4, the GitHub-network happy path is covered at the `tests/github_client.rs` level. These API integration tests cover the routing + project-state branches.

- [ ] **Step 4: Run tests**

  Run: `cargo test --test api_github -- --test-threads=1`
  Expected: 6 tests PASS.

- [ ] **Step 5: Build to verify everything compiles together**

  Run: `cargo build`
  Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

  ```bash
  git add src/api/github.rs src/api/mod.rs tests/api_github.rs
  git commit -m "feat(api): GET /api/projects/:id/github/{prs,issues}"
  ```

---

## Task 6: UI types + `api.ts` client functions

**Files:**
- Modify: `ui/src/types.ts` (add GitHub types + extend Project)
- Modify: `ui/src/api.ts` (add 4 new API functions)

- [ ] **Step 1: Extend `Project` and add new types**

  Modify `ui/src/types.ts`. Update the `Project` interface (around line 5):

  ```ts
  export interface Project {
    id: string;
    name: string;
    path: string;
    remote_url: string | null;
    created_at: string;
    github_repo?: string | null; // Plan 10a — null/absent when not linked
  }
  ```

  Append at the end of the file:

  ```ts
  // Plan 10a — GitHub read-only types
  export interface GithubUser {
    login: string;
    avatar_url: string;
  }

  export interface GithubStatus {
    connected: boolean;
    user: GithubUser | null;
    reason: string | null;
  }

  export interface GithubPr {
    number: number;
    title: string;
    author: string;
    draft: boolean;
    updated_at: string;
    url: string;
  }

  export interface GithubIssue {
    number: number;
    title: string;
    author: string;
    labels: string[];
    updated_at: string;
    url: string;
  }
  ```

- [ ] **Step 2: Add API client functions**

  Modify `ui/src/api.ts`. First add the new types to the import block at the top:

  ```ts
  import type {
    Project,
    Session,
    SignalRow,
    EventRow,
    PlanStep,
    CreateProjectReq,
    CreateSessionReq,
    SendReq,
    DirectoryListing,
    Directive,
    Postmortem,
    Decision,
    Recommendation,
    GithubStatus,
    GithubPr,
    GithubIssue,
  } from "./types";
  ```

  Then append these functions to the `api = {...}` object, just before the closing `}`:

  ```ts
    // Plan 10a — GitHub read-only endpoints

    githubStatus: () => request<GithubStatus>("/api/github/status"),

    linkGithubRepo: (projectId: string, repo: string) =>
      request<{ repo: string }>(
        `/api/projects/${projectId}/github/repo`,
        { method: "PUT", body: JSON.stringify({ repo }) },
      ),

    listGithubPrs: (projectId: string) =>
      request<GithubPr[]>(`/api/projects/${projectId}/github/prs`),

    listGithubIssues: (projectId: string) =>
      request<GithubIssue[]>(`/api/projects/${projectId}/github/issues`),
  ```

- [ ] **Step 3: Verify TS compiles**

  Run: `cd ui && npx tsc --noEmit`
  Expected: no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add ui/src/types.ts ui/src/api.ts
  git commit -m "feat(ui): types + api client for GitHub read-only endpoints"
  ```

---

## Task 7: UI `GithubStatusBadge` component + Dashboard mount

**Files:**
- Create: `ui/src/components/GithubStatusBadge.tsx`
- Create: `ui/src/components/GithubStatusBadge.test.tsx`
- Modify: `ui/src/components/Dashboard.tsx` (mount the badge in the header alongside RecBell)

- [ ] **Step 1: Write failing test**

  Create `ui/src/components/GithubStatusBadge.test.tsx`:

  ```tsx
  import { render, screen, waitFor } from "@testing-library/react";
  import { describe, it, expect, vi } from "vitest";
  import { GithubStatusBadge } from "./GithubStatusBadge";
  import { api } from "../api";

  describe("GithubStatusBadge", () => {
    it("renders connected user when status is connected", async () => {
      vi.spyOn(api, "githubStatus").mockResolvedValue({
        connected: true,
        user: { login: "octocat", avatar_url: "https://example/avatar.png" },
        reason: null,
      });
      render(<GithubStatusBadge />);
      await waitFor(() => {
        expect(screen.getByText(/@octocat/)).toBeInTheDocument();
      });
    });

    it("renders 'not connected' with reason when no token", async () => {
      vi.spyOn(api, "githubStatus").mockResolvedValue({
        connected: false,
        user: null,
        reason: "no token",
      });
      render(<GithubStatusBadge />);
      await waitFor(() => {
        expect(screen.getByText(/not connected/i)).toBeInTheDocument();
      });
    });

    it("renders silently while loading then renders status", async () => {
      let resolve: (v: any) => void;
      vi.spyOn(api, "githubStatus").mockReturnValue(
        new Promise((r) => { resolve = r; }) as any
      );
      render(<GithubStatusBadge />);
      // While loading, the badge can render an empty placeholder or nothing.
      // Only the post-resolve render is asserted.
      resolve!({ connected: false, user: null, reason: "no token" });
      await waitFor(() => {
        expect(screen.getByText(/not connected/i)).toBeInTheDocument();
      });
    });
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- GithubStatusBadge --run`
  Expected: FAIL — component doesn't exist.

- [ ] **Step 3: Create the component**

  Create `ui/src/components/GithubStatusBadge.tsx`:

  ```tsx
  import { useEffect, useState } from "react";
  import { api } from "../api";
  import type { GithubStatus } from "../types";

  /**
   * Plan 10a — Dashboard-header indicator. Polls /api/github/status once
   * on mount; no live refresh. Operators rotate tokens by restarting
   * the server.
   */
  export function GithubStatusBadge() {
    const [status, setStatus] = useState<GithubStatus | null>(null);

    useEffect(() => {
      let cancelled = false;
      api.githubStatus()
        .then((s) => { if (!cancelled) setStatus(s); })
        .catch(() => {
          if (!cancelled) {
            setStatus({ connected: false, user: null, reason: "fetch failed" });
          }
        });
      return () => { cancelled = true; };
    }, []);

    if (!status) {
      // Pre-resolve placeholder. Width holds the slot so nothing reflows
      // when the badge fills in.
      return <span className="text-xs text-neutral-600 px-2" aria-hidden="true">…</span>;
    }

    if (status.connected && status.user) {
      return (
        <span
          className="text-xs px-2 py-0.5 rounded bg-neutral-800 text-neutral-300"
          title="GitHub connected"
        >
          @{status.user.login}
        </span>
      );
    }

    return (
      <span
        className="text-xs px-2 py-0.5 rounded bg-neutral-800 text-yellow-500"
        title={`GitHub: ${status.reason ?? "not connected"}`}
      >
        GitHub: not connected
      </span>
    );
  }
  ```

- [ ] **Step 4: Run tests to verify pass**

  Run: `cd ui && npm test -- GithubStatusBadge --run`
  Expected: 3 tests PASS.

- [ ] **Step 5: Mount in Dashboard header**

  Modify `ui/src/components/Dashboard.tsx`. Add to the imports near `import { RecBell } from "./RecBell"`:

  ```tsx
  import { GithubStatusBadge } from "./GithubStatusBadge";
  ```

  Find the existing JSX block where `<RecBell ... />` is rendered (around line 145 per the recon) and place the badge next to it. The exact JSX site depends on the current Dashboard layout; mount it inside whatever header/toolbar element holds RecBell. Example:

  ```tsx
  <div className="flex items-center gap-2">
    <GithubStatusBadge />
    <RecBell ... />
  </div>
  ```

  If RecBell is a direct child of a flex container, just add `<GithubStatusBadge />` as a sibling immediately before it.

- [ ] **Step 6: Run full UI suite to confirm no regressions**

  Run: `cd ui && npm test -- --run`
  Expected: all PASS, including any existing Dashboard tests.

- [ ] **Step 7: Commit**

  ```bash
  git add ui/src/components/GithubStatusBadge.tsx \
          ui/src/components/GithubStatusBadge.test.tsx \
          ui/src/components/Dashboard.tsx
  git commit -m "feat(ui): GithubStatusBadge in dashboard header"
  ```

---

## Task 8: UI `LinkRepoModal`

**Files:**
- Create: `ui/src/components/LinkRepoModal.tsx`
- Create: `ui/src/components/LinkRepoModal.test.tsx`

This task ships the modal as a standalone component. Wiring it into a project card is part of Task 9 (so the panel and the link affordance land together in Dashboard).

- [ ] **Step 1: Write failing tests**

  Create `ui/src/components/LinkRepoModal.test.tsx`:

  ```tsx
  import { render, screen, fireEvent, waitFor } from "@testing-library/react";
  import { describe, it, expect, vi } from "vitest";
  import { LinkRepoModal } from "./LinkRepoModal";
  import { api } from "../api";

  describe("LinkRepoModal", () => {
    it("save success closes modal and calls onSaved", async () => {
      const onSaved = vi.fn();
      const onClose = vi.fn();
      vi.spyOn(api, "linkGithubRepo").mockResolvedValue({ repo: "octocat/hello" });

      render(<LinkRepoModal projectId="p" open={true} onClose={onClose} onSaved={onSaved} />);
      fireEvent.change(screen.getByPlaceholderText(/owner\/name/i), {
        target: { value: "octocat/hello" }
      });
      fireEvent.click(screen.getByRole("button", { name: /save/i }));

      await waitFor(() => {
        expect(api.linkGithubRepo).toHaveBeenCalledWith("p", "octocat/hello");
        expect(onSaved).toHaveBeenCalledWith("octocat/hello");
        expect(onClose).toHaveBeenCalled();
      });
    });

    it("displays inline error on 422 from API", async () => {
      vi.spyOn(api, "linkGithubRepo").mockRejectedValue(
        new Error('422: {"error":"not found or no access"}')
      );

      render(<LinkRepoModal projectId="p" open={true} onClose={() => {}} onSaved={() => {}} />);
      fireEvent.change(screen.getByPlaceholderText(/owner\/name/i), {
        target: { value: "octocat/missing" }
      });
      fireEvent.click(screen.getByRole("button", { name: /save/i }));

      await waitFor(() => {
        expect(screen.getByText(/not found or no access/i)).toBeInTheDocument();
      });
    });

    it("save button disabled when input is empty", () => {
      render(<LinkRepoModal projectId="p" open={true} onClose={() => {}} onSaved={() => {}} />);
      expect(screen.getByRole("button", { name: /save/i })).toBeDisabled();
    });
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- LinkRepoModal --run`
  Expected: FAIL — component doesn't exist.

- [ ] **Step 3: Create the component**

  Create `ui/src/components/LinkRepoModal.tsx`:

  ```tsx
  import { useState } from "react";
  import { api } from "../api";

  interface Props {
    projectId: string;
    open: boolean;
    onClose: () => void;
    onSaved: (repo: string) => void;
  }

  /**
   * Plan 10a — per-project settings modal for linking a GitHub repo.
   * The backend validates the repo via GitHub before writing; on 422
   * we surface the error message inline.
   */
  export function LinkRepoModal({ projectId, open, onClose, onSaved }: Props) {
    const [repo, setRepo] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    if (!open) return null;

    const submit = async () => {
      setBusy(true);
      setError(null);
      try {
        const res = await api.linkGithubRepo(projectId, repo.trim());
        onSaved(res.repo);
        setRepo("");
        onClose();
      } catch (e) {
        // Try to extract the error body from the request helper's
        // "<status>: <text>" message format.
        const msg = e instanceof Error ? e.message : String(e);
        const match = msg.match(/\d{3}:\s*(\{.*\})/);
        if (match) {
          try {
            const body = JSON.parse(match[1]);
            setError(typeof body.error === "string" ? body.error : msg);
          } catch {
            setError(msg);
          }
        } else {
          setError(msg);
        }
      } finally {
        setBusy(false);
      }
    };

    return (
      <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
        <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[420px]">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">Link GitHub repo</h2>
            <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
          </div>
          <p className="text-sm text-neutral-400 mb-2">
            Enter the repo as <code className="text-neutral-200">owner/name</code>.
          </p>
          <input
            type="text"
            placeholder="owner/name"
            value={repo}
            onChange={(e) => setRepo(e.target.value)}
            className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2 mb-2"
            disabled={busy}
          />
          {error && (
            <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">{error}</div>
          )}
          <div className="flex gap-2 justify-end">
            <button
              className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-white text-xs"
              onClick={onClose}
              disabled={busy}
            >Cancel</button>
            <button
              className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs disabled:opacity-50"
              onClick={submit}
              disabled={busy || repo.trim().length === 0}
            >Save</button>
          </div>
        </div>
      </div>
    );
  }
  ```

- [ ] **Step 4: Run tests**

  Run: `cd ui && npm test -- LinkRepoModal --run`
  Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add ui/src/components/LinkRepoModal.tsx ui/src/components/LinkRepoModal.test.tsx
  git commit -m "feat(ui): LinkRepoModal for per-project GitHub repo linkage"
  ```

---

## Task 9: UI `GithubPanel` + Dashboard wire-in

**Files:**
- Create: `ui/src/components/GithubPanel.tsx`
- Create: `ui/src/components/GithubPanel.test.tsx`
- Modify: `ui/src/components/Dashboard.tsx` (mount panel + LinkRepoModal trigger per project)

- [ ] **Step 1: Write failing tests for the panel**

  Create `ui/src/components/GithubPanel.test.tsx`:

  ```tsx
  import { render, screen, fireEvent, waitFor } from "@testing-library/react";
  import { describe, it, expect, vi } from "vitest";
  import { GithubPanel } from "./GithubPanel";
  import { api } from "../api";

  describe("GithubPanel", () => {
    it("renders empty state when no PRs and no issues", async () => {
      vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
      vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

      render(<GithubPanel projectId="p" repo="octocat/hello" />);
      await waitFor(() => {
        expect(screen.getByText(/no open PRs/i)).toBeInTheDocument();
        expect(screen.getByText(/no open issues/i)).toBeInTheDocument();
      });
    });

    it("renders PRs and issues lists", async () => {
      vi.spyOn(api, "listGithubPrs").mockResolvedValue([
        { number: 1, title: "First PR", author: "alice", draft: false,
          updated_at: "2026-05-01T00:00:00Z", url: "https://github.com/x/y/pull/1" }
      ]);
      vi.spyOn(api, "listGithubIssues").mockResolvedValue([
        { number: 2, title: "Bug report", author: "bob", labels: ["bug"],
          updated_at: "2026-05-02T00:00:00Z", url: "https://github.com/x/y/issues/2" }
      ]);

      render(<GithubPanel projectId="p" repo="octocat/hello" />);
      await waitFor(() => {
        expect(screen.getByText("First PR")).toBeInTheDocument();
        expect(screen.getByText("Bug report")).toBeInTheDocument();
      });
    });

    it("Refresh button refetches both lists", async () => {
      const prsMock = vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
      const issuesMock = vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

      render(<GithubPanel projectId="p" repo="octocat/hello" />);
      await waitFor(() => expect(prsMock).toHaveBeenCalledTimes(1));
      await waitFor(() => expect(issuesMock).toHaveBeenCalledTimes(1));

      fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
      await waitFor(() => expect(prsMock).toHaveBeenCalledTimes(2));
      await waitFor(() => expect(issuesMock).toHaveBeenCalledTimes(2));
    });

    it("renders error banner on API error", async () => {
      vi.spyOn(api, "listGithubPrs").mockRejectedValue(new Error("502: rate limited"));
      vi.spyOn(api, "listGithubIssues").mockResolvedValue([]);

      render(<GithubPanel projectId="p" repo="octocat/hello" />);
      await waitFor(() => {
        expect(screen.getByText(/rate limited/i)).toBeInTheDocument();
      });
    });
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- GithubPanel --run`
  Expected: FAIL — component doesn't exist.

- [ ] **Step 3: Create the panel**

  Create `ui/src/components/GithubPanel.tsx`:

  ```tsx
  import { useEffect, useState, useCallback } from "react";
  import { api } from "../api";
  import type { GithubPr, GithubIssue } from "../types";

  interface Props {
    projectId: string;
    repo: string; // "owner/name", just for display
  }

  /**
   * Plan 10a — per-project panel listing open PRs and open issues for the
   * linked repo. Manual refresh; no polling. Click row → opens github.com
   * in a new tab.
   */
  export function GithubPanel({ projectId, repo }: Props) {
    const [prs, setPrs] = useState<GithubPr[] | null>(null);
    const [issues, setIssues] = useState<GithubIssue[] | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    const fetchAll = useCallback(async () => {
      setBusy(true);
      setError(null);
      try {
        const [p, i] = await Promise.all([
          api.listGithubPrs(projectId),
          api.listGithubIssues(projectId),
        ]);
        setPrs(p);
        setIssues(i);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    }, [projectId]);

    useEffect(() => { fetchAll(); }, [fetchAll]);

    return (
      <div className="border border-neutral-700 rounded p-3 mt-2">
        <div className="flex items-center justify-between mb-2">
          <div className="text-sm font-medium">
            GitHub: <span className="text-neutral-400">{repo}</span>
          </div>
          <button
            className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600 disabled:opacity-50"
            onClick={fetchAll}
            disabled={busy}
          >Refresh</button>
        </div>

        {error && (
          <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">
            Error: {error}
          </div>
        )}

        <div className="grid grid-cols-2 gap-3">
          <div>
            <div className="text-xs text-neutral-500 mb-1">Open PRs</div>
            {prs === null ? (
              <div className="text-xs text-neutral-600">Loading…</div>
            ) : prs.length === 0 ? (
              <div className="text-xs text-neutral-600">No open PRs.</div>
            ) : (
              <ul className="space-y-1">
                {prs.map((p) => (
                  <li key={p.number} className="text-xs">
                    <a
                      href={p.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-400 hover:underline"
                    >#{p.number}</a>
                    {" "}
                    {p.draft && <span className="text-neutral-500">[draft]</span>}
                    {" "}
                    <span className="text-neutral-200">{p.title}</span>
                    <span className="text-neutral-500"> — @{p.author}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div>
            <div className="text-xs text-neutral-500 mb-1">Open issues</div>
            {issues === null ? (
              <div className="text-xs text-neutral-600">Loading…</div>
            ) : issues.length === 0 ? (
              <div className="text-xs text-neutral-600">No open issues.</div>
            ) : (
              <ul className="space-y-1">
                {issues.map((i) => (
                  <li key={i.number} className="text-xs">
                    <a
                      href={i.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-400 hover:underline"
                    >#{i.number}</a>
                    {" "}
                    <span className="text-neutral-200">{i.title}</span>
                    <span className="text-neutral-500"> — @{i.author}</span>
                    {i.labels.length > 0 && (
                      <span className="ml-1 text-neutral-500">
                        [{i.labels.join(", ")}]
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>
    );
  }
  ```

- [ ] **Step 4: Run tests**

  Run: `cd ui && npm test -- GithubPanel --run`
  Expected: 4 tests PASS.

- [ ] **Step 5: Wire panel + LinkRepoModal into Dashboard**

  Modify `ui/src/components/Dashboard.tsx`. Add imports near the top:

  ```tsx
  import { GithubPanel } from "./GithubPanel";
  import { LinkRepoModal } from "./LinkRepoModal";
  ```

  Find the per-project rendering site in Dashboard (where each project is shown — likely an iteration over `projects` somewhere mid-file). For each project, render either:
  - The `GithubPanel` if `project.github_repo` is non-null, OR
  - A small "Link GitHub repo" button that opens the `LinkRepoModal`

  Concretely add this state + handler at the top of the Dashboard function body:

  ```tsx
    const [linkModalProjectId, setLinkModalProjectId] = useState<string | null>(null);
  ```

  And in the per-project rendering area, add:

  ```tsx
    {project.github_repo ? (
      <GithubPanel projectId={project.id} repo={project.github_repo} />
    ) : (
      <button
        className="text-xs text-neutral-500 hover:text-neutral-300 underline mt-2"
        onClick={() => setLinkModalProjectId(project.id)}
      >Link GitHub repo</button>
    )}
  ```

  And finally near the modal-render area (where `RecModal` is rendered):

  ```tsx
    <LinkRepoModal
      projectId={linkModalProjectId ?? ""}
      open={linkModalProjectId !== null}
      onClose={() => setLinkModalProjectId(null)}
      onSaved={() => {
        setLinkModalProjectId(null);
        // Trigger a project list refresh so the new github_repo shows up.
        // useProjects exposes refresh via its hook return; check the
        // current API and call it. If there's no public refresh, reload
        // the page or add one to the hook.
        window.location.reload();
      }}
    />
  ```

  Note on the `window.location.reload()`: Dashboard already uses `useProjects` which fetches projects from the API. The natural fix is to expose a `refresh()` from `useProjects` and call it here instead of reloading. If the hook already has a `refresh()` (or similar), call that. If not, the reload is acceptable for this initial slice — we can refine in a follow-up.

- [ ] **Step 6: Run full UI suite to confirm no regressions**

  Run: `cd ui && npm test -- --run`
  Expected: all PASS, including any existing Dashboard tests.

- [ ] **Step 7: Commit**

  ```bash
  git add ui/src/components/GithubPanel.tsx \
          ui/src/components/GithubPanel.test.tsx \
          ui/src/components/Dashboard.tsx
  git commit -m "feat(ui): GithubPanel + Dashboard wire-in for linked repos"
  ```

---

## Final verification

After all 9 tasks land:

- [ ] **Run full backend test suite**

  Run: `cargo test -- --test-threads=1`
  Expected: PASS, including all new GitHub tests + the bumped `db_init` schema-version assertion.

- [ ] **Run full UI test suite**

  Run: `cd ui && npm test -- --run`
  Expected: PASS.

- [ ] **Build release artifact**

  Run: `cargo build --release`
  Expected: PASS, no warnings. Octocrab pulls in extra deps; first release build will be slow but should succeed.

- [ ] **Update memory file `project_plan10a_status.md`**

  Per project convention (see `project_plan9_status.md`), record completion: branch, commit range, test counts, decomposition placement (10a foundation; 10b/10c/10d/10e remain).
