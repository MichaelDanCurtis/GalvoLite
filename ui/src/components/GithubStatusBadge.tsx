import { useEffect, useState } from "react";
import { api } from "../api";
import type { GithubStatus } from "../types";

/**
 * Plan 10a — Dashboard-header indicator. Polls /api/github/status once
 * on mount; no live refresh. Operators rotate tokens by restarting
 * the server.
 */
export function GithubStatusBadge() {
  const [status, setStatus] = useState<GithubStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.githubStatus()
      .then((s) => { if (!cancelled) setStatus(s); })
      .catch(() => {
        if (!cancelled) {
          setStatus({ connected: false, user: null, reason: "fetch failed" });
        }
      });
    return () => { cancelled = true; };
  }, []);

  if (!status) {
    // Pre-resolve placeholder. Width holds the slot so nothing reflows
    // when the badge fills in.
    return <span className="text-xs text-neutral-600 px-2" aria-hidden="true">…</span>;
  }

  if (status.connected && status.user) {
    return (
      <span
        className="text-xs px-2 py-0.5 rounded bg-neutral-800 text-neutral-300"
        title="GitHub connected"
      >
        @{status.user.login}
      </span>
    );
  }

  return (
    <span
      className="text-xs px-2 py-0.5 rounded bg-neutral-800 text-yellow-500"
      title={`GitHub: ${status.reason ?? "not connected"}`}
    >
      GitHub: not connected
    </span>
  );
}
