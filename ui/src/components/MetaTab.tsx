import type { Session } from "../types";

interface Props {
  session: Session;
}

/**
 * Read-only display of a session's meta state: plan markdown, seed markdown,
 * handoff file link, and turn/token counters. The parent
 * (`ExpandedSessionView`) owns the `Session` record; Sub-plan 4 Task 10 patches
 * the five meta fields live from `session.meta_changed` events, so this
 * component re-renders automatically with no extra wiring.
 *
 * Plan/seed are rendered as plain preformatted text — no markdown renderer is
 * added as a dependency. Counters are formatted with thousands separators for
 * readability.
 */
export function MetaTab({ session }: Props) {
  const planMd = session.plan_markdown ?? "";
  const seedMd = session.seed_markdown ?? "";
  const handoffPath = session.handoff_path ?? null;
  const turns = session.turn_count ?? 0;
  const tokens = session.est_tokens ?? 0;

  return (
    <div className="p-4 text-sm overflow-y-auto h-full space-y-4 text-neutral-200">
      <section data-testid="meta-plan">
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Plan</h3>
        {planMd ? (
          <pre className="whitespace-pre-wrap font-mono text-xs bg-neutral-900 border border-neutral-800 rounded p-2">
            {planMd}
          </pre>
        ) : (
          <p className="text-neutral-500 italic">No plan set</p>
        )}
      </section>

      <section data-testid="meta-seed">
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Seed</h3>
        {seedMd ? (
          <pre className="whitespace-pre-wrap font-mono text-xs bg-neutral-900 border border-neutral-800 rounded p-2">
            {seedMd}
          </pre>
        ) : (
          <p className="text-neutral-500 italic">No seed set</p>
        )}
      </section>

      <section data-testid="meta-handoff">
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Handoff</h3>
        {handoffPath ? (
          // TODO(sub-plan-5): handoff_path is a local filesystem path, so this
          // <a href> is inert in a browser and unreachable from Tauri without
          // the opener plugin. Replace with a copy-path / open-in-editor action.
          <a
            href={handoffPath}
            className="text-blue-300 hover:underline break-all font-mono text-xs"
          >
            Handoff: {handoffPath}
          </a>
        ) : (
          <p className="text-neutral-500 italic">No handoff written</p>
        )}
      </section>

      <section data-testid="meta-counters">
        <h3 className="text-xs uppercase tracking-wide text-neutral-500 mb-1">Counters</h3>
        <div className="flex gap-4 text-neutral-300">
          <span data-testid="meta-turns">{turns.toLocaleString("en-US")} turns</span>
          <span data-testid="meta-tokens">{tokens.toLocaleString("en-US")} tokens</span>
        </div>
      </section>
    </div>
  );
}
