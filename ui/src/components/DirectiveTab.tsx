import { useEffect, useState } from "react";
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
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  function fetchDirective() {
    setLoadState("loading");
    api.getDirective(projectId).then(
      (result) => setLoadState(result),
      (e: Error) => {
        // On fetch error, treat as no directive rather than crashing.
        console.error("DirectiveTab: fetch failed", e);
        setLoadState(null);
      },
    );
  }

  useEffect(() => {
    fetchDirective();
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
    try {
      await api.putDirective(projectId, draft);
      setEditing(false);
      fetchDirective();
    } catch (e) {
      setError((e as Error).message);
    }
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
            onClick={handleSave}
            className="px-3 py-1 rounded bg-blue-700 hover:bg-blue-600 text-white text-xs"
          >
            Save
          </button>
          <button
            onClick={handleCancel}
            className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-neutral-200 text-xs"
          >
            Cancel
          </button>
        </div>
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
