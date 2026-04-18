import { render, screen } from "@testing-library/react";
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
