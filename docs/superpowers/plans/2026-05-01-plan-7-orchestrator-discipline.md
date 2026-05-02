# Sub-plan 7: Orchestrator Discipline + Memory Augmentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate primary-session drift by making every executor turn advance a state machine and every orchestrator decision flow from a structured directive — backed by two new flat-file artifacts (`directive.md`, postmortems, decisions) and revised CLAUDE.md profiles that forbid prose-only turns.

**Architecture:** Five pillars. (1) A new `.galvolite/directive.md` artifact carries the orchestration contract (phase DAG + autonomy + escalation rules); parser + REST endpoints + filesystem watcher mirror the existing plan.md handling. (2) Revised CLAUDE.md profiles (`IMPL`, `FIX`, `ORCH`, `BRAINSTORM`) require a state-machine-advancing tool call to end every turn and forbid orchestrator decision-making. (3) A new `write_postmortem` MCP tool lets terminal sessions emit `.galvolite/postmortems/<plan>-<part>.md`. (4) The existing `OperatorPromptRegistry::answer` path auto-writes `.galvolite/decisions/<sid>-<ts>.md` when an `ask_operator` answer lands. (5) `SessionManager::spawn_child` injects recent postmortems + decisions for the same plan part into the child's injection prompt.

**Tech Stack:** Rust 1.75+, axum 0.7, rusqlite, tokio, notify (already in tree), uuid, serde, serde_json, chrono. React 18 + Vitest for the UI delta.

**Spec:** [`docs/superpowers/specs/2026-05-01-orchestrator-discipline.md`](../specs/2026-05-01-orchestrator-discipline.md). Read it first.

**Conventions:**
- The user has authorized direct main-branch commits for this plan, matching Plans 4-6.
- TDD: red → green → commit per task. Run `cargo test` (and `cd ui && pnpm test`) before each commit.
- Backend changes commit separately from frontend even when topically related.
- Per memory `feedback_idle_sessions_no_cost.md`: do not add wall-clock polling. Watcher events drive the dashboard.
- Per memory `feedback_gui_parity.md`: every backend endpoint added here must surface in the dashboard before the plan closes.

---

## File Structure

**New files:**

| Path | Responsibility |
|---|---|
| `src/directive.rs` | `Directive` struct, markdown parser, atomic disk writer |
| `src/postmortem.rs` | `Postmortem` struct, frontmatter parser, list/load/glob helpers |
| `src/decisions.rs` | `Decision` struct + `record_decision()` writer; called from `OperatorPromptRegistry::answer` |
| `src/api/directive.rs` | `GET` / `PUT /api/projects/:id/directive` |
| `src/api/postmortems.rs` | `GET /api/projects/:id/postmortems` and `/decisions` |
| `src/mcp/tools/postmortem.rs` | `write_postmortem` MCP tool |
| `tests/directive_parser.rs` | Unit tests for directive markdown parsing |
| `tests/directive_api.rs` | Integration: directive REST endpoints |
| `tests/directive_watcher.rs` | Integration: notify-crate emits `DirectiveChanged` |
| `tests/postmortem_writer.rs` | Integration: `write_postmortem` MCP round-trip |
| `tests/decisions_autorecord.rs` | Integration: `OperatorPromptRegistry::answer` writes decision file |
| `tests/spawn_context_injection.rs` | Integration: spawned child sees postmortems + decisions in prompt |
| `tests/profiles_turn_discipline.rs` | Regression: profile strings contain required clauses |
| `ui/src/components/DirectiveTab.tsx` | Markdown viewer + edit toggle for directive |
| `ui/src/components/PostmortemsPanel.tsx` | List view of postmortems + decisions |
| `ui/src/components/DirectiveTab.test.tsx` | Vitest |
| `ui/src/components/PostmortemsPanel.test.tsx` | Vitest |

**Extended files:**

| Path | Change |
|---|---|
| `src/lib.rs` | Add `mod directive; mod postmortem; mod decisions;` |
| `src/api/mod.rs` | Wire 4 new routes (directive GET/PUT, postmortems list, decisions list) |
| `src/api/operator.rs` | `OperatorPromptRegistry::answer` calls `decisions::record_decision()` |
| `src/api/events.rs` | Add `DirectiveChanged`, `PostmortemWritten`, `DecisionRecorded` variants |
| `src/session_manager.rs` | `spawn_child` reads postmortems + decisions, prepends to injection prompt |
| `src/claude_md.rs` | Profile body diffs (turn-discipline + router rules) |
| `src/mcp/tools/mod.rs` | Register `postmortem::tools()` in `all()` |
| `src/state.rs` | Recognize `mcp.write_postmortem` event kind |
| `src/main.rs` (or wherever `notify::Watcher` is set up) | Watch `<project>/.galvolite/directive.md` per project |
| `ui/src/types.ts` | `Directive`, `Postmortem`, `Decision` types; new `DomainEvent` variants |
| `ui/src/api.ts` | `getDirective`, `putDirective`, `listPostmortems`, `listDecisions` |
| `ui/src/components/Dashboard.tsx` | Mount `DirectiveTab` + `PostmortemsPanel` per project |
| `ui/src/components/SpawnSessionsButton.tsx` (or wherever) | Disabled until both `plan.md` AND `directive.md` exist |

---

## Task 1: Directive parser (`src/directive.rs`)

**Files:**
- Create: `src/directive.rs`
- Modify: `src/lib.rs` (add `pub mod directive;`)
- Test: `tests/directive_parser.rs` (new)

The directive format from the design memo:

```markdown
# Directive: <name>

Plan: ./plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
|----|-------|----------|------------|
| P1 | 1     | full     | —          |
| P2 | 2, 3  | review   | P1         |

## Escalation rules

- On schema/migration change → ask_operator
- On executor stuck >3 cycles → notify_operator(level=warn)

## Notes

Free-form context.
```

Parser yields:

```rust
pub struct Directive {
    pub name: String,
    pub plan_path: PathBuf,
    pub phases: Vec<Phase>,
    pub escalation_rules: Vec<EscalationRule>,
    pub notes: String,
    pub raw_markdown: String,
}

pub struct Phase {
    pub id: String,           // "P1"
    pub parts: Vec<u32>,      // [2, 3]
    pub autonomy: Autonomy,   // Full | Review | Manual
    pub depends_on: Vec<String>, // ["P1"]
}

pub enum Autonomy { Full, Review, Manual }

pub struct EscalationRule {
    pub trigger: String,  // free-text after "On "
    pub action: String,   // free-text after "→ "
}
```

- [ ] **Step 1: Write the failing tests**

