import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { api } from "../api";

/**
 * Wires an xterm.js terminal to a GalvoLite session's PTY over the
 * `/api/sessions/:id/terminal` WebSocket.
 *
 * Contract:
 *   - Receives raw PTY bytes (Binary frames) and writes them verbatim.
 *     This preserves ANSI cursor / erase / SGR sequences.
 *   - Sends `{"type":"resize","cols":N,"rows":N}` Text frames whenever
 *     the container size changes (ResizeObserver + onopen). The backend
 *     forwards to `MasterPty::resize` so the child process sees SIGWINCH.
 *   - Reconnects silently on close (500ms backoff, capped at 5s).
 */
export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  sessionId: string | null,
) {
  const termRef = useRef<Terminal | null>(null);

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

    // WebGL is optional; fall back silently if the context fails to init
    // (headless CI / old GPUs / Tauri webview quirks).
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch {
      /* stay on DOM renderer */
    }

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
      // xterm's renderer initializes asynchronously; fit() before the
      // dimensions probe is ready panics. Defer to next frame.
      requestAnimationFrame(() => {
        try {
          fit.fit();
          sendResize();
        } catch {
          /* renderer not ready yet */
        }
      });
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
        // Silent reconnect with exponential backoff up to 5s.
        const delay = Math.min(500 * 2 ** reconnectAttempt, 5000);
        reconnectAttempt += 1;
        reconnectTimer = window.setTimeout(connect, delay);
      };
      ws.onerror = () => {
        // `onclose` will fire and drive the reconnect loop.
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
