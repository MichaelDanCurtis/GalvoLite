# Plan 10d — Issue → Session Spawn Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** Per-issue "Start session" in GithubPanel spawns a child session with issue title in task_summary + issue body in `.galvolite/issue.md`.

**Spec:** [docs/superpowers/specs/2026-05-09-plan-10d-issue-spawn-design.md](../specs/2026-05-09-plan-10d-issue-spawn-design.md)

---

## Tasks

| # | Task |
|---|---|
| 1 | Migration 0009 + Session.github_issue_url + DB setter + tests |
| 2 | Client::get_issue + IssueDetail + mockito tests |
| 3 | API POST /api/projects/:id/github/issues/:number/spawn + tests |
| 4 | UI types + api.ts + GithubPanel button + tests |

---

## Task 1: Migration 0009 + Session.github_issue_url

- [ ] **Migration `migrations/0009_sessions_github_issue_url.sql`**:
  ```sql
  -- Plan 10d Task 1 — track which GitHub issue spawned this session.
  ALTER TABLE sessions ADD COLUMN github_issue_url TEXT NULL;
  ```

- [ ] **`src/db.rs`** — register `SCHEMA_V9` const + `(9, SCHEMA_V9)` in MIGRATIONS; add `pub github_issue_url: Option<String>` field on `Session` struct (with `#[serde(skip_serializing_if = "Option::is_none")]`); update `insert_session` (`github_issue_url: None`), `get_session`, `list_sessions` SELECTs (add `, github_issue_url` and `r.get(18)?`); add setter:

  ```rust
      pub fn set_session_github_issue_url(&self, id: Uuid, url: Option<&str>) -> Result<()> {
          let conn = self.pool.get()?;
          let rows = conn.execute(
              "UPDATE sessions SET github_issue_url = ?1 WHERE id = ?2",
              params![url, id.to_string()],
          )?;
          if rows == 0 {
              return Err(Error::SessionNotFound(id));
          }
          Ok(())
      }
  ```

- [ ] **`tests/db_init.rs:19`** — bump assertion `version == 9`.

- [ ] **`tests/db_crud.rs`** — add `session_github_issue_url_round_trip` and `set_session_github_issue_url_unknown_session_errors` tests, mirroring the existing `session_pr_url_round_trip` pattern.

- [ ] **Run** `cargo test --test db_init --test db_crud -- --test-threads=1` then `cargo test -- --test-threads=1` (re-run on env-var-race exit-1).

- [ ] **Commit:** `feat(db): add sessions.github_issue_url column + setter`

---

## Task 2: `Client::get_issue` + `IssueDetail`

- [ ] **`src/github/client.rs`** — append:

  ```rust
  /// Plan 10d — single-issue detail for the spawn flow.
  #[derive(Debug, Clone, Serialize)]
  pub struct IssueDetail {
      pub number: u64,
      pub title: String,
      pub body: String,
      pub html_url: String,
  }
  ```

  And on `impl Client`:

  ```rust
      /// Plan 10d — fetch a single issue by number.
      pub async fn get_issue(&self, owner: &str, name: &str, number: u64) -> Result<IssueDetail, GithubError> {
          let issue = self.inner
              .issues(owner, name)
              .get(number)
              .await
              .map_err(map_err)?;
          Ok(IssueDetail {
              number: issue.number,
              title: issue.title,
              body: issue.body.unwrap_or_default(),
              html_url: issue.html_url.to_string(),
          })
      }
  ```

- [ ] **`src/github/mod.rs`** — add `IssueDetail` to the re-export.

