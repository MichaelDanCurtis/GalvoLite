import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExpandedSessionView } from "./ExpandedSessionView";
import { api } from "../api";
import type { Session } from "../types";

// Mock only the methods ExpandedSessionView touches. `terminalWsUrl` is used
// by useSessionEvents (which constructs a WebSocket) and by TerminalPreview's
// useTerminal hook.
vi.mock("../api", () => ({
  api: {
    getSessionEvents: vi.fn(),
    // ExpandedSessionView also fetches plan steps on mount (Task 14). Resolve
    // to an empty list so the Plan tab's effect finishes quietly and doesn't
    // race the Events-tab assertions below.
    getSessionPlanSteps: vi.fn(() => Promise.resolve([])),
    terminalWsUrl: vi.fn(() => "ws://localhost/stub"),
  },
}));

const mockGetSessionEvents = api.getSessionEvents as unknown as ReturnType<typeof vi.fn>;
const mockGetSessionPlanSteps = api.getSessionPlanSteps as unknown as ReturnType<typeof vi.fn>;

const stubSession: Session = {
  id: "sess-err",
  project_id: "p-1",
  role: "scout",
  branch: null,
  worktree_path: "/tmp/p",
  pty_pid: 1,
  state: "idle",
  autopilot: false,
  parent_session_id: null,
  task_summary: "",
  created_at: "2026-04-17T00:00:00Z",
  ended_at: null,
};

describe("ExpandedSessionView — events load error", () => {
  beforeEach(() => {
    mockGetSessionEvents.mockReset();
    mockGetSessionPlanSteps.mockReset();
    mockGetSessionPlanSteps.mockResolvedValue([]);
  });

  it("renders an error state when the initial events fetch fails", async () => {
    mockGetSessionEvents.mockRejectedValueOnce(new Error("boom"));
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onEnd={vi.fn()}
        onToggleAutopilot={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Events" }));
    await waitFor(() =>
      expect(screen.getByTestId("events-error")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("events-empty")).not.toBeInTheDocument();
  });

  it("shows the empty state (not the error) when the fetch returns []", async () => {
    mockGetSessionEvents.mockResolvedValueOnce([]);
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onEnd={vi.fn()}
        onToggleAutopilot={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Events" }));
    await waitFor(() =>
      expect(screen.getByTestId("events-empty")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("events-error")).not.toBeInTheDocument();
  });

  it("renders an error state when the initial plan_steps fetch fails", async () => {
    mockGetSessionEvents.mockResolvedValueOnce([]);
    mockGetSessionPlanSteps.mockReset();
    mockGetSessionPlanSteps.mockRejectedValueOnce(new Error("boom"));
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onEnd={vi.fn()}
        onToggleAutopilot={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Plan" }));
    await waitFor(() =>
      expect(screen.getByTestId("plan-error")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("plan-empty")).not.toBeInTheDocument();
  });

  it("renders fetched plan steps under the Plan tab", async () => {
    mockGetSessionEvents.mockResolvedValueOnce([]);
    mockGetSessionPlanSteps.mockReset();
    mockGetSessionPlanSteps.mockResolvedValueOnce([
      { hash: "h1", text: "first", status: "working" as const },
    ]);
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onEnd={vi.fn()}
        onToggleAutopilot={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: "Plan" }));
    await waitFor(() =>
      expect(screen.getByTestId("step-h1")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("step-h1")).toHaveAttribute("data-current", "true");
  });
});
