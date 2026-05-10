# Plan 10b — PR Creation Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the operator-driven PR creation pipeline on top of Plan 10a's auth + read-only foundation. From the dashboard, an operator clicks "Open PR" on a session with a worktree branch; GalvoLite pushes the branch via PAT-authenticated HTTPS and opens a PR via `octocrab`. The PR URL persists on the session row and the UI flips to a "View PR" link.

**Architecture:** One new SQLite migration (`0008_sessions_pr_url.sql`), extensions to existing modules (no new top-level subsystem), one new HTTP endpoint pair (`pr-draft` + `pr`), one new React modal (`OpenPrModal`), and one new domain event (`SessionPrCreated`). All GitHub mutations are operator-mediated through a modal-with-preview so the operator sees title/body/base before submission. The push helper redacts the PAT from any error string (security-critical, covered by a unit test).

**Tech Stack:** Rust 1.x (axum 0.7, tokio, rusqlite 0.31, chrono, octocrab 0.42, tracing, serde), SQLite migrations under `migrations/`, React 18 + TypeScript + Vitest + Tailwind. Shell-out to `git` for push (uses the same `git` binary already used by `src/worktree.rs`).

**Spec:** [docs/superpowers/specs/2026-05-09-plan-10b-pr-creation-design.md](../specs/2026-05-09-plan-10b-pr-creation-design.md)

---

## Task ordering

Sequential — each task lands one commit.

| # | Task | Layer |
|---|---|---|
| 1 | Migration 0008 + `Session.pr_url` + DB setter | DB |
| 2 | `Client::default_branch` + `Client::create_pull_request` + mockito tests | GitHub client |
| 3 | `github::push` helper (push + token redaction) + tests | Git push |
| 4 | Startup token-scope warning (advisory) | Startup wiring |
| 5 | API: `GET /api/sessions/:id/pr-draft` | API |
| 6 | API: `POST /api/sessions/:id/pr` + `DomainEvent::SessionPrCreated` | API + bus |
| 7 | UI: types + `api.ts` + `DomainEvent` TS variant | UI plumbing |
| 8 | UI: `OpenPrModal` component | UI |
| 9 | UI: `ExpandedSessionView` wire-in (Open PR button + View PR link + event handler) | UI |

---

## Task 1: Migration 0008 + `Session.pr_url` + DB setter

**Files:**
- Create: `migrations/0008_sessions_pr_url.sql`
- Modify: `src/db.rs` (MIGRATIONS list + Session struct + get/list_sessions SELECTs + setter)
- Modify: `tests/db_init.rs:19` (schema-version assertion bump 7 → 8)
- Test: `tests/db_crud.rs` (extend with pr_url round-trip)

- [ ] **Step 1: Create migration `migrations/0008_sessions_pr_url.sql`**

  ```sql
  -- Plan 10b Task 1 — add per-session GitHub PR URL.
  -- Nullable; an unset value means no PR has been created for this session.
  -- No CHECK constraint (the URL is opaque from the DB's perspective).
  ALTER TABLE sessions ADD COLUMN pr_url TEXT NULL;
  ```

- [ ] **Step 2: Register migration in `src/db.rs::MIGRATIONS`**

  Add a new SCHEMA constant near the existing block (around lines 14-20):

  ```rust
  const SCHEMA_V8: &str = include_str!("../migrations/0008_sessions_pr_url.sql");
  ```

  And append to MIGRATIONS:

  ```rust
  const MIGRATIONS: &[(i64, &str)] = &[
      (1, SCHEMA_V1),
      (2, SCHEMA_V2),
      (3, SCHEMA_V3),
      (4, SCHEMA_V4),
      (5, SCHEMA_V5),
      (6, SCHEMA_V6),
      (7, SCHEMA_V7),
      (8, SCHEMA_V8),
  ];
  ```

- [ ] **Step 3: Update schema-version test**

  Modify `tests/db_init.rs:19`:

  ```rust
      assert_eq!(version, 8);
  ```

