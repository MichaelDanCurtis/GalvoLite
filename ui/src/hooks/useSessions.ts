import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Session, CreateSessionReq, DomainEvent } from "../types";

export function useSessions() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .listSessions()
      .then((data) => {
        setSessions(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const applyEvent = useCallback((e: DomainEvent) => {
    setSessions((prev) => {
      // All handled DomainEvent variants carry session_id
      const sessionId =
        "session_id" in e ? (e as { session_id: string }).session_id : null;
      if (!sessionId) return prev;

      const i = prev.findIndex((s) => s.id === sessionId);
      if (i === -1) return prev;

      const session = prev[i];

      switch (e.type) {
        case "session.state_changed": {
          const updated = { ...session, state: e.next };
          const next = [...prev];
          next[i] = updated;
          return next;
        }
        case "session.role_changed": {
          const updated = { ...session, role: e.role };
          const next = [...prev];
          next[i] = updated;
          return next;
        }
        case "session.meta_changed": {
          const updated = {
            ...session,
            turn_count: e.turn_count,
            est_tokens: e.est_tokens,
            plan_markdown: e.plan_markdown,
            seed_markdown: e.seed_markdown,
            handoff_path: e.handoff_path,
          };
          const next = [...prev];
          next[i] = updated;
          return next;
        }
        case "session.children_resolved": {
          const updated = { ...session, waiting_on: null };
          const next = [...prev];
          next[i] = updated;
          return next;
        }
        default:
          return prev;
      }
    });
  }, []);

  const createSession = async (req: CreateSessionReq): Promise<Session> => {
    const s = await api.createSession(req);
    refresh();
    return s;
  };

  const sendText = (id: string, text: string) =>
    api.sendToSession(id, { text, newline: true });

  const triggerHandoff = (id: string) => api.triggerHandoff(id);

  const killSession = async (id: string): Promise<void> => {
    await api.killSession(id);
    refresh();
  };

  const toggleAutopilot = async (id: string, enabled: boolean): Promise<void> => {
    await api.setAutopilot(id, enabled);
    refresh();
  };

  return {
    sessions,
    loading,
    error,
    refresh,
    applyEvent,
    createSession,
    sendText,
    triggerHandoff,
    killSession,
    toggleAutopilot,
    // Test-only seam: lets vitest seed sessions without round-tripping the API.
    _seed: setSessions,
  };
}
