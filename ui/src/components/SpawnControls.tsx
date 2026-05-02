import { useEffect, useState } from "react";
import { api } from "../api";
import type { Directive } from "../types";

interface Props {
  projectId: string;
  onSpawn?: () => void;
}

type GateState =
  | { kind: "loading" }
  | { kind: "missing"; reasons: string[] }
  | { kind: "ready" };

export function SpawnControls({ projectId, onSpawn }: Props) {
  const [state, setState] = useState<GateState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    Promise.all([api.getPlan(projectId), api.getDirective(projectId)]).then(
      ([plan, dir]) => {
        if (cancelled) return;
        const reasons: string[] = [];
        if (plan === null) reasons.push("plan.md required");
        if (dir === null) reasons.push("directive.md required");
        setState(reasons.length > 0 ? { kind: "missing", reasons } : { kind: "ready" });
      },
      () => {
        if (cancelled) return;
        // On fetch failure, treat both as missing — defensive default.
        setState({
          kind: "missing",
          reasons: ["plan.md required", "directive.md required"],
        });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [projectId]);

  const disabled = state.kind !== "ready";
  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        disabled={disabled}
        onClick={() => onSpawn?.()}
        className={
          disabled
            ? "px-3 py-1 rounded bg-neutral-700 text-neutral-400 text-xs cursor-not-allowed"
            : "px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs"
        }
      >
        Spawn sessions
      </button>
      {state.kind === "missing" && (
        <span className="text-xs text-neutral-500">
          {state.reasons.join(" + ")}
        </span>
      )}
    </div>
  );
}

// Re-export the Directive type so test files can satisfy the mocked return
// without spelling the structural type out.
export type { Directive };