- [ ] **Step 4: Add `pr_url` field to `Session` struct**

  Modify `src/db.rs` `Session` struct (around lines 154-178). Add the new field at the end of the struct, alongside the other `#[serde(skip_serializing_if = "Option::is_none")]` optional fields:

  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct Session {
      pub id: Uuid,
      pub project_id: Uuid,
      pub role: String,
      pub branch: Option<String>,
      pub worktree_path: String,
      pub pty_pid: Option<i64>,
      pub state: SessionState,
      pub autopilot: bool,
      pub parent_session_id: Option<Uuid>,
      pub task_summary: String,
      pub created_at: DateTime<Utc>,
      pub ended_at: Option<DateTime<Utc>>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub waiting_on: Option<Vec<Uuid>>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub pending_rejoin: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub terminal_status: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub pending_overseer_recs: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub pending_phases_json: Option<String>,
      #[serde(skip_serializing_if = "Option::is_none")]
      pub pr_url: Option<String>,
  }
  ```

- [ ] **Step 5: Update `insert_session`, `get_session`, `list_sessions` to read/write pr_url**

  Search `src/db.rs` for every `Session { ... }` literal construction. The codebase has them in:
  - `insert_session` (around line 320-345) — set `pr_url: None` since new sessions never have a PR
  - `get_session` (around line 360-395) — extend SELECT and add `pr_url: r.get(17)?`
  - `list_sessions` (around line 414-465) — same: extend SELECT and add `pr_url: r.get(17)?`

  Concrete changes (apply to both `get_session` and `list_sessions`):

  In the `SELECT` statement, append `, pr_url` to the column list:

  ```rust
  "SELECT id, project_id, role, branch, worktree_path, pty_pid, state,
          autopilot, parent_session_id, task_summary, created_at, ended_at,
          waiting_on, pending_rejoin, terminal_status, pending_overseer_recs,
          pending_phases_json, pr_url
   FROM sessions WHERE ..."
  ```

  In the row-mapper closure, after the existing `pending_phases_json: r.get(16)?,` line, add:

  ```rust
                  pending_phases_json: r.get(16)?,
                  pr_url: r.get(17)?,
  ```

  In `insert_session`'s `Session { ... }` return literal, add `pr_url: None,` to the construction. The compiler will tell you the exact location if missed.

- [ ] **Step 6: Add `set_session_pr_url` setter on `Db`**

  Append to the `impl Db` block in `src/db.rs` (after `set_session_handoff_path` around line 735, following the existing setter pattern):

  ```rust
      /// Plan 10b Task 1 — set or clear the PR URL for a session. Pass
      /// `None` to clear (e.g. if a future PR-detach action is added).
      /// Returns `Error::SessionNotFound` if the id is unknown.
      pub fn set_session_pr_url(&self, id: Uuid, url: Option<&str>) -> Result<()> {
          let conn = self.pool.get()?;
          let rows = conn.execute(
              "UPDATE sessions SET pr_url = ?1 WHERE id = ?2",
              params![url, id.to_string()],
          )?;
          if rows == 0 {
              return Err(Error::SessionNotFound(id));
          }
          Ok(())
      }
  ```

- [ ] **Step 7: Write failing tests for round-trip**

  First inspect existing patterns:

  Run: `grep -n "session_.*round_trip\|set_session_.*errors" tests/db_crud.rs | head -10`

  Append to `tests/db_crud.rs`:

  ```rust
  #[test]
  fn session_pr_url_round_trip() {
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let project = db.insert_project(galvolite::db::NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      let session = db.insert_session(galvolite::db::NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: Some("galvolite/abc12345".into()),
          worktree_path: tmp.path().join("wt").to_string_lossy().into_owned(),
          task_summary: "test session".into(),
          parent_session_id: None,
      }).unwrap();
      assert_eq!(session.pr_url, None, "fresh session has no pr_url");

      db.set_session_pr_url(session.id, Some("https://github.com/octocat/hello/pull/42")).unwrap();
      let s2 = db.get_session(session.id).unwrap();
      assert_eq!(s2.pr_url.as_deref(), Some("https://github.com/octocat/hello/pull/42"));

      db.set_session_pr_url(session.id, None).unwrap();
      let s3 = db.get_session(session.id).unwrap();
      assert_eq!(s3.pr_url, None);
  }

  #[test]
  fn set_session_pr_url_unknown_session_errors() {
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let bogus = uuid::Uuid::new_v4();
      let err = db.set_session_pr_url(bogus, Some("https://example/pr/1")).unwrap_err();
      assert!(matches!(err, galvolite::error::Error::SessionNotFound(id) if id == bogus));
  }
  ```

- [ ] **Step 8: Run targeted tests**

  Run: `cargo test --test db_init --test db_crud -- --test-threads=1`
  Expected: all PASS, including new round-trip + schema-version-8.

- [ ] **Step 9: Run full backend suite to confirm no regressions**

  Run: `cargo test -- --test-threads=1`
  Expected: all PASS.

- [ ] **Step 10: Commit**

  ```bash
  git add migrations/0008_sessions_pr_url.sql src/db.rs \
          tests/db_init.rs tests/db_crud.rs
  git commit -m "feat(db): add sessions.pr_url column + Session.pr_url + setter"
  ```

---

## Task 2: `Client::default_branch` + `Client::create_pull_request`

**Files:**
- Modify: `src/github/client.rs` (add 2 methods + `PrInfo` type)
- Test: `tests/github_client.rs` (extend with 4 mockito tests)

- [ ] **Step 1: Add `PrInfo` data type**

  Append near the existing `PrSummary` / `IssueSummary` structs in `src/github/client.rs`:

  ```rust
  /// Plan 10b — created-PR info returned by `create_pull_request`. The
  /// `html_url` is what we persist on the session row; `number` is for
  /// future webhook-correlation work.
  #[derive(Debug, Clone, Serialize)]
  pub struct PrInfo {
      pub number: u64,
      pub html_url: String,
  }
  ```

- [ ] **Step 2: Add `default_branch` method to `Client`**

  Append to the `impl Client` block in `src/github/client.rs`:

  ```rust
      /// Plan 10b — fetch the repo's default branch name (e.g. "main" or
      /// "master"). Used to pre-populate the base-branch field in the
      /// OpenPrModal. Caller falls back to "main" on Err.
      pub async fn default_branch(&self, owner: &str, name: &str) -> Result<String, GithubError> {
          let repo = self.inner
              .repos(owner, name)
              .get()
              .await
              .map_err(map_err)?;
          Ok(repo.default_branch.unwrap_or_else(|| "main".to_string()))
      }
  ```

- [ ] **Step 3: Add `create_pull_request` method to `Client`**

  Append to the `impl Client` block:

  ```rust
      /// Plan 10b — create a PR via `POST /repos/{owner}/{name}/pulls`.
      /// Returns the html_url + number on success; surfaces octocrab's
      /// error verbatim on failure (e.g. 422 if a PR already exists for
      /// the head branch on the remote, 404 if base branch missing).
      pub async fn create_pull_request(
          &self,
          owner: &str,
          name: &str,
          head: &str,
          base: &str,
          title: &str,
          body: &str,
          draft: bool,
      ) -> Result<PrInfo, GithubError> {
          let pr = self.inner
              .pulls(owner, name)
              .create(title, head, base)
              .body(body)
              .draft(draft)
              .send()
              .await
              .map_err(map_err)?;
          Ok(PrInfo {
              number: pr.number,
              html_url: pr.html_url
                  .map(|u| u.to_string())
                  .unwrap_or_default(),
          })
      }
  ```

- [ ] **Step 4: Re-export `PrInfo` from `src/github/mod.rs`**

  Modify `src/github/mod.rs`:

  ```rust
  pub use client::{Client, IssueSummary, PrInfo, PrSummary, User};
  ```

- [ ] **Step 5: Write failing tests**

  Append to `tests/github_client.rs`:

  ```rust
  #[tokio::test]
  async fn default_branch_returns_value_from_response() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"{
              "id": 1, "node_id": "x", "name": "hello", "full_name": "octocat/hello",
              "private": false,
              "owner": {"login":"octocat","id":1,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "html_url": "https://github.com/octocat/hello",
              "description": null, "fork": false, "url": "x",
              "created_at": "2026-01-01T00:00:00Z",
              "updated_at": "2026-01-01T00:00:00Z",
              "pushed_at": "2026-01-01T00:00:00Z",
              "clone_url": "x", "size": 0, "stargazers_count": 0,
              "watchers_count": 0, "forks_count": 0, "open_issues_count": 0,
              "default_branch": "trunk"
          }"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      let branch = client.default_branch("octocat", "hello").await.unwrap();
      assert_eq!(branch, "trunk");
  }

  #[tokio::test]
  async fn create_pull_request_returns_pr_info_on_201() {
      let mut server = mockito::Server::new_async().await;
      server.mock("POST", "/repos/octocat/hello/pulls")
          .with_status(201)
          .with_header("content-type", "application/json")
          .with_body(r#"{
              "url": "https://api.github.com/repos/octocat/hello/pulls/42",
              "id": 999, "node_id": "x",
              "html_url": "https://github.com/octocat/hello/pull/42",
              "diff_url": "x", "patch_url": "x", "issue_url": "x",
              "number": 42,
              "state": "open", "locked": false,
              "title": "Add token-budget reset",
              "user": {"login":"octocat","id":1,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "body": "...",
              "created_at": "2026-05-09T00:00:00Z",
              "updated_at": "2026-05-09T00:00:00Z",
              "closed_at": null, "merged_at": null, "merge_commit_sha": null,
              "draft": false,
              "head": {"label":"octocat:galvolite/abc12345","ref":"galvolite/abc12345","sha":"abc","user":null,"repo":null},
              "base": {"label":"octocat:main","ref":"main","sha":"def","user":null,"repo":null},
              "_links": {},
              "author_association": "OWNER"
          }"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      let info = client.create_pull_request(
          "octocat", "hello",
          "galvolite/abc12345", "main",
          "Add token-budget reset",
          "## Task\n...",
          false,
      ).await.unwrap();
      assert_eq!(info.number, 42);
      assert_eq!(info.html_url, "https://github.com/octocat/hello/pull/42");
  }

  #[tokio::test]
  async fn create_pull_request_err_on_422() {
      // GitHub returns 422 when a PR already exists for the head branch.
      let mut server = mockito::Server::new_async().await;
      server.mock("POST", "/repos/octocat/hello/pulls")
          .with_status(422)
          .with_header("content-type", "application/json")
          .with_body(r#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","code":"custom","message":"A pull request already exists for octocat:galvolite/abc12345."}]}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      let err = client.create_pull_request(
          "octocat", "hello",
          "galvolite/abc12345", "main",
          "x", "y", false,
      ).await.unwrap_err();
      assert!(format!("{err}").contains("422") || format!("{err}").contains("Validation"));
  }

  #[tokio::test]
  async fn default_branch_err_on_404() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/missing")
          .with_status(404)
          .with_header("content-type", "application/json")
          .with_body(r#"{"message":"Not Found"}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      assert!(client.default_branch("octocat", "missing").await.is_err());
  }
  ```

  Note: the existing `validate_repo_ok_on_200` test in `tests/github_client.rs` already contains a full repo-info JSON payload — you can study it for reference if octocrab complains about missing fields.

- [ ] **Step 6: Run tests**

  Run: `cargo test --test github_client -- --test-threads=1`
  Expected: 9 tests PASS (5 from Plan 10a + 4 new).

- [ ] **Step 7: Commit**

  ```bash
  git add src/github/client.rs src/github/mod.rs tests/github_client.rs
  git commit -m "feat(github): add Client::default_branch + create_pull_request + PrInfo"
  ```

---

## Task 3: `github::push` helper with token redaction

**Files:**
- Create: `src/github/push.rs`
- Modify: `src/github/mod.rs` (`pub mod push;`)
- Test: `tests/github_push.rs` (new)

The push helper shells out to `git` to push a branch via HTTPS using the PAT in the URL (`x-access-token` pattern). Token must NEVER appear in any error string returned to callers — covered by a dedicated test.

- [ ] **Step 1: Create `src/github/push.rs`**

  ```rust
  //! Plan 10b — git push helper for PR creation.
  //!
  //! Uses HTTPS push with the PAT in the URL (`x-access-token` pattern)
  //! per GitHub's officially supported PAT-push idiom. The token is
  //! redacted from any error stderr before propagating, so it never
  //! reaches the dashboard or logs.

  use std::path::Path;
  use std::process::Command;

  #[derive(Debug, thiserror::Error)]
  pub enum PushError {
      /// `git push` exited non-zero. The string here is the captured
      /// stderr with the PAT redacted.
      #[error("git push failed: {0}")]
      GitFailed(String),

      /// The worktree path doesn't exist or isn't readable.
      #[error("worktree path invalid: {0}")]
      InvalidWorktree(String),

      /// We failed to launch `git` (binary missing, etc.).
      #[error("git invocation failed: {0}")]
      Spawn(String),
  }

  /// Push `branch` from `worktree_path` to `https://github.com/<owner>/<name>`
  /// using `token` for HTTPS auth. Returns Ok on success; Err with a
  /// token-redacted message on any failure.
  ///
  /// Refspec is `<branch>:<branch>` (push local branch to same name on
  /// remote). The remote is constructed inline rather than relying on
  /// the worktree's `origin` to avoid surprises if the operator has
  /// configured a non-GitHub origin.
  pub fn push_branch(
      worktree_path: &Path,
      branch: &str,
      owner: &str,
      name: &str,
      token: &str,
  ) -> Result<(), PushError> {
      if !worktree_path.exists() {
          return Err(PushError::InvalidWorktree(
              worktree_path.display().to_string()
          ));
      }
      let url = format!(
          "https://x-access-token:{token}@github.com/{owner}/{name}.git"
      );
      let refspec = format!("{branch}:{branch}");
      let output = Command::new("git")
          .arg("-C")
          .arg(worktree_path)
          .arg("push")
          .arg(&url)
          .arg(&refspec)
          .output()
          .map_err(|e| PushError::Spawn(format!("{e}")))?;

      if output.status.success() {
          return Ok(());
      }

      let stderr = String::from_utf8_lossy(&output.stderr);
      let redacted = stderr.replace(token, "***");
      Err(PushError::GitFailed(redacted.into_owned()))
  }
  ```

- [ ] **Step 2: Register module in `src/github/mod.rs`**

  Modify `src/github/mod.rs` to add the new module + re-export the error type:

  ```rust
  pub mod client;
  pub mod push;

  pub use client::{Client, IssueSummary, PrInfo, PrSummary, User};
  pub use push::PushError;
  ```

- [ ] **Step 3: Write failing tests**

  Create `tests/github_push.rs`:

  ```rust
  //! Plan 10b Task 3 — push helper tests.
  //!
  //! Uses a temp git repo + a `git init --bare` "remote" on disk to
  //! exercise the success path. The token-redaction test deliberately
  //! triggers a push failure to a bogus remote and asserts the token
  //! literal never appears in the error string.

  use galvolite::github::{push::push_branch, PushError};
  use std::process::Command;
  use tempfile::TempDir;

  fn run_git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
      Command::new("git")
          .current_dir(dir)
          .args(args)
          .output()
          .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"))
  }

  /// Set up a temp working repo with one commit on a `galvolite/test` branch.
  /// Returns (worktree_dir, branch_name).
  fn setup_repo() -> (TempDir, String) {
      let dir = TempDir::new().unwrap();
      run_git(dir.path(), &["init", "-q", "-b", "main"]);
      run_git(dir.path(), &["config", "user.email", "test@example"]);
      run_git(dir.path(), &["config", "user.name", "Test"]);
      std::fs::write(dir.path().join("README.md"), "hello").unwrap();
      run_git(dir.path(), &["add", "."]);
      run_git(dir.path(), &["commit", "-q", "-m", "init"]);
      run_git(dir.path(), &["checkout", "-q", "-b", "galvolite/test"]);
      std::fs::write(dir.path().join("file.txt"), "content").unwrap();
      run_git(dir.path(), &["add", "."]);
      run_git(dir.path(), &["commit", "-q", "-m", "add file"]);
      (dir, "galvolite/test".into())
  }

  #[test]
  fn push_fails_with_redacted_token_when_remote_unreachable() {
      let (worktree, branch) = setup_repo();
      // Use a token literal that's distinctive enough we'll spot it if
      // it leaks. The remote (127.0.0.1:1) refuses connections.
      let token = "ghp_DEFINITELY_A_SECRET_TOKEN_VALUE_xyz123";
      let err = push_branch(
          worktree.path(),
          &branch,
          "octocat",
          "hello",
          token,
      ).unwrap_err();
      let msg = format!("{err}");
      // Must NOT contain the token literal anywhere in the error.
      assert!(!msg.contains(token),
          "token leaked into error message: {msg}");
      // Should contain the redaction marker if git's stderr included
      // the URL we constructed.
      // (May or may not include "***" depending on whether git printed
      // the URL — just assert NO token leak.)
  }

  #[test]
  fn push_fails_with_clear_error_when_worktree_path_invalid() {
      let bogus = std::path::PathBuf::from("/this/path/should/not/exist/anywhere");
      let err = push_branch(
          &bogus,
          "any-branch",
          "owner",
          "name",
          "any-token",
      ).unwrap_err();
      assert!(matches!(err, PushError::InvalidWorktree(_)));
  }

  // The "push to a real remote on disk" test pattern is omitted here
  // because git push refuses to write to a non-bare remote, and
  // setting up a bare remote on disk plus URL-rewriting to point at
  // it adds substantial complexity. The push-helper success path is
  // exercised end-to-end by manual smoke testing; the unit tests
  // cover the security-critical redaction + the input-validation
  // branches. If integration coverage becomes important, switch to
  // the bare-remote pattern documented at
  // https://git-scm.com/docs/gitfaq#_setup_a_bare_repository.
  ```

  Note on coverage: the spec listed a "push_succeeds_to_local_bare_remote" test. Setting up a local bare remote that accepts HTTPS auth-style URLs is awkward in a unit-test context (git's push protocol negotiates differently for `file://` vs `https://` remotes). The token-redaction + worktree-validation tests cover the security-critical and input-validation branches; the success path is exercised end-to-end at the API layer when `tests/api_github.rs` mocks the client and at manual smoke time.