```rust
// tests/directive_parser.rs
use galvolite::directive::{parse, Autonomy};

const SAMPLE: &str = r#"# Directive: Sub-plan 7

Plan: ./plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
|----|-------|----------|------------|
| P1 | 1     | full     | —          |
| P2 | 2, 3  | review   | P1         |
| P3 | 4     | manual   | P1, P2     |

## Escalation rules

- On schema change → ask_operator
- On executor stuck >3 cycles → notify_operator(level=warn)

## Notes

Free-form context for the orchestrator.
"#;

#[test]
fn parses_full_directive() {
    let d = parse(SAMPLE).unwrap();
    assert_eq!(d.name, "Sub-plan 7");
    assert_eq!(d.plan_path.to_str().unwrap(), "./plan.md");

    assert_eq!(d.phases.len(), 3);
    assert_eq!(d.phases[0].id, "P1");
    assert_eq!(d.phases[0].parts, vec![1]);
    assert!(matches!(d.phases[0].autonomy, Autonomy::Full));
    assert!(d.phases[0].depends_on.is_empty());

    assert_eq!(d.phases[1].parts, vec![2, 3]);
    assert!(matches!(d.phases[1].autonomy, Autonomy::Review));
    assert_eq!(d.phases[1].depends_on, vec!["P1"]);

    assert_eq!(d.phases[2].depends_on, vec!["P1", "P2"]);

    assert_eq!(d.escalation_rules.len(), 2);
    assert!(d.escalation_rules[0].trigger.contains("schema change"));
    assert!(d.escalation_rules[0].action.contains("ask_operator"));

    assert!(d.notes.starts_with("Free-form"));
}

#[test]
fn rejects_circular_dependencies() {
    let bad = "# D\n\nPlan: p\n\n## Phases\n\n| ID | Parts | Autonomy | Depends on |\n|----|-------|----------|------------|\n| P1 | 1     | full     | P2         |\n| P2 | 2     | full     | P1         |\n";
    assert!(parse(bad).is_err());
}

#[test]
fn rejects_unknown_autonomy() {
    let bad = SAMPLE.replace("full", "yolo");
    assert!(parse(&bad).is_err());
}

#[test]
fn rejects_dependency_on_undefined_phase() {
    let bad = SAMPLE.replace("P1, P2", "P1, P9");
    assert!(parse(&bad).is_err());
}
```

- [ ] **Step 2: Run tests, verify they fail**

```bash
cargo test --test directive_parser
```
Expected: 4 failures, "module not found" for `galvolite::directive`.

- [ ] **Step 3: Implement `src/directive.rs`**

```rust
//! Directive parser. Source of truth for orchestrator routing.

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Directive {
    pub name: String,
    pub plan_path: PathBuf,
    pub phases: Vec<Phase>,
    pub escalation_rules: Vec<EscalationRule>,
    pub notes: String,
    pub raw_markdown: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Phase {
    pub id: String,
    pub parts: Vec<u32>,
    pub autonomy: Autonomy,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Autonomy { Full, Review, Manual }

#[derive(Debug, Clone, serde::Serialize)]
pub struct EscalationRule {
    pub trigger: String,
    pub action: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing required section: {0}")]
    MissingSection(&'static str),
    #[error("malformed phase row: {0}")]
    BadPhaseRow(String),
    #[error("unknown autonomy level: {0} (expected: full | review | manual)")]
    BadAutonomy(String),
    #[error("phase {phase} depends on undefined phase {dep}")]
    UndefinedDep { phase: String, dep: String },
    #[error("circular dependency detected involving {0}")]
    CircularDep(String),
}

pub fn parse(s: &str) -> Result<Directive, ParseError> {
    // 1. Title from first `# Directive: <name>` line.
    let name = extract_title(s).ok_or(ParseError::MissingSection("# Directive: <name>"))?;
    // 2. Plan path from `Plan: <path>` line.
    let plan_path = extract_plan_path(s).ok_or(ParseError::MissingSection("Plan:"))?;
    // 3. Phases from `## Phases` section's markdown table.
    let phases = parse_phases(s)?;
    // 4. Escalation rules from `## Escalation rules` bullets.
    let escalation_rules = parse_escalation(s);
    // 5. Notes — anything after `## Notes`.
    let notes = extract_section(s, "## Notes").unwrap_or_default();

    let d = Directive {
        name, plan_path, phases, escalation_rules,
        notes, raw_markdown: s.to_string(),
    };
    validate_deps(&d)?;
    Ok(d)
}

// Helpers below: extract_title, extract_plan_path, parse_phases (parses the
// markdown table line by line, splitting on `|`), parse_escalation (bullet
// lines under the heading), extract_section, validate_deps (DFS for cycles +
// undefined-dep check). Keep them ~30 LOC each; standard string slicing,
// no markdown-parser crate needed.
```

- [ ] **Step 4: Run tests, verify pass**

```bash
cargo test --test directive_parser
```
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/directive.rs src/lib.rs tests/directive_parser.rs
git commit -m "feat(directive): markdown parser for .galvolite/directive.md"
```

---

## Task 2: Atomic directive writer + path helpers

**Files:**
- Modify: `src/directive.rs`
- Test: extend `tests/directive_parser.rs`

Add `directive_path(project: &Project) -> PathBuf` and `write_atomic(path, contents)`. Atomic = write to `path.tmp` then `rename`. Mirror the pattern in `src/handoff.rs`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn write_atomic_round_trips() {
    use galvolite::directive::{parse, write_atomic};
    let tmp = tempfile::TempDir::new().unwrap();
    let p = tmp.path().join("directive.md");
    write_atomic(&p, SAMPLE).unwrap();
    let read_back = std::fs::read_to_string(&p).unwrap();
    let d = parse(&read_back).unwrap();
    assert_eq!(d.name, "Sub-plan 7");
}

#[test]
fn write_atomic_does_not_leave_tmp_file_on_success() {
    use galvolite::directive::write_atomic;
    let tmp = tempfile::TempDir::new().unwrap();
    let p = tmp.path().join("directive.md");
    write_atomic(&p, SAMPLE).unwrap();
    let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "expected only directive.md, got {entries:?}");
}
```

- [ ] **Step 2-4: Implement → run → verify**

```rust
pub fn directive_path(project_path: &std::path::Path) -> PathBuf {
    project_path.join(".galvolite").join("directive.md")
}

pub fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(directive): atomic writer + path helpers"
```

---

## Task 3: REST endpoints — `GET` / `PUT /api/projects/:id/directive`

**Files:**
- Create: `src/api/directive.rs`
- Modify: `src/api/mod.rs` (route wiring)
- Modify: `src/api/events.rs` (add `DirectiveChanged` variant)
- Test: `tests/directive_api.rs` (new)

Pattern: mirror existing `src/api/spawn.rs` + the plan-file endpoints (find them with `grep -n "plan.md\|plan_md" src/api/`).

`GET` returns parsed `Directive` JSON or `404` if file missing.
`PUT` accepts `{ markdown: String }`, parses to validate, writes atomically, publishes `DirectiveChanged` domain event. `400` on parse error with the error message in the body.

- [ ] **Step 1: Write integration test scaffold**

```rust
// tests/directive_api.rs
// (use the existing integration-test harness — copy structure from
// tests/spawn_e2e.rs or tests/operator_prompt_endpoint.rs)

