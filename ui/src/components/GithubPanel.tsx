import { useEffect, useState, useRef, useCallback } from "react";
import { api } from "../api";
import type { GithubPr, GithubIssue } from "../types";

interface Props {
  projectId: string;
  repo: string; // "owner/name", just for display
}

/**
 * Plan 10a — per-project panel listing open PRs and open issues for the
 * linked repo. Manual refresh; no polling. Click row → opens github.com
 * in a new tab.
 */
export function GithubPanel({ projectId, repo }: Props) {
  const [prs, setPrs] = useState<GithubPr[] | null>(null);
  const [issues, setIssues] = useState<GithubIssue[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Track the latest in-flight fetch so a stale projectId-change response
  // can't overwrite a fresh one. Each fetch captures its own token; only
  // the run that still matches `latestRun.current` writes state.
  const latestRun = useRef(0);

  const fetchAll = useCallback(async () => {
    const myRun = ++latestRun.current;
    setBusy(true);
    setError(null);
    try {
      const [p, i] = await Promise.all([
        api.listGithubPrs(projectId),
        api.listGithubIssues(projectId),
      ]);
      if (latestRun.current === myRun) {
        setPrs(p);
        setIssues(i);
      }
    } catch (e) {
      if (latestRun.current === myRun) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (latestRun.current === myRun) {
        setBusy(false);
      }
    }
  }, [projectId]);

  useEffect(() => {
    fetchAll();
    // Bump the run counter on cleanup so a fetch in flight when projectId
    // changes can't write back to state with stale data.
    return () => { latestRun.current++; };
  }, [fetchAll]);

  return (
    <div className="border border-neutral-700 rounded p-3 mt-2">
      <div className="flex items-center justify-between mb-2">
        <div className="text-sm font-medium">
          GitHub: <span className="text-neutral-400">{repo}</span>
        </div>
        <button
          className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600 disabled:opacity-50"
          onClick={fetchAll}
          disabled={busy}
        >Refresh</button>
      </div>

      {error && (
        <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">
          Error: {error}
        </div>
      )}

      <div className="grid grid-cols-2 gap-3">
        <div>
          <h3 className="text-xs text-neutral-500 mb-1 font-normal">Open PRs</h3>
          {prs === null ? (
            <div className="text-xs text-neutral-600">Loading…</div>
          ) : prs.length === 0 ? (
            <div className="text-xs text-neutral-600">No open PRs.</div>
          ) : (
            <ul className="space-y-1">
              {prs.map((p) => (
                <li key={p.number} className="text-xs">
                  <a
                    href={p.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-400 hover:underline"
                  >#{p.number}</a>
                  {" "}
                  {p.draft && <span className="text-neutral-500">[draft]</span>}
                  {" "}
                  <span className="text-neutral-200">{p.title}</span>
                  <span className="text-neutral-500"> — @{p.author}</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div>
          <h3 className="text-xs text-neutral-500 mb-1 font-normal">Open issues</h3>
          {issues === null ? (
            <div className="text-xs text-neutral-600">Loading…</div>
          ) : issues.length === 0 ? (
            <div className="text-xs text-neutral-600">No open issues.</div>
          ) : (
            <ul className="space-y-1">
              {issues.map((i) => (
                <li key={i.number} className="text-xs">
                  <a
                    href={i.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-400 hover:underline"
                  >#{i.number}</a>
                  {" "}
                  <span className="text-neutral-200">{i.title}</span>
                  <span className="text-neutral-500"> — @{i.author}</span>
                  {i.labels.length > 0 && (
                    <span className="ml-1 text-neutral-500">
                      [{i.labels.join(", ")}]
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