- [ ] **`tests/github_client.rs`** — add 2 mockito tests:

  ```rust
  #[tokio::test]
  async fn get_issue_returns_detail_on_200() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello/issues/7")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"{
              "url": "x",
              "id": 99,
              "node_id": "x",
              "html_url": "https://github.com/octocat/hello/issues/7",
              "number": 7,
              "state": "open",
              "title": "Bug: thing broken",
              "body": "Reproduces when…",
              "user": {"login":"alice","id":1,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "labels": [],
              "assignee": null,
              "assignees": [],
              "milestone": null,
              "locked": false,
              "comments": 0,
              "closed_at": null,
              "created_at": "2026-05-01T00:00:00Z",
              "updated_at": "2026-05-01T00:00:00Z",
              "author_association": "CONTRIBUTOR"
          }"#)
          .create_async()
          .await;
      let client = Client::with_token("t", Some(server.url())).unwrap();
      let issue = client.get_issue("octocat", "hello", 7).await.unwrap();
      assert_eq!(issue.number, 7);
      assert_eq!(issue.title, "Bug: thing broken");
      assert!(issue.body.contains("Reproduces"));
  }

  #[tokio::test]
  async fn get_issue_err_on_404() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello/issues/9999")
          .with_status(404)
          .with_body(r#"{"message":"Not Found"}"#)
          .create_async()
          .await;
      let client = Client::with_token("t", Some(server.url())).unwrap();
      assert!(client.get_issue("octocat", "hello", 9999).await.is_err());
  }
  ```

  (octocrab's strict deserializer may need additional fields; adapt as needed.)

- [ ] **Commit:** `feat(github): add Client::get_issue + IssueDetail`

---

## Task 3: API `POST /api/projects/:id/github/issues/:number/spawn`

- [ ] **`src/api/github.rs`** — append:

  ```rust
  #[derive(Debug, Serialize)]
  pub struct SpawnFromIssueResp {
      pub session_id: Uuid,
  }

  /// `POST /api/projects/:id/github/issues/:number/spawn` — fetches issue,
  /// spawns child session under the project's orchestrator with issue
  /// title in task_summary + issue body written to `<worktree>/.galvolite/issue.md`.
  pub async fn spawn_from_issue(
      State(mgr): State<Arc<SessionManager>>,
      Path((project_id, number)): Path<(Uuid, u64)>,
  ) -> Response {
      let project = match mgr.db().get_project(project_id) {
          Ok(p) => p,
          Err(crate::error::Error::ProjectNotFound(_))
          | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
              return (StatusCode::NOT_FOUND,
                  Json(serde_json::json!({"error": "project not found"}))).into_response();
          }
          Err(e) => return crate::api::err_to_response(e),
      };
      let Some(repo_str) = project.github_repo.as_ref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no linked repo"}))).into_response();
      };
      let Some((owner, name)) = parse_repo(repo_str) else {
          return (StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "stored repo has bad format"}))).into_response();
      };

      let Some(orchestrator) = mgr.find_orchestrator_for_project(project_id) else {
          return (StatusCode::CONFLICT,
              Json(serde_json::json!({"error": "no orchestrator session running"}))).into_response();
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };

      let issue = match client.get_issue(owner, name, number).await {
          Ok(i) => i,
          Err(e) => {
              tracing::warn!(%project_id, %number, error = %e, "get_issue failed");
              return (StatusCode::BAD_GATEWAY,
                  Json(serde_json::json!({"error": format!("{e}")}))).into_response();
          }
      };

      let task = if issue.title.chars().count() > 200 {
          issue.title.chars().take(200).collect::<String>()
      } else {
          issue.title.clone()
      };

      let new_session = match mgr.spawn_child(orchestrator.id, "implementation", &task, None).await {
          Ok(s) => s,
          Err(e) => return crate::api::err_to_response(e),
      };

      // Write issue body to the new worktree's .galvolite/issue.md so the
      // operator/Claude can reference it (best-effort; failures don't roll back).
      let issue_md = std::path::PathBuf::from(&new_session.worktree_path).join(".galvolite/issue.md");
      if let Some(parent) = issue_md.parent() {
          let _ = std::fs::create_dir_all(parent);
      }
      let content = format!("# {}\n\n{}\n\n---\n_From: {}_\n", issue.title, issue.body, issue.html_url);
      if let Err(e) = std::fs::write(&issue_md, content) {
          tracing::warn!(?issue_md, error = %e, "failed to write .galvolite/issue.md");
      }

      // Track the issue link on the session row.
      if let Err(e) = mgr.db().set_session_github_issue_url(new_session.id, Some(&issue.html_url)) {
          tracing::warn!(session_id = %new_session.id, error = ?e, "set_session_github_issue_url failed");
      }

      Json(SpawnFromIssueResp { session_id: new_session.id }).into_response()
  }
  ```

- [ ] **`src/api/mod.rs`** — register route:

  ```rust
          .route("/api/projects/:id/github/issues/:number/spawn", post(github::spawn_from_issue))
  ```

- [ ] **`tests/api_github.rs`** — add 3 tests (same pattern as Plan 10b's `link_repo` tests). Skip the GitHub-network happy path (covered by client-level mockito tests).

  ```rust
  #[tokio::test]
  async fn spawn_from_issue_404_when_no_linked_repo() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");
      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      let req = Request::builder()
          .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
          .method(HttpMethod::POST)
          .body(Body::empty()).unwrap();
      let resp = setup.app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);
      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn spawn_from_issue_503_when_token_unset() {
      let _g = env_lock().lock().await;
      std::env::remove_var("GITHUB_TOKEN");
      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      // Insert a "fake orchestrator" so we get past the 409 check.
      setup.db.insert_session(NewSession {
          project_id: project.id,
          role: "implementation".into(),
          branch: None,
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "orch".into(),
          parent_session_id: None,
      }).unwrap();
      let req = Request::builder()
          .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
          .method(HttpMethod::POST)
          .body(Body::empty()).unwrap();
      let resp = setup.app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }

  #[tokio::test]
  async fn spawn_from_issue_409_when_no_orchestrator() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_TOKEN", "x");
      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();
      // No session inserted = no orchestrator.
      let req = Request::builder()
          .uri(format!("/api/projects/{}/github/issues/7/spawn", project.id))
          .method(HttpMethod::POST)
          .body(Body::empty()).unwrap();
      let resp = setup.app.oneshot(req).await.unwrap();
      assert_eq!(resp.status(), StatusCode::CONFLICT);
      std::env::remove_var("GITHUB_TOKEN");
  }
  ```

  Note: `find_orchestrator_for_project` returns `Some` if any session has `parent_session_id IS NULL && ended_at IS NULL`. The 503 test inserts a fake "orchestrator" to pass this gate.

- [ ] **Commit:** `feat(api): POST /api/projects/:id/github/issues/:number/spawn`

---

## Task 4: UI types + api.ts + GithubPanel button

- [ ] **`ui/src/types.ts`** — add `github_issue_url?: string | null` to `Session`. Append:

  ```ts
  export interface SpawnFromIssueResp {
    session_id: string;
  }
  ```

- [ ] **`ui/src/api.ts`** — add `SpawnFromIssueResp` import. Append:

  ```ts
    spawnFromIssue: (projectId: string, issueNumber: number) =>
      request<SpawnFromIssueResp>(
        `/api/projects/${projectId}/github/issues/${issueNumber}/spawn`,
        { method: "POST" },
      ),
  ```

- [ ] **`ui/src/components/GithubPanel.tsx`** — extend each issue list item to render a "Start session" button. State for the in-flight spawn (busy, error). Keep diff small:

  Add to props (or component-internal state):
  ```tsx
    const [spawnBusy, setSpawnBusy] = useState<number | null>(null); // issue number being spawned
    const [spawnError, setSpawnError] = useState<string | null>(null);
  ```

  In each `<li key={i.number}>` for issues, after the existing label render, add:
  ```tsx
                    <button
                      className="ml-2 text-[10px] px-1 py-0.5 rounded bg-blue-700 hover:bg-blue-600 text-white disabled:opacity-50"
                      disabled={spawnBusy !== null}
                      onClick={async () => {
                        setSpawnBusy(i.number);
                        setSpawnError(null);
                        try {
                          await api.spawnFromIssue(projectId, i.number);
                        } catch (e) {
                          setSpawnError(e instanceof Error ? e.message : String(e));
                        } finally {
                          setSpawnBusy(null);
                        }
                      }}
                    >Start session</button>
  ```

  Render an error banner (after the existing `error` block) for `spawnError`:
  ```tsx
          {spawnError && (
            <div className="text-xs text-red-400 mb-2">Spawn error: {spawnError}</div>
          )}
  ```

- [ ] **`ui/src/components/GithubPanel.test.tsx`** — add a test:

  ```tsx
  it("Start session button calls spawnFromIssue", async () => {
    vi.spyOn(api, "listGithubPrs").mockResolvedValue([]);
    vi.spyOn(api, "listGithubIssues").mockResolvedValue([
      { number: 7, title: "Bug", author: "alice", labels: [], updated_at: "2026-05-01T00:00:00Z", url: "https://github.com/x/y/issues/7" }
    ]);
    const spawn = vi.fn().mockResolvedValue({ session_id: "new-id" });
    vi.spyOn(api, "spawnFromIssue").mockImplementation(spawn);

    render(<GithubPanel projectId="p" repo="x/y" />);
    await screen.findByText("Bug");
    fireEvent.click(screen.getByRole("button", { name: /start session/i }));
    await waitFor(() => {
      expect(spawn).toHaveBeenCalledWith("p", 7);
    });
  });
  ```

- [ ] **Commit:** `feat(ui): GithubPanel "Start session" button + types + api`

---

## Final verification

- `cargo test -- --test-threads=1` (re-run on env-race exit-1)
- `cd ui && npm test -- --run`
- `cargo build --release`
- Update `project_plan10d_status.md`
