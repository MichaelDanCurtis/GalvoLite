# Plan 3: Cross-surface fixes + role-as-state Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the three defects Michael surfaced after Plan 1b/Plan 2 manual testing — folder picker that works in both Tauri and browser surfaces, role transitions visible mid-session, and an interactive terminal — so GalvoLite is actually usable for daily work and ready for eventual Tailscale exposure.

**Architecture:** Three independent vertical slices, each touching backend (Rust) + frontend (React) where needed. (A) Folder picker branches on `__TAURI_INTERNALS__`: native dialog in webview, server-driven `<DirectoryBrowserModal>` over a new `GET /api/fs/list` endpoint in browser. Both surfaces remain functionally identical. (B) New `[ROLE:SET role="..."]` signal joins the existing protocol; SignalPump persists the role to `sessions.role` and the dashboard renders a colored badge that updates live via the existing 2s polling. (C) xterm `onData` sends `{type:"input", data}` text frames to the terminal WebSocket; new `ClientFrame::Input` arm dispatches to `SessionManager::write_pty` which routes through the existing `PtyHandleShared::write` channel. Read-only mode is gated on `session.autopilot`.

**Tech Stack:** Rust + axum 0.7, React 18 + TypeScript + Vite + xterm.js, Tauri v2 with `tauri-plugin-dialog`, Vitest + React Testing Library.

---

## File Structure

### Section A — Cross-surface folder picker

**Backend:**
- `src/api.rs` (modify) — add `GET /api/fs/list?path=<absolute>` route + handler. Returns `{path, parent, entries: [{name, is_dir}]}`. Lists immediate children only. Path must be absolute and existing; relative or missing → 400.
- `src/error.rs` (modify) — reuse existing `Error::InvalidPath`; no new variants.

**Frontend:**
- `ui/src/types.ts` (modify) — add `DirectoryListing` and `DirectoryEntry` types.
- `ui/src/api.ts` (modify) — add `api.listDirectory(path)`.
- `ui/src/components/modals/DirectoryBrowserModal.tsx` (create) — modal: header shows current path, parent-up button, scrollable list of dir entries, "Use this folder" button. Used only in browser surface.
- `ui/src/components/modals/DirectoryBrowserModal.test.tsx` (create) — render, navigate into a child, navigate up, select.
- `ui/src/components/modals/AddProjectModal.tsx` (modify) — try/catch around the Tauri dialog call (surface errors in the existing `error` slot); branch on `__TAURI_INTERNALS__` — Tauri uses native dialog, browser opens `DirectoryBrowserModal`.

### Section B — Role as session state

**Backend:**
- `src/signals.rs` (modify) — add `Signal::RoleSet { role: String }` variant. Parses `[ROLE:SET role="<name>"]`. `kind() = "ROLE:SET"`. `payload() = Some(role)`.
- `src/db.rs` (modify) — add `Db::set_session_role(id, role)`.
- `src/pty.rs` (modify) — `SignalPump::start` task gains a side-effect: when `Signal::RoleSet { role }` is parsed, also call `db.set_session_role(session_id, &role)` before forwarding to the channel.

**Frontend:**
- `ui/src/components/RoleBadge.tsx` (create) — small colored badge component. Maps role string → bg/fg classes. Unknown roles render as a neutral badge.
- `ui/src/components/RoleBadge.test.tsx` (create) — renders all four known roles + unknown.
- `ui/src/components/SessionRow.tsx` (modify) — replace bare `{session.role}` text with `<RoleBadge role={session.role} />` in the metadata row.
- `ui/src/components/ExpandedSessionView.tsx` (modify) — replace `<strong>{session.role}</strong>` with `<RoleBadge role={session.role} />` in the header.
- `ui/src/components/modals/StartSessionModal.tsx` (modify) — remove the `role` input entirely. Rely on backend default of `"implementation"` (already in `api.rs:145`).
- `ui/src/components/modals/StartSessionModal.test.tsx` (modify) — drop the role input assertions.

### Section C — Interactive terminal

**Backend:**
- `src/session_manager.rs` (modify) — add `pub async fn write_pty(&self, id: Uuid, bytes: Vec<u8>) -> Result<()>`. Looks up the running session, clones the `PtyHandleShared`, drops the lock, then awaits `handle.write(&bytes)`.
- `src/api.rs` (modify) — add `ClientFrame::Input { data: String }`. In `handle_terminal_ws` reader loop, on Input frame call `mgr.write_pty(session_id, data.into_bytes()).await`.

**Frontend:**
- `ui/src/hooks/useTerminal.ts` (modify) — `useTerminal(containerRef, sessionId, opts?: { readOnly?: boolean })`. Add `term.onData(data => ws.send(JSON.stringify({type:"input", data})))` — gated on `!opts?.readOnly`.
- `ui/src/hooks/useTerminal.test.ts` (create) — uses a mock WebSocket; asserts that interactive mode sends input frames and read-only mode does not.
- `ui/src/components/TerminalPreview.tsx` (modify) — accept `readOnly?: boolean` prop and pass to `useTerminal`. Default `true` (the row preview stays read-only).
- `ui/src/components/ExpandedSessionView.tsx` (modify) — pass `readOnly={session.autopilot}` to `<TerminalPreview>`.

---

### Task 1: Add API integration test for `GET /api/fs/list`

**Files:**
- Modify: `tests/api_integration.rs`

- [ ] **Step 1: Add the failing test**

Append to `tests/api_integration.rs`:

```rust
#[tokio::test]
async fn fs_list_returns_dir_entries() {
    use tempfile::tempdir;
    let tmp = tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("alpha")).unwrap();
    std::fs::create_dir(root.join("beta")).unwrap();
    std::fs::write(root.join("file.txt"), b"x").unwrap();

    let (addr, _mgr, _data_dir) = spawn_test_server().await;
    let url = format!(
        "http://{addr}/api/fs/list?path={}",
        urlencoding::encode(root.to_str().unwrap())
    );
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["path"], root.to_str().unwrap());
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"file.txt"));
    let alpha = entries.iter().find(|e| e["name"] == "alpha").unwrap();
    assert_eq!(alpha["is_dir"], true);
    let f = entries.iter().find(|e| e["name"] == "file.txt").unwrap();
    assert_eq!(f["is_dir"], false);
}

#[tokio::test]
async fn fs_list_rejects_relative_path() {
    let (addr, _mgr, _data_dir) = spawn_test_server().await;
    let url = format!("http://{addr}/api/fs/list?path=relative/path");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn fs_list_rejects_missing_path() {
    let (addr, _mgr, _data_dir) = spawn_test_server().await;
    let url = format!("http://{addr}/api/fs/list?path=/no/such/dir/anywhere/here");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), 400);
}
```

