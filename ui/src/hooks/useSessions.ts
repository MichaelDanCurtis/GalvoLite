import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Session } from "../types";

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

  return { sessions, loading, error, refresh };
}
