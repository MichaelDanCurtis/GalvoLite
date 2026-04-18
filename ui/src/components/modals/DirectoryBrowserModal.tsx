import { useEffect, useState, useCallback } from "react";
import { ArrowUp, Folder } from "lucide-react";
import { Modal } from "./Modal";
import { api } from "../../api";
import type { DirectoryListing } from "../../types";

interface Props {
  open: boolean;
  initialPath: string;
  onClose: () => void;
  onPick: (path: string) => void;
}

export function DirectoryBrowserModal({ open, initialPath, onClose, onPick }: Props) {
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.listDirectory(path);
      setListing(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) load(initialPath);
  }, [open, initialPath, load]);

  return (
    <Modal open={open} onClose={onClose} title="Choose a folder">
      <div className="flex flex-col gap-2 min-w-[360px]">
        <div className="flex items-center gap-2">
          <button
            type="button"
            aria-label="Up to parent"
            disabled={!listing?.parent}
            className="p-1 border border-neutral-700 rounded text-neutral-300 hover:text-neutral-100 hover:border-neutral-500 disabled:opacity-30 disabled:hover:border-neutral-700"
            onClick={() => listing?.parent && load(listing.parent)}
          >
            <ArrowUp size={14} />
          </button>
          <div className="flex-1 truncate text-xs font-mono bg-neutral-900 px-2 py-1 rounded">
            {listing?.path ?? initialPath}
          </div>
        </div>

        <div className="border border-neutral-800 rounded max-h-72 overflow-y-auto">
          {loading && (
            <div className="px-3 py-2 text-xs text-neutral-500">Loading…</div>
          )}
          {error && (
            <div className="px-3 py-2 text-xs text-red-400" role="alert">
              {error}
            </div>
          )}
          {!loading && !error && listing && listing.entries.length === 0 && (
            <div className="px-3 py-2 text-xs text-neutral-500">Empty.</div>
          )}
          {!loading &&
            !error &&
            listing?.entries.map((e) => {
              if (e.is_dir) {
                const childPath = listing.path.endsWith("/")
                  ? `${listing.path}${e.name}`
                  : `${listing.path}/${e.name}`;
                return (
                  <button
                    key={e.name}
                    type="button"
                    className="flex items-center gap-2 w-full text-left px-3 py-1.5 text-sm text-neutral-200 hover:bg-neutral-800"
                    onClick={() => load(childPath)}
                  >
                    <Folder size={14} className="text-amber-400 shrink-0" />
                    <span className="truncate">{e.name}</span>
                  </button>
                );
              }
              return (
                <div
                  key={e.name}
                  className="flex items-center gap-2 px-3 py-1.5 text-sm text-neutral-500"
                >
                  <span className="w-3.5 shrink-0" />
                  <span className="truncate">{e.name}</span>
                </div>
              );
            })}
        </div>

        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!listing}
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            onClick={() => listing && onPick(listing.path)}
          >
            Use this folder
          </button>
        </div>
      </div>
    </Modal>
  );
}
