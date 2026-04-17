import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onConfirm: () => Promise<unknown>;
  role: string;
}

export function EndSessionModal({ open, onClose, onConfirm, role }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handle() {
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
    <Modal open={open} onClose={onClose} title="End Session">
      <div className="flex flex-col gap-3 text-sm text-neutral-300">
        <p>
          Send SIGTERM to this <strong>{role}</strong> session. History is
          preserved.
        </p>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-sm bg-red-700 hover:bg-red-600 disabled:opacity-50 rounded text-white"
            onClick={handle}
            disabled={busy}
          >
            {busy ? "Ending…" : "End"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