#[tokio::test]
async fn put_then_get_directive() {
    let h = TestHarness::new().await;
    let project = h.create_project_at_tempdir().await;

    let resp = h.client
        .put(format!("/api/projects/{}/directive", project.id))
        .json(&json!({"markdown": SAMPLE_DIRECTIVE}))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = h.client
        .get(format!("/api/projects/{}/directive", project.id))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(body["name"], "Sub-plan 7");
    assert_eq!(body["phases"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn put_directive_with_invalid_markdown_returns_400() {
    let h = TestHarness::new().await;
    let project = h.create_project_at_tempdir().await;
    let resp = h.client.put(format!("/api/projects/{}/directive", project.id))
        .json(&json!({"markdown": "not a directive"}))
        .send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn get_missing_directive_returns_404() {
    let h = TestHarness::new().await;
    let project = h.create_project_at_tempdir().await;
    let resp = h.client.get(format!("/api/projects/{}/directive", project.id))
        .send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn put_directive_publishes_directive_changed_event() {
    let h = TestHarness::new().await;
    let project = h.create_project_at_tempdir().await;
    let mut events = h.subscribe_events().await;
    h.put_directive(project.id, SAMPLE_DIRECTIVE).await;
    let ev = events.recv().await.unwrap();
    assert_matches!(ev, DomainEvent::DirectiveChanged { project_id, .. } if project_id == project.id);
}
```

- [ ] **Step 2: Run, verify fails (route not wired)**

```bash
cargo test --test directive_api
```
Expected: 4 failures (404 / 405 / no route).

- [ ] **Step 3: Implement `src/api/directive.rs`**

```rust
use axum::{extract::{Path, State}, http::StatusCode, Json};
use uuid::Uuid;
use crate::api::events::DomainEvent;
use crate::directive::{self, Directive};
use crate::AppState;

pub async fn get_directive(
    State(s): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Directive>, StatusCode> {
    let project = s.db.get_project(project_id).map_err(|_| StatusCode::NOT_FOUND)?;
    let path = directive::directive_path(std::path::Path::new(&project.path));
    let content = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let d = directive::parse(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(d))
}

#[derive(serde::Deserialize)]
pub struct PutBody { pub markdown: String }

pub async fn put_directive(
    State(s): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(body): Json<PutBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let project = s.db.get_project(project_id)
        .map_err(|_| (StatusCode::NOT_FOUND, "project not found".into()))?;
    // Validate first; reject before writing.
    directive::parse(&body.markdown)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let path = directive::directive_path(std::path::Path::new(&project.path));
    directive::write_atomic(&path, &body.markdown)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    s.event_bus.publish(DomainEvent::DirectiveChanged { project_id });
    Ok(StatusCode::OK)
}
```

- [ ] **Step 4: Wire routes in `src/api/mod.rs`**

```rust
.route("/api/projects/:id/directive",
       axum::routing::get(directive::get_directive)
       .put(directive::put_directive))
```

- [ ] **Step 5: Add `DirectiveChanged` to `src/api/events.rs::DomainEvent`**

```rust
DirectiveChanged { project_id: Uuid },
```

- [ ] **Step 6: Run, verify pass**

```bash
cargo test --test directive_api
```

- [ ] **Step 7: Commit**

```bash
git commit -am "feat(api): GET/PUT /api/projects/:id/directive"
```

---

## Task 4: Filesystem watcher emits `DirectiveChanged` on out-of-band edits

**Files:**
- Modify: wherever the `notify` watcher is initialized (search: `grep -rn "notify::\|RecommendedWatcher\|new_immediate" src/`)
- Test: `tests/directive_watcher.rs` (new)

Goal: a human edits `.galvolite/directive.md` directly (or git pull lands an update), and the dashboard sees it without a manual refresh.

- [ ] **Step 1: Locate existing watcher setup**

```bash
grep -rn "notify::Watcher\|RecommendedWatcher\|notify::recommended" src/
```

If a watcher already exists for `plan.md`, extend the path-match arm. If not, the plan.md handling may be polling on demand — in that case, add a project-scoped watcher that registers `<project.path>/.galvolite/` on project-load, and emits `DirectiveChanged` on writes to `directive.md`.

- [ ] **Step 2: Failing test**

```rust
// tests/directive_watcher.rs
#[tokio::test]
async fn external_edit_emits_directive_changed() {
    let h = TestHarness::new().await;
    let project = h.create_project_at_tempdir().await;
    let mut events = h.subscribe_events().await;

    // Write the file directly via the filesystem (not via the API).
    let path = std::path::Path::new(&project.path)
        .join(".galvolite").join("directive.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, SAMPLE_DIRECTIVE).unwrap();

    // Watcher should debounce + emit within ~500ms.
    let ev = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        events.recv()
    ).await.expect("watcher did not fire").unwrap();
    assert_matches!(ev, DomainEvent::DirectiveChanged { project_id, .. } if project_id == project.id);
}
```

- [ ] **Step 3: Implement watcher arm**

Pattern (depending on existing setup):

```rust
// Inside the watcher event handler, after debounce:
match event_path.file_name().and_then(|s| s.to_str()) {
    Some("directive.md") if event_path.parent().and_then(|p| p.file_name())
                                       .and_then(|s| s.to_str()) == Some(".galvolite") => {
        // Resolve project_id from path prefix → publish.
        if let Some(project_id) = resolve_project_for_path(&event_path, &registry) {
            event_bus.publish(DomainEvent::DirectiveChanged { project_id });
        }
    }
    Some("plan.md") => { /* existing arm */ }
    _ => {}
}
```

- [ ] **Step 4: Run, verify pass**

```bash
cargo test --test directive_watcher -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(watcher): emit DirectiveChanged on out-of-band directive edits"
```

---

## Task 5: Frontend types + API client

**Files:**
- Modify: `ui/src/types.ts`
- Modify: `ui/src/api.ts`
- Test: `ui/src/api.test.ts` (extend or create)

- [ ] **Step 1: Add types**

```ts
// ui/src/types.ts
export type Autonomy = 'full' | 'review' | 'manual';

export interface Phase {
  id: string;
  parts: number[];
  autonomy: Autonomy;
  depends_on: string[];
}

export interface EscalationRule {
  trigger: string;
  action: string;
}

export interface Directive {
  name: string;
  plan_path: string;
  phases: Phase[];
  escalation_rules: EscalationRule[];
  notes: string;
  raw_markdown: string;
}

// Extend DomainEvent union:
export type DomainEvent =
  | ...existing variants...
  | { kind: 'directive.changed'; project_id: string };
```

- [ ] **Step 2: Add API client methods**

```ts
// ui/src/api.ts
export async function getDirective(projectId: string): Promise<Directive | null> {
  const r = await fetch(`/api/projects/${projectId}/directive`);
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`getDirective: ${r.status}`);
  return r.json();
}

export async function putDirective(projectId: string, markdown: string): Promise<void> {
  const r = await fetch(`/api/projects/${projectId}/directive`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ markdown }),
  });
  if (!r.ok) {
    const msg = await r.text();
    throw new Error(`putDirective: ${r.status} ${msg}`);
  }
}
```

- [ ] **Step 3: Test (existing harness)**

```ts
test('getDirective parses 404 as null', async () => {
  fetchMock.mockResponseOnce('', { status: 404 });
  expect(await getDirective('p1')).toBeNull();
});

test('putDirective surfaces 400 message', async () => {
  fetchMock.mockResponseOnce('bad markdown', { status: 400 });
  await expect(putDirective('p1', 'x')).rejects.toThrow('400 bad markdown');
});
```

- [ ] **Step 4: Run, commit**

```bash
cd ui && pnpm test src/api.test.ts && cd ..
git commit -am "feat(ui): types + api client for directive"
```

---

## Task 6: `DirectiveTab` component

**Files:**
- Create: `ui/src/components/DirectiveTab.tsx`
- Create: `ui/src/components/DirectiveTab.test.tsx`
- Modify: `ui/src/components/Dashboard.tsx` (mount tab)

UX: read-only markdown viewer by default. "Edit" button toggles a `<textarea>` with Save / Cancel. On Save, calls `putDirective`; on 400, shows the parse error inline. Subscribes to `directive.changed` events to refresh content live.

- [ ] **Step 1: Failing test**

```tsx
// ui/src/components/DirectiveTab.test.tsx
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { DirectiveTab } from './DirectiveTab';

test('renders directive name + phases', async () => {
  vi.mocked(getDirective).mockResolvedValue({
    name: 'Test', plan_path: './plan.md',
    phases: [{ id: 'P1', parts: [1], autonomy: 'full', depends_on: [] }],
    escalation_rules: [], notes: '', raw_markdown: '...',
  });
  render(<DirectiveTab projectId="p1" />);
  await waitFor(() => expect(screen.getByText(/Test/)).toBeInTheDocument());
  expect(screen.getByText('P1')).toBeInTheDocument();
});

test('shows "no directive" empty state on 404', async () => {
  vi.mocked(getDirective).mockResolvedValue(null);
  render(<DirectiveTab projectId="p1" />);
  await waitFor(() => expect(screen.getByText(/no directive yet/i)).toBeInTheDocument());
});

test('Save shows inline error on 400', async () => {
  vi.mocked(getDirective).mockResolvedValue(null);
  vi.mocked(putDirective).mockRejectedValue(new Error('400 missing required section'));
  render(<DirectiveTab projectId="p1" />);
  fireEvent.click(await screen.findByText(/edit/i));
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'broken' } });
  fireEvent.click(screen.getByText(/save/i));
  await waitFor(() => expect(screen.getByText(/missing required section/i)).toBeInTheDocument());
});
```

- [ ] **Step 2: Implement component**

```tsx
import { useEffect, useState } from 'react';
import { getDirective, putDirective } from '../api';
import type { Directive } from '../types';
import { useDomainEvents } from '../hooks/useDomainEvents';

