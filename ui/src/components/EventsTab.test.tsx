import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { EventsTab } from "./EventsTab";
import type { EventRow } from "../types";

describe("EventsTab", () => {
  it("renders newest-first rows with kind + source", () => {
    // Caller is expected to pass rows in newest-first order — the parent
    // gets them that way from the API (ORDER BY id DESC) and from the push
    // channel (we prepend). EventsTab does not resort.
    const events: EventRow[] = [
      { id: 2, kind: "hook.UserPromptSubmit", source: "hook", ts: "t2", payload: {} },
      { id: 1, kind: "system.session_started", source: "system", ts: "t1", payload: {} },
    ];
    render(<EventsTab events={events} />);
    const rows = screen.getAllByTestId("event-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("hook.UserPromptSubmit");
    expect(rows[0]).toHaveTextContent("hook");
    expect(rows[1]).toHaveTextContent("system.session_started");
    expect(rows[1]).toHaveTextContent("system");
  });

  it("renders an empty state when events is empty", () => {
    render(<EventsTab events={[]} />);
    expect(screen.queryAllByTestId("event-row")).toHaveLength(0);
    expect(screen.getByTestId("events-empty")).toBeInTheDocument();
  });

  it("shows the timestamp for each row", () => {
    const events: EventRow[] = [
      { id: 1, kind: "system.x", source: "system", ts: "2026-04-20T00:00:00Z", payload: {} },
    ];
    render(<EventsTab events={events} />);
    expect(screen.getByTestId("event-row")).toHaveTextContent("2026-04-20T00:00:00Z");
  });
});
