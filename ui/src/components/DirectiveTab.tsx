import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Directive } from "../types";

// TODO: subscribe to `directive.changed` events once the backend wires
// DirectiveChanged through a global event channel. Currently,
// events.rs::applies_to returns false for that variant on per-session
// WebSockets, so live external-edit updates are not available. Refetch on
// mount + after own PUT covers the common path.

interface Props {
  projectId: string;
}

type LoadState = "loading" | Directive | null;

export function DirectiveTab({ projectId }: Props) {
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Generation counter so that a stale in-flight `getDirective` resolution
  // (from a previous projectId, or from before a parallel save kicked off
  // its own refetch) cannot clobber state for the current generation.
  const genRef = useRef(0);

  function loadDirective(myGen: number) {
    setLoadState("loading");
    setLoadError(null);
    api.getDirective(projectId).then(
      (result) => {
        if (genRef.current !== myGen) return;
        setLoadState(result);
      },
      (e: unknown) => {
        if (genRef.current !== myGen) return;
        const msg = e instanceof Error ? e.message : String(e);
        console.error("DirectiveTab: fetch failed", e);
        setLoadError(msg);
        setLoadState(null);
      },
    );
  }

  useEffect(() => {
    const myGen = ++genRef.current;
    loadDirective(myGen);
    return () => {
      // Bump the gen so any still-pending response is ignored.
      genRef.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  function handleCreate() {
    setDraft("");
    setError(null);
    setEditing(true);
  }

  function handleEdit() {
    const directive = loadState as Directive;
    setDraft(directive.raw_markdown);
    setError(null);
    setEditing(true);
  }

  function handleCancel() {
    setError(null);
    setEditing(false);
  }

  async function handleSave() {
    const myGen = genRef.current;
    setError(null);
    try {
      await api.putDirective(projectId, draft);
      if (genRef.current !== myGen) return;
      setEditing(false);
      loadDirective(myGen);
    } catch (e: unknown) {
      if (genRef.current !== myGen) return;
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function handleRetry() {
    loadDirective(++genRef.current);
  }

  // ── Loading ───────────────────────────────────────────────────────────────
  if (loadState === "loading") {
    return (
      <div className="p-4 text-sm text-neutral-400">Loading directive...</div>
    );
  }

  // ── Editing ───────────────────────────────────────────────────────────────
  if (editing) {
    return (
      <div className="p-4 text-sm overflow-y-auto h-full space-y-3 text-neutral-200">
        <textarea
          rows={20}
          cols={80}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          className="w-full font-mono text-xs bg-neutral-900 border border-neutral-800 rounded p-2 text-neutral-200 resize-y"
        />
        {error && (
          <pre className="whitespace-pre-wrap text-xs text-red-400 bg-neutral-900 border border-neutral-800 rounded p-2">
            {error}
          </pre>
        )}
        <div className="flex gap-2">
          <button
            type="button"
            onClick={handleSave}
            className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs"
          >
            Save
          </button>
          <button
            type="button"
            onClick={handleCancel}
            className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-neutral-200 text-xs"
          >
            Cancel
          </button>
        </div>
      </div>
    );
  }

  // ── Fetch error ──────────────────────────────────────────────────────────
  // Surfaces the failure distinctly from the empty/create state so a user
  // does not click Create against an unreachable directive and overwrite it.
  if (loadError !== null) {
    return (
      <div className="p-4 text-sm overflow-y-auto h-full space-y-3 text-neutral-200">
        <p className="text-red-400">Couldn't load directive: {loadError}</p>
        <button
          type="button"
          onClick={handleRetry}
          className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-neutral-200 text-xs"
        >
          Retry
        </button>
      </div>
    );
  }

  // ── Empty state ───────────────────────────────────────────────────────────
  if (loadState === null) {
    return (
      <div className="p-4 text-sm overflow-y-auto h-full space-y-3 text-neutral-200">
        <p className="text-neutral-400 italic">
          No directive yet. Brainstorm should produce one.
        </p>
        <button
          type="button"
          onClick={handleCreate}
          className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs"
        >
          Create
        </button>
      </div>
    );
  }

  // ── View (Directive present) ──────────────────────────────────────────────
  const directive = loadState;

  return (
    <div className="p-4 text-sm overflow-y-auto h-full space-y-4 text-neutral-200">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-semibold text-neutral-100">{directive.name}</h2>
        <button
          type="button"
          onClick={handleEdit}
          className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-neutral-200 text-xs"
        >
          Edit
        </button>
      </div>

      {/* Phases table */}
      <section>
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Phases</h3>
        <table className="w-full text-xs border-collapse">
          <thead>
            <tr className="text-neutral-500 border-b border-neutral-800">
              <th className="text-left py-1 pr-3">ID</th>
              <th className="text-left py-1 pr-3">Parts</th>
              <th className="text-left py-1 pr-3">Autonomy</th>
              <th className="text-left py-1">Depends on</th>
            </tr>
          </thead>
          <tbody>
            {directive.phases.map((phase) => (
              <tr key={phase.id} className="border-b border-neutral-800/50">
                <td className="py-1 pr-3 font-mono">{phase.id}</td>
                <td className="py-1 pr-3">{phase.parts.join(", ")}</td>
                <td className="py-1 pr-3">{phase.autonomy}</td>
                <td className="py-1">
                  {phase.depends_on.length > 0 ? phase.depends_on.join(", ") : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      {/* Escalation rules */}
      <section>
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">
          Escalation Rules
        </h3>
        <ul className="space-y-1 text-xs text-neutral-300">
          {directive.escalation_rules.map((rule, i) => (
            <li key={i}>
              On {rule.trigger} → {rule.action}
            </li>
          ))}
        </ul>
      </section>

      {/* Notes */}
      {directive.notes && (
        <section>
          <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Notes</h3>
          <pre className="whitespace-pre-wrap font-mono text-xs bg-neutral-900 border border-neutral-800 rounded p-2">
            {directive.notes}
          </pre>
        </section>
      )}
    </div>
  );
}