export function DirectiveTab({ projectId }: { projectId: string }) {
  const [d, setD] = useState<Directive | null | 'loading'>('loading');
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string | null>(null);

  async function load() {
    setD(await getDirective(projectId));
  }

  useEffect(() => { load(); }, [projectId]);
  useDomainEvents(ev => {
    if (ev.kind === 'directive.changed' && ev.project_id === projectId) load();
  });

  async function save() {
    setError(null);
    try {
      await putDirective(projectId, draft);
      setEditing(false);
      // load() will be triggered by directive.changed event.
    } catch (e: any) {
      setError(e.message);
    }
  }

  if (d === 'loading') return <div>Loading directive...</div>;
  if (!d && !editing) return (
    <div>
      <p>No directive yet. Brainstorm should produce one.</p>
      <button onClick={() => { setDraft(''); setEditing(true); }}>Create</button>
    </div>
  );

  if (editing) return (
    <div>
      <textarea value={draft} onChange={e => setDraft(e.target.value)} rows={20} cols={80} />
      <div>
        <button onClick={save}>Save</button>
        <button onClick={() => { setEditing(false); setError(null); }}>Cancel</button>
      </div>
      {error && <pre className="error">{error}</pre>}
    </div>
  );

  return (
    <div>
      <h2>{d!.name}</h2>
      <table>
        <thead><tr><th>ID</th><th>Parts</th><th>Autonomy</th><th>Depends on</th></tr></thead>
        <tbody>
          {d!.phases.map(p => (
            <tr key={p.id}>
              <td>{p.id}</td>
              <td>{p.parts.join(', ')}</td>
              <td>{p.autonomy}</td>
              <td>{p.depends_on.join(', ') || '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Escalation rules</h3>
      <ul>{d!.escalation_rules.map((r, i) => <li key={i}>On {r.trigger} → {r.action}</li>)}</ul>
      {d!.notes && <><h3>Notes</h3><pre>{d!.notes}</pre></>}
      <button onClick={() => { setDraft(d!.raw_markdown); setEditing(true); }}>Edit</button>
    </div>
  );
}
```

- [ ] **Step 3: Mount in `Dashboard.tsx`** as a tab next to the existing plan view (find with `grep -n "plan" ui/src/components/Dashboard.tsx`).

- [ ] **Step 4: Run, commit**

```bash
cd ui && pnpm test src/components/DirectiveTab.test.tsx && cd ..
git commit -am "feat(ui): DirectiveTab component + Dashboard mount"
```

---

## Task 7: `IMPL_PROFILE` + `FIX_PROFILE` turn-discipline clause

**Files:**
- Modify: `src/claude_md.rs:29-82` (IMPL_PROFILE) and `src/claude_md.rs:204-252` (FIX_PROFILE)
- Test: `tests/profiles_turn_discipline.rs` (new)

- [ ] **Step 1: Failing regression test**

```rust
// tests/profiles_turn_discipline.rs
use galvolite::claude_md::{implementation_profile, fix_profile, orchestrator_profile, brainstorm_profile};

#[test]
fn impl_profile_requires_turn_discipline() {
    let p = implementation_profile();
    assert!(p.contains("Turn Discipline"), "missing 'Turn Discipline' heading");
    assert!(p.contains("Never end a turn with prose only"));
    assert!(p.contains("plan_check_step"));
    assert!(p.contains("status_report"));
    assert!(p.contains("ask_operator"));
}

#[test]
fn fix_profile_requires_turn_discipline() {
    let p = fix_profile();
    assert!(p.contains("Turn Discipline"));
    assert!(p.contains("Never end a turn with prose only"));
}

#[test]
fn orch_profile_forbids_decisions() {
    let p = orchestrator_profile();
    assert!(p.contains("You do not decide"));
    assert!(p.contains("ask_operator"));
    assert!(p.contains("explicitly forbidden"));
}

#[test]
fn brainstorm_profile_emits_directive() {
    let p = brainstorm_profile();
    assert!(p.contains("directive.md"));
    assert!(p.contains("plan.md"));
}
```

- [ ] **Step 2: Run, verify fails**

```bash
cargo test --test profiles_turn_discipline
```
Expected: 4 failures.

- [ ] **Step 3: Edit `IMPL_PROFILE` constant — append after the "## Scope Rules" section:**

```markdown
## Turn Discipline

Every turn must end by calling exactly ONE of:

- `plan_check_step({...})` — you completed a plan step
- `status_report({kind: 'working', note: '<what next>'})` — heartbeat with the
  *next* step you're about to take, stated explicitly
- `ask_operator({...})` — the directive does not cover this decision
- `request_attention({...})` — blocked, need human
- `write_handoff({...})` then `status_report({kind: 'complete'})` — done

**Never end a turn with prose only.** If you find yourself about to write
"Now I'll think about X..." or "Let me consider Y..." — stop and call
`status_report` with that thought as the note. The next turn picks it up
cleanly with a fresh head.

This rule eliminates the "drift between two thoughts in the same turn"
failure mode. The harness has every tool you need; the rule just forces
their use.
```

- [ ] **Step 4: Edit `FIX_PROFILE` constant — same Turn Discipline section appended after "## Pattern".**

- [ ] **Step 5: Run, verify pass for impl + fix tests**

```bash
cargo test --test profiles_turn_discipline -- impl_profile fix_profile
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(profiles): turn-discipline clause for impl + fix"
```

---

## Task 8: `ORCH_PROFILE` router rules

**Files:**
- Modify: `src/claude_md.rs:84-111` (ORCH_PROFILE)

- [ ] **Step 1: Replace ORCH_PROFILE body's `## Pattern` section + add `## You Do Not Decide` section:**

```markdown
## Inputs (you read both, never edit either)

- `.galvolite/plan.md` — what to build (immutable; only humans edit)
- `.galvolite/directive.md` — how to route (immutable; only humans edit)

## Pattern

1. Read directive. Identify phases whose deps are satisfied.
2. Read recent `.galvolite/postmortems/` and `.galvolite/decisions/` for
   prior context on these phases — the harness will surface them in your
   spawn injection prompts.
3. For each ready phase, `spawn_session` per part with the autonomy level
   from the directive.
4. `wait_children`. On rejoin, call `plan_check_step` for the completed
   parts and advance to the next ready phase.
5. Repeat until directive's terminal phase is reached, then
   `write_handoff` + `status_report({kind: 'complete'})`.

## You Do Not Decide

The directive decided. On any decision not encoded in the directive, you
MUST call `ask_operator`. Examples that REQUIRE `ask_operator`:

- An executor surfaces an architectural concern not in plan.md
- Two executors need the same file
- A phase looks parallelizable but directive says sequential (or vice versa)
- An executor requests scope expansion

You are explicitly forbidden from:

- Adding work not in plan.md
- Resolving inter-executor conflicts yourself
- Editing plan.md or directive.md (only humans edit those)
- Skipping `ask_operator` because "the answer seems obvious"
- Choosing your own phase order if the directive specifies one

## Turn Discipline

Same rule as executors: every turn ends with `plan_check_step`,
`status_report`, `ask_operator`, `request_attention`, or
`write_handoff` + `status_report({kind: 'complete'})`. Never prose-only.
```

- [ ] **Step 2: Run, commit**

```bash
cargo test --test profiles_turn_discipline -- orch
git commit -am "feat(profiles): orchestrator router + you-do-not-decide rules"
```

---

## Task 9: `BRAINSTORM_PROFILE` directive emission

**Files:**
- Modify: `src/claude_md.rs:113-159` (BRAINSTORM_PROFILE)

- [ ] **Step 1: Append after the existing "## Pattern" numbered list:**

```markdown
6. When the option matrix collapses to a chosen path, write BOTH artifacts:
   - `.galvolite/plan.md` — scope (parts list)
   - `.galvolite/directive.md` — phase DAG, autonomy flags, escalation rules

   Format for `directive.md`:

   ```markdown
   # Directive: <name>

   Plan: ./plan.md

   ## Phases

   | ID | Parts | Autonomy | Depends on |
   |----|-------|----------|------------|
   | P1 | 1     | full     | —          |
   | P2 | 2, 3  | review   | P1         |

   ## Escalation rules

   - On schema/migration change → ask_operator
   - On executor stuck >3 cycles → notify_operator(level=warn)

   ## Notes

   <free-form context for the orchestrator>
   ```

   Autonomy: `full` (auto-merge on green CI), `review` (human approves PR),
   `manual` (human authorizes each dispatch).

7. After both files exist, call `set_plan({markdown})` with the plan.md
   contents and `request_attention({reason: "spawn approval"})` to hand off
   to the operator. The operator will click "Spawn sessions" in the
   dashboard, which is gated on both files existing.
```

- [ ] **Step 2: Run, commit**

```bash
cargo test --test profiles_turn_discipline -- brainstorm
git commit -am "feat(profiles): brainstorm emits both plan.md and directive.md"
```

---

## Task 10: `write_postmortem` MCP tool

**Files:**
- Create: `src/postmortem.rs`
- Create: `src/mcp/tools/postmortem.rs`
- Modify: `src/mcp/tools/mod.rs` (register)
- Modify: `src/lib.rs` (mod declaration)
- Modify: `src/state.rs` (recognize `mcp.write_postmortem` event)
- Modify: `src/api/events.rs` (add `PostmortemWritten`)
- Test: `tests/postmortem_writer.rs` (new)

Tool signature:

```rust
write_postmortem({
    plan_part: Number,                  // required
    outcome: 'shipped' | 'stuck' | 'escalated',  // required
    cycles: Number,                     // required
    duration_seconds: Number,           // required
    what_worked: String,                // markdown
    gotchas: String,                    // markdown
    follow_ups: String,                 // markdown
})
```

Writes `<project.path>/.galvolite/postmortems/part-<N>-<session-id>.md` with
YAML frontmatter:

```markdown
---
plan_part: 5
session_id: <uuid>
outcome: shipped
cycles: 3
duration_seconds: 2820
written_at: 2026-05-01T15:23:00Z
---

## What worked
...

## Gotchas
...

## Follow-ups
...
```

- [ ] **Step 1: `src/postmortem.rs` — struct + writer**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Postmortem {
    pub plan_part: u32,
    pub session_id: Uuid,
    pub outcome: Outcome,
    pub cycles: u32,
    pub duration_seconds: u64,
    pub written_at: DateTime<Utc>,
    pub what_worked: String,
    pub gotchas: String,
    pub follow_ups: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome { Shipped, Stuck, Escalated }

impl Postmortem {
    pub fn render(&self) -> String {
        format!(
"---
plan_part: {part}
session_id: {sid}
outcome: {outcome:?}
cycles: {cycles}
duration_seconds: {dur}
written_at: {ts}
---

## What worked

{worked}

## Gotchas

{gotchas}

## Follow-ups

{follow}
",
            part = self.plan_part, sid = self.session_id,
            outcome = self.outcome, cycles = self.cycles,
            dur = self.duration_seconds, ts = self.written_at.to_rfc3339(),
            worked = self.what_worked, gotchas = self.gotchas, follow = self.follow_ups,
        )
    }
}

pub fn postmortems_dir(project_path: &Path) -> PathBuf {
    project_path.join(".galvolite").join("postmortems")
}

pub fn postmortem_path(project_path: &Path, part: u32, sid: Uuid) -> PathBuf {
    postmortems_dir(project_path).join(format!("part-{part}-{sid}.md"))
}

pub fn write(project_path: &Path, pm: &Postmortem) -> std::io::Result<PathBuf> {
    let dir = postmortems_dir(project_path);
    std::fs::create_dir_all(&dir)?;
    let path = postmortem_path(project_path, pm.plan_part, pm.session_id);
    std::fs::write(&path, pm.render())?;
    Ok(path)
}

pub fn list_for_part(project_path: &Path, part: u32) -> std::io::Result<Vec<Postmortem>> {
    let dir = postmortems_dir(project_path);
    if !dir.exists() { return Ok(vec![]); }
    let prefix = format!("part-{part}-");
    let mut out = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(&prefix) && name_str.ends_with(".md") {
            if let Ok(s) = std::fs::read_to_string(entry.path()) {
                if let Ok(pm) = parse(&s) { out.push(pm); }
            }
        }
    }
    out.sort_by_key(|pm| pm.written_at);
    Ok(out)
}

pub fn parse(_s: &str) -> Result<Postmortem, ParseError> {
    // serde_yaml frontmatter + section split. ~25 LOC.
    todo!("frontmatter split + serde_yaml deserialize + section regex")
}
```

- [ ] **Step 2: `src/mcp/tools/postmortem.rs` — MCP wrapper**

Pattern: copy `src/mcp/tools/tier1_role_handoff.rs::write_handoff` as scaffolding (same shape: arg deserialize, side-effect, ToolOutcome event).

```rust
pub fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "write_postmortem",
        description: "Write a postmortem for a finished plan part. The harness uses these to brief future executors on the same part.",
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["plan_part", "outcome", "cycles", "duration_seconds"],
            "properties": {
                "plan_part": { "type": "integer" },
                "outcome": { "enum": ["shipped", "stuck", "escalated"] },
                "cycles": { "type": "integer" },
                "duration_seconds": { "type": "integer" },
                "what_worked": { "type": "string", "default": "" },
                "gotchas": { "type": "string", "default": "" },
                "follow_ups": { "type": "string", "default": "" },
            }
        }),
        handler: Box::new(|ctx, args| Box::pin(write_postmortem(ctx, args))),
    }]
}

