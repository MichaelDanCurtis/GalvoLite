import { useEffect, useRef } from "react";
import { api } from "../api";
import type { DomainEvent } from "../types";

interface UseProjectEventsOptions {
  onEvent: (e: DomainEvent) => void;
  onError?: (reason: string) => void;
}

/**
 * Subscribes to the per-project WebSocket (Plan 10f) and forwards
 * DomainEvent JSON text frames whose `applies_to_project(projectId)` is
 * true on the server side. Auto-reconnects on unexpected close with
 * exponential backoff (1s, 2s, 4s, 8s, then capped at 16s) up to 10
 * total attempts; after exhaustion calls `onError(reason)` once.
 *
 * Normal close (code 1000) is treated as intentional — no reconnect.
 *
 * The hook is keyed by `projectId`; passing null disables the
 * subscription and closes any open WS.
 */
export function useProjectEvents(
  projectId: string | null,
  { onEvent, onError }: UseProjectEventsOptions,
) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  onEventRef.current = onEvent;
  onErrorRef.current = onError;

  useEffect(() => {
    if (!projectId) return;

    let cancelled = false;
    let attempt = 0;
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const open = () => {
      if (cancelled) return;
      ws = new WebSocket(api.projectEventsWsUrl(projectId));
      ws.onmessage = (ev) => {
        if (typeof ev.data !== "string") return;
        try {
          const parsed = JSON.parse(ev.data) as DomainEvent;
          if (parsed && typeof parsed === "object" && "type" in parsed) {
            onEventRef.current(parsed);
          }
        } catch {
          /* malformed; drop */
        }
      };
      ws.onclose = (ev) => {
        if (cancelled) return;
        // Normal close = no reconnect.
        if (ev.code === 1000) return;
        // 10 total attempts = 1 initial + 9 reconnects.
        // When attempt reaches 9, we've already retried 9 times; the next
        // close exhausts the budget.
        if (attempt >= 9) {
          onErrorRef.current?.(`reconnect exhausted after 10 attempts`);
          return;
        }
        // Backoff schedule: 1s, 2s, 4s, 8s, then cap at 16s for any further attempts.
        const backoffMs = Math.min(16000, 1000 * 2 ** attempt);
        attempt += 1;
        reconnectTimer = setTimeout(open, backoffMs);
      };
    };

    open();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      try { ws?.close(1000); } catch { /* already closed */ }
    };
  }, [projectId]);
}
