# Plan 10e — Webhook Ingest → DomainEvents Design

**Date:** 2026-05-09
**Status:** Auto-approved (operator delegated autonomous execution)
**Scope:** Single PLAN.md. Final sub-plan of 10a-10e.

## Goal

Accept GitHub webhook deliveries on a new endpoint, validate the HMAC-SHA256 signature against a configured secret, and translate two event types into bus DomainEvents that future subscribers can react to. No automatic side-effects in v1 — emit events, let future work consume them.

## Architecture

| File | Role |
|---|---|
| `Cargo.toml` | Add `hmac = "0.12"` dependency |
| `src/github/webhook.rs` (new) | HMAC-SHA256 validator: `verify_signature(body: &[u8], signature_header: &str, secret: &str) -> Result<(), WebhookError>`. Constant-time compare via `subtle`. |
| `src/github/mod.rs` | `pub mod webhook;` + re-export `WebhookError` |
| `src/api/events.rs` | Two new `DomainEvent` variants: `PullRequestMerged { project_id, pr_url, pr_number }`, `IssueOpened { project_id, issue_url, issue_number, title }`. Both project-scoped (false in `applies_to`). |
| `src/api/webhook.rs` (new) | `webhook_handler` for `POST /api/github/webhook`. Reads raw body via axum's `Bytes` extractor, validates signature against `GITHUB_WEBHOOK_SECRET` env var, parses JSON, routes by `X-GitHub-Event` header. Translates `pull_request` (action=closed AND merged=true) and `issues` (action=opened); emits bus events. Unknown events / no matching project → 200 noop. |
| `src/api/mod.rs` | `pub mod webhook;` + register route |
| `ui/src/types.ts` | Add two new `DomainEvent` variants for parity |

**Locked design calls:**
- HMAC-SHA256 validation against `GITHUB_WEBHOOK_SECRET` env var (returns 401 on missing/invalid signature; 503 if env var unset)
- Constant-time signature compare via `subtle::ConstantTimeEq` (already in deps)
- `repository.full_name` from payload → match against `projects.github_repo`. No matching project → 200 noop (legitimate; webhook may fire for repos not linked to any GalvoLite project)
- Per-project routing: only emit DomainEvent if a matching project exists
- No persistence of webhook deliveries — bus events are the only effect

## Error matrix

| Trigger | Response |
|---|---|
| `GITHUB_WEBHOOK_SECRET` unset | 503 `{"error": "GITHUB_WEBHOOK_SECRET not set"}` |
| Missing or malformed `X-Hub-Signature-256` header | 401 `{"error": "missing or malformed signature"}` |
| Signature mismatch | 401 `{"error": "signature mismatch"}` |
| Body not valid JSON | 400 |
| Unknown `X-GitHub-Event` (anything other than `pull_request`/`issues`/`ping`) | 200 `{"status": "noop", "reason": "unhandled event"}` |
| No matching project for repository | 200 `{"status": "noop", "reason": "no matching project"}` |
| `pull_request` action != closed OR not merged | 200 noop |
| `issues` action != opened | 200 noop |
| `ping` event | 200 `{"status": "pong"}` (GitHub's webhook test ping) |

## Testing

- `tests/github_webhook.rs` (new) — HMAC validator unit tests: known-good signature passes, known-bad fails, malformed header fails, missing header fails
- `tests/api_webhook.rs` (new) — handler integration tests:
  - 503 when `GITHUB_WEBHOOK_SECRET` unset
  - 401 on signature mismatch
  - 200 + bus event published for valid `pull_request` merged payload
  - 200 + bus event published for valid `issues` opened payload
  - 200 noop for unknown event type
  - 200 noop for unmatched repository
  - 200 pong for ping event

## Out of scope
- Webhook event persistence
- Auto-close issue when matching session ships PR (defer to 10e.x or future work)
- Comment-back to issue on session-state changes
- More event types (push, pr review submitted, etc.) — easy to add later
- Dashboard UI for webhook config / event history
