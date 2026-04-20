import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import type { DomainEvent, EventRow, Session } from "../types";
import { api } from "../api";
import { useSessionEvents } from "../hooks/useSessionEvents";
import { TerminalPreview } from "./TerminalPreview";
import { SessionRail } from "./SessionRail";
import { RoleBadge } from "./RoleBadge";
import { EventsTab } from "./EventsTab";

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

  // Live append from the push channel. Prepend (newest-first) and
  // deduplicate against the initial fetch + any concurrent pushes — the
  // initial GET and the first live frame can race when a session has just
  // emitted an event. `prev_state`/`next_state` are explicitly null — the
  // push payload doesn't carry them, and this matches the DB helper's
  // shape for rows with no state transition.
  useSessionEvents(session.id, (e: DomainEvent) => {
    if (e.type !== "session.event_appended") return;
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
              <div data-testid="tab-panel-events" className="h-full overflow-hidden">
                {events.length === 0 && loadError ? (
                  <div
                    data-testid="events-error"
                    className="p-4 text-sm text-red-400"
                  >
                    Could not load events
                  </div>
                ) : (
                  <EventsTab events={events} />
                )}
              </div>
            )}
            {tab === "plan" && (
              <div data-testid="tab-panel-plan" className="p-4 text-sm text-neutral-400">Plan checklist — coming in Task 14</div>
            )}
            {tab === "meta" && (
              <div data-testid="tab-panel-meta" className="p-4 text-sm text-neutral-400">Meta — coming in Task 15</div>
            )}
          </div>
          <SessionRail
            autopilot={session.autopilot}
            onSend={onSend}
            onHandoff={onHandoff}
            onEnd={onEnd}
            onToggleAutopilot={onToggleAutopilot}
          />
        </div>
      </motion.div>
    </motion.div>
  );
}