async fn write_postmortem(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let pm = build_postmortem_from_args(&ctx, args)?;
    let project = ctx.session_manager.get_project_for_session(ctx.session_id)?;
    let path = postmortem::write(std::path::Path::new(&project.path), &pm)
        .map_err(|e| tool_err(format!("write failed: {e}")))?;
    ctx.event_bus.publish(DomainEvent::PostmortemWritten {
        project_id: project.id, plan_part: pm.plan_part, path: path.clone(),
    });
    ToolOutcome::Ok {
        result: serde_json::json!({ "path": path }),
        kind: "mcp.write_postmortem".into(),
        ...
    }
}
```

- [ ] **Step 3: Tests for round-trip + listing**

```rust
// tests/postmortem_writer.rs
#[tokio::test]
async fn write_postmortem_persists_and_lists() {
    let h = TestHarness::new().await;
    let session = h.create_session_with_project().await;
    h.call_mcp(&session, "write_postmortem", json!({
        "plan_part": 5, "outcome": "shipped", "cycles": 3,
        "duration_seconds": 2820,
        "what_worked": "TDD", "gotchas": "flake on macOS",
        "follow_ups": "see SEED-007",
    })).await.unwrap();

    let list = h.client.get(format!("/api/projects/{}/postmortems?part=5", session.project_id))
        .send().await.unwrap().json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["outcome"], "shipped");
}
```

- [ ] **Step 4: Wire registration + state machine + event variant.**

- [ ] **Step 5: Run, commit**

```bash
cargo test --test postmortem_writer
git commit -am "feat(mcp): write_postmortem tool + flat-file storage"
```

---

## Task 11: Decision auto-record on `ask_operator` answer

**Files:**
- Create: `src/decisions.rs`
- Modify: `src/api/operator.rs::OperatorPromptRegistry::answer` (call `decisions::record`)
- Modify: `src/api/events.rs` (add `DecisionRecorded`)
- Modify: `src/lib.rs`
- Test: `tests/decisions_autorecord.rs` (new)

`src/api/operator.rs:101-120` is where `answer()` lives. After successfully sending the answer onto the oneshot, also call `decisions::record(...)` — the question + answer survive even if the requesting MCP turn has timed out.

- [ ] **Step 1: `src/decisions.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub session_id: Uuid,        // who asked
    pub asked_at: DateTime<Utc>,
    pub question: String,
    pub answer: String,          // stringified, even if structured
    pub choices: Option<Vec<String>>,
}

