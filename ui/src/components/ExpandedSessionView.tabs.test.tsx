import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ExpandedSessionView } from "./ExpandedSessionView";
import type { Session } from "../types";

const stubSession: Session = {
  id: "sess-1",
  project_id: "p-1",
  role: "implementation",
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

function renderView() {
  return render(
    <ExpandedSessionView
      session={stubSession}
      onClose={() => {}}
      onSend={vi.fn()}
      onHandoff={vi.fn()}
      onEnd={vi.fn()}
      onToggleAutopilot={vi.fn()}
    />,
  );
}

describe("ExpandedSessionView tabs", () => {
  it("renders four tab labels", () => {
    renderView();
    for (const label of ["Terminal", "Events", "Plan", "Meta"]) {
      expect(screen.getByRole("tab", { name: label })).toBeInTheDocument();
    }
  });

  it("starts on Terminal tab", () => {
    renderView();
    expect(screen.getByRole("tab", { name: "Terminal" })).toHaveAttribute("aria-selected", "true");
  });

  it("activates Events tab on click", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "Events" }));
    expect(screen.getByTestId("tab-panel-events")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Events" })).toHaveAttribute("aria-selected", "true");
  });

  it("activates Plan tab on click", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "Plan" }));
    expect(screen.getByTestId("tab-panel-plan")).toBeInTheDocument();
  });

  it("activates Meta tab on click", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "Meta" }));
    expect(screen.getByTestId("tab-panel-meta")).toBeInTheDocument();
  });
});
