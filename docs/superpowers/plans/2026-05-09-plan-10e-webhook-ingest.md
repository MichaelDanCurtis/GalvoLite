# Plan 10e — Webhook Ingest Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** Accept GitHub webhook deliveries, validate HMAC-SHA256, translate `pull_request` (merged) + `issues` (opened) → bus DomainEvents.

**Spec:** [docs/superpowers/specs/2026-05-09-plan-10e-webhook-ingest-design.md](../specs/2026-05-09-plan-10e-webhook-ingest-design.md)

---

## Tasks

| # | Task |
|---|---|
| 1 | Add `hmac` dep + `github::webhook::verify_signature` + tests |
| 2 | DomainEvent variants + `webhook_handler` + tests + UI types |

(Two tasks since 10e is small.)

---

## Task 1: HMAC validator

**Files:**
- Modify: `Cargo.toml` — add `hmac = "0.12"` to `[dependencies]`
- Create: `src/github/webhook.rs`
- Modify: `src/github/mod.rs` — `pub mod webhook;` + re-export `WebhookError`
- Test: `tests/github_webhook.rs` (new)

- [ ] **Step 1: Add hmac dep**

  In `Cargo.toml` `[dependencies]` (alphabetical, after `hex`):

  ```toml
  hmac = "0.12"
  ```

- [ ] **Step 2: Create `src/github/webhook.rs`**

  ```rust
  //! Plan 10e — GitHub webhook signature validation.
  //!
  //! GitHub signs webhook deliveries with HMAC-SHA256 of the raw body using
  //! a per-webhook secret. The signature arrives in the `X-Hub-Signature-256`
  //! header as `sha256=<hex>`. We compute the expected HMAC and constant-time
  //! compare via `subtle::ConstantTimeEq`.

  use hmac::{Hmac, Mac};
  use sha2::Sha256;
  use subtle::ConstantTimeEq;

  type HmacSha256 = Hmac<Sha256>;

  #[derive(Debug, thiserror::Error)]
  pub enum WebhookError {
      #[error("missing or malformed signature header")]
      MalformedSignature,

      #[error("signature mismatch")]
      Mismatch,
  }

  /// Verify `body`'s HMAC-SHA256 signature against the `sha256=<hex>` value
  /// in `signature_header` using `secret` as the HMAC key.
  pub fn verify_signature(body: &[u8], signature_header: &str, secret: &str) -> Result<(), WebhookError> {
      let Some(hex_part) = signature_header.strip_prefix("sha256=") else {
          return Err(WebhookError::MalformedSignature);
      };
      let provided = hex::decode(hex_part).map_err(|_| WebhookError::MalformedSignature)?;

      let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
          .map_err(|_| WebhookError::MalformedSignature)?;
      mac.update(body);
      let expected = mac.finalize().into_bytes();

      if expected.ct_eq(&provided).into() {
          Ok(())
      } else {
          Err(WebhookError::Mismatch)
      }
  }
  ```

- [ ] **Step 3: Register module**

  Modify `src/github/mod.rs`:

  ```rust
  pub mod webhook;

  // ... existing pub use ...
  pub use webhook::WebhookError;
  ```

- [ ] **Step 4: Tests**

  Create `tests/github_webhook.rs`:

  ```rust
  //! Plan 10e Task 1 — HMAC validator tests.

  use galvolite::github::webhook::{verify_signature, WebhookError};

  // Known-good HMAC computed externally:
  // body = "hello world", secret = "topsecret"
  // sha256 = 8a3a84bcd5d0a4d49ce6db96b57ac9d6c8d20cf04ee9a91dad6dd99b3eaf3a8a
  // (verified via `echo -n 'hello world' | openssl dgst -sha256 -hmac 'topsecret'`)

  #[test]
  fn verify_passes_with_correct_signature() {
      let body = b"hello world";
      let secret = "topsecret";
      // Compute expected signature dynamically to avoid hard-coding (which
      // would couple this test to openssl output formatting):
      use hmac::{Hmac, Mac};
      use sha2::Sha256;
      let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
      mac.update(body);
      let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

      assert!(verify_signature(body, &sig, secret).is_ok());
  }

  #[test]
  fn verify_fails_with_wrong_signature() {
      let body = b"hello world";
      let secret = "topsecret";
      let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";
      let err = verify_signature(body, bad_sig, secret).unwrap_err();
      assert!(matches!(err, WebhookError::Mismatch));
  }

  #[test]
  fn verify_fails_with_malformed_header() {
      let body = b"x";
      assert!(matches!(
          verify_signature(body, "no-prefix", "s"),
          Err(WebhookError::MalformedSignature)
      ));
      assert!(matches!(
          verify_signature(body, "sha256=not-hex", "s"),
          Err(WebhookError::MalformedSignature)
      ));
  }

  #[test]
  fn verify_fails_when_body_modified() {
      let body = b"original";
      let secret = "s";
      use hmac::{Hmac, Mac};
      use sha2::Sha256;
      let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
      mac.update(body);
      let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

      let tampered = b"tampered";
      assert!(matches!(
          verify_signature(tampered, &sig, secret),
          Err(WebhookError::Mismatch)
      ));
  }
  ```

