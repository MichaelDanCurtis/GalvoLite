import { useState } from "react";
import { api } from "../../api";
import { Modal } from "./Modal";

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

  const submit = async () => {
    if (busy || repo.trim().length === 0) return;
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
    <Modal open={open} onClose={onClose} title="Link GitHub repo">
      <form
        className="flex flex-col gap-2 text-sm text-neutral-300"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <p className="text-sm text-neutral-400">
          Enter the repo as <code className="text-neutral-200">owner/name</code>.
        </p>
        <input
          type="text"
          placeholder="owner/name"
          value={repo}
          onChange={(e) => setRepo(e.target.value)}
          className="w-full text-sm bg-neutral-800 border border-neutral-700 rounded p-2"
          disabled={busy}
          autoFocus
        />
        {error && (
          <div className="text-xs text-red-400 whitespace-pre-wrap" role="alert">
            {error}
          </div>
        )}
        <div className="flex gap-2 justify-end mt-1">
          <button
            type="button"
            className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-white text-xs"
            onClick={onClose}
            disabled={busy}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs disabled:opacity-50"
            disabled={busy || repo.trim().length === 0}
          >
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}
