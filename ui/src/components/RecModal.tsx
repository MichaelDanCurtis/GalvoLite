import { useEffect, useState } from "react";
import { api } from "../api";
import type { Recommendation } from "../types";

interface Props { projectId: string; open: boolean; onClose: () => void; }

export function RecModal({ projectId, open, onClose }: Props) {
  const [recs, setRecs] = useState<Recommendation[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    api.listRecs(projectId, "pending").then(setRecs).catch(() => setRecs([]));
  }, [open, projectId]);

  if (!open) return null;

  const refresh = () => api.listRecs(projectId, "pending").then(setRecs);

  const sevColor = (s: string) =>
    s === "urgent" ? "bg-red-700" : s === "warn" ? "bg-yellow-700" : "bg-neutral-700";

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[640px] max-h-[80vh] overflow-y-auto">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-lg font-semibold">Overseer recommendations</h2>
          <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
        </div>
        {recs.length === 0 && (
          <div className="text-neutral-500 text-sm py-8 text-center">No pending recommendations.</div>
        )}
        {recs.map((r) => (
          <div key={r.id} className="border border-neutral-700 rounded p-3 mb-2">
            <div className="flex items-start gap-2">
              <input
                type="checkbox"
                aria-label={`select ${r.title}`}
                checked={selected.has(r.id)}
                onChange={(e) => {
                  const next = new Set(selected);
                  e.target.checked ? next.add(r.id) : next.delete(r.id);
                  setSelected(next);
                }}
              />
              <div className="flex-1">
                <div className="flex items-center gap-2">
                  <span className={`text-[10px] uppercase rounded px-1 py-0.5 ${sevColor(r.severity)}`}>
                    {r.severity}
                  </span>
                  <span className="font-medium">{r.title}</span>
                </div>
                <div className="text-sm text-neutral-300 mt-1 whitespace-pre-wrap">{r.body}</div>
                <div className="text-xs text-neutral-500 mt-1 italic">{r.rationale}</div>
              </div>
              <div className="flex flex-col gap-1">
                <button
                  className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
                  disabled={busy}
                  onClick={async () => { setBusy(true); await api.deferRec(projectId, r.id); await refresh(); setBusy(false); }}
                >Defer</button>
              </div>
            </div>
          </div>
        ))}
        {recs.length > 0 && (
          <div className="flex gap-2 justify-end mt-3">
            <button
              className="px-3 py-1 rounded bg-emerald-700 hover:bg-emerald-600 text-white text-xs disabled:opacity-50"
              disabled={busy || selected.size === 0}
              onClick={async () => {
                setBusy(true);
                await api.actRecs(projectId, [...selected], "make");
                setSelected(new Set()); await refresh(); setBusy(false);
              }}
            >Make selected ({selected.size})</button>
            <button
              className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-white text-xs disabled:opacity-50"
              disabled={busy || selected.size === 0}
              onClick={async () => {
                setBusy(true);
                await api.actRecs(projectId, [...selected], "dismiss");
                setSelected(new Set()); await refresh(); setBusy(false);
              }}
            >Dismiss</button>
          </div>
        )}
      </div>
    </div>
  );
}