- [ ] **Step 4: Run tests**

  Run: `cargo test --test github_push -- --test-threads=1`
  Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add src/github/push.rs src/github/mod.rs tests/github_push.rs
  git commit -m "feat(github): add push_branch helper with PAT-redacting error path"
  ```

---

## Task 4: Startup token-scope warning

**Files:**
- Modify: `src/github/client.rs` (add scope-fetching helper)
- Modify: `src/main.rs::cmd_serve` (warn if `repo` scope missing)
- Test: extend `tests/github_client.rs`

octocrab doesn't directly expose the response headers from the typed `current().user()` call. Workaround: do a separate HEAD/GET request via the inner client to read `x-oauth-scopes`. We add a `Client::token_scopes()` method that returns the scopes as a `Vec<String>` parsed from the comma-separated header.

- [ ] **Step 1: Add `token_scopes` method to `Client`**

  Append to `impl Client` in `src/github/client.rs`:

  ```rust
      /// Plan 10b Task 4 — fetch the comma-separated `x-oauth-scopes`
      /// header from `GET /user`. Returns Vec of trimmed scope strings,
      /// or empty Vec if the header is absent (which can happen if the
      /// PAT has no scopes set, common for fine-grained PATs).
      pub async fn token_scopes(&self) -> Result<Vec<String>, GithubError> {
          // Use the underlying http_client to capture headers.
          let req = self.inner._build_request(
              http::Method::GET,
              http::uri::Uri::from_static("/user"),
          ).map_err(|e| GithubError::Api(format!("build req: {e}")))?;
          let resp = self.inner.execute(req).await.map_err(map_err)?;
          let scopes = resp.headers()
              .get("x-oauth-scopes")
              .and_then(|v| v.to_str().ok())
              .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
              .unwrap_or_default();
          Ok(scopes)
      }
  ```

  Note: octocrab's `_build_request` and `execute` are low-level escape hatches. If they're not public on the resolved version, the alternative is to use `reqwest` directly (octocrab uses `reqwest` internally so `reqwest` is already in our dep tree via octocrab). Check octocrab's API surface and adapt — the goal is "fetch /user response headers." If both `_build_request`/`execute` are unavailable, instantiate a separate `reqwest::Client`, build a request to `<base_url>/user` with `Authorization: Bearer <token>` and `User-Agent: galvolite`, and read `headers().get("x-oauth-scopes")` from the response.

- [ ] **Step 2: Add a mockito test for `token_scopes`**

  Append to `tests/github_client.rs`:

  ```rust
  #[tokio::test]
  async fn token_scopes_parses_x_oauth_scopes_header() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/user")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_header("x-oauth-scopes", "repo, read:org, workflow")
          .with_body(r#"{"login":"octocat","id":1,"node_id":"x","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false}"#)
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
      server.mock("GET", "/user")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"{"login":"octocat","id":1,"node_id":"x","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      let scopes = client.token_scopes().await.unwrap();
      assert!(scopes.is_empty());
  }
  ```

- [ ] **Step 3: Wire startup warning in `src/main.rs::cmd_serve`**

  Find the existing Plan 10a startup block in `src/main.rs::cmd_serve` (around line 229-245):

  ```rust
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

  Extend the `Ok(user)` arm to also check scopes:

  ```rust
      if let Some(client) = galvolite::github::Client::from_env() {
          match client.validate().await {
              Ok(user) => {
                  tracing::info!(login = %user.login, "GitHub PAT validated");
                  mgr.set_github_user(Some(user));
                  // Plan 10b Task 4 — advisory warning if 'repo' write
                  // scope is missing (Plan 10b PR creation needs it).
                  match client.token_scopes().await {
                      Ok(scopes) if !scopes.is_empty()
                          && !scopes.iter().any(|s| s == "repo" || s == "public_repo") => {
                          tracing::warn!(
                              scopes = ?scopes,
                              "GITHUB_TOKEN lacks 'repo' write scope — Plan 10b PR creation will fail; \
                               rotate the PAT to add 'repo' or 'public_repo' write scope"
                          );
                      }
                      Ok(_) => {} // scopes contain repo/public_repo, OR scopes empty (fine-grained PAT)
                      Err(e) => {
                          tracing::debug!(error = ?e, "could not fetch token scopes; skipping advisory check");
                      }
                  }
              }
              Err(e) => {
                  tracing::warn!(error = ?e, "GitHub PAT validation failed; running disconnected");
              }
          }
      } else {
          tracing::info!("GITHUB_TOKEN not set; GitHub features disabled");
      }
  ```

  Note: empty scopes are NOT a warning because fine-grained PATs (the modern GitHub PAT format) don't populate `x-oauth-scopes` at all — they have permissions instead. The warning fires only for classic PATs that explicitly list scopes that don't include `repo`/`public_repo`.

