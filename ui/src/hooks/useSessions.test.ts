import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessions } from "./useSessions";

describe("useSessions session.children_resolved", () => {
  it("clears waiting_on on the parent", async () => {
    const { result } = renderHook(() => useSessions());
    act(() => {
      result.current._seed?.([
        { id: "p", waiting_on: ["c1", "c2"] } as any,
      ]);
    });
    act(() => {
      result.current.applyEvent({
        type: "session.children_resolved",
        session_id: "p",
        children: [
          { id: "c1", role: "implementation", status: "complete", handoff: "/h.md" },
          { id: "c2", role: "implementation", status: "complete", handoff: null },
        ],
      });
    });
    const parent = result.current.sessions.find((s) => s.id === "p");
    expect(parent?.waiting_on).toBeNull();
  });
});