pub fn decisions_dir(project_path: &Path) -> PathBuf {
    project_path.join(".galvolite").join("decisions")
}

pub fn record(project_path: &Path, d: &Decision) -> std::io::Result<PathBuf> {
    let dir = decisions_dir(project_path);
    std::fs::create_dir_all(&dir)?;
    let ts = d.asked_at.format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{ts}.md", d.session_id));
    let body = render(d);
    std::fs::write(&path, body)?;
    Ok(path)
}

pub fn list_recent(project_path: &Path, limit: usize) -> std::io::Result<Vec<Decision>> {
    // Read dir, parse frontmatter, sort by asked_at desc, take(limit). ~20 LOC.
    todo!()
}

fn render(d: &Decision) -> String {
    format!(
"---
session_id: {sid}
asked_at: {ts}
choices: {choices:?}
---

## Question

{q}

## Answer

{a}
",
        sid = d.session_id, ts = d.asked_at.to_rfc3339(),
        choices = d.choices.as_deref().unwrap_or(&[]),
        q = d.question, a = d.answer,
    )
}
```

- [ ] **Step 2: Hook into `OperatorPromptRegistry`**

The registry needs the project path to write into. Two options:
(a) Look up via `SessionManager::get_project_for_session(session_id)`.
(b) Stash the question + session_id + project_path on `register_and_wait` so `answer` has them locally.

(b) is simpler — extend the waiters map's value to a struct holding both the
oneshot sender AND a `RecordContext { session_id, project_path, question, choices }`. Then
`answer()` writes the decision before returning.

- [ ] **Step 3: Failing test**

```rust
#[tokio::test]
async fn ask_operator_answer_writes_decision_file() {
    let h = TestHarness::new().await;
    let session = h.create_session_with_project().await;
    let project = h.get_project(session.project_id).await;

    // Spawn the ask_operator MCP call in background.
    let h2 = h.clone();
    let sid = session.id;
    let task = tokio::spawn(async move {
        h2.call_mcp(&sid, "ask_operator", json!({
            "question": "Should P5 wait for P4?",
            "choices": ["yes", "no"],
            "timeout_secs": 10,
        })).await
    });

    // Give the MCP call time to register the prompt.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Look up the prompt id from the events stream + answer it.
    let prompt_id = h.last_operator_prompt_id().await;
    h.client.post(format!("/api/operator/prompt/{prompt_id}/answer"))
        .json(&json!({"answer": "no"}))
        .send().await.unwrap();

    let _ = task.await.unwrap();

    let dir = std::path::Path::new(&project.path).join(".galvolite/decisions");
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let body = std::fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert!(body.contains("Should P5 wait for P4?"));
    assert!(body.contains("\"no\""));
}
```

- [ ] **Step 4: Run, commit**

```bash
cargo test --test decisions_autorecord
git commit -am "feat(decisions): auto-record ask_operator answers to .galvolite/decisions/"
```

---

## Task 12: Spawn-time context injection

**Files:**
- Modify: `src/session_manager.rs:319-...` (`spawn_child`)
- Test: `tests/spawn_context_injection.rs` (new)

When `spawn_child` builds the injection prompt for a new executor, prepend a "## Prior context" section sourced from:
- `postmortem::list_for_part(project_path, part)` — most recent 3
- `decisions::list_recent(project_path, 10)` — most recent 10, filter to last 7 days

Cap total prepended bytes at 2 KiB. If overflow, truncate oldest first.

The `task` argument to `spawn_session` should include a `plan_part: Option<u32>` field so the harness knows which postmortems are relevant. Extend `spawn_session` MCP tool schema.

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn spawn_with_plan_part_injects_relevant_postmortem() {
    let h = TestHarness::new().await;
    let parent = h.create_session_with_project().await;
    let project = h.get_project(parent.project_id).await;

    // Seed a postmortem for plan_part=5.
    galvolite::postmortem::write(
        std::path::Path::new(&project.path),
        &Postmortem {
            plan_part: 5, session_id: Uuid::new_v4(),
            outcome: Outcome::Stuck, cycles: 4,
            duration_seconds: 3600, written_at: Utc::now(),
            what_worked: "".into(),
            gotchas: "DB lock contention with parallel migrations".into(),
            follow_ups: "".into(),
        },
    ).unwrap();

    // Spawn a child for plan_part=5; inspect the rendered injection prompt.
    let child = h.call_mcp(&parent.id, "spawn_session", json!({
        "role": "implementation",
        "task": "Re-attempt part 5",
        "plan_part": 5,
    })).await.unwrap();

    let injection = h.get_injection_prompt(child["session_id"].as_str().unwrap()).await;
    assert!(injection.contains("Prior context"));
    assert!(injection.contains("DB lock contention"));
}

#[tokio::test]
async fn injection_caps_at_2kib_and_drops_oldest() {
    // Write 10 postmortems for part=7, each with 500 bytes of gotchas.
    // Spawn for part=7. Assert injection contains the most-recent 4-5,
    // not all 10, and total injected context < 2 KiB.
}
```

