import { useEffect, useRef, useState, useCallback } from "react";
import { api } from "../api";
import type { PrReviewsBundle } from "../types";

interface Props {
  sessionId: string;
}

/**
 * Plan 10c — per-session PR reviews + inline comments panel.
 * Manual refresh; no polling. Mounted in ExpandedSessionView when
 * the session has a pr_url.
 */
export function PrReviewsPanel({ sessionId }: Props) {
  const [bundle, setBundle] = useState<PrReviewsBundle | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Monotonic token to ignore stale fetch responses (e.g. user clicks
  // Refresh twice in rapid succession; only the latest result wins).
  const tokenRef = useRef(0);

  const fetchAll = useCallback(async () => {
    const myToken = ++tokenRef.current;
    setBusy(true);
    setError(null);
    try {
      const res = await api.getPrReviews(sessionId);
      if (myToken === tokenRef.current) setBundle(res);
    } catch (e) {
      if (myToken === tokenRef.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (myToken === tokenRef.current) setBusy(false);
    }
  }, [sessionId]);

  useEffect(() => {
    fetchAll();
    return () => {
      // Bump the token so any in-flight fetch becomes a no-op on resolve.
      ++tokenRef.current;
    };
  }, [fetchAll]);

  return (
    <div className="border border-neutral-700 rounded p-3 mt-2">
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-sm font-medium">PR Reviews</h3>
        <button
          className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600 disabled:opacity-50"
          onClick={fetchAll}
          disabled={busy}
        >
          Refresh
        </button>
      </div>

      {error && (
        <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">
          Error: {error}
        </div>
      )}

      {bundle === null && !error && (
        <div className="text-xs text-neutral-600">Loading…</div>
      )}

      {bundle && bundle.reviews.length === 0 && bundle.comments.length === 0 && (
        <div className="text-xs text-neutral-600">No reviews yet.</div>
      )}

      {bundle && bundle.reviews.length > 0 && (
        <div className="mb-3">
          <div className="text-xs text-neutral-500 mb-1">Reviews</div>
          <ul className="space-y-2">
            {bundle.reviews.map((r) => (
              <li key={r.id} className="text-xs">
                <div>
                  <a
                    href={r.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-400 hover:underline"
                  >
                    @{r.author}
                  </a>
                  <span
                    className={`ml-2 px-1 rounded text-[10px] uppercase ${
                      r.state === "APPROVED"
                        ? "bg-emerald-900 text-emerald-300"
                        : r.state === "CHANGES_REQUESTED"
                        ? "bg-red-900 text-red-300"
                        : "bg-neutral-800 text-neutral-300"
                    }`}
                  >
                    {r.state}
                  </span>
                </div>
                {r.body && (
                  <div className="text-neutral-300 mt-1 whitespace-pre-wrap">
                    {r.body}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {bundle && bundle.comments.length > 0 && (
        <div>
          <div className="text-xs text-neutral-500 mb-1">Inline comments</div>
          <ul className="space-y-2">
            {bundle.comments.map((c) => (
              <li key={c.id} className="text-xs">
                <div>
                  <a
                    href={c.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-400 hover:underline"
                  >
                    @{c.author}
                  </a>
                  <span className="ml-2 text-neutral-500 font-mono">
                    {c.path}
                    {c.line != null ? `:${c.line}` : ""}
                  </span>
                </div>
                <div className="text-neutral-300 mt-1 whitespace-pre-wrap">
                  {c.body}
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
