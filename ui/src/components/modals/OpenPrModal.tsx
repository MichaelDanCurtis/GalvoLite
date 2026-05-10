import { useEffect, useState } from "react";
import { api } from "../../api";
import { Modal } from "./Modal";

interface Props {
  sessionId: string;
  open: boolean;
  onClose: () => void;
  /** Called on success with (pr_url, pr_number). Parent typically closes
   *  the modal in this callback. */
  onCreated: (prUrl: string, prNumber: number) => void;
}

/**
 * Plan 10b — modal that fetches a draft PR (title/body/base) for a
 * session, lets the operator edit, and submits to /api/sessions/:id/pr.
 * Built on the shared `Modal` primitive (LinkRepoModal pattern).
 */
export function OpenPrModal({ sessionId, open, onClose, onCreated }: Props) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [base, setBase] = useState("");
  const [commitsAhead, setCommitsAhead] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    api.getPrDraft(sessionId)
      .then((draft) => {
        if (cancelled) return;
        setTitle(draft.title);
        setBody(draft.body);
        setBase(draft.base);
        setCommitsAhead(draft.commits_ahead);
        setLoading(false);
      })
      .catch((e) => {
        if (cancelled) return;
        const msg = e instanceof Error ? e.message : String(e);
        setError(extractErrorMessage(msg));
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [open, sessionId]);

  const submit = async () => {
    if (busy || title.trim().length === 0 || !commitsAhead || commitsAhead === 0) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.openPr(sessionId, { title: title.trim(), body, base: base.trim() });
      onCreated(res.pr_url, res.pr_number);
      onClose();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(extractErrorMessage(msg));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal open={open} onClose={onClose} title="Open PR for session">
      <div className="p-4 space-y-3 min-w-[480px] max-w-[640px]">
        {loading && (
          <div className="text-sm text-neutral-500">Loading draft…</div>
        )}
        {!loading && (
          <>
            <div>
              <label className="text-xs text-neutral-400 block mb-1">Title</label>
              <input
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2"
                disabled={busy}
                autoFocus
              />
            </div>
            <div>
              <label className="text-xs text-neutral-400 block mb-1">Base branch</label>
              <input
                type="text"
                value={base}
                onChange={(e) => setBase(e.target.value)}
                className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2"
                disabled={busy}
              />
            </div>
            <div>
              <label className="text-xs text-neutral-400 block mb-1">Body</label>
              <textarea
                value={body}
                onChange={(e) => setBody(e.target.value)}
                rows={12}
                className="w-full text-xs bg-neutral-800 border border-neutral-700 rounded p-2 font-mono"
                disabled={busy}
              />
            </div>
            <div className="text-xs text-neutral-500">
              {commitsAhead === 0
                ? "0 commits ahead of base — nothing to PR"
                : `${commitsAhead ?? "?"} commits ahead of ${base || "base"}`}
            </div>
            {error && (
              <div role="alert" className="text-xs text-red-400 whitespace-pre-wrap">
                {error}
              </div>
            )}
            <div className="flex gap-2 justify-end">
              <button
                className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-white text-xs"
                onClick={onClose}
                disabled={busy}
              >Cancel</button>
              <button
                className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs disabled:opacity-50"
                onClick={submit}
                disabled={busy || title.trim().length === 0 || commitsAhead === 0}
              >Submit</button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}

/** Parse the request helper's "<status>: <body>" error format and
 *  extract `body.error` if present. Falls back to the raw message. */
function extractErrorMessage(msg: string): string {
  const match = msg.match(/\d{3}:\s*(\{.*\})/);
  if (match) {
    try {
      const body = JSON.parse(match[1]);
      if (typeof body.error === "string") return body.error;
    } catch { /* fall through */ }
  }
  return msg;
}