If `tests/api_integration.rs` does not already use `urlencoding`, add `urlencoding = "2"` to the `[dev-dependencies]` of `Cargo.toml`. Likewise verify `tempfile` is already a dev-dep (it is — used by other tests).

- [ ] **Step 2: Run the new tests to confirm they fail**

Run: `cargo test --test api_integration fs_list 2>&1 | tail -20`
Expected: three tests fail with 404 or compile error referencing the missing route.

### Task 2: Implement `GET /api/fs/list`

**Files:**
- Modify: `src/api.rs:18-30` (router) and add handler at end-of-file.

- [ ] **Step 1: Add the route**

In the `router` function (around `src/api.rs:28`), add this line before `.layer(CorsLayer::permissive())`:

```rust
        .route("/api/fs/list", get(list_directory))
```

- [ ] **Step 2: Add the handler**

Append after `handle_terminal_ws` (end of file before any tests):

```rust
// --- Filesystem browser (server-driven directory picker) ---

#[derive(serde::Deserialize)]
struct FsListQuery {
    path: String,
}

#[derive(serde::Serialize)]
struct FsEntry {
    name: String,
    is_dir: bool,
}

#[derive(serde::Serialize)]
struct FsListing {
    path: String,
    parent: Option<String>,
    entries: Vec<FsEntry>,
}

async fn list_directory(
    axum::extract::Query(q): axum::extract::Query<FsListQuery>,
) -> Response {
    let p = std::path::PathBuf::from(&q.path);
    if !p.is_absolute() {
        return (StatusCode::BAD_REQUEST, "path must be absolute").into_response();
    }
    let canonical = match p.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("path not accessible: {e}"))
                .into_response()
        }
    };
    if !canonical.is_dir() {
        return (StatusCode::BAD_REQUEST, "path is not a directory").into_response();
    }

    let mut entries: Vec<FsEntry> = match std::fs::read_dir(&canonical) {
        Ok(rd) => rd
            .filter_map(|r| r.ok())
            .map(|de| {
                let name = de.file_name().to_string_lossy().into_owned();
                let is_dir = de
                    .file_type()
                    .map(|t| t.is_dir())
                    .unwrap_or(false);
                FsEntry { name, is_dir }
            })
            .collect(),
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("read_dir failed: {e}"))
                .into_response()
        }
    };
    // Dirs first, then files; case-insensitive name within each group.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = canonical
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.to_owned());

    Json(FsListing {
        path: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    })
    .into_response()
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --test api_integration fs_list 2>&1 | tail -15`
Expected: three `fs_list_*` tests pass.

- [ ] **Step 4: Run the full Rust suite to confirm no regression**

Run: `cargo test --workspace 2>&1 | tail -10`
Expected: all green.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/api.rs tests/api_integration.rs
git commit -m "feat(api): GET /api/fs/list — directory browser endpoint"
```

### Task 3: Frontend `api.listDirectory` + types

**Files:**
- Modify: `ui/src/types.ts`
- Modify: `ui/src/api.ts`

- [ ] **Step 1: Add the types**

Append to `ui/src/types.ts`:

```ts
export interface DirectoryEntry {
  name: string;
  is_dir: boolean;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  entries: DirectoryEntry[];
}
```

- [ ] **Step 2: Update the imports in `ui/src/api.ts`**

Change the type import block at the top to:

```ts
import type {
  Project,
  Session,
  SignalRow,
  CreateProjectReq,
  CreateSessionReq,
  SendReq,
  DirectoryListing,
} from "./types";
```

- [ ] **Step 3: Add the api method**

Inside the `api` object literal, append before `terminalWsUrl`:

```ts
  listDirectory: (path: string) =>
    request<DirectoryListing>(
      `/api/fs/list?path=${encodeURIComponent(path)}`,
    ),
```

- [ ] **Step 4: Run UI tests + typecheck**

Run: `cd ui && npm test && npm run build && cd ..`
Expected: 26+ tests pass, build succeeds.

- [ ] **Step 5: Commit**

```bash
git add ui/src/types.ts ui/src/api.ts
git commit -m "feat(ui): listDirectory api client + DirectoryListing types"
```

### Task 4: `<DirectoryBrowserModal>` component + tests

**Files:**
- Create: `ui/src/components/modals/DirectoryBrowserModal.tsx`
- Create: `ui/src/components/modals/DirectoryBrowserModal.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `ui/src/components/modals/DirectoryBrowserModal.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DirectoryBrowserModal } from "./DirectoryBrowserModal";
import { api } from "../../api";

vi.mock("../../api", () => ({
  api: {
    listDirectory: vi.fn(),
  },
}));

const mockListDirectory = api.listDirectory as unknown as ReturnType<typeof vi.fn>;

describe("DirectoryBrowserModal", () => {
  beforeEach(() => {
    mockListDirectory.mockReset();
  });

  it("lists entries for the initial path and lets the user pick the current dir", async () => {
    mockListDirectory.mockResolvedValueOnce({
      path: "/Users/me",
      parent: "/Users",
      entries: [
        { name: "Code", is_dir: true },
        { name: "Documents", is_dir: true },
        { name: "notes.txt", is_dir: false },
      ],
    });

    const onPick = vi.fn();
    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={onPick}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("Code")).toBeInTheDocument();
    });
    expect(screen.getByText("/Users/me")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /use this folder/i }));
    expect(onPick).toHaveBeenCalledWith("/Users/me");
  });

  it("navigates into a child directory when its row is clicked", async () => {
    mockListDirectory
      .mockResolvedValueOnce({
        path: "/Users/me",
        parent: "/Users",
        entries: [{ name: "Code", is_dir: true }],
      })
      .mockResolvedValueOnce({
        path: "/Users/me/Code",
        parent: "/Users/me",
        entries: [{ name: "GalvoLite", is_dir: true }],
      });

    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={() => {}}
      />,
    );

    await waitFor(() => screen.getByText("Code"));
    await userEvent.click(screen.getByText("Code"));
    await waitFor(() => screen.getByText("GalvoLite"));
    expect(mockListDirectory).toHaveBeenCalledWith("/Users/me/Code");
  });

  it("navigates to parent when the up button is clicked", async () => {
    mockListDirectory
      .mockResolvedValueOnce({
        path: "/Users/me",
        parent: "/Users",
        entries: [],
      })
      .mockResolvedValueOnce({
        path: "/Users",
        parent: "/",
        entries: [],
      });

    render(
      <DirectoryBrowserModal
        open
        initialPath="/Users/me"
        onClose={() => {}}
        onPick={() => {}}
      />,
    );

    await waitFor(() => screen.getByText("/Users/me"));
    await userEvent.click(screen.getByRole("button", { name: /up/i }));
    await waitFor(() => screen.getByText("/Users"));
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npm test DirectoryBrowserModal 2>&1 | tail -15`
Expected: tests fail (component file not found).

