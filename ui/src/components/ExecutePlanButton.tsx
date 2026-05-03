import { useEffect, useState } from "react";
import { PlayCircle } from "lucide-react";
import { api } from "../api";

interface Props { projectId: string; onExecute?: () => void; }

type GateState =
  | { kind: "loading" }
  | { kind: "missing"; reasons: string[] }
  | { kind: "ready" };

export function ExecutePlanButton({ projectId, onExecute }: Props) {
  const [state, setState] = useState<GateState>({ kind: "loading" });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    Promise.all([api.getPlan(projectId), api.getDirective(projectId)]).then(
      ([plan, dir]) => {
        if (cancelled) return;
        const reasons: string[] = [];
        if (plan === null) reasons.push("plan.md required");
        if (dir === null) reasons.push("directive.md required");
        setState(reasons.length ? { kind: "missing", reasons } : { kind: "ready" });
      },
      () => !cancelled && setState({ kind: "missing", reasons: ["plan.md required", "directive.md required"] }),
    );
    return () => { cancelled = true; };
  }, [projectId]);

  const disabled = state.kind !== "ready" || busy;
  const cls = disabled
    ? "px-3 py-1 rounded bg-neutral-700 text-neutral-400 text-xs cursor-not-allowed flex items-center gap-1"
    : "px-3 py-1 rounded bg-emerald-700 hover:bg-emerald-600 text-white text-xs flex items-center gap-1";

  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        disabled={disabled}
        className={cls}
        onClick={async () => {
          setBusy(true); setErr(null);
          try { await api.executePlan(projectId); onExecute?.(); }
          catch (e: any) { setErr(e?.message ?? "execute failed"); }
          finally { setBusy(false); }
        }}
      >
        <PlayCircle size={14} /> Execute plan
      </button>
      {state.kind === "missing" && (
        <span className="text-xs text-neutral-500">{state.reasons.join(" + ")}</span>
      )}
      {err && <span className="text-xs text-red-400">{err}</span>}
    </div>
  );
}
