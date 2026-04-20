import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessions } from "./useSessions";

vi.mock("../api", () => ({
  api: {
    listSessions: vi.fn().mockResolvedValue([
      {
        id: "s1",
        project_id: "p1",
        role: "implementation",
        branch: null,
        worktree_path: "/tmp/s1",
        pty_pid: 100,
        state: "idle",
        autopilot: false,
        parent_session_id: null,
        task_summary: "",
        created_at: "t0",
        ended_at: null,
      },
    ]),
  },
}));

describe("useSessions push updates", () => {
  it("applies session.state_changed without re-fetching", async () => {
    const { result } = renderHook(() => useSessions());
    await act(async () => {});
    expect(result.current.sessions[0].state).toBe("idle");
    act(() => {
      result.current.applyEvent({
        type: "session.state_changed",
        session_id: "s1", prev: "idle", next: "working", last_event_ts: "t",
      });
    });
    expect(result.current.sessions[0].state).toBe("working");
  });

  it("applies session.role_changed", async () => {
    const { result } = renderHook(() => useSessions());
    await act(async () => {});
    act(() => {
      result.current.applyEvent({
        type: "session.role_changed",
        session_id: "s1", role: "scout",
      });
    });
    expect(result.current.sessions[0].role).toBe("scout");
  });

  it("applies session.meta_changed", async () => {
    const { result } = renderHook(() => useSessions());
    await act(async () => {});
    act(() => {
      result.current.applyEvent({
        type: "session.meta_changed",
        session_id: "s1",
        turn_count: 5,
        est_tokens: 1234,
        plan_markdown: "## plan",
        seed_markdown: null,
        handoff_path: null,
      });
    });
    expect(result.current.sessions[0].turn_count).toBe(5);
    expect(result.current.sessions[0].est_tokens).toBe(1234);
    expect(result.current.sessions[0].plan_markdown).toBe("## plan");
  });

  it("ignores events for unknown session ids", async () => {
    const { result } = renderHook(() => useSessions());
    await act(async () => {});
    const snapshot = result.current.sessions;
    act(() => {
      result.current.applyEvent({
        type: "session.state_changed",
        session_id: "unknown", prev: "idle", next: "working", last_event_ts: "t",
      });
    });
    // Should not mutate the array
    expect(result.current.sessions).toBe(snapshot);
  });
});
