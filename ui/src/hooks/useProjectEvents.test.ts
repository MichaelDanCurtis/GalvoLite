import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useProjectEvents } from "./useProjectEvents";

class MockWS {
  static all: MockWS[] = [];
  static last(): MockWS { return MockWS.all[MockWS.all.length - 1]; }
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  readyState = 1;
  static OPEN = 1;
  closed = false;
  constructor(public url: string) { MockWS.all.push(this); }
  send() {}
  close() { this.closed = true; }
}

describe("useProjectEvents", () => {
  let originalWS: typeof WebSocket;
  beforeEach(() => {
    MockWS.all = [];
    originalWS = globalThis.WebSocket;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = MockWS;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = originalWS;
  });

  it("opens a WS to the per-project events URL and forwards typed events", () => {
    const onEvent = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent }));
    expect(MockWS.last().url).toContain("/api/projects/p1/events");
    act(() => {
      MockWS.last().onmessage!({
        data: JSON.stringify({
          type: "github.issue_opened",
          project_id: "p1",
          issue_url: "u",
          issue_number: 1,
          title: "t",
        }),
      } as MessageEvent);
    });
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({
      type: "github.issue_opened",
      issue_number: 1,
    }));
  });

  it("drops malformed JSON silently", () => {
    const onEvent = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent }));
    act(() => {
      MockWS.last().onmessage!({ data: "not json" } as MessageEvent);
    });
    expect(onEvent).not.toHaveBeenCalled();
  });

  it("closes the WS on unmount", () => {
    const { unmount } = renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const ws = MockWS.last();
    unmount();
    expect(ws.closed).toBe(true);
  });

  it("reconnects on unexpected close (code != 1000)", () => {
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const firstWs = MockWS.last();
    expect(MockWS.all.length).toBe(1);
    act(() => {
      firstWs.onclose!({ code: 1006 }); // abnormal close
    });
    act(() => {
      vi.advanceTimersByTime(1100); // first backoff = 1000ms
    });
    expect(MockWS.all.length).toBe(2);
  });

  it("does NOT reconnect on normal close (code = 1000)", () => {
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const firstWs = MockWS.last();
    act(() => {
      firstWs.onclose!({ code: 1000 });
    });
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(MockWS.all.length).toBe(1);
  });

  it("calls onError after 10 failed reconnect attempts", () => {
    const onError = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn(), onError }));
    // Fail 10 times in a row, advancing past each backoff
    for (let i = 0; i < 10; i++) {
      act(() => { MockWS.last().onclose!({ code: 1006 }); });
      act(() => { vi.advanceTimersByTime(20000); });
    }
    expect(onError).toHaveBeenCalledTimes(1);
  });
});
