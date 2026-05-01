import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SessionRow } from "./SessionRow";
import type { Session } from "../types";

const baseSession: Session = {
  id: "s1", project_id: "p1", role: "implementation",
  branch: null, worktree_path: "/tmp/wt", pty_pid: null,
  state: "idle", autopilot: false,
  parent_session_id: null, task_summary: "task",
  created_at: new Date().toISOString(), ended_at: null,
};

describe("SessionRow", () => {
  it("renders depth padding inline style", () => {
    const { container } = render(
      <SessionRow session={baseSession} depth={2} onRefresh={() => {}} />
    );
    expect(container.firstChild).toHaveStyle({ paddingLeft: "48px" });
  });
  // Regression: smoke bug 1 — Tailwind `px-4` (16px) collided with inline
  // `paddingLeft: depth * 16`, so depth=1 rendered identically to depth=0.
  // Lock the invariant numerically: each depth level must increase paddingLeft.
  it("paddingLeft strictly increases with depth", () => {
    const px = (depth: number) => {
      const { container } = render(
        <SessionRow session={baseSession} depth={depth} onRefresh={() => {}} />
      );
      return parseInt(
        (container.firstChild as HTMLElement).style.paddingLeft,
        10
      );
    };
    const p0 = px(0);
    const p1 = px(1);
    const p2 = px(2);
    expect(p1).toBeGreaterThan(p0);
    expect(p2).toBeGreaterThan(p1);
  });
  it("renders waiting badge when waiting_on populated", () => {
    const s: Session = { ...baseSession, waiting_on: ["c1", "c2", "c3"] };
    render(<SessionRow session={s} onRefresh={() => {}} />);
    expect(screen.getByTestId("waiting-badge")).toHaveTextContent("waiting on 3");
  });
  it("omits waiting badge when waiting_on is null", () => {
    render(<SessionRow session={baseSession} onRefresh={() => {}} />);
    expect(screen.queryByTestId("waiting-badge")).toBeNull();
  });
});
