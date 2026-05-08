import { useState } from "react";
import { api } from "../api";

interface Props {
  projectId: string;
  open: boolean;
  onClose: () => void;
  onSaved: (repo: string) => void;
}

/**
 * Plan 10a — per-project settings modal for linking a GitHub repo.
 * The backend validates the repo via GitHub before writing; on 422
 * we surface the error message inline.
 */
export function LinkRepoModal({ projectId, open, onClose, onSaved }: Props) {
  const [repo, setRepo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await api.linkGithubRepo(projectId, repo.trim());
      onSaved(res.repo);
      setRepo("");
      onClose();
    } catch (e) {
      // Try to extract the error body from the request helper's
      // "<status>: <text>" message format.
      const msg = e instanceof Error ? e.message : String(e);
      const match = msg.match(/\d{3}:\s*(\{.*\})/);
      if (match) {
        try {
          const body = JSON.parse(match[1]);
          setError(typeof body.error === "string" ? body.error : msg);
        } catch {
          setError(msg);
        }
      } else {
        setError(msg);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[420px]">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-lg font-semibold">Link GitHub repo</h2>
          <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
        </div>
        <p className="text-sm text-neutral-400 mb-2">
          Enter the repo as <code className="text-neutral-200">owner/name</code>.
        </p>
        <input
          type="text"
          placeholder="owner/name"
          value={repo}
          onChange={(e) => setRepo(e.target.value)}
          className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2 mb-2"
          disabled={busy}
        />
        {error && (
          <div className="text-xs text-red-400 mb-2 whitespace-pre-wrap">{error}</div>
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
            disabled={busy || repo.trim().length === 0}
          >Save</button>
        </div>
      </div>
    </div>
  );
}
