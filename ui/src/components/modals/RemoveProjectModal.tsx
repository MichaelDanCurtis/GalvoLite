import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onConfirm: () => Promise<unknown>;
  projectName: string;
  sessionCount: number;
}

export function RemoveProjectModal({
  open,
  onClose,
  onConfirm,
  projectName,
  sessionCount,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleConfirm() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Remove Project">
      <div className="flex flex-col gap-3 text-sm text-neutral-300">
        <p>
          Remove <strong className="text-neutral-100">{projectName}</strong> from
          GalvoLite?
        </p>
        <p className="text-xs text-neutral-400">
          This kills {sessionCount} session{sessionCount === 1 ? "" : "s"} and
          deletes all history (signals + cycles). The workspace directory on
          disk is not touched.
        </p>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <button
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-sm bg-red-700 hover:bg-red-600 disabled:opacity-50 rounded text-white"
            onClick={handleConfirm}
            disabled={busy}
          >
            {busy ? "Removing…" : "Remove"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
