import type { PlanStep, PlanStepStatus } from "../types";

interface Props {
  steps: PlanStep[];
}

/**
 * Status → {glyph, className} map per Sub-plan 4 Task 14:
 *   pending  → ○
 *   working  → ◐
 *   done     → ●
 *   blocked  → ◆
 *
 * Kept as a single record (rather than two parallel `Record<PlanStepStatus,
 * …>` objects) so a new status can't be added to one table and forgotten in
 * the other.
 */
const STATUS_META: Record<PlanStepStatus, { glyph: string; className: string }> = {
  pending: { glyph: "\u25CB", className: "text-neutral-500" },
  working: { glyph: "\u25D0", className: "text-blue-300" },
  done:    { glyph: "\u25CF", className: "text-green-400" },
  blocked: { glyph: "\u25C6", className: "text-red-400" },
};

/**
 * Live checklist for the dashboard's Plan tab. Purely presentational — the
 * parent (`ExpandedSessionView`) owns the `steps` array and applies
 * `session.plan_replaced` / `session.plan_step_changed` deltas to it.
 *
 * The first `status === "working"` step is highlighted as the current one
 * (`data-current="true"`). Done steps are collapsed under a `<details>` so
 * long plans stay scannable.
 */
export function PlanTab({ steps }: Props) {
  if (steps.length === 0) {
    return (
      <div
        data-testid="plan-empty"
        className="p-4 text-sm text-neutral-500"
      >
        No plan steps yet.
      </div>
    );
  }

  const firstWorkingHash = steps.find((s) => s.status === "working")?.hash;
  const doneSteps = steps.filter((s) => s.status === "done");
  const activeSteps = steps.filter((s) => s.status !== "done");

  return (
    <div className="p-4 text-sm overflow-y-auto h-full">
      {activeSteps.length > 0 && (
        <ul className="space-y-1">
          {activeSteps.map((s) => (
            <StepRow
              key={s.hash}
              step={s}
              current={s.hash === firstWorkingHash}
            />
          ))}
        </ul>
      )}
      {doneSteps.length > 0 && (
        <details
          data-testid="plan-done-details"
          className="mt-4 text-neutral-400"
        >
          <summary className="cursor-pointer select-none text-xs uppercase tracking-wide text-neutral-500">
            Completed ({doneSteps.length})
          </summary>
          <ul className="mt-2 space-y-1">
            {doneSteps.map((s) => (
              <StepRow key={s.hash} step={s} current={false} />
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function StepRow({ step, current }: { step: PlanStep; current: boolean }) {
  const { glyph, className } = STATUS_META[step.status];
  return (
    <li
      data-testid={`step-${step.hash}`}
      data-current={current ? "true" : undefined}
      className={`flex items-start gap-2 px-2 py-1 rounded ${
        current ? "bg-neutral-800/60" : ""
      }`}
    >
      <span
        aria-label={step.status}
        className={`font-mono ${className}`}
      >
        {glyph}
      </span>
      <span className="flex-1 min-w-0">
        <span className={step.status === "done" ? "line-through opacity-60" : ""}>
          {step.text}
        </span>
        {step.note && (
          <span className="block text-xs text-neutral-500 mt-0.5">
            {step.note}
          </span>
        )}
      </span>
    </li>
  );
}
