import { useEffect, useState } from "react";
import { api } from "../api";
import type { Decision, Postmortem } from "../types";

// TODO: subscribe to `postmortem.written` and `decision.recorded` events for
// live refresh once the backend wires PostmortemWritten / DecisionRecorded
// through a project-scoped event channel. Currently events.rs::applies_to
// returns false for both variants on per-session WebSockets, so live external
// updates are not available. Refetch on mount + after own actions covers the
// common path. (Same pattern as DirectiveTab.)

interface Props {
  projectId: string;
}

type Loadable<T> = { kind: "loading" } | { kind: "error"; msg: string } | { kind: "ok"; data: T };

export function PostmortemsPanel({ projectId }: Props) {
  const [pms, setPms] = useState<Loadable<Postmortem[]>>({ kind: "loading" });
  const [decs, setDecs] = useState<Loadable<Decision[]>>({ kind: "loading" });
  const [pmsOpen, setPmsOpen] = useState(true);
  const [decsOpen, setDecsOpen] = useState(true);
  const [expandedPm, setExpandedPm] = useState<string | null>(null); // session_id
  const [expandedDec, setExpandedDec] = useState<string | null>(null); // session_id+asked_at

  useEffect(() => {
    let cancelled = false;
    setPms({ kind: "loading" });
    setDecs({ kind: "loading" });
    api.listPostmortems(projectId).then(
      (data) => { if (!cancelled) setPms({ kind: "ok", data }); },
      (e) => { if (!cancelled) setPms({ kind: "error", msg: e instanceof Error ? e.message : String(e) }); },
    );
    api.listDecisions(projectId).then(
      (data) => { if (!cancelled) setDecs({ kind: "ok", data }); },
      (e) => { if (!cancelled) setDecs({ kind: "error", msg: e instanceof Error ? e.message : String(e) }); },
    );
    return () => { cancelled = true; };
  }, [projectId]);

  const pmCount = pms.kind === "ok" ? pms.data.length : 0;
  const decCount = decs.kind === "ok" ? decs.data.length : 0;

  return (
    <div className="p-3 text-sm text-neutral-200 space-y-3">
      {/* Postmortems */}
      <section>
        <button
          type="button"
          onClick={() => setPmsOpen(!pmsOpen)}
          className="flex items-center gap-2 text-xs uppercase tracking-wide text-neutral-400 hover:text-neutral-200"
        >
          <span>{pmsOpen ? "▾" : "▸"}</span>
          <span>Postmortems</span>
          <span className="px-1.5 py-0.5 rounded bg-neutral-800 text-neutral-300 text-[10px]">{pmCount}</span>
        </button>
        {pmsOpen && (
          <div className="mt-1 ml-4 space-y-1">
            {pms.kind === "loading" && <div className="text-xs text-neutral-500">Loading...</div>}
            {pms.kind === "error" && <div className="text-xs text-red-400">Error: {pms.msg}</div>}
            {pms.kind === "ok" && pms.data.length === 0 && <div className="text-xs text-neutral-500 italic">No postmortems yet.</div>}
            {pms.kind === "ok" && pms.data.map((pm) => (
              <div key={`${pm.session_id}-${pm.written_at}`} className="text-xs">
                <button
                  type="button"
                  onClick={() => setExpandedPm(expandedPm === pm.session_id ? null : pm.session_id)}
                  className="text-left w-full hover:bg-neutral-800 rounded px-1 py-0.5"
                >
                  <span className="text-neutral-400">part {pm.plan_part}</span>
                  <span className="ml-2 text-neutral-300">{pm.outcome}</span>
                  <span className="ml-2 text-neutral-500">{pm.cycles} cycles</span>
                </button>
                {expandedPm === pm.session_id && (
                  <pre className="whitespace-pre-wrap font-mono text-[11px] bg-neutral-900 border border-neutral-800 rounded p-2 mt-1 ml-1">
{`What worked:
${pm.what_worked}

Gotchas:
${pm.gotchas}

Follow-ups:
${pm.follow_ups}`}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Decisions */}
      <section>
        <button
          type="button"
          onClick={() => setDecsOpen(!decsOpen)}
          className="flex items-center gap-2 text-xs uppercase tracking-wide text-neutral-400 hover:text-neutral-200"
        >
          <span>{decsOpen ? "▾" : "▸"}</span>
          <span>Decisions</span>
          <span className="px-1.5 py-0.5 rounded bg-neutral-800 text-neutral-300 text-[10px]">{decCount}</span>
        </button>
        {decsOpen && (
          <div className="mt-1 ml-4 space-y-1">
            {decs.kind === "loading" && <div className="text-xs text-neutral-500">Loading...</div>}
            {decs.kind === "error" && <div className="text-xs text-red-400">Error: {decs.msg}</div>}
            {decs.kind === "ok" && decs.data.length === 0 && <div className="text-xs text-neutral-500 italic">No decisions yet.</div>}
            {decs.kind === "ok" && decs.data.map((d) => {
              const id = `${d.session_id}-${d.asked_at}`;
              return (
                <div key={id} className="text-xs">
                  <button
                    type="button"
                    onClick={() => setExpandedDec(expandedDec === id ? null : id)}
                    className="text-left w-full hover:bg-neutral-800 rounded px-1 py-0.5 truncate"
                  >
                    <span className="text-neutral-300">{d.question}</span>
                  </button>
                  {expandedDec === id && (
                    <pre className="whitespace-pre-wrap font-mono text-[11px] bg-neutral-900 border border-neutral-800 rounded p-2 mt-1 ml-1">
{`Q: ${d.question}

A: ${d.answer}`}
                    </pre>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