- [ ] **Step 3: Implement the component**

Create `ui/src/components/modals/DirectoryBrowserModal.tsx`:

```tsx
import { useEffect, useState, useCallback } from "react";
import { ArrowUp, Folder } from "lucide-react";
import { Modal } from "./Modal";
import { api } from "../../api";
import type { DirectoryListing } from "../../types";

interface Props {
  open: boolean;
  initialPath: string;
  onClose: () => void;
  onPick: (path: string) => void;
}

export function DirectoryBrowserModal({ open, initialPath, onClose, onPick }: Props) {
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.listDirectory(path);
      setListing(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) load(initialPath);
  }, [open, initialPath, load]);

  return (
    <Modal open={open} onClose={onClose} title="Choose a folder">
      <div className="flex flex-col gap-2 min-w-[360px]">
        <div className="flex items-center gap-2">
          <button
            type="button"
            aria-label="Up to parent"
            disabled={!listing?.parent}
            className="p-1 border border-neutral-700 rounded text-neutral-300 hover:text-neutral-100 hover:border-neutral-500 disabled:opacity-30 disabled:hover:border-neutral-700"
            onClick={() => listing?.parent && load(listing.parent)}
          >
            <ArrowUp size={14} />
          </button>
          <div className="flex-1 truncate text-xs font-mono bg-neutral-900 px-2 py-1 rounded">
            {listing?.path ?? initialPath}
          </div>
        </div>

        <div className="border border-neutral-800 rounded max-h-72 overflow-y-auto">
          {loading && (
            <div className="px-3 py-2 text-xs text-neutral-500">Loading…</div>
          )}
          {error && (
            <div className="px-3 py-2 text-xs text-red-400" role="alert">
              {error}
            </div>
          )}
          {!loading && !error && listing && listing.entries.length === 0 && (
            <div className="px-3 py-2 text-xs text-neutral-500">Empty.</div>
          )}
          {!loading &&
            !error &&
            listing?.entries.map((e) => {
              if (e.is_dir) {
                const childPath = listing.path.endsWith("/")
                  ? `${listing.path}${e.name}`
                  : `${listing.path}/${e.name}`;
                return (
                  <button
                    key={e.name}
                    type="button"
                    className="flex items-center gap-2 w-full text-left px-3 py-1.5 text-sm text-neutral-200 hover:bg-neutral-800"
                    onClick={() => load(childPath)}
                  >
                    <Folder size={14} className="text-amber-400 shrink-0" />
                    <span className="truncate">{e.name}</span>
                  </button>
                );
              }
              return (
                <div
                  key={e.name}
                  className="flex items-center gap-2 px-3 py-1.5 text-sm text-neutral-500"
                >
                  <span className="w-3.5 shrink-0" />
                  <span className="truncate">{e.name}</span>
                </div>
              );
            })}
        </div>

        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!listing}
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            onClick={() => listing && onPick(listing.path)}
          >
            Use this folder
          </button>
        </div>
      </div>
    </Modal>
  );
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ui && npm test DirectoryBrowserModal 2>&1 | tail -10`
Expected: 3/3 pass.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/modals/DirectoryBrowserModal.tsx ui/src/components/modals/DirectoryBrowserModal.test.tsx
git commit -m "feat(ui): DirectoryBrowserModal for browser-side folder picking"
```

### Task 5: Branch AddProjectModal on surface + add try/catch

**Files:**
- Modify: `ui/src/components/modals/AddProjectModal.tsx`
- Modify: `ui/src/components/modals/AddProjectModal.test.tsx`

- [ ] **Step 1: Update the test to assert both code paths**

Replace the contents of `ui/src/components/modals/AddProjectModal.test.tsx` with:

```tsx
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AddProjectModal } from "./AddProjectModal";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));
vi.mock("../../api", () => ({
  api: {
    listDirectory: vi.fn().mockResolvedValue({
      path: "/Users/me",
      parent: "/Users",
      entries: [{ name: "Project", is_dir: true }],
    }),
  },
}));

import { open as tauriOpen } from "@tauri-apps/plugin-dialog";

describe("AddProjectModal", () => {
  beforeEach(() => {
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockReset();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("submits name + path", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<AddProjectModal open onClose={() => {}} onSubmit={onSubmit} />);
    await userEvent.type(screen.getByLabelText(/name/i), "demo");
    await userEvent.type(screen.getByLabelText(/path/i), "/tmp/demo");
    await userEvent.click(screen.getByRole("button", { name: /^add$/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({ name: "demo", path: "/tmp/demo" }));
  });

  it("calls Tauri dialog when running inside Tauri", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockResolvedValueOnce("/picked/by/tauri");
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    await waitFor(() =>
      expect(screen.getByLabelText(/path/i)).toHaveValue("/picked/by/tauri"),
    );
  });

  it("opens DirectoryBrowserModal when not running inside Tauri", async () => {
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    // The directory browser modal exposes "Use this folder"
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /use this folder/i }),
      ).toBeInTheDocument(),
    );
  });

  it("surfaces Tauri dialog errors instead of swallowing them", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    (tauriOpen as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("dialog plugin not initialized"),
    );
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: /browse for folder/i }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(/dialog plugin not initialized/i),
    );
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npm test AddProjectModal 2>&1 | tail -15`
Expected: at least the new "DirectoryBrowserModal" and "surfaces errors" tests fail.

- [ ] **Step 3: Update the component**

Replace the contents of `ui/src/components/modals/AddProjectModal.tsx`:

```tsx
import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Modal } from "./Modal";
import { DirectoryBrowserModal } from "./DirectoryBrowserModal";

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function pickViaTauri(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({ directory: true, multiple: false });
  return typeof result === "string" ? result : null;
}

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (req: { name: string; path: string }) => Promise<unknown>;
}

