# Plan 10c — PR Review Surfacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement task-by-task.

**Goal:** Surface GitHub PR reviews + inline comments in the dashboard for sessions with a linked PR. Read-only.

**Architecture:** Extends `src/github/client.rs` with `list_pr_reviews` (parallel GET via `tokio::join!`), one new `GET /api/sessions/:id/pr-reviews` endpoint, one new `PrReviewsPanel` React component mounted in `ExpandedSessionView` when `session.pr_url` is non-null.

**Tech Stack:** Rust 1.x (axum, tokio, octocrab), React + TypeScript + Vitest. No new dependencies.

**Spec:** [docs/superpowers/specs/2026-05-09-plan-10c-pr-review-surfacing-design.md](../specs/2026-05-09-plan-10c-pr-review-surfacing-design.md)

---

## Task ordering

| # | Task |
|---|---|
| 1 | Backend: `list_pr_reviews` on Client + types + mockito tests |
| 2 | Backend: `pr_reviews` API handler + tests |
| 3 | UI: types + api client function |
| 4 | UI: `PrReviewsPanel` component + tests + ExpandedSessionView mount |

---

## Task 1: `Client::list_pr_reviews` + types

**Files:**
- Modify: `src/github/client.rs` (add 3 types + 1 method)
- Modify: `src/github/mod.rs` (re-export)
- Test: `tests/github_client.rs` (extend with 2 tests)

- [ ] **Step 1: Add types**

  Append to `src/github/client.rs`:

  ```rust
  /// Plan 10c — top-level PR review summary (one per submitted review).
  #[derive(Debug, Clone, Serialize)]
  pub struct ReviewSummary {
      pub id: u64,
      pub author: String,
      pub state: String, // APPROVED | CHANGES_REQUESTED | COMMENTED | DISMISSED | PENDING
      pub body: String,
      pub submitted_at: String, // RFC 3339; empty if pending
      pub url: String,          // html_url
  }

  /// Plan 10c — inline review comment (anchored to a file/line).
  #[derive(Debug, Clone, Serialize)]
  pub struct ReviewCommentSummary {
      pub id: u64,
      pub author: String,
      pub body: String,
      pub path: String,
      pub line: Option<u64>, // None for outdated/file-level comments
      pub url: String,
  }

  /// Plan 10c — combined bundle returned to the dashboard.
  #[derive(Debug, Clone, Serialize)]
  pub struct PrReviewsBundle {
      pub reviews: Vec<ReviewSummary>,
      pub comments: Vec<ReviewCommentSummary>,
  }
  ```

- [ ] **Step 2: Add `list_pr_reviews` method**

  Append to `impl Client`:

  ```rust
      /// Plan 10c — fetch top-level reviews + inline comments for a PR
      /// in parallel via `tokio::join!`. Returns both halves as a single
      /// bundle. On either fetch failing, returns the first error
      /// encountered.
      pub async fn list_pr_reviews(
          &self,
          owner: &str,
          name: &str,
          pr_number: u64,
      ) -> Result<PrReviewsBundle, GithubError> {
          let reviews_fut = self.inner.pulls(owner, name).list_reviews(pr_number).send();
          let comments_fut = self.inner.pulls(owner, name).list_comments(Some(pr_number)).send();
          let (reviews_res, comments_res) = tokio::join!(reviews_fut, comments_fut);

          let reviews_page = reviews_res.map_err(map_err)?;
          let comments_page = comments_res.map_err(map_err)?;

          let reviews: Vec<ReviewSummary> = reviews_page.items.into_iter().map(|r| ReviewSummary {
              id: r.id.into_inner(),
              author: r.user.map(|u| u.login).unwrap_or_default(),
              state: r.state.map(|s| format!("{:?}", s).to_uppercase()).unwrap_or_else(|| "PENDING".into()),
              body: r.body.unwrap_or_default(),
              submitted_at: r.submitted_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
              url: r.html_url.map(|u| u.to_string()).unwrap_or_default(),
          }).collect();

          let comments: Vec<ReviewCommentSummary> = comments_page.items.into_iter().map(|c| ReviewCommentSummary {
              id: c.id.into_inner(),
              author: c.user.map(|u| u.login).unwrap_or_default(),
              body: c.body,
              path: c.path,
              line: c.line.or(c.original_line),
              url: c.html_url.to_string(),
          }).collect();

          Ok(PrReviewsBundle { reviews, comments })
      }
  ```

  Note: octocrab's exact field names for review/comment models vary by version — the implementer should adapt if the resolved 0.42 API uses different names (e.g. `r.user` may be `Option<Author>`, etc.). The shape goal is what matters: extract author login, body, state, html_url, path/line for comments.

