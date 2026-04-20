import { useEffect, useRef } from "react";
import { api } from "../api";
import type { DomainEvent } from "../types";

/**
 * Subscribes to the per-session WebSocket and forwards DomainEvent JSON text
 * frames to the consumer. Binary frames (PTY bytes) are ignored — those
 * belong to useTerminal. Malformed JSON is dropped silently.
 *
 * The same `/api/sessions/:id/terminal` endpoint serves both binary PTY
 * frames (xterm's concern) and text DomainEvent frames (this hook's
 * concern). Opening an independent WebSocket here keeps the boundaries
 * clean — consumers outside the terminal view (e.g., Dashboard polling
 * replacement, global App-level toast/prompt) don't need xterm.
 */
export function useSessionEvents(
  sessionId: string | null,
  onEvent: (e: DomainEvent) => void,
) {
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  useEffect(() => {
    if (!sessionId) return;
    const ws = new WebSocket(api.terminalWsUrl(sessionId));
    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return; // binary = PTY bytes; ignore
      try {
        const parsed = JSON.parse(ev.data) as DomainEvent;
        if (parsed && typeof parsed === "object" && "type" in parsed) {
          onEventRef.current(parsed);
        }
      } catch {
        /* malformed; drop */
      }
    };
    return () => {
      try { ws.close(); } catch { /* already closed */ }
    };
  }, [sessionId]);
}
