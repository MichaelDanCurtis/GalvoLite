import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (text: string) => Promise<unknown>;
}

export function SendModal({ open, onClose, onSubmit }: Props) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(text);
      setText("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Send to Session">
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <textarea
          className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono min-h-[120px]"
          value={text}
          onChange={(e) => setText(e.target.value)}
          autoFocus
          placeholder="Type message to send…"
        />
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
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
            disabled={!text.trim() || busy}
          >
            {busy ? "Sending…" : "Send"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
