import { useEffect, useState } from "react";
import { api } from "../api";
import type { Recommendation } from "../types";

interface Props { projectId: string; open: boolean; onClose: () => void; }

type StatusFilter = "pending" | "deferred" | "acted" | "dismissed" | "archived";
const FILTERS: StatusFilter[] = ["pending", "deferred", "acted", "dismissed", "archived"];

function emptyCopy(f: StatusFilter): string {
  switch (f) {
    case "pending": return "No pending recommendations.";
    case "deferred": return "No deferred recommendations.";
    case "acted": return "No acted recommendations.";
    case "dismissed": return "No dismissed recommendations.";
    case "archived": return "No archived recommendations.";
  }
}

function RecActions({
  projectId,
  rec,
  busy,
  setBusy,
  onChanged,
}: {
  projectId: string;
  rec: Recommendation;
  busy: boolean;
  setBusy: (b: boolean) => void;
  onChanged: (next: Recommendation) => void;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  return (
    <div className="flex flex-col gap-1">
      <button
        className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          await api.deferRec(projectId, rec.id);
          // Caller refetches the list; nothing to replace here.
          setBusy(false);
        }}
      >Defer</button>
      <button
        className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
        disabled={busy}
        onClick={() => setOpen((v) => !v)}
      >{open ? "Cancel" : "Re-explain"}</button>
      {open && (
        <div className="mt-1">
          <textarea
            className="w-full text-xs bg-neutral-800 border border-neutral-700 rounded p-1"
            rows={2}
            placeholder="What's unclear about this recommendation?"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <button
            className="mt-1 text-xs px-2 py-0.5 rounded bg-blue-700 hover:bg-blue-600 disabled:opacity-50"
            disabled={busy || text.trim().length === 0}
            onClick={async () => {
              setBusy(true);
              try {
                await api.explainRec(projectId, rec.id, text);
                onChanged({ ...rec, rationale: text });
                setOpen(false);
                setText("");
              } finally {
                setBusy(false);
              }
            }}
          >Submit</button>
        </div>
      )}
    </div>
  );
}

export function RecModal({ projectId, open, onClose }: Props) {
  const [filter, setFilter] = useState<StatusFilter>("pending");
  const [recs, setRecs] = useState<Recommendation[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    api.listRecs(projectId, filter).then(setRecs).catch(() => setRecs([]));
    setSelected(new Set());
  }, [open, projectId, filter]);

  if (!open) return null;

  const refresh = () => api.listRecs(projectId, filter).then(setRecs);
  const sevColor = (s: string) =>
    s === "urgent" ? "bg-red-700" : s === "warn" ? "bg-yellow-700" : "bg-neutral-700";
  const canAct = filter === "pending"; // make/dismiss/defer only meaningful on pending

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[640px] max-h-[80vh] overflow-y-auto">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-lg font-semibold">Overseer recommendations</h2>
          <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
        </div>

        <div className="flex gap-1 mb-3" role="tablist" aria-label="Filter recommendations by status">
          {FILTERS.map((f) => (
            <button
              key={f}
              role="tab"
              aria-selected={filter === f}
              onClick={() => setFilter(f)}
              className={
                "px-2 py-1 text-xs rounded " +
                (filter === f
                  ? "bg-blue-700 text-white"
                  : "bg-neutral-800 text-neutral-300 hover:bg-neutral-700")
              }
            >
              {f.charAt(0).toUpperCase() + f.slice(1)}
            </button>
          ))}
        </div>

        {recs.length === 0 && (
          <div className="text-neutral-500 text-sm py-8 text-center">{emptyCopy(filter)}</div>
        )}
        {recs.map((r) => (
          <div key={r.id} className="border border-neutral-700 rounded p-3 mb-2">
            <div className="flex items-start gap-2">
              {canAct && (
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
              )}
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
              {canAct && (
                <RecActions
                  projectId={projectId}
                  rec={r}
                  busy={busy}
                  setBusy={setBusy}
                  onChanged={(next) => setRecs((rs) => rs.map((x) => (x.id === next.id ? next : x)))}
                />
              )}
            </div>
          </div>
        ))}
        {canAct && recs.length > 0 && (
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
