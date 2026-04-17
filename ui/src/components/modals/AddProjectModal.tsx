import { useState } from "react";
import { FolderOpen } from "lucide-react";
import { Modal } from "./Modal";

async function pickDirectory(): Promise<string | null> {
  const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({ directory: true, multiple: false });
  return typeof result === "string" ? result : null;
}

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (req: { name: string; path: string }) => Promise<unknown>;
}

export function AddProjectModal({ open, onClose, onSubmit }: Props) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const canSubmit = name.trim().length > 0 && path.trim().length > 0 && !busy;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ name: name.trim(), path: path.trim() });
      setName("");
      setPath("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Add Project">
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Name</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Path</span>
          <div className="flex gap-1">
            <input
              className="flex-1 bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/absolute/path/to/workspace"
            />
            <button
              type="button"
              title="Browse…"
              aria-label="Browse for folder"
              className="px-2 py-1 border border-neutral-700 rounded text-neutral-300 hover:text-neutral-100 hover:border-neutral-500"
              onClick={async () => {
                const picked = await pickDirectory();
                if (picked) setPath(picked);
              }}
            >
              <FolderOpen size={14} />
            </button>
          </div>
        </label>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            disabled={!canSubmit}
          >
            {busy ? "Adding…" : "Add"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
