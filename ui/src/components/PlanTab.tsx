import type { PlanStep, PlanStepStatus } from "../types";

interface Props {
  steps: PlanStep[];
}

/**
 * Status → glyph map per Sub-plan 4 Task 14:
 *   pending  → ○
 *   working  → ◐
 *   done     → ●
 *   blocked  → ◆
 */
const STATUS_GLYPH: Record<PlanStepStatus, string> = {
  pending: "\u25CB",
  working: "\u25D0",
  done:    "\u25CF",
  blocked: "\u25C6",
};

const STATUS_CLASS: Record<PlanStepStatus, string> = {
  pending: "text-neutral-500",
  working: "text-blue-300",
  done:    "text-green-400",
  blocked: "text-red-400",
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
  const glyph = STATUS_GLYPH[step.status];
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
        className={`font-mono ${STATUS_CLASS[step.status]}`}
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