export function AddProjectModal({ open, onClose, onSubmit }: Props) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [browserOpen, setBrowserOpen] = useState(false);

  const canSubmit = name.trim().length > 0 && path.trim().length > 0 && !busy;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ name: name.trim(), path: path.trim() });
      setName("");
      setPath("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleBrowseClick() {
    setError(null);
    if (isTauri()) {
      try {
        const picked = await pickViaTauri();
        if (picked) setPath(picked);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } else {
      setBrowserOpen(true);
    }
  }

  function defaultBrowserPath(): string {
    if (path.trim()) return path.trim();
    // Most users pick a project under their home dir; "/" is a safer
    // universal fallback when home cannot be inferred from the URL.
    return "/";
  }

  return (
    <>
      <Modal open={open} onClose={onClose} title="Add Project">
        <form onSubmit={handleSubmit} className="flex flex-col gap-3">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-neutral-400">Name</span>
            <input
              className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-neutral-400">Path</span>
            <div className="flex gap-1">
              <input
                className="flex-1 bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                placeholder="/absolute/path/to/workspace"
              />
              <button
                type="button"
                title="Browse…"
                aria-label="Browse for folder"
                className="px-2 py-1 border border-neutral-700 rounded text-neutral-300 hover:text-neutral-100 hover:border-neutral-500"
                onClick={handleBrowseClick}
              >
                <FolderOpen size={14} />
              </button>
            </div>
          </label>
          {error && (
            <p className="text-xs text-red-400" role="alert">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2 pt-2">
            <button
              type="button"
              className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
              disabled={!canSubmit}
            >
              {busy ? "Adding…" : "Add"}
            </button>
          </div>
        </form>
      </Modal>

      <DirectoryBrowserModal
        open={browserOpen}
        initialPath={defaultBrowserPath()}
        onClose={() => setBrowserOpen(false)}
        onPick={(p) => {
          setPath(p);
          setBrowserOpen(false);
        }}
      />
    </>
  );
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd ui && npm test AddProjectModal 2>&1 | tail -15`
Expected: 4/4 pass.

- [ ] **Step 5: Run full UI suite + build**

Run: `cd ui && npm test && npm run build && cd ..`
Expected: all tests pass, build succeeds.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/modals/AddProjectModal.tsx ui/src/components/modals/AddProjectModal.test.tsx
git commit -m "feat(ui): cross-surface folder picker — Tauri native or DirectoryBrowserModal"
```

### Task 6: Section A manual smoke verification

**Files:** none (verification only)

- [ ] **Step 1: Verify in Tauri dev mode**

Run: `cargo tauri dev` from the worktree root.
Click `Add Project` → click the folder icon. Expect: native macOS folder dialog appears. Pick a folder. Expect: path field populates. Click Cancel.

- [ ] **Step 2: Verify in browser mode**

With Tauri still running, in Safari/Chrome navigate to `http://localhost:3001/`. Click `Add Project` → click the folder icon. Expect: `DirectoryBrowserModal` appears showing `/`. Click into a directory; the listing updates. Click `Use this folder` — path field populates.

- [ ] **Step 3: Note any issues**

If anything fails, file a fix task and resolve before moving to Section B.

---

### Task 7: Add `[ROLE:SET]` signal parsing test + impl

**Files:**
- Modify: `src/signals.rs`

- [ ] **Step 1: Add the failing test**

Append inside the existing `#[cfg(test)]` block at the bottom of `src/signals.rs` (or create one if absent — match existing style elsewhere in the crate):

```rust
#[cfg(test)]
mod role_signal_tests {
    use super::*;

    #[test]
    fn parses_role_set() {
        let s = parse_signal("[ROLE:SET role=\"brainstorm\"]").unwrap();
        assert_eq!(s, Signal::RoleSet { role: "brainstorm".into() });
        assert_eq!(s.kind(), "ROLE:SET");
        assert_eq!(s.payload().as_deref(), Some("brainstorm"));
    }

    #[test]
    fn rejects_role_with_no_value() {
        assert!(parse_signal("[ROLE:SET]").is_none());
    }

    #[test]
    fn rejects_role_with_unknown_key() {
        assert!(parse_signal("[ROLE:SET name=\"brainstorm\"]").is_none());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib signals 2>&1 | tail -15`
Expected: compile error — `Signal::RoleSet` does not exist.

- [ ] **Step 3: Add the variant**

In `src/signals.rs`, add a variant to the `Signal` enum (around line 14):

```rust
    SeedNew { id: String },
    RoleSet { role: String },
}
```

In the `kind()` match (around line 28):

```rust
            Self::SeedNew { .. } => "SEED:NEW",
            Self::RoleSet { .. } => "ROLE:SET",
        }
    }
```

In `payload()` (around line 37):

```rust
            Self::SeedNew { id } => Some(id.clone()),
            Self::RoleSet { role } => Some(role.clone()),
            _ => None,
        }
    }
```

In `parse_signal`'s parameterized match (around line 75), add an arm:

```rust
        "SEED:NEW" => {
            let id = extract_quoted(rest, "id")?;
            Some(Signal::SeedNew { id })
        }
        "ROLE:SET" => {
            let role = extract_quoted(rest, "role")?;
            Some(Signal::RoleSet { role })
        }
        _ => None,
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib signals 2>&1 | tail -10`
Expected: all signal tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/signals.rs
git commit -m "feat(signals): add ROLE:SET signal variant"
```

### Task 8: `Db::set_session_role`

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Locate the existing setter pattern**

Find `Db::set_session_autopilot` in `src/db.rs` — it's the closest analogue. Note its exact signature for consistency.

- [ ] **Step 2: Write the failing test**

Append to the `#[cfg(test)]` module at the bottom of `src/db.rs`:

```rust
    #[test]
    fn set_session_role_updates_row() {
        let (_tmp, db) = make_db();
        let project = db
            .add_project(NewProject {
                name: "p".into(),
                path: "/tmp/p".into(),
                remote_url: None,
            })
            .unwrap();
        let session = db
            .start_session(NewSession {
                project_id: project.id,
                role: "implementation".into(),
                branch: None,
                worktree_path: "/tmp/p".into(),
                pty_pid: None,
                task_summary: "t".into(),
            })
            .unwrap();

        db.set_session_role(session.id, "brainstorm").unwrap();

        let updated = db.get_session(session.id).unwrap();
        assert_eq!(updated.role, "brainstorm");
    }

    #[test]
    fn set_session_role_session_not_found() {
        let (_tmp, db) = make_db();
        let res = db.set_session_role(uuid::Uuid::new_v4(), "x");
        assert!(matches!(res, Err(crate::error::Error::SessionNotFound(_))));
    }
```

If `make_db`, `NewProject`, `NewSession` helpers in the test module use slightly different field names, adjust to match (the surrounding tests will show the canonical form).

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib db::tests::set_session_role 2>&1 | tail -10`
Expected: compile failure — method does not exist.

- [ ] **Step 4: Add the method**

Locate `Db::set_session_autopilot` in `src/db.rs`. Immediately after it, add:

```rust
    /// Replace the `role` column on a session row. Used by SignalPump when
    /// it observes a `[ROLE:SET role="..."]` signal so the UI can reflect
    /// mid-session role transitions.
    pub fn set_session_role(&self, id: Uuid, role: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let n = conn
            .execute(
                "UPDATE sessions SET role = ?1 WHERE id = ?2",
                rusqlite::params![role, id.to_string()],
            )
            .map_err(Error::Db)?;
        if n == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }
```

(If `rusqlite::params!` and `Error::Db` are not the patterns used by `set_session_autopilot`, mirror its exact form instead.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib db::tests::set_session_role 2>&1 | tail -10`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/db.rs
git commit -m "feat(db): set_session_role for live role transitions"
```

### Task 9: SignalPump persists role transitions

**Files:**
- Modify: `src/pty.rs`
- Modify: `tests/api_integration.rs` (or `src/pty.rs` integration test)

- [ ] **Step 1: Write the failing test**

Append a test to `tests/api_integration.rs` that uses the existing `mock-claude` fixture if available. If `mock-claude` does not have a way to emit a custom signal, an in-process unit test of the SignalPump task is acceptable.

Simplest viable test, in `src/pty.rs` `#[cfg(test)]` module (create one if absent — use existing tests in the crate as a template for spawning `mock-claude` and reading signals):

```rust
#[cfg(test)]
mod role_pump_tests {
    use super::*;
    use crate::db::{Db, NewProject, NewSession};
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn role_signal_writes_to_session_row() {
        // Arrange: in-memory db + a fake session row.
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open(tmp.path().join("test.sqlite")).unwrap();
        let project = db
            .add_project(NewProject {
                name: "p".into(),
                path: tmp.path().to_string_lossy().into_owned(),
                remote_url: None,
            })
            .unwrap();
        let session = db
            .start_session(NewSession {
                project_id: project.id,
                role: "implementation".into(),
                branch: None,
                worktree_path: tmp.path().to_string_lossy().into_owned(),
                pty_pid: None,
                task_summary: "t".into(),
            })
            .unwrap();

        // Spawn a pty running `printf` to emit the role signal exactly once
        // followed by EOF.
        let pty = PtyHandle::spawn(PtyOpts {
            command: "printf".into(),
            args: vec!["[ROLE:SET role=\"brainstorm\"]\n".into()],
            cwd: tmp.path().to_path_buf(),
            env: vec![],
        })
        .unwrap();

        let cycle: CycleSlot = Arc::new(std::sync::Mutex::new(None));
        let mut pump = SignalPump::start(pty, db.clone(), session.id, cycle);

        // Drain the channel so the task processes the line.
        let _ = tokio::time::timeout(Duration::from_secs(2), pump.next_signal()).await;

        // Allow the side-effect write to land.
        sleep(Duration::from_millis(50)).await;

        let updated = db.get_session(session.id).unwrap();
        assert_eq!(updated.role, "brainstorm");
    }
}
```

(Match the actual `PtyOpts` field names; reuse `tempfile` from existing dev-deps.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib role_pump 2>&1 | tail -15`
Expected: test runs but assertion fails — role still "implementation".

- [ ] **Step 3: Add the side effect to SignalPump**

In `src/pty.rs` around line 270 (the `tokio::spawn` task inside `SignalPump::start`), update the loop to also call `db.set_session_role` for `RoleSet`:

```rust
        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    let cur_cycle = *cycle_id.lock().unwrap();
                    let _ = db.record_signal(
                        session_id,
                        cur_cycle,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if let Signal::RoleSet { role } = &sig {
                        let _ = db.set_session_role(session_id, role);
                    }
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                }
            }
        });
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib role_pump 2>&1 | tail -10`
Expected: 1 test passes.

- [ ] **Step 5: Run the full Rust suite + clippy**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/pty.rs
git commit -m "feat(pty): SignalPump persists ROLE:SET to sessions.role"
```

### Task 10: `<RoleBadge>` component + tests

**Files:**
- Create: `ui/src/components/RoleBadge.tsx`
- Create: `ui/src/components/RoleBadge.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `ui/src/components/RoleBadge.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { RoleBadge } from "./RoleBadge";

describe("RoleBadge", () => {
  it.each([
    ["orchestrator", "ORCH"],
    ["brainstorm", "BRAIN"],
    ["implementation", "IMPL"],
    ["scout", "SCOUT"],
  ])("renders the abbreviation for known role %s", (role, abbr) => {
    render(<RoleBadge role={role} />);
    expect(screen.getByText(abbr)).toBeInTheDocument();
  });

  it("renders unknown role uppercased", () => {
    render(<RoleBadge role="something-weird" />);
    expect(screen.getByText("SOMETHING-WEIRD")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npm test RoleBadge 2>&1 | tail -10`
Expected: tests fail (component missing).

- [ ] **Step 3: Implement the component**

Create `ui/src/components/RoleBadge.tsx`:

```tsx
interface Props {
  role: string;
}

const STYLES: Record<string, { bg: string; fg: string; label: string }> = {
  orchestrator: { bg: "bg-purple-900", fg: "text-purple-200", label: "ORCH" },
  brainstorm: { bg: "bg-sky-900", fg: "text-sky-200", label: "BRAIN" },
  implementation: { bg: "bg-emerald-900", fg: "text-emerald-200", label: "IMPL" },
  scout: { bg: "bg-amber-900", fg: "text-amber-200", label: "SCOUT" },
};

export function RoleBadge({ role }: Props) {
  const style = STYLES[role.toLowerCase()] ?? {
    bg: "bg-neutral-800",
    fg: "text-neutral-300",
    label: role.toUpperCase(),
  };
  return (
    <span
      className={`inline-block text-[10px] font-bold tracking-wider px-1.5 py-0.5 rounded ${style.bg} ${style.fg}`}
    >
      {style.label}
    </span>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ui && npm test RoleBadge 2>&1 | tail -10`
Expected: 5/5 pass.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/RoleBadge.tsx ui/src/components/RoleBadge.test.tsx
git commit -m "feat(ui): RoleBadge for orchestrator/brainstorm/implementation/scout"
```

### Task 11: Wire `<RoleBadge>` into SessionRow + ExpandedSessionView

**Files:**
- Modify: `ui/src/components/SessionRow.tsx`
- Modify: `ui/src/components/ExpandedSessionView.tsx`

- [ ] **Step 1: Update SessionRow**

In `ui/src/components/SessionRow.tsx`, add the import:

```tsx
import { RoleBadge } from "./RoleBadge";
```

Replace the metadata div (the second child of the `flex-1 min-w-0` block, currently `<div className="text-xs text-gray-500 truncate">{session.role}…</div>`) with:

```tsx
        <div className="flex items-center gap-2 text-xs text-gray-500">
          <RoleBadge role={session.role} />
          {session.branch && <span className="truncate">{session.branch}</span>}
          <span>{age}</span>
        </div>
```

- [ ] **Step 2: Update ExpandedSessionView header**

In `ui/src/components/ExpandedSessionView.tsx`, add the import:

```tsx
import { RoleBadge } from "./RoleBadge";
```

Replace the header's `<strong>{session.role}</strong>` with:

```tsx
            <RoleBadge role={session.role} />{" "}
```

- [ ] **Step 3: Run the UI suite**

Run: `cd ui && npm test 2>&1 | tail -10`
Expected: all tests pass. (Existing SessionRow / ExpandedSessionView tests assert on summary text, not on the role string structure, so they should still pass — if any do break, update them to query the badge by abbreviation.)

- [ ] **Step 4: Build**

Run: `cd ui && npm run build 2>&1 | tail -5`
Expected: build succeeds.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/SessionRow.tsx ui/src/components/ExpandedSessionView.tsx
git commit -m "feat(ui): show RoleBadge in SessionRow and ExpandedSessionView header"
```

### Task 12: Drop role input from StartSessionModal

**Files:**
- Modify: `ui/src/components/modals/StartSessionModal.tsx`
- Modify: `ui/src/components/modals/StartSessionModal.test.tsx`

- [ ] **Step 1: Update the test**

In `ui/src/components/modals/StartSessionModal.test.tsx`, remove any `getByLabelText(/role/i)` lookups and any assertion that the form sends `role`. The two tests should reduce to: (a) submitting with task + (default) command works, calling `onSubmit` with `{ project_id, task, claude_cmd }` (no role); (b) Start button is disabled until a task is entered.

Replace the file body (keeping vitest imports) with the equivalent of:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StartSessionModal } from "./StartSessionModal";

describe("StartSessionModal", () => {
  it("submits task + claude command without a role field", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={onSubmit}
        projectId="proj-1"
      />,
    );
    await userEvent.type(screen.getByLabelText(/task/i), "do thing");
    await userEvent.click(screen.getByRole("button", { name: /^start$/i }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit).toHaveBeenCalledWith({
      project_id: "proj-1",
      task: "do thing",
      claude_cmd: "claude",
    });
  });

  it("disables Start until task is non-empty", async () => {
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={vi.fn()}
        projectId="proj-1"
      />,
    );
    const start = screen.getByRole("button", { name: /^start$/i });
    expect(start).toBeDisabled();
    await userEvent.type(screen.getByLabelText(/task/i), "x");
    expect(start).toBeEnabled();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npm test StartSessionModal 2>&1 | tail -15`
Expected: existing form sends a `role` key → first test fails.

- [ ] **Step 3: Update the component**

In `ui/src/components/modals/StartSessionModal.tsx`, remove the `role` state, remove the role `<label>` block, and drop `role` from the submitted body.

The relevant changes inside `StartSessionModal`:

```tsx
export function StartSessionModal({ open, onClose, onSubmit, projectId }: Props) {
  const [task, setTask] = useState("");
  const [cmd, setCmd] = useState("claude");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSubmit = task.trim().length > 0 && !busy;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit({
        project_id: projectId,
        task: task.trim(),
        claude_cmd: cmd.trim(),
      });
      setTask("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }
```

And remove the entire `<label>` block for "Role" (the one with the role `<input>`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ui && npm test StartSessionModal 2>&1 | tail -10`
Expected: 2/2 pass.

- [ ] **Step 5: Run full UI suite + build**

Run: `cd ui && npm test && npm run build && cd ..`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/modals/StartSessionModal.tsx ui/src/components/modals/StartSessionModal.test.tsx
git commit -m "refactor(ui): drop role input — role is now session state, not start config"
```

---

### Task 13: `SessionManager::write_pty`

**Files:**
- Modify: `src/session_manager.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)]` block at the bottom of `src/session_manager.rs` (or wherever session manager tests live in the crate — see Task 6 / earlier plans for the canonical test fixture):

```rust
    #[tokio::test]
    async fn write_pty_sends_input_to_running_session() {
        let (_tmp_data, mgr) = make_manager().await;
        let project = mgr
            .add_project(crate::db::NewProject {
                name: "p".into(),
                path: "/tmp".into(),
                remote_url: None,
            })
            .unwrap();

        // `cat` echoes whatever it reads on stdin to stdout.
        let session = mgr
            .spawn_session(
                project.id,
                "echo test".into(),
                "implementation".into(),
                "cat".into(),
                vec![],
            )
            .await
            .unwrap();

        let mut rx = mgr.subscribe_output(session.id).unwrap();
        mgr.write_pty(session.id, b"hello\n".to_vec()).await.unwrap();

        // cat will echo "hello\n" back.
        let payload = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let bytes = rx.recv().await.unwrap();
                if bytes.windows(5).any(|w| w == b"hello") {
                    return bytes;
                }
            }
        })
        .await
        .unwrap();
        assert!(payload.windows(5).any(|w| w == b"hello"));
    }

    #[tokio::test]
    async fn write_pty_returns_session_not_running() {
        let (_tmp_data, mgr) = make_manager().await;
        let res = mgr
            .write_pty(uuid::Uuid::new_v4(), b"x".to_vec())
            .await;
        assert!(matches!(res, Err(crate::error::Error::SessionNotRunning(_))));
    }
```

(Match the existing `make_manager` test helper signature; if absent, mirror the `spawn_test_server` helper in `tests/api_integration.rs`.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib write_pty 2>&1 | tail -15`
Expected: compile failure — `write_pty` does not exist.

- [ ] **Step 3: Implement the method**

In `src/session_manager.rs`, immediately after `resize_pty` (around line 410), add:

```rust
    /// Write bytes to the running session's pty (i.e. into the child
    /// process's stdin). Used by the terminal WebSocket for keystrokes.
    ///
    /// Holds the registry lock only long enough to clone the
    /// `PtyHandleShared`, then releases before awaiting the write — so
    /// other registry operations are not blocked on PTY backpressure.
    pub async fn write_pty(&self, id: Uuid, bytes: Vec<u8>) -> Result<()> {
        let handle = {
            let running = self.running.lock().unwrap();
            let rs = running.get(&id).ok_or(Error::SessionNotRunning(id))?;
            rs.handle.clone()
        };
        handle.write(&bytes).await
    }
```

(If the registry struct field is not `running` or the handle is not `rs.handle`, mirror the exact form used by `resize_pty` and `subscribe_output`.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib write_pty 2>&1 | tail -10`
Expected: 2/2 pass.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs
git commit -m "feat(session-manager): write_pty for terminal input forwarding"
```

### Task 14: `ClientFrame::Input` arm in terminal WS

**Files:**
- Modify: `src/api.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/api_integration.rs`:

```rust
#[tokio::test]
async fn terminal_ws_forwards_input_to_pty() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (addr, mgr, _data_dir) = spawn_test_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(
            project.id,
            "echo".into(),
            "implementation".into(),
            "cat".into(),
            vec![],
        )
        .await
        .unwrap();

    let url = format!("ws://{addr}/api/sessions/{}/terminal", session.id);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    ws.send(Message::Text(
        r#"{"type":"input","data":"ping\n"}"#.into(),
    ))
    .await
    .unwrap();

    let echoed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Binary(b) if b.windows(4).any(|w| w == b"ping") => return b,
                _ => continue,
            }
        }
    })
    .await
    .unwrap();
    assert!(echoed.windows(4).any(|w| w == b"ping"));
}
```

If `tokio-tungstenite` is not already a dev-dep, add it to `[dev-dependencies]`: `tokio-tungstenite = "0.21"`. (Check existing tests — it may already be present.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test api_integration terminal_ws_forwards_input 2>&1 | tail -15`
Expected: timeout — ping is never echoed because the input frame is dropped.

- [ ] **Step 3: Add the Input arm**

In `src/api.rs` around line 258, extend `ClientFrame`:

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientFrame {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
}
```

In `handle_terminal_ws` reader loop (around line 295), extend the match:

```rust
                        match frame {
                            ClientFrame::Resize { cols, rows } => {
                                let _ = mgr.resize_pty(session_id, cols, rows);
                            }
                            ClientFrame::Input { data } => {
                                let _ = mgr.write_pty(session_id, data.into_bytes()).await;
                            }
                        }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test api_integration terminal_ws_forwards_input 2>&1 | tail -10`
Expected: passes within 2s.

- [ ] **Step 5: Run the full Rust suite + clippy**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/api.rs tests/api_integration.rs
git commit -m "feat(api): ClientFrame::Input — forward keystrokes to pty"
```

### Task 15: `useTerminal` `onData` + `readOnly`

**Files:**
- Modify: `ui/src/hooks/useTerminal.ts`
- Create: `ui/src/hooks/useTerminal.test.ts`

- [ ] **Step 1: Write the failing test**

Create `ui/src/hooks/useTerminal.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useRef } from "react";
import { useTerminal } from "./useTerminal";

class FakeWebSocket {
  static OPEN = 1;
  static instances: FakeWebSocket[] = [];
  readyState = FakeWebSocket.OPEN;
  binaryType = "arraybuffer";
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: MessageEvent) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
    queueMicrotask(() => this.onopen?.());
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.onclose?.();
  }
}

describe("useTerminal", () => {
  let originalWS: typeof WebSocket;

  beforeEach(() => {
    FakeWebSocket.instances = [];
    originalWS = global.WebSocket;
    (global as unknown as { WebSocket: unknown }).WebSocket = FakeWebSocket;
  });
  afterEach(() => {
    (global as unknown as { WebSocket: unknown }).WebSocket = originalWS;
  });

  function setup(readOnly: boolean) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const { result } = renderHook(() => {
      const ref = useRef(container);
      return useTerminal(ref, "session-1", { readOnly });
    });
    return { container, result };
  }

  it("forwards xterm onData to a websocket input frame when interactive", async () => {
    const { result } = setup(false);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    const term = result.current.current!;
    // Simulate a keystroke
    (term as unknown as { _core?: unknown });
    // xterm exposes a public-ish path: `.input(data, true)` triggers onData listeners synchronously
    (term as unknown as { _core: { _onData: { fire: (s: string) => void } } })._core._onData.fire(
      "a",
    );

    const ws = FakeWebSocket.instances[0];
    expect(ws.sent.some((s) => s.includes('"type":"input"') && s.includes('"a"'))).toBe(
      true,
    );
  });

  it("does NOT send input frames when readOnly", async () => {
    const { result } = setup(true);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    const term = result.current.current!;
    (term as unknown as { _core: { _onData: { fire: (s: string) => void } } })._core._onData.fire(
      "a",
    );

    const ws = FakeWebSocket.instances[0];
    expect(ws.sent.some((s) => s.includes('"type":"input"'))).toBe(false);
  });
});
```

If poking xterm internals via `_core` proves brittle in the test runner, fall back to a wrapper: refactor `useTerminal` so the data-handler callback can be injected (a `__testHooks` parameter) and assert the callback is wired only when `!readOnly`. Keep the surface internal — do not export.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ui && npm test useTerminal 2>&1 | tail -15`
Expected: tests fail — `readOnly` parameter not honored, no input frame sent.

- [ ] **Step 3: Update the hook**

Replace the contents of `ui/src/hooks/useTerminal.ts` with:

```ts
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";

interface Options {
  readOnly?: boolean;
}

/**
 * Wires an xterm.js terminal to a GalvoLite session's PTY over the
 * `/api/sessions/:id/terminal` WebSocket.
 *
 *   - Server → client: raw PTY bytes (Binary frames), preserving ANSI.
 *   - Client → server: `{type:"resize",cols,rows}` on container resize,
 *                      `{type:"input",data}` on keystroke (if !readOnly).
 *   - Reconnects with backoff on close.
 */
export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  sessionId: string | null,
  opts: Options = {},
) {
  const termRef = useRef<Terminal | null>(null);
  const readOnlyRef = useRef<boolean>(!!opts.readOnly);

  useEffect(() => {
    readOnlyRef.current = !!opts.readOnly;
  }, [opts.readOnly]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !sessionId) return;

    const term = new Terminal({
      fontSize: 13,
      fontFamily:
        '"JetBrains Mono","Menlo","SF Mono",ui-monospace,monospace',
      theme: { background: "#0a0a0a", foreground: "#e5e5e5" },
      scrollback: 10_000,
      allowProposedApi: true,
      convertEol: false,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    container.replaceChildren();
    term.open(container);
    termRef.current = term;

    let ws: WebSocket | null = null;
    let cancelled = false;
    let reconnectAttempt = 0;
    let reconnectTimer: number | null = null;

    const sendResize = () => {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      const { cols, rows } = term;
      ws.send(JSON.stringify({ type: "resize", cols, rows }));
    };

    const safeFit = () => {
      requestAnimationFrame(() => {
        try {
          fit.fit();
          sendResize();
        } catch {
          /* renderer not ready yet */
        }
      });
    };

    // Wire input. We always attach the listener so a runtime readOnly
    // toggle takes effect without re-creating the terminal.
    term.onData((data) => {
      if (readOnlyRef.current) return;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify({ type: "input", data }));
    });

    const connect = () => {
      if (cancelled) return;
      const url = api.terminalWsUrl(sessionId);
      ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";

      ws.onopen = () => {
        reconnectAttempt = 0;
        safeFit();
      };
      ws.onmessage = (ev) => {
        if (ev.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(ev.data));
        } else if (typeof ev.data === "string") {
          term.write(ev.data);
        }
      };
      ws.onclose = () => {
        if (cancelled) return;
        const delay = Math.min(500 * 2 ** reconnectAttempt, 5000);
        reconnectAttempt += 1;
        reconnectTimer = window.setTimeout(connect, delay);
      };
      ws.onerror = () => {
        /* onclose drives reconnect */
      };
    };

    connect();

    const ro = new ResizeObserver(() => safeFit());
    ro.observe(container);
    window.addEventListener("resize", safeFit);

    return () => {
      cancelled = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      ro.disconnect();
      window.removeEventListener("resize", safeFit);
      if (ws && ws.readyState <= WebSocket.OPEN) ws.close();
      term.dispose();
      termRef.current = null;
    };
  }, [containerRef, sessionId]);

  return termRef;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ui && npm test useTerminal 2>&1 | tail -10`
Expected: 2/2 pass.

If `_core._onData.fire` access proves unreliable in jsdom, swap the test approach: dispatch a real keyboard event into the textarea xterm renders (`container.querySelector('textarea')`) and assert the same. Update the test, keep the hook unchanged.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useTerminal.ts ui/src/hooks/useTerminal.test.ts
git commit -m "feat(ui): useTerminal sends input frames; readOnly opt"
```

### Task 16: `<TerminalPreview>` accepts `readOnly`; ExpandedSessionView passes autopilot

**Files:**
- Modify: `ui/src/components/TerminalPreview.tsx`
- Modify: `ui/src/components/ExpandedSessionView.tsx`

- [ ] **Step 1: Update TerminalPreview**

Replace `ui/src/components/TerminalPreview.tsx` with:

```tsx
import { useRef } from "react";
import { useTerminal } from "../hooks/useTerminal";
import "@xterm/xterm/css/xterm.css";

interface Props {
  sessionId: string;
  mini?: boolean;
  readOnly?: boolean;
}

export function TerminalPreview({ sessionId, mini = false, readOnly = true }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  useTerminal(containerRef, sessionId, { readOnly });

  if (mini) {
    return (
      <div
        className="w-48 h-20 overflow-hidden rounded border border-gray-700 bg-black"
        style={{ transform: "scale(0.4)", transformOrigin: "top left", width: "480px", height: "200px" }}
      >
        <div ref={containerRef} className="w-full h-full" />
      </div>
    );
  }

  return (
    <div className="h-full w-full bg-black overflow-hidden">
      <div ref={containerRef} className="h-full w-full" />
    </div>
  );
}
```

- [ ] **Step 2: Pass `readOnly={session.autopilot}` from ExpandedSessionView**

In `ui/src/components/ExpandedSessionView.tsx`, change:

```tsx
            <TerminalPreview sessionId={session.id} />
```

to:

```tsx
            <TerminalPreview sessionId={session.id} readOnly={session.autopilot} />
```

- [ ] **Step 3: Run the UI suite + build**

Run: `cd ui && npm test && npm run build && cd ..`
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/TerminalPreview.tsx ui/src/components/ExpandedSessionView.tsx
git commit -m "feat(ui): interactive terminal in expanded view; gated on autopilot"
```

### Task 17: Section C manual verification

**Files:** none (verification only)

- [ ] **Step 1: Stop any running tauri dev, then re-launch**

Run: `cargo tauri dev`

- [ ] **Step 2: Drive a session interactively**

Add a project pointing at any directory you have. Start a session with `claude_cmd = "bash"` (use the Command field). Click the session row to expand. Click into the terminal area.

Expect:
- The cursor accepts focus.
- Typing `echo hi` and pressing Enter shows `hi` printed.
- Resizing the window updates xterm dimensions (no garbled rendering).

- [ ] **Step 3: Toggle autopilot and verify lockout**

Click the autopilot zap icon in the rail. The badge in SessionRow should turn green. Try typing into the terminal — keystrokes should NOT appear.

Toggle off; typing works again.

- [ ] **Step 4: Note any issues**

If anything fails, file a fix task and resolve before moving to Task 18.

### Task 18: Full verification battery + README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Run the full battery**

Run:
```bash
cargo test --workspace \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cd ui && npm test && npm run build && cd ..
```
Expected: all green.

- [ ] **Step 2: Update README**

In the section that documents the dashboard (added in Plan 2), add a "Plan 3 additions" subsection with these bullet points:

```markdown
### Plan 3 additions

- Folder picker works in both the Tauri desktop app (native dialog) and the browser (server-driven directory browser at `GET /api/fs/list`). Identical functionality on both surfaces — important for forthcoming Tailscale-exposed remote access.
- Roles (orchestrator / brainstorm / implementation / scout) are now session state, not start-time configuration. Sessions can transition by emitting the `[ROLE:SET role="..."]` signal; the dashboard updates the role badge live.
- The expanded session view is interactive: click into the terminal and type. Toggling Autopilot disables input (read-only mode).
- New endpoint: `GET /api/fs/list?path=<absolute>` — returns directory entries for the given absolute path.
- New signal: `[ROLE:SET role="<name>"]` — see `files/LUCIDITY-CLAUDE-MD-PROFILES.md`. Currently only the `implementation` profile body ships; other roles are accepted as state but use the same body until Plan 5.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: Plan 3 additions — cross-surface picker, role state, interactive terminal"
```

---

## Verification Summary

Once all 18 tasks are complete:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd ui && npm test && npm run build && cd ..
cargo tauri build  # optional release bundle check
```

All green + the manual smoke checks in Tasks 6 and 17 pass = Plan 3 ready to merge. Use the `superpowers:finishing-a-development-branch` skill to merge `plan-2-dashboard-crud-parity` (which carries Plan 2 + Plan 3) back to `main`. Memory entries to update on merge:
- `project_plan2_progress.md` → final status with merge commit
- `project_roadmap.md` → mark Plan 3 done, advance to Plan 4 (spawn/rejoin)