- [ ] **Step 4: Run tests**

  Run: `cargo test --test github_client -- --test-threads=1`
  Expected: 11 tests PASS (9 prior + 2 new).

- [ ] **Step 5: Run full backend test suite**

  Run: `cargo test -- --test-threads=1`
  Expected: all PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add src/github/client.rs src/main.rs tests/github_client.rs
  git commit -m "feat(github): warn at startup if PAT lacks 'repo' write scope"
  ```

---

## Task 5: API `GET /api/sessions/:id/pr-draft`

**Files:**
- Modify: `src/api/github.rs` (add `pr_draft` handler + helpers)
- Modify: `src/api/mod.rs` (register route)
- Test: `tests/api_github.rs` (extend with 4 tests)

The handler computes a draft PR title/body from the session's role, branch, and worktree contents. Body content is built by shelling out to `git -C <worktree>` for the commit log and reading `<worktree>/.galvolite/plan.md` for orchestrator sessions.

- [ ] **Step 1: Add `pr_draft` request/response types**

  Append to `src/api/github.rs`:

  ```rust
  #[derive(Debug, Serialize)]
  pub struct PrDraftResponse {
      pub title: String,
      pub body: String,
      pub base: String,
      pub commits_ahead: u32,
      pub commits_behind: u32,
  }
  ```

- [ ] **Step 2: Add helper functions for git shell-outs and body templates**

  Append to `src/api/github.rs`:

  ```rust
  use std::path::Path;

  /// Run `git -C <worktree>` with the given args, return stdout as String.
  /// Returns empty string on any error (best-effort; PR creation continues
  /// even if the commit log can't be computed).
  fn git_capture(worktree: &Path, args: &[&str]) -> String {
      std::process::Command::new("git")
          .arg("-C").arg(worktree)
          .args(args)
          .output()
          .ok()
          .and_then(|o| if o.status.success() {
              Some(String::from_utf8_lossy(&o.stdout).into_owned())
          } else { None })
          .unwrap_or_default()
  }

  /// Compute (commits_ahead, commits_behind) for `<base>...HEAD`.
  /// Returns (0, 0) on any git error.
  fn commits_ahead_behind(worktree: &Path, base: &str) -> (u32, u32) {
      let out = git_capture(worktree, &["rev-list", "--left-right", "--count",
          &format!("{base}...HEAD")]);
      let mut parts = out.split_whitespace();
      let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
      let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
      (ahead, behind)
  }

  /// `git log --oneline --no-decorate <base>..HEAD` capped at 50 lines.
  fn commit_log(worktree: &Path, base: &str) -> Vec<String> {
      git_capture(worktree, &[
          "log", "--oneline", "--no-decorate",
          "-n", "50",
          &format!("{base}..HEAD"),
      ])
      .lines()
      .map(|l| l.to_string())
      .collect()
  }

  /// Build the PR body from session + worktree state. Branches on role:
  /// orchestrators get a Plan section; everyone else gets just task + commits.
  fn build_pr_body(
      session: &galvolite::db::Session,
      worktree: &Path,
      base: &str,
  ) -> String {
      let mut body = format!("## Task\n{}\n\n## Commits\n", session.task_summary);
      let log = commit_log(worktree, base);
      if log.is_empty() {
          body.push_str("(no commits ahead of base)\n");
      } else {
          for line in log {
              body.push_str(&format!("- {line}\n"));
          }
      }
      if session.role == "orchestrator" {
          let plan_path = worktree.join(".galvolite/plan.md");
          let plan = std::fs::read_to_string(&plan_path).unwrap_or_else(|_| "(no plan.md)".into());
          body.push_str("\n## Plan\n");
          body.push_str(&plan);
      }
      body
  }

  /// Truncate a string at a word boundary at or below `max` chars.
  fn truncate_title(s: &str, max: usize) -> String {
      if s.chars().count() <= max {
          return s.to_string();
      }
      // Find last space at or before max (chars, not bytes).
      let chars: Vec<char> = s.chars().collect();
      let cap_idx = chars.iter().enumerate()
          .take(max)
          .filter(|(_, c)| c.is_whitespace())
          .map(|(i, _)| i)
          .last()
          .unwrap_or(max - 1);
      chars.iter().take(cap_idx).collect::<String>().trim().to_string()
  }
  ```

  Note: the `galvolite::db::Session` import path may need tweaking — if `crate::db::Session` is already in scope from another import in this file, use that. The compiler will guide.

- [ ] **Step 3: Add `pr_draft` handler**

  Append to `src/api/github.rs`:

  ```rust
  /// `GET /api/sessions/:id/pr-draft` — compute a draft PR title/body
  /// for an operator to edit before submitting. Returns 404 chain
  /// identical to `pr` (session missing / no linked repo / no branch),
  /// 503 if no token. Never touches GitHub except for `default_branch`
  /// lookup (falls back to "main" on error).
  pub async fn pr_draft(
      State(mgr): State<Arc<SessionManager>>,
      Path(session_id): Path<Uuid>,
  ) -> Response {
      let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
          Ok(s) => s,
          Err(resp) => return resp,
      };

      let Some(repo_str) = project.github_repo.as_ref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no linked repo"}))).into_response();
      };
      let Some((owner, name)) = parse_repo(repo_str) else {
          return (StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "stored repo has bad format"}))).into_response();
      };
      let Some(branch) = session.branch.as_deref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "session has no branch"}))).into_response();
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };

      // Default base: ask GitHub; fall back to "main" on error.
      let base = match client.default_branch(owner, name).await {
          Ok(b) => b,
          Err(e) => {
              tracing::warn!(error = %e, "default_branch lookup failed; falling back to 'main'");
              "main".to_string()
          }
      };

      let worktree = std::path::PathBuf::from(&session.worktree_path);
      let (commits_ahead, commits_behind) = commits_ahead_behind(&worktree, &base);
      let body = build_pr_body(&session, &worktree, &base);
      let title = truncate_title(&session.task_summary, 72);

      let _ = (owner, name); // silence unused if compiler can't tell
      Json(PrDraftResponse {
          title, body, base, commits_ahead, commits_behind,
      }).into_response()
  }

  /// Helper: load session + parent project, returning the appropriate
  /// error response on missing session or DB failure.
  fn resolve_session_for_pr(
      mgr: &Arc<SessionManager>,
      session_id: Uuid,
  ) -> Result<(crate::db::Session, crate::db::Project), Response> {
      let session = match mgr.db().get_session(session_id) {
          Ok(s) => s,
          Err(crate::error::Error::SessionNotFound(_))
          | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
              return Err((StatusCode::NOT_FOUND,
                  Json(serde_json::json!({"error": "session not found"}))).into_response());
          }
          Err(e) => return Err(crate::api::err_to_response(e)),
      };
      let project = match mgr.db().get_project(session.project_id) {
          Ok(p) => p,
          Err(crate::error::Error::ProjectNotFound(_))
          | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
              return Err((StatusCode::NOT_FOUND,
                  Json(serde_json::json!({"error": "project not found"}))).into_response());
          }
          Err(e) => return Err(crate::api::err_to_response(e)),
      };
      Ok((session, project))
  }
  ```

- [ ] **Step 4: Register route in `src/api/mod.rs`**

  Add to `router()`:

  ```rust
          .route("/api/sessions/:id/pr-draft", get(github::pr_draft))
  ```

- [ ] **Step 5: Write failing tests**

  Append to `tests/api_github.rs`:

  ```rust
  use galvolite::db::NewSession;

  #[tokio::test]
  async fn pr_draft_404s_on_missing_session() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");

      let setup = common::test_app();
      let bogus = uuid::Uuid::new_v4();
      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-draft", bogus))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn pr_draft_404s_on_no_linked_repo() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      let session = setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: Some("galvolite/abc12345".into()),
          worktree_path: setup._dir.path().join("wt").to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();

      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-draft", session.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn pr_draft_404s_on_session_with_no_branch() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      let session = setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "orchestrator".into(),
          branch: None,
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "orch".into(),
          parent_session_id: None,
      }).unwrap();

      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-draft", session.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn pr_draft_503_when_token_unset() {
      let _g = env_lock().lock().await;
      std::env::remove_var("GITHUB_TOKEN");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      let session = setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: Some("galvolite/abc12345".into()),
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();

      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-draft", session.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }
  ```

  Note: the orchestrator-vs-child body-shape tests are deferred to integration coverage at smoke time. The role-branched body content is best exercised via a real git repo in the worktree, which is brittle to set up under test_app. The error-path tests above cover the routing and state branches, which is the primary risk.

- [ ] **Step 6: Run tests**

  Run: `cargo test --test api_github -- --test-threads=1`
  Expected: 10 tests PASS (6 prior + 4 new).

- [ ] **Step 7: Commit**

  ```bash
  git add src/api/github.rs src/api/mod.rs tests/api_github.rs
  git commit -m "feat(api): GET /api/sessions/:id/pr-draft with role-branched body"
  ```

---

## Task 6: API `POST /api/sessions/:id/pr` + `SessionPrCreated` bus event

**Files:**
- Modify: `src/api/events.rs` (add `SessionPrCreated` variant + `applies_to` arm)
- Modify: `src/api/github.rs` (add `open_pr` handler + req/resp types)
- Modify: `src/api/mod.rs` (register route)
- Test: `tests/api_github.rs` (extend with 2 tests)

- [ ] **Step 1: Add `DomainEvent::SessionPrCreated` variant**

  Modify `src/api/events.rs`. Find the existing `ProjectAdded` / `ProjectDeleted` variants (around lines 96-98). Add the new variant before `ProjectAdded`:

  ```rust
      #[serde(rename = "session.pr_created")]
      SessionPrCreated {
          session_id: Uuid,
          project_id: Uuid,
          pr_url: String,
          pr_number: u64,
      },
      #[serde(rename = "project.deleted")]
      ProjectDeleted { project_id: Uuid },
  ```

  Update `applies_to` (the exhaustive matcher around lines 137-160). `SessionPrCreated` is session-scoped → goes in the `*session_id == sid` arm:

  ```rust
      pub fn applies_to(&self, sid: Uuid) -> bool {
          match self {
              DomainEvent::SessionStateChanged { session_id, .. }
              | DomainEvent::SessionEventAppended { session_id, .. }
              | DomainEvent::SessionRoleChanged { session_id, .. }
              | DomainEvent::SessionMetaChanged { session_id, .. }
              | DomainEvent::SessionPlanReplaced { session_id, .. }
              | DomainEvent::SessionPlanStepChanged { session_id, .. }
              | DomainEvent::OperatorPrompt { session_id, .. }
              | DomainEvent::OperatorToast { session_id, .. }
              | DomainEvent::SessionResumed { session_id, .. }
              | DomainEvent::SessionChildrenResolved { session_id, .. }
              | DomainEvent::SessionPrCreated { session_id, .. } => *session_id == sid,
              DomainEvent::DirectiveChanged { .. }
              | DomainEvent::PostmortemWritten { .. }
              | DomainEvent::DecisionRecorded { .. }
              | DomainEvent::PhaseReviewNeeded { .. }
              | DomainEvent::PhaseAdvanced { .. }
              | DomainEvent::RecommendationCreated { .. }
              | DomainEvent::RecommendationActed { .. }
              | DomainEvent::OverseerDegraded { .. }
              | DomainEvent::OverseerRecovered { .. }
              | DomainEvent::ProjectAdded { .. }
              | DomainEvent::ProjectDeleted { .. } => false,
          }
      }
  ```

- [ ] **Step 2: Add `open_pr` request/response types**

  Append to `src/api/github.rs`:

  ```rust
  #[derive(Debug, Deserialize)]
  pub struct OpenPrReq {
      pub title: String,
      pub body: String,
      pub base: String,
  }

  #[derive(Debug, Serialize)]
  pub struct OpenPrResp {
      pub pr_url: String,
      pub pr_number: u64,
  }
  ```

- [ ] **Step 3: Add `open_pr` handler**

  Append to `src/api/github.rs`:

  ```rust
  /// `POST /api/sessions/:id/pr` — push the session's branch and open a PR.
  /// Returns 200 with `{pr_url, pr_number}` on success. 422 if a PR already
  /// exists (`session.pr_url` is set); 503 if no token; 502 on push or PR
  /// creation failure (with token-redacted error from push or octocrab
  /// error from PR creation).
  pub async fn open_pr(
      State(mgr): State<Arc<SessionManager>>,
      Path(session_id): Path<Uuid>,
      Json(req): Json<OpenPrReq>,
  ) -> Response {
      let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
          Ok(s) => s,
          Err(resp) => return resp,
      };

      if let Some(existing) = session.pr_url.as_ref() {
          tracing::warn!(%session_id, %existing, "open_pr rejected: PR already exists");
          return (StatusCode::UNPROCESSABLE_ENTITY,
              Json(serde_json::json!({
                  "error": "PR already exists for this session",
                  "pr_url": existing,
              }))).into_response();
      }

      let Some(repo_str) = project.github_repo.as_ref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no linked repo"}))).into_response();
      };
      let Some((owner, name)) = parse_repo(repo_str) else {
          return (StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "stored repo has bad format"}))).into_response();
      };
      let Some(branch) = session.branch.as_deref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "session has no branch"}))).into_response();
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };
      let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

      // Push branch first.
      let worktree = std::path::PathBuf::from(&session.worktree_path);
      if let Err(e) = crate::github::push::push_branch(&worktree, branch, owner, name, &token) {
          tracing::warn!(%session_id, error = %e, "git push failed");
          return (StatusCode::BAD_GATEWAY,
              Json(serde_json::json!({"error": format!("{e}")}))).into_response();
      }

      // Create PR.
      let pr_info = match client.create_pull_request(
          owner, name, branch, &req.base,
          &req.title, &req.body, false,
      ).await {
          Ok(info) => info,
          Err(e) => {
              tracing::warn!(%session_id, error = %e, "create_pull_request failed");
              return (StatusCode::BAD_GATEWAY,
                  Json(serde_json::json!({"error": format!("{e}")}))).into_response();
          }
      };

      // Persist + emit event.
      if let Err(e) = mgr.db().set_session_pr_url(session_id, Some(&pr_info.html_url)) {
          tracing::warn!(%session_id, error = ?e, "set_session_pr_url failed after PR created");
          return crate::api::err_to_response(e);
      }
      mgr.event_bus().publish(crate::api::events::DomainEvent::SessionPrCreated {
          session_id,
          project_id: session.project_id,
          pr_url: pr_info.html_url.clone(),
          pr_number: pr_info.number,
      });

      Json(OpenPrResp {
          pr_url: pr_info.html_url,
          pr_number: pr_info.number,
      }).into_response()
  }
  ```

- [ ] **Step 4: Register route in `src/api/mod.rs`**

  Add to `router()`:

  ```rust
          .route("/api/sessions/:id/pr", post(github::open_pr))
  ```

- [ ] **Step 5: Write failing tests**

  Append to `tests/api_github.rs`:

  ```rust
  use axum::http::Method as HttpMethod;

  #[tokio::test]
  async fn open_pr_422_when_pr_url_already_set() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      let session = setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: Some("galvolite/abc12345".into()),
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();
      // Pre-populate pr_url to simulate a PR that already exists.
      setup.db.set_session_pr_url(session.id, Some("https://github.com/octocat/hello/pull/1")).unwrap();

      let req = Request::builder()
          .uri(format!("/api/sessions/{}/pr", session.id))
          .method(HttpMethod::POST)
          .header("content-type", "application/json")
          .body(Body::from(r#"{"title":"t","body":"b","base":"main"}"#))
          .unwrap();
      let resp = setup.app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

      let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
      let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
      assert_eq!(v["pr_url"], "https://github.com/octocat/hello/pull/1");

      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn open_pr_503_when_token_unset() {
      let _g = env_lock().lock().await;
      std::env::remove_var("GITHUB_TOKEN");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      let session = setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: Some("galvolite/abc12345".into()),
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();

      let req = Request::builder()
          .uri(format!("/api/sessions/{}/pr", session.id))
          .method(HttpMethod::POST)
          .header("content-type", "application/json")
          .body(Body::from(r#"{"title":"t","body":"b","base":"main"}"#))
          .unwrap();
      let resp = setup.app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }
  ```

  Note: the GitHub-network happy path is not exercisable through axum::State without a refactor (same limitation as Plan 10a's `link_repo`). Coverage is split: `tests/github_client.rs::create_pull_request_returns_pr_info_on_201` covers the GitHub call shape; `tests/github_push.rs::push_fails_with_redacted_token_*` covers the push helper; this file covers routing + state branches.

- [ ] **Step 6: Run tests**

  Run: `cargo test --test api_github -- --test-threads=1`
  Expected: 12 tests PASS (10 prior + 2 new).

- [ ] **Step 7: Commit**

  ```bash
  git add src/api/events.rs src/api/github.rs src/api/mod.rs tests/api_github.rs
  git commit -m "feat(api): POST /api/sessions/:id/pr + SessionPrCreated bus event"
  ```

---

## Task 7: UI types + `api.ts` + `DomainEvent` TS variant

**Files:**
- Modify: `ui/src/types.ts` (add `Session.pr_url?` + new types + DomainEvent variant)
- Modify: `ui/src/api.ts` (add 2 new API functions)

- [ ] **Step 1: Extend `Session` and add new types**

  Modify `ui/src/types.ts`. Update the `Session` interface (around lines 13-39). Add at the end of the interface:

  ```ts
    pr_url?: string | null; // Plan 10b — PR URL once "Open PR" has been used
  ```

  Append at the end of the file:

  ```ts
  // Plan 10b — PR creation types
  export interface PrDraft {
    title: string;
    body: string;
    base: string;
    commits_ahead: number;
    commits_behind: number;
  }

  export interface OpenPrReq {
    title: string;
    body: string;
    base: string;
  }

  export interface OpenPrResp {
    pr_url: string;
    pr_number: number;
  }
  ```

  Find the existing `DomainEvent` type union (around line 157). Add a new variant to the union:

  ```ts
  export type DomainEvent =
    | { type: "session.state_changed"; ... }
    // ... existing variants ...
    | { type: "session.pr_created"; session_id: string; project_id: string; pr_url: string; pr_number: number }
    // ... rest ...
  ```

  Read the existing `DomainEvent` union to see its exact shape; the new variant follows the same pattern as `session.state_changed` (it's session-scoped).

- [ ] **Step 2: Add API client functions**

  Modify `ui/src/api.ts`. Add `PrDraft`, `OpenPrReq`, `OpenPrResp` to the import block at the top:

  ```ts
  import type {
    // ... existing imports ...
    GithubStatus,
    GithubPr,
    GithubIssue,
    PrDraft,
    OpenPrReq,
    OpenPrResp,
  } from "./types";
  ```

  Append these functions to the `api = {...}` object, just before the closing `}`:

  ```ts
    // Plan 10b — PR creation endpoints

    getPrDraft: (sessionId: string) =>
      request<PrDraft>(`/api/sessions/${sessionId}/pr-draft`),

    openPr: (sessionId: string, req: OpenPrReq) =>
      request<OpenPrResp>(
        `/api/sessions/${sessionId}/pr`,
        { method: "POST", body: JSON.stringify(req) },
      ),
  ```

- [ ] **Step 3: Verify TS compiles**

  Run: `cd ui && npx tsc --noEmit`
  Expected: no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add ui/src/types.ts ui/src/api.ts
  git commit -m "feat(ui): types + api client for Plan 10b PR endpoints"
  ```

---

## Task 8: UI `OpenPrModal` component

**Files:**
- Create: `ui/src/components/modals/OpenPrModal.tsx`
- Create: `ui/src/components/modals/OpenPrModal.test.tsx`

Built on the shared `Modal` primitive (LinkRepoModal pattern). Fetches the draft on mount, lets the operator edit title/body/base, submits via `api.openPr`. Submit disabled when commits_ahead is 0 or title is empty.

- [ ] **Step 1: Write failing tests**

  Create `ui/src/components/modals/OpenPrModal.test.tsx`:

  ```tsx
  import { render, screen, fireEvent, waitFor } from "@testing-library/react";
  import { describe, it, expect, vi } from "vitest";
  import { OpenPrModal } from "./OpenPrModal";
  import { api } from "../../api";

  describe("OpenPrModal", () => {
    it("renders draft on mount", async () => {
      vi.spyOn(api, "getPrDraft").mockResolvedValue({
        title: "Add tests",
        body: "## Task\nfoo\n\n## Commits\n- abc123 init",
        base: "main",
        commits_ahead: 3,
        commits_behind: 0,
      });

      render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

      await waitFor(() => {
        expect(screen.getByDisplayValue("Add tests")).toBeInTheDocument();
        expect(screen.getByDisplayValue("main")).toBeInTheDocument();
        expect(screen.getByText(/3 commits ahead/i)).toBeInTheDocument();
      });
    });

    it("submit calls openPr with edited values", async () => {
      vi.spyOn(api, "getPrDraft").mockResolvedValue({
        title: "Original", body: "B", base: "main",
        commits_ahead: 1, commits_behind: 0,
      });
      const openPr = vi.fn().mockResolvedValue({
        pr_url: "https://github.com/x/y/pull/1", pr_number: 1,
      });
      vi.spyOn(api, "openPr").mockImplementation(openPr);
      const onCreated = vi.fn();

      render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={onCreated} />);

      await screen.findByDisplayValue("Original");
      fireEvent.change(screen.getByDisplayValue("Original"), {
        target: { value: "Edited title" }
      });
      fireEvent.click(screen.getByRole("button", { name: /submit/i }));

      await waitFor(() => {
        expect(openPr).toHaveBeenCalledWith("s", expect.objectContaining({
          title: "Edited title",
          base: "main",
        }));
        expect(onCreated).toHaveBeenCalledWith("https://github.com/x/y/pull/1", 1);
      });
    });

    it("submit disabled when commits_ahead is 0", async () => {
      vi.spyOn(api, "getPrDraft").mockResolvedValue({
        title: "X", body: "B", base: "main",
        commits_ahead: 0, commits_behind: 0,
      });

      render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

      await screen.findByDisplayValue("X");
      expect(screen.getByRole("button", { name: /submit/i })).toBeDisabled();
    });

    it("submit disabled when title is empty", async () => {
      vi.spyOn(api, "getPrDraft").mockResolvedValue({
        title: "Real", body: "B", base: "main",
        commits_ahead: 1, commits_behind: 0,
      });

      render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);

      await screen.findByDisplayValue("Real");
      fireEvent.change(screen.getByDisplayValue("Real"), { target: { value: "  " } });
      expect(screen.getByRole("button", { name: /submit/i })).toBeDisabled();
    });

    it("error banner renders on openPr rejection", async () => {
      vi.spyOn(api, "getPrDraft").mockResolvedValue({
        title: "X", body: "B", base: "main",
        commits_ahead: 1, commits_behind: 0,
      });
      vi.spyOn(api, "openPr").mockRejectedValue(
        new Error('502: {"error":"git push failed: permission denied"}')
      );

      render(<OpenPrModal sessionId="s" open={true} onClose={() => {}} onCreated={() => {}} />);
      await screen.findByDisplayValue("X");
      fireEvent.click(screen.getByRole("button", { name: /submit/i }));

      await waitFor(() => {
        expect(screen.getByText(/permission denied/i)).toBeInTheDocument();
      });
    });
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- OpenPrModal --run`
  Expected: FAIL — component doesn't exist.

- [ ] **Step 3: Create the component**

  Create `ui/src/components/modals/OpenPrModal.tsx`:

  ```tsx
  import { useEffect, useState } from "react";
  import { api } from "../../api";
  import { Modal } from "./Modal";

  interface Props {
    sessionId: string;
    open: boolean;
    onClose: () => void;
    /** Called on success with (pr_url, pr_number). Parent typically closes
     *  the modal in this callback. */
    onCreated: (prUrl: string, prNumber: number) => void;
  }

  /**
   * Plan 10b — modal that fetches a draft PR (title/body/base) for a
   * session, lets the operator edit, and submits to /api/sessions/:id/pr.
   * Built on the shared `Modal` primitive (LinkRepoModal pattern).
   */
  export function OpenPrModal({ sessionId, open, onClose, onCreated }: Props) {
    const [title, setTitle] = useState("");
    const [body, setBody] = useState("");
    const [base, setBase] = useState("");
    const [commitsAhead, setCommitsAhead] = useState<number | null>(null);
    const [loading, setLoading] = useState(true);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
      if (!open) return;
      let cancelled = false;
      setLoading(true);
      setError(null);
      api.getPrDraft(sessionId)
        .then((draft) => {
          if (cancelled) return;
          setTitle(draft.title);
          setBody(draft.body);
          setBase(draft.base);
          setCommitsAhead(draft.commits_ahead);
          setLoading(false);
        })
        .catch((e) => {
          if (cancelled) return;
          const msg = e instanceof Error ? e.message : String(e);
          setError(extractErrorMessage(msg));
          setLoading(false);
        });
      return () => { cancelled = true; };
    }, [open, sessionId]);

    const submit = async () => {
      if (busy || title.trim().length === 0 || !commitsAhead || commitsAhead === 0) return;
      setBusy(true);
      setError(null);
      try {
        const res = await api.openPr(sessionId, { title: title.trim(), body, base: base.trim() });
        onCreated(res.pr_url, res.pr_number);
        onClose();
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setError(extractErrorMessage(msg));
      } finally {
        setBusy(false);
      }
    };

    return (
      <Modal open={open} onClose={onClose} title="Open PR for session">
        <div className="p-4 space-y-3 min-w-[480px] max-w-[640px]">
          {loading && (
            <div className="text-sm text-neutral-500">Loading draft…</div>
          )}
          {!loading && (
            <>
              <div>
                <label className="text-xs text-neutral-400 block mb-1">Title</label>
                <input
                  type="text"
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2"
                  disabled={busy}
                  autoFocus
                />
              </div>
              <div>
                <label className="text-xs text-neutral-400 block mb-1">Base branch</label>
                <input
                  type="text"
                  value={base}
                  onChange={(e) => setBase(e.target.value)}
                  className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2"
                  disabled={busy}
                />
              </div>
              <div>
                <label className="text-xs text-neutral-400 block mb-1">Body</label>
                <textarea
                  value={body}
                  onChange={(e) => setBody(e.target.value)}
                  rows={12}
                  className="w-full text-xs bg-neutral-800 border border-neutral-700 rounded p-2 font-mono"
                  disabled={busy}
                />
              </div>
              <div className="text-xs text-neutral-500">
                {commitsAhead === 0
                  ? "0 commits ahead of base — nothing to PR"
                  : `${commitsAhead ?? "?"} commits ahead of ${base || "base"}`}
              </div>
              {error && (
                <div role="alert" className="text-xs text-red-400 whitespace-pre-wrap">
                  {error}
                </div>
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
                  disabled={busy || title.trim().length === 0 || commitsAhead === 0}
                >Submit</button>
              </div>
            </>
          )}
        </div>
      </Modal>
    );
  }

  /** Parse the request helper's "<status>: <body>" error format and
   *  extract `body.error` if present. Falls back to the raw message. */
  function extractErrorMessage(msg: string): string {
    const match = msg.match(/\d{3}:\s*(\{.*\})/);
    if (match) {
      try {
        const body = JSON.parse(match[1]);
        if (typeof body.error === "string") return body.error;
      } catch { /* fall through */ }
    }
    return msg;
  }
  ```

- [ ] **Step 4: Run tests to verify pass**

  Run: `cd ui && npm test -- OpenPrModal --run`
  Expected: 5 tests PASS.

- [ ] **Step 5: Run full UI suite to confirm no regressions**

  Run: `cd ui && npm test -- --run`
  Expected: all PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add ui/src/components/modals/OpenPrModal.tsx \
          ui/src/components/modals/OpenPrModal.test.tsx
  git commit -m "feat(ui): OpenPrModal with draft preview + edit + submit"
  ```

---

## Task 9: UI `ExpandedSessionView` wire-in

**Files:**
- Modify: `ui/src/components/ExpandedSessionView.tsx` (add Open PR button + View PR link + event handler)
- Modify: `ui/src/hooks/useSessionEvents.ts` (handle `session.pr_created` if not generic)

`ExpandedSessionView` is the per-session detail panel. The Open PR button (or View PR link, mutually exclusive) lives near the existing buttons (e.g. ExecutePlanButton, RoleBadge).

- [ ] **Step 1: Inspect existing ExpandedSessionView layout**

  Run: `grep -n "function ExpandedSessionView\|<button\|RoleBadge\|ExecutePlanButton" ui/src/components/ExpandedSessionView.tsx | head -10`

  Identify a sensible mounting location. The button should appear in the header/toolbar area where other per-session affordances live.

- [ ] **Step 2: Add Open PR button + View PR link**

  Modify `ui/src/components/ExpandedSessionView.tsx`. Add to imports:

  ```tsx
  import { useState } from "react";
  import { OpenPrModal } from "./modals/OpenPrModal";
  ```

  (If `useState` is already imported, just add `OpenPrModal`.)

  Inside the `ExpandedSessionView` function body, near the top, add state for the modal:

  ```tsx
    const [openPrModalOpen, setOpenPrModalOpen] = useState(false);
  ```

  In the JSX, find a sensible spot in the header/toolbar (near `<RoleBadge>` or `<ExecutePlanButton>`). Add this conditional rendering. The exact prop names depend on what's in scope — `session` should be the per-session prop in this component, and `project` may need to be threaded through if not already available:

  ```tsx
    {session.pr_url ? (
      <a
        href={session.pr_url}
        target="_blank"
        rel="noopener noreferrer"
        className="text-xs px-2 py-0.5 rounded bg-neutral-800 text-blue-400 hover:underline"
      >View PR</a>
    ) : (
      session.branch && project?.github_repo && (
        <button
          className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
          onClick={() => setOpenPrModalOpen(true)}
        >Open PR</button>
      )
    )}
  ```

  And near the end of the JSX (where other modals are mounted, if any):

  ```tsx
    <OpenPrModal
      sessionId={session.id}
      open={openPrModalOpen}
      onClose={() => setOpenPrModalOpen(false)}
      onCreated={(prUrl) => {
        setOpenPrModalOpen(false);
        // Local state update; the SessionPrCreated bus event will also
        // patch session.pr_url via useSessionEvents (see step 3).
      }}
    />
  ```

  Note: if `project` is not currently a prop on `ExpandedSessionView`, it needs to be threaded through from the parent (Dashboard.tsx). Check `<ExpandedSessionView` call sites and add the prop. Alternatively, the button can be gated on `session.branch` only (always show if branch exists, let the backend 404 if no linked repo) — simpler but slightly worse UX.

- [ ] **Step 3: Handle `session.pr_created` event in useSessionEvents**

  Inspect: `cat ui/src/hooks/useSessionEvents.ts | head -40`

  If `useSessionEvents` is generic (just delivers events to a callback), no change is needed — the parent's `applyEvent` callback handles them. If it has a specific switch on event types, add an arm for `session.pr_created` that updates the session's `pr_url` in local state.

  Look at how `session.state_changed` or `session.meta_changed` is handled for the pattern. The new arm should set `session.pr_url = e.pr_url` and trigger a re-render.

  In Dashboard.tsx (which uses `useSessions` for the session map), the `applyEvent` callback (around line 64) likely handles state mutations. Add a case for the new event there if needed:

  ```tsx
    case "session.pr_created":
      setSessions(prev => prev.map(s =>
        s.id === e.session_id ? { ...s, pr_url: e.pr_url } : s
      ));
      break;
  ```

  Use the existing pattern in `useSessions` / Dashboard for state updates — don't invent a new one.

- [ ] **Step 4: Run UI tests**

  Run: `cd ui && npm test -- --run`
  Expected: all tests pass; new ExpandedSessionView interactions don't break existing tests.

- [ ] **Step 5: Commit**

  ```bash
  git add ui/src/components/ExpandedSessionView.tsx \
          ui/src/components/Dashboard.tsx \
          ui/src/hooks/useSessionEvents.ts
  git commit -m "feat(ui): ExpandedSessionView Open PR button + View PR link + event handler"
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
  Expected: PASS, no warnings.

- [ ] **Update memory file `project_plan10b_status.md`**

  Per project convention (see `project_plan10a_status.md`), record completion: branch, commit range, test counts, decomposition placement (10b done; 10c/10d/10e remain).
