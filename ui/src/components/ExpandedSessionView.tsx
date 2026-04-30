import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import type { DomainEvent, EventRow, PlanStep, Session } from "../types";
import { api } from "../api";
import { useSessionEvents } from "../hooks/useSessionEvents";
import { TerminalPreview } from "./TerminalPreview";
import { SessionRail } from "./SessionRail";
import { RoleBadge } from "./RoleBadge";
import { EventsTab } from "./EventsTab";
import { PlanTab } from "./PlanTab";
import { MetaTab } from "./MetaTab";
import { TabPanel } from "./TabPanel";

type Tab = "terminal" | "events" | "plan" | "meta";

interface Props {
  session: Session;
  onClose: () => void;
  onSend: (origin: DOMRect) => void;
  onHandoff: (origin: DOMRect) => void;
  onEnd: (origin: DOMRect) => void;
  onToggleAutopilot: (next: boolean) => void;
}

export function ExpandedSessionView({
  session,
  onClose,
  onSend,
  onHandoff,
  onEnd,
  onToggleAutopilot,
}: Props) {
  const [tab, setTab] = useState<Tab>("terminal");
  const [events, setEvents] = useState<EventRow[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [planSteps, setPlanSteps] = useState<PlanStep[]>([]);
  const [planError, setPlanError] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Initial fetch — populate newest-first (backend orders by id DESC).
  // Reset when the session id changes so re-opening a different session
  // does not show stale rows.
  useEffect(() => {
    let cancelled = false;
    setEvents([]);
    setLoadError(false);
    api.getSessionEvents(session.id, 100).then(
      (rows) => {
        if (!cancelled) {
          setEvents(rows);
          setLoadError(false);
        }
      },
      () => {
        if (!cancelled) setLoadError(true);
      },
    );
    return () => { cancelled = true; };
  }, [session.id]);

  // Initial fetch for the Plan tab. Same cancel-safe + reset-on-id-change
  // pattern as the Events fetch above — re-opening a different session must
  // not briefly show the previous session's plan.
  useEffect(() => {
    let cancelled = false;
    setPlanSteps([]);
    setPlanError(false);
    api.getSessionPlanSteps(session.id).then(
      (rows) => {
        if (!cancelled) {
          setPlanSteps(rows);
          setPlanError(false);
        }
      },
      () => {
        if (!cancelled) setPlanError(true);
      },
    );
    return () => { cancelled = true; };
  }, [session.id]);

  // Live append from the push channel. Prepend (newest-first) and
  // deduplicate against the initial fetch + any concurrent pushes — the
  // initial GET and the first live frame can race when a session has just
  // emitted an event. `prev_state`/`next_state` are explicitly null — the
  // push payload doesn't carry them, and this matches the DB helper's
  // shape for rows with no state transition.
  useSessionEvents(session.id, (e: DomainEvent) => {
    if (e.type === "session.event_appended") {
      if (e.session_id !== session.id) return;
      setEvents((prev) => {
        if (prev.some((row) => row.id === e.event_id)) return prev;
        const row: EventRow = {
          id: e.event_id,
          kind: e.kind,
          source: e.source,
          ts: e.ts,
          payload: e.payload,
          prev_state: null,
          next_state: null,
        };
        return [row, ...prev];
      });
      return;
    }
    if (e.type === "session.plan_replaced") {
      if (e.session_id !== session.id) return;
      // Full replacement — the producer (Sub-plan 5 `set_plan`) is always
      // authoritative on this event.
      setPlanSteps(e.steps);
      setPlanError(false);
      return;
    }
    if (e.type === "session.plan_step_changed") {
      if (e.session_id !== session.id) return;
      setPlanSteps((prev) => {
        const idx = prev.findIndex((s) => s.hash === e.step_hash);
        if (idx === -1) {
          // Unknown hash — likely a step appended after our initial fetch
          // resolved but before this WS subscription started receiving
          // frames (or a `set_plan` we missed during a reconnect window).
          // Refetch the authoritative list. On failure, leave UI as-is;
          // the next `plan_replaced` will fully refresh.
          api
            .getSessionPlanSteps(session.id)
            .then((rows) => setPlanSteps(rows))
            .catch(() => { /* ignore; next plan_replaced will refresh */ });
          return prev;
        }
        const next = prev.slice();
        next[idx] = {
          ...prev[idx],
          status: e.next_status,
          note: e.note ?? prev[idx].note,
        };
        return next;
      });
      return;
    }
  });

  return (
    <motion.div
      data-testid="expanded-backdrop"
      className="fixed inset-0 z-40 bg-black/70 flex items-center justify-center"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      onClick={onClose}
    >
      <motion.div
        className="bg-neutral-950 border border-neutral-800 rounded-lg shadow-2xl flex flex-col"
        style={{ width: "92vw", height: "92vh" }}
        initial={{ scale: 0.85, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.85, opacity: 0 }}
        transition={{ type: "spring", stiffness: 240, damping: 24 }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-4 py-2 border-b border-neutral-800 flex items-center justify-between text-xs text-neutral-300">
          <div>
            <RoleBadge role={session.role} />{" "}
            <span className="opacity-50">· {session.id} · {session.state}</span>
          </div>
          <div className="opacity-60">ESC or click outside to close</div>
        </header>
        <div role="tablist" className="flex gap-1 px-4 py-1 border-b border-neutral-800 text-xs text-neutral-300">
          {(["terminal", "events", "plan", "meta"] as const).map((t) => {
            const label = t.charAt(0).toUpperCase() + t.slice(1);
            return (
              <button
                key={t}
                role="tab"
                aria-selected={tab === t}
                onClick={() => setTab(t)}
                className={`px-3 py-1 rounded-t ${tab === t ? "bg-neutral-800 text-neutral-100" : "hover:bg-neutral-900 text-neutral-400"}`}
              >
                {label}
              </button>
            );
          })}
        </div>
        <div className="flex flex-1 min-h-0">
          <div className="flex-1 min-w-0">
            {tab === "terminal" && (
              <TerminalPreview sessionId={session.id} readOnly={session.autopilot} />
            )}
            {tab === "events" && (
              <TabPanel
                testId="tab-panel-events"
                showError={events.length === 0 && loadError}
                errorTestId="events-error"
                errorText="Could not load events"
              >
                <EventsTab events={events} />
              </TabPanel>
            )}
            {tab === "plan" && (
              <TabPanel
                testId="tab-panel-plan"
                showError={planSteps.length === 0 && planError}
                errorTestId="plan-error"
                errorText="Could not load plan"
              >
                <PlanTab steps={planSteps} />
              </TabPanel>
            )}
            {tab === "meta" && (
              <div data-testid="tab-panel-meta" className="flex-1 min-h-0 h-full">
                <MetaTab session={session} />
              </div>
            )}
          </div>
          <SessionRail
            autopilot={session.autopilot}
            onSend={onSend}
            onHandoff={onHandoff}
            onEnd={onEnd}
            onToggleAutopilot={onToggleAutopilot}
            onOpenEvents={() => setTab("events")}
          />
        </div>
      </motion.div>
    </motion.div>
  );
}
