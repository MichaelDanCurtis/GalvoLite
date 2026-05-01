import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Dashboard } from "./Dashboard";

vi.mock("../hooks/useSessions", () => ({
  useSessions: () => ({
    sessions: [
      { id: "P", parent_session_id: null, task_summary: "parent", state: "idle", role: "orchestrator", autopilot: false, created_at: new Date().toISOString(), project_id: "x", branch: null, worktree_path: "/", pty_pid: null, ended_at: null },
      { id: "C", parent_session_id: "P", task_summary: "child", state: "idle", role: "implementation", autopilot: false, created_at: new Date().toISOString(), project_id: "x", branch: null, worktree_path: "/", pty_pid: null, ended_at: null },
    ],
    loading: false, error: null,
    refresh: () => {}, applyEvent: () => {},
    createSession: async () => ({}), sendText: async () => {},
    triggerHandoff: async () => {}, killSession: async () => {},
    toggleAutopilot: async () => {},
  }),
}));
vi.mock("../hooks/useProjects", () => ({
  useProjects: () => ({ projects: [{ id: "x", name: "x", path: "/", remote_url: null, created_at: new Date().toISOString() }], loading: false, error: null, createProject: async () => ({}), removeProject: async () => {} }),
}));
vi.mock("../hooks/useSessionEvents", () => ({
  useSessionEvents: () => {},
}));

describe("Dashboard nesting", () => {
  it("renders child indented under parent", () => {
    render(<Dashboard />);
    const rows = screen.getAllByText(/parent|child/);
    expect(rows[0].textContent).toContain("parent");
    expect(rows[1].textContent).toContain("child");
  });
});