- [ ] **Step 3: Re-export from `src/github/mod.rs`**

  Update the re-export line:

  ```rust
  pub use client::{Client, IssueSummary, PrInfo, PrReviewsBundle, PrSummary, ReviewCommentSummary, ReviewSummary, User};
  ```

- [ ] **Step 4: Write failing tests**

  Append to `tests/github_client.rs`:

  ```rust
  #[tokio::test]
  async fn list_pr_reviews_returns_bundle_on_success() {
      let mut server = mockito::Server::new_async().await;
      // Reviews list
      server.mock("GET", "/repos/octocat/hello/pulls/42/reviews")
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"[{
              "id": 100, "node_id": "x",
              "user": {"login":"alice","id":1,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "body": "LGTM",
              "state": "APPROVED",
              "html_url": "https://github.com/octocat/hello/pull/42#pullrequestreview-100",
              "pull_request_url": "x",
              "_links": {},
              "submitted_at": "2026-05-01T00:00:00Z",
              "commit_id": "abc",
              "author_association": "OWNER"
          }]"#)
          .create_async()
          .await;
      // Inline comments list
      server.mock("GET", "/repos/octocat/hello/pulls/comments")
          .match_query(mockito::Matcher::Any)
          .with_status(200)
          .with_header("content-type", "application/json")
          .with_body(r#"[{
              "url": "x",
              "pull_request_review_id": 100,
              "id": 200,
              "node_id": "x",
              "diff_hunk": "@@ -1,1 +1,1 @@",
              "path": "src/main.rs",
              "commit_id": "abc",
              "original_commit_id": "abc",
              "user": {"login":"bob","id":2,"node_id":"y","avatar_url":"x","gravatar_id":"","url":"x","html_url":"x","type":"User","site_admin":false},
              "body": "consider extracting this",
              "created_at": "2026-05-01T00:00:00Z",
              "updated_at": "2026-05-01T00:00:00Z",
              "html_url": "https://github.com/octocat/hello/pull/42#discussion_r200",
              "pull_request_url": "x",
              "author_association": "CONTRIBUTOR",
              "_links": {},
              "line": 42,
              "original_line": 42
          }]"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      let bundle = client.list_pr_reviews("octocat", "hello", 42).await.unwrap();
      assert_eq!(bundle.reviews.len(), 1);
      assert_eq!(bundle.reviews[0].author, "alice");
      assert!(bundle.reviews[0].state.contains("APPROVED"));
      assert_eq!(bundle.comments.len(), 1);
      assert_eq!(bundle.comments[0].path, "src/main.rs");
      assert_eq!(bundle.comments[0].line, Some(42));
  }

  #[tokio::test]
  async fn list_pr_reviews_err_on_404() {
      let mut server = mockito::Server::new_async().await;
      server.mock("GET", "/repos/octocat/hello/pulls/9999/reviews")
          .with_status(404)
          .with_body(r#"{"message":"Not Found"}"#)
          .create_async()
          .await;
      // Comments mock not strictly required since tokio::join! returns
      // both errors; one suffices.
      server.mock("GET", "/repos/octocat/hello/pulls/comments")
          .match_query(mockito::Matcher::Any)
          .with_status(404)
          .with_body(r#"{"message":"Not Found"}"#)
          .create_async()
          .await;

      let client = Client::with_token("t", Some(server.url())).unwrap();
      assert!(client.list_pr_reviews("octocat", "hello", 9999).await.is_err());
  }
  ```

  Note: octocrab's strict deserializer may require additional fields in the JSON payloads. Adapt as needed (study existing `validate_repo_ok_on_200` for the pattern).

- [ ] **Step 5: Run tests + commit**

  ```bash
  cargo test --test github_client -- --test-threads=1
  git add src/github/client.rs src/github/mod.rs tests/github_client.rs
  git commit -m "feat(github): add Client::list_pr_reviews + Review/Comment summary types"
  ```