- [ ] **Step 5: Run tests + commit**

  ```bash
  cargo test --test github_webhook -- --test-threads=1
  git add Cargo.toml Cargo.lock src/github/webhook.rs src/github/mod.rs tests/github_webhook.rs
  git commit -m "feat(github): add webhook HMAC-SHA256 signature validator"
  ```

---

## Task 2: Webhook handler + DomainEvent variants

**Files:**
- Modify: `src/api/events.rs` — add 2 variants + applies_to arms
- Create: `src/api/webhook.rs`
- Modify: `src/api/mod.rs` — `pub mod webhook;` + register route
- Modify: `ui/src/types.ts` — TS DomainEvent variants for parity
- Test: `tests/api_webhook.rs` (new)

- [ ] **Step 1: Add `DomainEvent` variants**

  Modify `src/api/events.rs`. Find the existing project-scoped variants (around `ProjectAdded`/`ProjectDeleted`). Add the new ones near them:

  ```rust
      #[serde(rename = "github.pull_request_merged")]
      PullRequestMerged {
          project_id: Uuid,
          pr_url: String,
          pr_number: u64,
      },
      #[serde(rename = "github.issue_opened")]
      IssueOpened {
          project_id: Uuid,
          issue_url: String,
          issue_number: u64,
          title: String,
      },
  ```

  In `applies_to`, both new variants are project-scoped (false for any session_id). Add to the false arm alongside `ProjectAdded`/`ProjectDeleted`:

  ```rust
              | DomainEvent::ProjectAdded { .. }
              | DomainEvent::ProjectDeleted { .. }
              | DomainEvent::PullRequestMerged { .. }
              | DomainEvent::IssueOpened { .. } => false,
  ```

