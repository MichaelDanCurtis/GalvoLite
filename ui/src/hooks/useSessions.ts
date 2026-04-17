import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Session, CreateSessionReq } from "../types";

export function useSessions(pollIntervalMs = 2000) {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  const refresh = () => {
    api
      .listSessions()
      .then((data) => {
        setSessions(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
    // TODO: at small intervals an older response can race a newer one and overwrite it; default 2s is safe.
    intervalRef.current = setInterval(refresh, pollIntervalMs);
    return () => clearInterval(intervalRef.current);
  }, [pollIntervalMs]);

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

  return { sessions, loading, error, refresh, createSession, sendText, triggerHandoff, killSession, toggleAutopilot };
}
