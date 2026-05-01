import type { Session } from "../types";
import { STATE_COLORS } from "../types";
import { api } from "../api";
import { RoleBadge } from "./RoleBadge";

interface Props {
  session: Session;
  onExpand?: (s: Session) => void;
  onRefresh: () => void;
  depth?: number;
}

export function SessionRow({ session, onExpand, onRefresh, depth }: Props) {
  const waitingCount = session.waiting_on?.length ?? 0;
  const dotColor = STATE_COLORS[session.state] ?? "bg-gray-400";

  const handleKill = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await api.killSession(session.id);
    onRefresh();
  };

  const handleHandoff = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await api.triggerHandoff(session.id);
  };

  const age = (() => {
    const ms = Date.now() - new Date(session.created_at).getTime();
    const mins = Math.floor(ms / 60000);
    if (mins < 60) return `${mins}m`;
    return `${Math.floor(mins / 60)}h ${mins % 60}m`;
  })();

  return (
    <div
      role="button"
      tabIndex={0}
      className="flex items-center gap-3 px-4 py-3 w-full text-left cursor-pointer border-b border-gray-800 hover:bg-gray-800/50 focus:outline-none focus:bg-gray-800/70"
      style={{ paddingLeft: depth ? depth * 16 : undefined }}
      onClick={() => onExpand?.(session)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onExpand?.(session);
        }
      }}
    >
      <div
        className={`w-2.5 h-2.5 rounded-full shrink-0 ${dotColor}`}
        data-testid="session-state-dot"
        data-state={session.state}
      />

      <div className="flex-1 min-w-0">
        <div className="text-sm truncate">{session.task_summary}</div>
        <div className="flex items-center gap-2 text-xs text-gray-500">
          <RoleBadge role={session.role} />
          {session.branch && <span className="truncate">{session.branch}</span>}
          <span>{age}</span>
          {session.last_event_kind?.startsWith("mcp.handoff") && (
            <span
              data-testid="handoff-tick"
              className="text-xs text-emerald-400"
              title="Handoff recorded"
              aria-label="handoff recorded"
            >
              ✓
            </span>
          )}
        </div>
      </div>

      {waitingCount > 0 && (
        <span
          data-testid="waiting-badge"
          className="text-xs px-2 py-0.5 rounded bg-amber-900 text-amber-300"
        >
          waiting on {waitingCount}
        </span>
      )}

      <span className="text-xs px-2 py-0.5 rounded bg-gray-700 text-gray-300">
        {session.state}
      </span>

      <span
        className={`text-xs px-2 py-0.5 rounded ${
          session.autopilot
            ? "bg-green-900 text-green-300"
            : "bg-gray-700 text-gray-500"
        }`}
      >
        {session.autopilot ? "auto" : "manual"}
      </span>

      {session.state === "working" && (
        <div className="flex gap-1">
          <button
            className="text-xs px-2 py-1 rounded bg-blue-800 hover:bg-blue-700 text-blue-200"
            onClick={handleHandoff}
            title="Trigger handoff"
          >
            Handoff
          </button>
          <button
            className="text-xs px-2 py-1 rounded bg-red-900 hover:bg-red-800 text-red-200"
            onClick={handleKill}
            title="Kill session"
          >
            Kill
          </button>
        </div>
      )}
    </div>
  );
}
