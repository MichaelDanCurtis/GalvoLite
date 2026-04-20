import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { SessionRow } from "./SessionRow";
import type { Session } from "../types";

function fakeSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "x",
    project_id: "p1",
    role: "implementation",
    branch: null,
    worktree_path: "/tmp/x",
    pty_pid: null,
    state: "idle",
    autopilot: false,
    parent_session_id: null,
    task_summary: "task",
    created_at: new Date().toISOString(),
    ended_at: null,
    ...overrides,
  };
}

const states = [
  "starting", "idle", "working", "awaiting_input",
  "needs_attention", "error", "ended",
] as const;

describe("SessionRow seven-state colors", () => {
  for (const s of states) {
    it(`renders a distinct dot for state=${s}`, () => {
      render(<SessionRow session={fakeSession({ state: s })} onRefresh={() => {}} />);
      const dot = screen.getByTestId("session-state-dot");
      expect(dot.getAttribute("data-state")).toBe(s);
    });
  }

  it("shows role badge abbreviation", () => {
    render(<SessionRow session={fakeSession({ role: "orchestrator" })} onRefresh={() => {}} />);
    expect(screen.getByText("ORCH")).toBeInTheDocument();
  });

  it("shows handoff tick when last_event_kind starts with mcp.handoff", () => {
    render(<SessionRow session={fakeSession({ last_event_kind: "mcp.handoff_ready" })} onRefresh={() => {}} />);
    expect(screen.getByTestId("handoff-tick")).toBeInTheDocument();
  });

  it("does not show handoff tick for other event kinds", () => {
    render(<SessionRow session={fakeSession({ last_event_kind: "system.session_started" })} onRefresh={() => {}} />);
    expect(screen.queryByTestId("handoff-tick")).toBeNull();
  });
});