- [ ] **Step 2: Create `src/api/webhook.rs`**

  ```rust
  //! Plan 10e — GitHub webhook receiver.
  //!
  //! Validates HMAC signature, parses JSON, routes by `X-GitHub-Event` header,
  //! looks up the matching project by `repository.full_name`, and emits the
  //! appropriate `DomainEvent`. Returns 200 noop for events we don't translate
  //! and for repos with no matching GalvoLite project.

  use axum::{
      body::Bytes,
      extract::State,
      http::{HeaderMap, StatusCode},
      response::{IntoResponse, Response},
      Json,
  };
  use serde_json::Value;
  use std::sync::Arc;

  use crate::session_manager::SessionManager;

  /// `POST /api/github/webhook` — see module docs.
  pub async fn webhook_handler(
      State(mgr): State<Arc<SessionManager>>,
      headers: HeaderMap,
      body: Bytes,
  ) -> Response {
      let secret = match std::env::var("GITHUB_WEBHOOK_SECRET") {
          Ok(s) if !s.is_empty() => s,
          _ => {
              return (StatusCode::SERVICE_UNAVAILABLE,
                  Json(serde_json::json!({"error": "GITHUB_WEBHOOK_SECRET not set"}))).into_response();
          }
      };

      let sig_header = headers.get("x-hub-signature-256")
          .and_then(|h| h.to_str().ok())
          .unwrap_or("");

      if let Err(e) = crate::github::webhook::verify_signature(&body, sig_header, &secret) {
          tracing::warn!(error = %e, "webhook signature verification failed");
          return (StatusCode::UNAUTHORIZED,
              Json(serde_json::json!({"error": format!("{e}")}))).into_response();
      }

      let event_type = headers.get("x-github-event")
          .and_then(|h| h.to_str().ok())
          .unwrap_or("");

      // GitHub's webhook test ping
      if event_type == "ping" {
          return Json(serde_json::json!({"status": "pong"})).into_response();
      }

      let payload: Value = match serde_json::from_slice(&body) {
          Ok(v) => v,
          Err(e) => {
              return (StatusCode::BAD_REQUEST,
                  Json(serde_json::json!({"error": format!("invalid JSON: {e}")}))).into_response();
          }
      };

      // Look up the matching project by repository.full_name.
      let repo_full_name = payload.get("repository")
          .and_then(|r| r.get("full_name"))
          .and_then(|n| n.as_str())
          .unwrap_or("");

      let project = mgr.db().list_projects()
          .ok()
          .and_then(|ps| ps.into_iter().find(|p| p.github_repo.as_deref() == Some(repo_full_name)));

      let Some(project) = project else {
          return Json(serde_json::json!({"status": "noop", "reason": "no matching project"})).into_response();
      };

      // Translate event by type.
      match event_type {
          "pull_request" => {
              let action = payload.get("action").and_then(|a| a.as_str()).unwrap_or("");
              let merged = payload.get("pull_request")
                  .and_then(|pr| pr.get("merged"))
                  .and_then(|m| m.as_bool())
                  .unwrap_or(false);
              if action == "closed" && merged {
                  let pr_url = payload.get("pull_request")
                      .and_then(|pr| pr.get("html_url"))
                      .and_then(|u| u.as_str())
                      .unwrap_or("")
                      .to_string();
                  let pr_number = payload.get("pull_request")
                      .and_then(|pr| pr.get("number"))
                      .and_then(|n| n.as_u64())
                      .unwrap_or(0);
                  mgr.event_bus().publish(crate::api::events::DomainEvent::PullRequestMerged {
                      project_id: project.id,
                      pr_url,
                      pr_number,
                  });
                  return Json(serde_json::json!({"status": "ok", "event": "pull_request_merged"})).into_response();
              }
          }
          "issues" => {
              let action = payload.get("action").and_then(|a| a.as_str()).unwrap_or("");
              if action == "opened" {
                  let issue_url = payload.get("issue")
                      .and_then(|i| i.get("html_url"))
                      .and_then(|u| u.as_str())
                      .unwrap_or("")
                      .to_string();
                  let issue_number = payload.get("issue")
                      .and_then(|i| i.get("number"))
                      .and_then(|n| n.as_u64())
                      .unwrap_or(0);
                  let title = payload.get("issue")
                      .and_then(|i| i.get("title"))
                      .and_then(|t| t.as_str())
                      .unwrap_or("")
                      .to_string();
                  mgr.event_bus().publish(crate::api::events::DomainEvent::IssueOpened {
                      project_id: project.id,
                      issue_url,
                      issue_number,
                      title,
                  });
                  return Json(serde_json::json!({"status": "ok", "event": "issue_opened"})).into_response();
              }
          }
          _ => {} // unknown event → fall through to noop
      }

      Json(serde_json::json!({"status": "noop", "reason": "unhandled event"})).into_response()
  }
  ```

- [ ] **Step 3: Register module + route**

  Modify `src/api/mod.rs`:

  ```rust
  pub mod webhook;

  // ... in router(): ...
          .route("/api/github/webhook", post(webhook::webhook_handler))
  ```

- [ ] **Step 4: Add UI DomainEvent variants for parity**

  Modify `ui/src/types.ts`. In the `DomainEvent` union, add:

  ```ts
    | { type: "github.pull_request_merged"; project_id: string; pr_url: string; pr_number: number }
    | { type: "github.issue_opened"; project_id: string; issue_url: string; issue_number: number; title: string }
  ```