---

## Task 2: API `GET /api/sessions/:id/pr-reviews`

**Files:**
- Modify: `src/api/github.rs` (add `pr_reviews` handler + helper)
- Modify: `src/api/mod.rs` (route)
- Test: `tests/api_github.rs` (extend with 2 tests)

- [ ] **Step 1: Add helper to extract PR number from URL**

  Append to `src/api/github.rs`:

  ```rust
  /// Plan 10c — extract PR number from a GitHub PR URL.
  /// Matches `https://github.com/<owner>/<name>/pull/<number>` (with or
  /// without trailing path/fragment). Returns None if shape unexpected.
  fn parse_pr_number(url: &str) -> Option<u64> {
      let after_pull = url.split("/pull/").nth(1)?;
      let num_str: String = after_pull.chars().take_while(|c| c.is_ascii_digit()).collect();
      num_str.parse().ok()
  }
  ```

- [ ] **Step 2: Add `pr_reviews` handler**

  Append to `src/api/github.rs`:

  ```rust
  /// `GET /api/sessions/:id/pr-reviews` — returns reviews + inline comments
  /// for the session's linked PR. 404 if session has no pr_url; 503 if no
  /// token; 502 on GitHub error; 500 if pr_url has unexpected shape.
  pub async fn pr_reviews(
      State(mgr): State<Arc<SessionManager>>,
      Path(session_id): Path<Uuid>,
  ) -> Response {
      let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
          Ok(s) => s,
          Err(resp) => return resp,
      };

      let Some(pr_url) = session.pr_url.as_deref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no PR for this session"}))).into_response();
      };
      let Some(pr_number) = parse_pr_number(pr_url) else {
          return (StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "could not parse PR number from url"}))).into_response();
      };

      let Some(repo_str) = project.github_repo.as_ref() else {
          return (StatusCode::NOT_FOUND,
              Json(serde_json::json!({"error": "no linked repo"}))).into_response();
      };
      let Some((owner, name)) = parse_repo(repo_str) else {
          return (StatusCode::INTERNAL_SERVER_ERROR,
              Json(serde_json::json!({"error": "stored repo has bad format"}))).into_response();
      };

      let Some(client) = crate::github::Client::from_env() else {
          return (StatusCode::SERVICE_UNAVAILABLE,
              Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"}))).into_response();
      };

      match client.list_pr_reviews(owner, name, pr_number).await {
          Ok(bundle) => Json(bundle).into_response(),
          Err(e) => {
              tracing::warn!(%session_id, %pr_number, error = %e, "list_pr_reviews failed");
              (StatusCode::BAD_GATEWAY,
                  Json(serde_json::json!({"error": format!("{e}")}))).into_response()
          }
      }
  }
  ```

- [ ] **Step 3: Register route**

  In `src/api/mod.rs`:

  ```rust
          .route("/api/sessions/:id/pr-reviews", get(github::pr_reviews))
  ```

- [ ] **Step 4: Tests**

  Append to `tests/api_github.rs`:

  ```rust
  #[tokio::test]
  async fn pr_reviews_404_when_no_pr_url() {
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
          branch: Some("galvolite/abc".into()),
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();
      // No pr_url set.
      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-reviews", session.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::NOT_FOUND);
      std::env::remove_var("GITHUB_TOKEN");
  }

  #[tokio::test]
  async fn pr_reviews_503_when_token_unset() {
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
          branch: Some("galvolite/abc".into()),
          worktree_path: setup._dir.path().to_string_lossy().into_owned(),
          task_summary: "test".into(),
          parent_session_id: None,
      }).unwrap();
      setup.db.set_session_pr_url(session.id, Some("https://github.com/octocat/hello/pull/42")).unwrap();
      let resp = setup.app.oneshot(
          Request::builder()
              .uri(format!("/api/sessions/{}/pr-reviews", session.id))
              .body(Body::empty()).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }
  ```

- [ ] **Step 5: Run tests + commit**

  ```bash
  cargo test --test api_github -- --test-threads=1
  git add src/api/github.rs src/api/mod.rs tests/api_github.rs
  git commit -m "feat(api): GET /api/sessions/:id/pr-reviews"
  ```

---

## Task 3: UI types + api client

**Files:**
- Modify: `ui/src/types.ts`
- Modify: `ui/src/api.ts`

- [ ] **Step 1: Add types**

  Append to `ui/src/types.ts`:

  ```ts
  // Plan 10c — PR review surfacing
  export interface PrReview {
    id: number;
    author: string;
    state: string;
    body: string;
    submitted_at: string;
    url: string;
  }

  export interface PrReviewComment {
    id: number;
    author: string;
    body: string;
    path: string;
    line: number | null;
    url: string;
  }

  export interface PrReviewsBundle {
    reviews: PrReview[];
    comments: PrReviewComment[];
  }
  ```

- [ ] **Step 2: Add api function**

  Modify `ui/src/api.ts`. Add `PrReviewsBundle` to the imports, then append before the closing `}`:

  ```ts
    // Plan 10c — PR reviews
    getPrReviews: (sessionId: string) =>
      request<PrReviewsBundle>(`/api/sessions/${sessionId}/pr-reviews`),
  ```

- [ ] **Step 3: Compile + commit**

  ```bash
  cd ui && npx tsc --noEmit && cd ..
  git add ui/src/types.ts ui/src/api.ts
  git commit -m "feat(ui): types + api client for PR reviews"
  ```

---

## Task 4: UI `PrReviewsPanel` + ExpandedSessionView mount

**Files:**
- Create: `ui/src/components/PrReviewsPanel.tsx`
- Create: `ui/src/components/PrReviewsPanel.test.tsx`
- Modify: `ui/src/components/ExpandedSessionView.tsx` (mount panel when pr_url set)

- [ ] **Step 1: Write failing tests**

  Create `ui/src/components/PrReviewsPanel.test.tsx`:

  ```tsx
  import { render, screen, fireEvent, waitFor } from "@testing-library/react";
  import { describe, it, expect, vi } from "vitest";
  import { PrReviewsPanel } from "./PrReviewsPanel";
  import { api } from "../api";

  describe("PrReviewsPanel", () => {
    it("renders empty state when no reviews and no comments", async () => {
      vi.spyOn(api, "getPrReviews").mockResolvedValue({ reviews: [], comments: [] });
      render(<PrReviewsPanel sessionId="s" />);
      await waitFor(() => {
        expect(screen.getByText(/no reviews/i)).toBeInTheDocument();
      });
    });

    it("renders reviews and comments", async () => {
      vi.spyOn(api, "getPrReviews").mockResolvedValue({
        reviews: [{ id: 1, author: "alice", state: "APPROVED", body: "LGTM", submitted_at: "2026-05-01T00:00:00Z", url: "https://github.com/x/y/pull/1#pullrequestreview-1" }],
        comments: [{ id: 2, author: "bob", body: "extract this", path: "src/main.rs", line: 42, url: "https://github.com/x/y/pull/1#discussion_r2" }],
      });
      render(<PrReviewsPanel sessionId="s" />);
      await waitFor(() => {
        expect(screen.getByText("alice")).toBeInTheDocument();
        expect(screen.getByText(/APPROVED/i)).toBeInTheDocument();
        expect(screen.getByText("bob")).toBeInTheDocument();
        expect(screen.getByText(/src\/main\.rs/)).toBeInTheDocument();
        expect(screen.getByText(/extract this/)).toBeInTheDocument();
      });
    });

    it("Refresh button refetches", async () => {
      const m = vi.spyOn(api, "getPrReviews").mockResolvedValue({ reviews: [], comments: [] });
      render(<PrReviewsPanel sessionId="s" />);
      await waitFor(() => expect(m).toHaveBeenCalledTimes(1));
      fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
      await waitFor(() => expect(m).toHaveBeenCalledTimes(2));
    });

    it("error banner on api error", async () => {
      vi.spyOn(api, "getPrReviews").mockRejectedValue(new Error("502: bad gateway"));
      render(<PrReviewsPanel sessionId="s" />);
      await waitFor(() => {
        expect(screen.getByText(/bad gateway/i)).toBeInTheDocument();
      });
    });
  });
  ```

- [ ] **Step 2: Create the component**

  Create `ui/src/components/PrReviewsPanel.tsx`:

  ```tsx
  import { useEffect, useRef, useState, useCallback } from "react";
  import { api } from "../api";
  import type { PrReviewsBundle } from "../types";

  interface Props {
    sessionId: string;
  }

  /**
   * Plan 10c — per-session PR reviews + inline comments panel.
   * Manual refresh; no polling. Mounted in ExpandedSessionView when
   * the session has a pr_url.
   */
  export function PrReviewsPanel({ sessionId }: Props) {
    const [bundle, setBundle] = useState<PrReviewsBundle | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const tokenRef = useRef(0);

    const fetchAll = useCallback(async () => {
      const myToken = ++tokenRef.current;
      setBusy(true);
      setError(null);
      try {
        const res = await api.getPrReviews(sessionId);
        if (myToken === tokenRef.current) setBundle(res);
      } catch (e) {
        if (myToken === tokenRef.current) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (myToken === tokenRef.current) setBusy(false);
      }
    }, [sessionId]);

    useEffect(() => {
      fetchAll();
      return () => { ++tokenRef.current; };
    }, [fetchAll]);

    return (
      <div className="border border-neutral-700 rounded p-3 mt-2">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-sm font-medium">PR Reviews</h3>
          <button
            className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600 disabled:opacity-50"
            onClick={fetchAll}
            disabled={busy}
          >Refresh</button>
        </div>

        {error && (
          <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">Error: {error}</div>
        )}

        {bundle === null && !error && (
          <div className="text-xs text-neutral-600">Loading…</div>
        )}

        {bundle && bundle.reviews.length === 0 && bundle.comments.length === 0 && (
          <div className="text-xs text-neutral-600">No reviews yet.</div>
        )}

        {bundle && bundle.reviews.length > 0 && (
          <div className="mb-3">
            <div className="text-xs text-neutral-500 mb-1">Reviews</div>
            <ul className="space-y-2">
              {bundle.reviews.map((r) => (
                <li key={r.id} className="text-xs">
                  <div>
                    <a href={r.url} target="_blank" rel="noopener noreferrer" className="text-blue-400 hover:underline">
                      @{r.author}
                    </a>
                    <span className={`ml-2 px-1 rounded text-[10px] uppercase ${r.state === "APPROVED" ? "bg-emerald-900 text-emerald-300" : r.state === "CHANGES_REQUESTED" ? "bg-red-900 text-red-300" : "bg-neutral-800 text-neutral-300"}`}>
                      {r.state}
                    </span>
                  </div>
                  {r.body && <div className="text-neutral-300 mt-1 whitespace-pre-wrap">{r.body}</div>}
                </li>
              ))}
            </ul>
          </div>
        )}

        {bundle && bundle.comments.length > 0 && (
          <div>
            <div className="text-xs text-neutral-500 mb-1">Inline comments</div>
            <ul className="space-y-2">
              {bundle.comments.map((c) => (
                <li key={c.id} className="text-xs">
                  <div>
                    <a href={c.url} target="_blank" rel="noopener noreferrer" className="text-blue-400 hover:underline">
                      @{c.author}
                    </a>
                    <span className="ml-2 text-neutral-500 font-mono">
                      {c.path}{c.line != null ? `:${c.line}` : ""}
                    </span>
                  </div>
                  <div className="text-neutral-300 mt-1 whitespace-pre-wrap">{c.body}</div>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    );
  }
  ```

- [ ] **Step 3: Mount in ExpandedSessionView**

  Modify `ui/src/components/ExpandedSessionView.tsx`. Add to imports:

  ```tsx
  import { PrReviewsPanel } from "./PrReviewsPanel";
  ```

  Find a sensible location near where the View PR link is rendered. Below the toolbar, in the body area, add:

  ```tsx
  {session.pr_url && <PrReviewsPanel sessionId={session.id} />}
  ```

- [ ] **Step 4: Run tests + commit**

  ```bash
  cd ui && npm test -- PrReviewsPanel --run && npm test -- --run && cd ..
  git add ui/src/components/PrReviewsPanel.tsx \
          ui/src/components/PrReviewsPanel.test.tsx \
          ui/src/components/ExpandedSessionView.tsx
  git commit -m "feat(ui): PrReviewsPanel + ExpandedSessionView mount"
  ```

---

## Final verification

- `cargo test -- --test-threads=1` — all green (cross-binary env-var race may flake; re-run if exit-1)
- `cd ui && npm test -- --run` — all green
- `cargo build --release` — clean
- Update `project_plan10c_status.md` per project convention
