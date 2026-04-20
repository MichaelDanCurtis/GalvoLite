import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { CanvasAddon } from "@xterm/addon-canvas";
import { WebglAddon } from "@xterm/addon-webgl";
import { api } from "../api";

interface Options {
  readOnly?: boolean;
}

/**
 * In-place rewrite of UTF-8 bytes for U+2591 ░, U+2592 ▒, U+2593 ▓ to
 * U+2588 █ (full block). xterm's WebGL renderer draws ▒ as a literal dot
 * grid via its custom-glyph table (`"▒":[[1,0],[0,0],[0,1],[0,0]]`), which
 * is the "correct" Unicode rendering but doesn't match what Claude Code
 * expects — iTerm/Terminal.app render ▒ as a solid translucent block, and
 * Claude Code uses ▒ as a selection/highlight background. Substituting to
 * █ makes WebGL's `blockElementDefinitions` kick in and draw a solid 8×8
 * filled rect, which visually matches iTerm.
 *
 * UTF-8: ░=E2 96 91, ▒=E2 96 92, ▓=E2 96 93, █=E2 96 88. Same byte length,
 * so we just patch the third byte.
 *
 * Edge case: if a frame ends mid-codepoint (after E2 or E2 96), the last
 * byte arrives in the next frame — we won't catch that one. Rare enough
 * to be a non-issue; xterm itself buffers partial UTF-8 fine.
 */
export function mapKeystrokeForPty(data: string): string {
  // xterm.js emits "\n" for the Return key but PTYs (and Claude Code) want
  // "\r". Translate every LF to CR. CRLF stays as CRR which the line
  // discipline coalesces back to CR.
  return data.replace(/\n/g, "\r");
}

function substituteShadingToBlock(buf: Uint8Array): void {
  const n = buf.length - 2;
  for (let i = 0; i < n; i++) {
    if (
      buf[i] === 0xe2 &&
      buf[i + 1] === 0x96 &&
      buf[i + 2] >= 0x91 &&
      buf[i + 2] <= 0x93
    ) {
      buf[i + 2] = 0x88;
      i += 2;
    }
  }
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

    let disposed = false;
    let teardown: (() => void) | null = null;

    // xterm measures cell metrics from whatever font is loaded at construction
    // time. Await the webfont so JetBrains Mono glyphs render into a JBM-sized
    // cell, not a Menlo-sized one — otherwise Unicode block chars and pixel-art
    // look squished.
    void (async () => {
      try {
        await Promise.all([
          document.fonts.load('13px "JetBrains Mono"'),
          document.fonts.load('bold 13px "JetBrains Mono"'),
        ]);
      } catch {
        /* fall back to whatever is currently loaded */
      }
      if (disposed) return;

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
      // Renderer preference: WebGL → Canvas → DOM. WebGL is the only renderer
      // that draws block-element glyphs (▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐) as solid filled
      // rects via `blockElementDefinitions` — needed because we substitute
      // ▒ → █ in the incoming bytestream below to match iTerm-style solid
      // selection highlighting. StrictMode is disabled in main.tsx to keep
      // WebGL stable across HMR double-mounts.
      try {
        const webgl = new WebglAddon();
        // onContextLoss can fire DURING term.dispose() (which destroys the
        // GL context as a side effect), at which point the addon's internal
        // state is already torn down. Calling webgl.dispose() then throws
        // `Cannot read properties of undefined (reading '_isDisposed')`,
        // which propagates out of term.dispose() and crashes the React tree
        // (no error boundary above ExpandedSessionView). Guard the second
        // dispose so unmount doesn't blow up the whole UI.
        let webglDisposed = false;
        webgl.onContextLoss(() => {
          if (webglDisposed) return;
          webglDisposed = true;
          try { webgl.dispose(); } catch { /* already torn down */ }
        });
        term.loadAddon(webgl);
      } catch {
        try { term.loadAddon(new CanvasAddon()); } catch { /* DOM fallback */ }
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
        requestAnimationFrame(() => {
          try {
            fit.fit();
          } catch {
            /* renderer not ready yet — send anyway so the server still
               produces a snapshot; ResizeObserver will retry shortly. */
          }
          sendResize();
        });
      };

      const handleData = (data: string) => {
        if (readOnlyRef.current) return;
        if (!ws || ws.readyState !== WebSocket.OPEN) return;
        ws.send(JSON.stringify({ type: "input", data: mapKeystrokeForPty(data) }));
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
          // Always send a Resize on open: the server defers the snapshot
          // until the client reports its grid dims, so a missing Resize
          // here means a blank terminal on re-attach. fit() may not have
          // run yet, but sending current dims (defaults if so) at least
          // triggers the snapshot; safeFit re-sends with measured dims.
          sendResize();
          safeFit();
        };
        ws.onmessage = (ev) => {
          if (ev.data instanceof ArrayBuffer) {
            const u8 = new Uint8Array(ev.data);
            substituteShadingToBlock(u8);
            term.write(u8);
          } else if (typeof ev.data === "string") {
            term.write(ev.data.replace(/[\u2591\u2592\u2593]/g, "\u2588"));
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

      teardown = () => {
        cancelled = true;
        if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
        ro.disconnect();
        window.removeEventListener("resize", safeFit);
        if (ws && ws.readyState <= WebSocket.OPEN) ws.close();
        try { term.dispose(); } catch { /* renderer may throw on teardown */ }
        termRef.current = null;
      };
    })();

    return () => {
      disposed = true;
      teardown?.();
    };
  }, [containerRef, sessionId]);

  return termRef;
}
