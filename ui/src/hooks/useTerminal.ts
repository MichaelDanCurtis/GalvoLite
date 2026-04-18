import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";

interface Options {
  readOnly?: boolean;
}

/**
 * Wires an xterm.js terminal to a GalvoLite session's PTY over the
 * `/api/sessions/:id/terminal` WebSocket.
 *
 *   - Server → client: raw PTY bytes (Binary frames), preserving ANSI.
 *   - Client → server: `{type:"resize",cols,rows}` on container resize,
 *                      `{type:"input",data}` on keystroke (if !readOnly).
 *   - Reconnects with backoff on close.
 */
export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  sessionId: string | null,
  opts: Options = {},
) {
  const termRef = useRef<Terminal | null>(null);
  const readOnlyRef = useRef<boolean>(!!opts.readOnly);

  useEffect(() => {
    readOnlyRef.current = !!opts.readOnly;
  }, [opts.readOnly]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !sessionId) return;

    const term = new Terminal({
      fontSize: 13,
      fontFamily:
        '"JetBrains Mono","Menlo","SF Mono",ui-monospace,monospace',
      theme: { background: "#0a0a0a", foreground: "#e5e5e5" },
      scrollback: 10_000,
      allowProposedApi: true,
      convertEol: false,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    container.replaceChildren();
    term.open(container);
    termRef.current = term;

    let ws: WebSocket | null = null;
    let cancelled = false;
    let reconnectAttempt = 0;
    let reconnectTimer: number | null = null;

    const sendResize = () => {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      const { cols, rows } = term;
      ws.send(JSON.stringify({ type: "resize", cols, rows }));
    };

    const safeFit = () => {
      requestAnimationFrame(() => {
        try {
          fit.fit();
          sendResize();
        } catch {
          /* renderer not ready yet */
        }
      });
    };

    const handleData = (data: string) => {
      if (readOnlyRef.current) return;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(JSON.stringify({ type: "input", data }));
    };

    term.onData(handleData);

    // Internal test hooks — not exported, not part of the public API.
    (term as unknown as { __testHooks: { fireData: (s: string) => void } }).__testHooks = {
      fireData: handleData,
    };

    const connect = () => {
      if (cancelled) return;
      const url = api.terminalWsUrl(sessionId);
      ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";

      ws.onopen = () => {
        reconnectAttempt = 0;
        safeFit();
      };
      ws.onmessage = (ev) => {
        if (ev.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(ev.data));
        } else if (typeof ev.data === "string") {
          term.write(ev.data);
        }
      };
      ws.onclose = () => {
        if (cancelled) return;
        const delay = Math.min(500 * 2 ** reconnectAttempt, 5000);
        reconnectAttempt += 1;
        reconnectTimer = window.setTimeout(connect, delay);
      };
      ws.onerror = () => {
        /* onclose drives reconnect */
      };
    };

    connect();

    const ro = new ResizeObserver(() => safeFit());
    ro.observe(container);
    window.addEventListener("resize", safeFit);

    return () => {
      cancelled = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      ro.disconnect();
      window.removeEventListener("resize", safeFit);
      if (ws && ws.readyState <= WebSocket.OPEN) ws.close();
      term.dispose();
      termRef.current = null;
    };
  }, [containerRef, sessionId]);

  return termRef;
}
