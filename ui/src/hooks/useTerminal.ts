import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";

export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  sessionId: string | null
) {
  const termRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!containerRef.current || !sessionId) return;

    const term = new Terminal({
      fontSize: 12,
      fontFamily: "Menlo, monospace",
      theme: {
        background: "#0a0a0a",
        foreground: "#e5e5e5",
      },
      scrollback: 5000,
      convertEol: true,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    containerRef.current.replaceChildren();
    term.open(containerRef.current);
    fitAddon.fit();
    termRef.current = term;

    const url = api.terminalWsUrl(sessionId);
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onmessage = (ev) => {
      term.writeln(ev.data);
    };

    ws.onclose = () => {
      term.writeln("\r\n[connection closed]");
    };

    const onResize = () => fitAddon.fit();
    window.addEventListener("resize", onResize);

    return () => {
      window.removeEventListener("resize", onResize);
      ws.close();
      term.dispose();
      termRef.current = null;
      wsRef.current = null;
    };
  }, [containerRef, sessionId]);

  return termRef;
}
