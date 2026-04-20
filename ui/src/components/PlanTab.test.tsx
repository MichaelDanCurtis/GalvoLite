import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { PlanTab } from "./PlanTab";
import type { PlanStep } from "../types";

const steps: PlanStep[] = [
  { hash: "h1", text: "do thing",   status: "done" },
  { hash: "h2", text: "do other",   status: "working" },
  { hash: "h3", text: "future",     status: "pending" },
];

describe("PlanTab", () => {
  it("highlights the current working step", () => {
    render(<PlanTab steps={steps} />);
    expect(screen.getByTestId("step-h2")).toHaveAttribute("data-current", "true");
  });

  it("renders correct status pill for each", () => {
    render(<PlanTab steps={steps} />);
    expect(screen.getByTestId("step-h1")).toHaveTextContent("\u25CF"); // done ●
    expect(screen.getByTestId("step-h2")).toHaveTextContent("\u25D0"); // working ◐
    expect(screen.getByTestId("step-h3")).toHaveTextContent("\u25CB"); // pending ○
  });

  it("renders the blocked status glyph", () => {
    const blocked: PlanStep[] = [
      { hash: "hb", text: "stuck", status: "blocked" },
    ];
    render(<PlanTab steps={blocked} />);
    // ◆ is the blocked glyph per the Sub-plan 4 Task 14 spec.
    expect(screen.getByTestId("step-hb")).toHaveTextContent("\u25C6");
  });

  it("renders an empty state when no steps are present", () => {
    render(<PlanTab steps={[]} />);
    expect(screen.getByTestId("plan-empty")).toBeInTheDocument();
    // No step rows at all.
    expect(screen.queryByTestId(/^step-/)).toBeNull();
  });

  it("collapses done steps under a <details> element by default", () => {
    render(<PlanTab steps={steps} />);
    const details = screen.getByTestId("plan-done-details");
    // `open` attribute is absent when the <details> starts collapsed.
    expect(details).not.toHaveAttribute("open");
    // Summary should advertise the count.
    expect(details).toHaveTextContent(/Completed \(1\)/);
    // The done step is still rendered (inside the details), but not the
    // non-done ones under it.
    expect(screen.getByTestId("step-h1")).toBeInTheDocument();
  });

  it("marks only one working step as current even if multiple exist", () => {
    const many: PlanStep[] = [
      { hash: "a", text: "a", status: "working" },
      { hash: "b", text: "b", status: "working" },
    ];
    render(<PlanTab steps={many} />);
    expect(screen.getByTestId("step-a")).toHaveAttribute("data-current", "true");
    // Second working step does not receive the data-current flag.
    expect(screen.getByTestId("step-b")).not.toHaveAttribute("data-current");
  });
});