- [ ] **Step 2: Implement injection helper**

```rust
fn build_prior_context(project_path: &Path, plan_part: Option<u32>) -> String {
    let mut blocks = vec![];
    let cap = 2048;

    if let Some(part) = plan_part {
        if let Ok(pms) = postmortem::list_for_part(project_path, part) {
            for pm in pms.iter().rev().take(3) {
                blocks.push(format!(
                    "### Postmortem (part {}, {:?}, {} cycles)\n\nGotchas: {}\n\nFollow-ups: {}\n",
                    pm.plan_part, pm.outcome, pm.cycles,
                    pm.gotchas.lines().take(5).collect::<Vec<_>>().join(" "),
                    pm.follow_ups.lines().take(3).collect::<Vec<_>>().join(" "),
                ));
            }
        }
    }

    if let Ok(decs) = decisions::list_recent(project_path, 10) {
        for d in decs.iter().take(5) {
            blocks.push(format!("### Decision: {}\n\n{}\n", d.question, d.answer));
        }
    }

    let mut out = String::from("## Prior context\n\n");
    for b in blocks {
        if out.len() + b.len() > cap { break; }
        out.push_str(&b);
    }
    if out.len() == "## Prior context\n\n".len() { return String::new(); }
    out
}
```

Then in `spawn_child`, after assembling the existing injection prompt, prepend `build_prior_context(...)`.

- [ ] **Step 3: Extend `spawn_session` MCP tool schema** to accept optional `plan_part: u32`. Pass through to `spawn_child`.

- [ ] **Step 4: Run, commit**

```bash
cargo test --test spawn_context_injection
git commit -am "feat(spawn): inject postmortems + decisions into child context"
```

---

## Task 13: Spawn-button gate (requires `directive.md` + `plan.md`)

**Files:**
- Modify: `ui/src/components/Dashboard.tsx` (or wherever the Spawn button lives — `grep -n "Spawn" ui/src/components/`)
- Test: extend the relevant component test

- [ ] **Step 1: Failing test**

