import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { ExpandedSessionView } from "./ExpandedSessionView";
import type { Session } from "../types";

const stubSession: Session = {
  id: "sess-1",
  project_id: "p-1",
  role: "scout",
  branch: null,
  worktree_path: "/tmp/p",
  pty_pid: 123,
  state: "working",
  autopilot: false,
  parent_session_id: null,
  task_summary: "hello",
  created_at: "2026-04-17T00:00:00Z",
  ended_at: null,
};

describe("ExpandedSessionView", () => {
  it("renders session metadata", () => {
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    expect(screen.getByText(/SCOUT/)).toBeInTheDocument();
    expect(screen.getByText(/sess-1/)).toBeInTheDocument();
  });

  it("closes on backdrop click", async () => {
    const onClose = vi.fn();
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={onClose}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByTestId("expanded-backdrop"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes on ESC", async () => {
    const onClose = vi.fn();
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={onClose}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });
});

// Mock api + WebSocket for the refetch-on-unknown-step-hash test below.
// Kept in its own describe block so the unmocked tests above remain
// untouched.
class MockWS {
  static last: MockWS | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  readyState = 1;
  static OPEN = 1;
  constructor(public url: string) { MockWS.last = this; }
  send() {}
  close() {}
}

vi.mock("../api", () => ({
  api: {
    getSessionEvents: vi.fn(() => Promise.resolve([])),
    // Default resolves to []. Per-test `mockResolvedValueOnce` calls override.
    getSessionPlanSteps: vi.fn(() => Promise.resolve([])),
    terminalWsUrl: vi.fn(() => "ws://localhost/stub"),
  },
}));

describe("ExpandedSessionView — plan_step_changed unknown hash", () => {
  it("refetches plan when plan_step_changed arrives with an unknown hash", async () => {
    const { api } = await import("../api");
    const mockGetPlanSteps = api.getSessionPlanSteps as unknown as ReturnType<typeof vi.fn>;
    mockGetPlanSteps.mockReset();
    // First fetch: empty plan (step "h-new" hasn't shown up yet).
    mockGetPlanSteps.mockResolvedValueOnce([]);
    // Refetch (after unknown-hash event): authoritative list including the
    // new step.
    mockGetPlanSteps.mockResolvedValueOnce([
      { hash: "h-new", text: "appended late", status: "working" as const },
    ]);

    const originalWS = globalThis.WebSocket;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = MockWS;

    try {
      render(
        <ExpandedSessionView
          session={stubSession}
          onClose={() => {}}
          onSend={vi.fn()}
          onHandoff={vi.fn()}
          onToggleAutopilot={vi.fn()}
          onEnd={vi.fn()}
        />,
      );

      // Open Plan tab so we can observe rendered output.
      fireEvent.click(screen.getByRole("tab", { name: "Plan" }));

      // Wait for initial fetch to settle (empty → plan-empty visible).
      await waitFor(() =>
        expect(screen.getByTestId("plan-empty")).toBeInTheDocument(),
      );
      expect(mockGetPlanSteps).toHaveBeenCalledTimes(1);

      // Inject the WS event with an unknown hash.
      act(() => {
        MockWS.last!.onmessage!({
          data: JSON.stringify({
            type: "session.plan_step_changed",
            session_id: stubSession.id,
            step_hash: "h-new",
            prev_status: "pending",
            next_status: "working",
            note: null,
          }),
        } as MessageEvent);
      });

      // Component should refetch and apply the new authoritative list.
      await waitFor(() => expect(mockGetPlanSteps).toHaveBeenCalledTimes(2));
      await waitFor(() =>
        expect(screen.getByTestId("step-h-new")).toBeInTheDocument(),
      );
    } finally {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (globalThis as any).WebSocket = originalWS;
    }
  });
});