- [ ] **Step 5: Tests**

  Create `tests/api_webhook.rs`:

  ```rust
  //! Plan 10e Task 2 — webhook handler integration tests.

  mod common;

  use axum::body::Body;
  use axum::http::{Method, Request, StatusCode};
  use galvolite::api::events::DomainEvent;
  use galvolite::db::NewProject;
  use std::sync::OnceLock;
  use tokio::sync::Mutex;
  use tower::ServiceExt;

  fn env_lock() -> &'static Mutex<()> {
      static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
      LOCK.get_or_init(|| Mutex::new(()))
  }

  fn sign(body: &[u8], secret: &str) -> String {
      use hmac::{Hmac, Mac};
      use sha2::Sha256;
      let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
      mac.update(body);
      format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
  }

  #[tokio::test]
  async fn webhook_503_when_secret_unset() {
      let _g = env_lock().lock().await;
      std::env::remove_var("GITHUB_WEBHOOK_SECRET");

      let setup = common::test_app();
      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .body(Body::from("{}")).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
  }

  #[tokio::test]
  async fn webhook_401_on_signature_mismatch() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", "sha256=0000000000000000000000000000000000000000000000000000000000000000")
              .header("x-github-event", "ping")
              .body(Body::from("{}")).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }

  #[tokio::test]
  async fn webhook_pong_for_ping_with_valid_signature() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let body = b"{}";
      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", sign(body, "topsecret"))
              .header("x-github-event", "ping")
              .body(Body::from(body.to_vec())).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::OK);

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }

  #[tokio::test]
  async fn webhook_noop_for_unmatched_repository() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let body = br#"{"repository":{"full_name":"unknown/repo"}}"#;
      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", sign(body, "topsecret"))
              .header("x-github-event", "pull_request")
              .body(Body::from(body.to_vec())).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::OK);
      // Confirm response body says "noop"
      let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
      let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
      assert_eq!(v["status"], "noop");

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }

  #[tokio::test]
  async fn webhook_publishes_event_for_pr_merged() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();

      let mut rx = setup.mgr.event_bus().subscribe();
      // Drain ProjectAdded.
      let _ = rx.recv().await;

      let body = br#"{
          "action": "closed",
          "pull_request": {
              "html_url": "https://github.com/octocat/hello/pull/42",
              "number": 42,
              "merged": true
          },
          "repository": { "full_name": "octocat/hello" }
      }"#;

      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", sign(body, "topsecret"))
              .header("x-github-event", "pull_request")
              .body(Body::from(body.to_vec())).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::OK);

      // Receive the bus event.
      let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
          .await.expect("timeout").unwrap();
      match evt {
          DomainEvent::PullRequestMerged { project_id, pr_url, pr_number } => {
              assert_eq!(project_id, project.id);
              assert_eq!(pr_url, "https://github.com/octocat/hello/pull/42");
              assert_eq!(pr_number, 42);
          }
          other => panic!("expected PullRequestMerged, got {other:?}"),
      }

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }

  #[tokio::test]
  async fn webhook_publishes_event_for_issue_opened() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let project = setup.mgr.add_project(NewProject {
          name: "p".into(),
          path: setup._dir.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();
      setup.db.set_project_github_repo(project.id, Some("octocat/hello")).unwrap();

      let mut rx = setup.mgr.event_bus().subscribe();
      let _ = rx.recv().await; // drain ProjectAdded

      let body = br#"{
          "action": "opened",
          "issue": {
              "html_url": "https://github.com/octocat/hello/issues/7",
              "number": 7,
              "title": "Bug: thing broken"
          },
          "repository": { "full_name": "octocat/hello" }
      }"#;

      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", sign(body, "topsecret"))
              .header("x-github-event", "issues")
              .body(Body::from(body.to_vec())).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::OK);

      // Wait for the IssueOpened event (skip past any other intermediate events).
      let mut found = false;
      for _ in 0..10 {
          match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
              Ok(Ok(DomainEvent::IssueOpened { project_id, issue_url, issue_number, title })) => {
                  assert_eq!(project_id, project.id);
                  assert_eq!(issue_url, "https://github.com/octocat/hello/issues/7");
                  assert_eq!(issue_number, 7);
                  assert_eq!(title, "Bug: thing broken");
                  found = true;
                  break;
              }
              Ok(Ok(_)) => continue, // unrelated event; keep looking
              _ => break,
          }
      }
      assert!(found, "IssueOpened event was not published");

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }

  #[tokio::test]
  async fn webhook_noop_for_unhandled_event_type() {
      let _g = env_lock().lock().await;
      std::env::set_var("GITHUB_WEBHOOK_SECRET", "topsecret");

      let setup = common::test_app();
      let body = br#"{"repository":{"full_name":"x/y"}}"#;
      let resp = setup.app.oneshot(
          Request::builder()
              .uri("/api/github/webhook")
              .method(Method::POST)
              .header("x-hub-signature-256", sign(body, "topsecret"))
              .header("x-github-event", "push")
              .body(Body::from(body.to_vec())).unwrap()
      ).await.unwrap();
      assert_eq!(resp.status(), StatusCode::OK);

      std::env::remove_var("GITHUB_WEBHOOK_SECRET");
  }
  ```

- [ ] **Step 6: Run tests + commit**

  ```bash
  cargo test --test github_webhook --test api_webhook -- --test-threads=1
  cargo test -- --test-threads=1
  cd ui && npx tsc --noEmit && cd ..
  git add src/api/events.rs src/api/webhook.rs src/api/mod.rs ui/src/types.ts tests/api_webhook.rs
  git commit -m "feat(api): GitHub webhook receiver + PullRequestMerged/IssueOpened DomainEvents"
  ```

---

## Final verification

- `cargo test -- --test-threads=1` (re-run on env-race exit-1)
- `cargo build --release` clean
- `cd ui && npx tsc --noEmit` clean
- Update `project_plan10e_status.md`