```tsx
test('Spawn button disabled when directive missing', async () => {
  vi.mocked(getPlan).mockResolvedValue({ markdown: '## Parts\n- 1' });
  vi.mocked(getDirective).mockResolvedValue(null);
  render(<SpawnControls projectId="p1" />);
  const btn = await screen.findByText(/spawn sessions/i);
  expect(btn).toBeDisabled();
  expect(screen.getByText(/directive.md required/i)).toBeInTheDocument();
});

test('Spawn button enabled when both present', async () => {
  vi.mocked(getPlan).mockResolvedValue({ markdown: '## Parts\n- 1' });
  vi.mocked(getDirective).mockResolvedValue({ name: 'X', /* ... */ });
  render(<SpawnControls projectId="p1" />);
  expect(await screen.findByText(/spawn sessions/i)).toBeEnabled();
});
```

- [ ] **Step 2: Implement gating logic in the component.**

- [ ] **Step 3: Run, commit**

```bash
cd ui && pnpm test src/components/SpawnControls.test.tsx && cd ..
git commit -am "feat(ui): require directive.md before Spawn sessions"
```

---

## Task 14: `PostmortemsPanel` component (parity)

**Files:**
- Create: `ui/src/components/PostmortemsPanel.tsx`
- Create: `ui/src/components/PostmortemsPanel.test.tsx`
- Modify: `ui/src/api.ts` (`listPostmortems`, `listDecisions`)
- Modify: `ui/src/components/Dashboard.tsx` (mount panel)

Per memory `feedback_gui_parity.md`: every backend endpoint must surface in the dashboard.

- [ ] **Step 1: API endpoints — already covered in Task 10 spec, but verify wiring of `GET /api/projects/:id/postmortems` and `GET /api/projects/:id/decisions` into `src/api/mod.rs`. If missing, add them now (handler reads `postmortems_dir` / `decisions_dir`, parses each file, returns JSON array).**

- [ ] **Step 2: API client + types**

```ts
export async function listPostmortems(projectId: string): Promise<Postmortem[]> { ... }
export async function listDecisions(projectId: string): Promise<Decision[]> { ... }
```

- [ ] **Step 3: Component — two collapsible sections, "Postmortems" and "Decisions". Each shows a count badge. Click a row to expand to full markdown body.**

- [ ] **Step 4: Subscribe to `postmortem.written` and `decision.recorded` events for live refresh.**

- [ ] **Step 5: Run, commit**

```bash
cd ui && pnpm test src/components/PostmortemsPanel.test.tsx && cd ..
git commit -am "feat(ui): PostmortemsPanel + Dashboard mount"
```

---

## Task 15: E2E smoke + memory updates

**Files:**
- Create: `tests/orchestrator_discipline_e2e.rs` (new)
- Update: `~/.claude/projects/.../memory/MEMORY.md` (point to plan-7 status memo)
- Create: `~/.claude/projects/.../memory/project_plan7_status.md`

E2E happy path:
1. Spin up TestHarness with mock-claude binary configured to play out:
   - Brainstorm session: writes plan.md + directive.md, calls `set_plan`, `request_attention`.
   - Operator answers attention with "spawn approved".
   - Orchestrator session spawns: reads directive, spawns 1 child for P1, `wait_children`, marks complete, `status_report({kind: 'complete'})`.
   - Child: writes postmortem on completion.
2. Assert: directive.md exists; plan.md exists; postmortem file exists; decision file exists; orchestrator never edited plan.md or directive.md (file mtimes unchanged after orchestrator spawn).

This proves the discipline holds end-to-end with the mock harness.

- [ ] **Run all tests**

```bash
cargo test
cd ui && pnpm test && cd ..
```

- [ ] **Smoke in browser:** spin GalvoLite with `cargo run`, create a brainstorm session in a fixture project, drive it through writing plan + directive, click Spawn, verify orchestrator+child flow. Capture screenshots in commit message if any UI quirks.

- [ ] **Memory update**

```markdown
<!-- ~/.claude/projects/.../memory/project_plan7_status.md -->
---
name: Plan 7 status
description: Sub-plan 7 (orchestrator discipline + memory augmentation) DONE on main as of 2026-05-XX
type: project
---
Sub-plan 7 shipped <commit hash> on main. Adds:
- .galvolite/directive.md as orchestration contract (parser + REST + watcher + UI)
- Turn-discipline rule in IMPL/FIX profiles; router rule + you-do-not-decide in ORCH; directive emission in BRAINSTORM
- write_postmortem MCP tool + auto-decision-log on ask_operator answer
- Spawn-time context injection: child sees recent postmortems + decisions for its plan_part
- Dashboard parity: DirectiveTab + PostmortemsPanel
Tests added: ~10 new integration files. cargo test green; pnpm test green.
```

- [ ] **Final commit**

```bash
git commit -am "test(plan7): E2E smoke + memory update for orchestrator discipline"
```

---

## Self-Review Checklist

Run before declaring the plan ready for execution:

1. **Spec coverage** — every section of `2026-05-01-orchestrator-discipline.md` mapped to a task:
   - §2.1 directive — Tasks 1-6, 9
   - §2.2 turn discipline — Task 7
   - §2.3 orchestrator-as-router — Task 8
   - §2.4 postmortems — Task 10, 14
   - §2.5 decisions — Task 11, 14
   - §3 profile diffs — Tasks 7, 8, 9
   - §4 implementation buckets — covered by Tasks 1-15

2. **Type consistency** — `Directive`, `Phase`, `Autonomy`, `Postmortem`, `Outcome`, `Decision`: all defined in Tasks 1, 10, 11; referenced consistently in 5, 6, 12, 14.

3. **Placeholder scan** — Tasks 1, 10, 11 each have one `todo!()` or "~XX LOC" annotation for helper functions (parse helpers, list_recent). These are *intentional shorthand* for boilerplate — the executor expands them following the patterns shown. Acceptable per spec section length tradeoff. NOT placeholders for missing design decisions.

4. **GUI parity** — every backend endpoint (directive GET/PUT, postmortems GET, decisions GET) has a Dashboard surface (DirectiveTab in Task 6, PostmortemsPanel in Task 14). Spawn-button gate (Task 13) closes the loop on directive consumption.

5. **TDD discipline** — every task starts with a failing test before any implementation step. Confirmed by inspection.

6. **Frequent commits** — every task ends in a commit. 15 tasks → 15+ commits, matches the existing plan cadence.

---

## Execution Handoff

Plan complete and saved to
`docs/superpowers/plans/2026-05-01-plan-7-orchestrator-discipline.md`.

**Two execution options:**

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task,
   review between tasks. Best for keeping orchestrator/executor cleanliness
   high (eat your own dogfood: this plan IS about discipline).

2. **Inline Execution** — execute tasks in this session using
   `superpowers:executing-plans`, batch with checkpoints for review.

Which approach?
