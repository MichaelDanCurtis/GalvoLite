import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useRef } from "react";
import { useTerminal } from "./useTerminal";

class FakeWebSocket {
  static OPEN = 1;
  static instances: FakeWebSocket[] = [];
  readyState = FakeWebSocket.OPEN;
  binaryType = "arraybuffer";
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: MessageEvent) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
    queueMicrotask(() => this.onopen?.());
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.onclose?.();
  }
}

describe("useTerminal", () => {
  let originalWS: typeof WebSocket;

  beforeEach(() => {
    FakeWebSocket.instances = [];
    originalWS = globalThis.WebSocket;
    (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeWebSocket;
  });
  afterEach(() => {
    (globalThis as unknown as { WebSocket: unknown }).WebSocket = originalWS;
  });

  function setup(readOnly: boolean) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const { result } = renderHook(() => {
      const ref = useRef(container);
      return useTerminal(ref, "session-1", { readOnly });
    });
    return { container, result };
  }

  it("forwards xterm onData to a websocket input frame when interactive", async () => {
    const { result } = setup(false);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });

    // Use __testHooks to invoke the onData callback directly
    const term = result.current.current!;
    const hooks = (term as unknown as { __testHooks?: { fireData: (s: string) => void } }).__testHooks;
    hooks?.fireData("a");

    const ws = FakeWebSocket.instances[0];
    expect(ws.sent.some((s) => s.includes('"type":"input"') && s.includes('"a"'))).toBe(true);
  });

  it("does NOT send input frames when readOnly", async () => {
    const { result } = setup(true);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });

    const term = result.current.current!;
    const hooks = (term as unknown as { __testHooks?: { fireData: (s: string) => void } }).__testHooks;
    hooks?.fireData("a");

    const ws = FakeWebSocket.instances[0];
    expect(ws.sent.some((s) => s.includes('"type":"input"'))).toBe(false);
  });
});
