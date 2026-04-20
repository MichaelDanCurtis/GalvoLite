import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessionEvents } from "./useSessionEvents";

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

describe("useSessionEvents", () => {
  let originalWS: typeof WebSocket;
  beforeEach(() => {
    originalWS = globalThis.WebSocket;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = MockWS;
  });
  afterEach(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = originalWS;
  });

  it("parses text frames and forwards typed events to the consumer", () => {
    const onEvent = vi.fn();
    renderHook(() => useSessionEvents("00000000-0000-0000-0000-000000000001", onEvent));
    act(() => {
      MockWS.last!.onmessage!({
        data: JSON.stringify({
          type: "operator.toast",
          session_id: "00000000-0000-0000-0000-000000000001",
          level: "info",
          message: "hi",
        }),
      } as MessageEvent);
    });
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({ type: "operator.toast", message: "hi" }));
  });

  it("ignores binary frames", () => {
    const onEvent = vi.fn();
    renderHook(() => useSessionEvents("sid", onEvent));
    act(() => {
      MockWS.last!.onmessage!({ data: new ArrayBuffer(8) } as unknown as MessageEvent);
    });
    expect(onEvent).not.toHaveBeenCalled();
  });

  it("drops malformed JSON silently", () => {
    const onEvent = vi.fn();
    renderHook(() => useSessionEvents("sid", onEvent));
    act(() => {
      MockWS.last!.onmessage!({ data: "not json" } as MessageEvent);
    });
    expect(onEvent).not.toHaveBeenCalled();
  });
});
