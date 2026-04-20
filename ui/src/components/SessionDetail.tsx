import { useEffect, useState } from "react";
import { api } from "../api";
import type { Session, SignalRow } from "../types";
import { STATE_COLORS } from "../types";
import { TerminalPreview } from "./TerminalPreview";

interface Props {
  session: Session;
  onRefresh: () => void;
}

export function SessionDetail({ session, onRefresh }: Props) {
  const [signals, setSignals] = useState<SignalRow[]>([]);

  useEffect(() => {
    api.getSignals(session.id).then(setSignals).catch(() => {});
  }, [session.id]);

  const handleKill = async () => {
    await api.killSession(session.id);
    onRefresh();
  };

  const handleHandoff = async () => {
    await api.triggerHandoff(session.id);
  };

  const dotColor = STATE_COLORS[session.state] ?? "bg-gray-400";
  const isRunning = !["ended", "error"].includes(session.state);

  return (
    <div className="border-l border-gray-800 p-4 space-y-4 overflow-auto">
      <div className="flex items-center gap-2">
        <div className={`w-3 h-3 rounded-full ${dotColor}`} />
        <h2 className="text-lg font-semibold">{session.task_summary}</h2>
      </div>

      <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
        <dt className="text-gray-500">Role</dt>
        <dd>{session.role}</dd>
        <dt className="text-gray-500">State</dt>
        <dd>{session.state}</dd>
        <dt className="text-gray-500">Branch</dt>
        <dd>{session.branch ?? "(none)"}</dd>
        <dt className="text-gray-500">Worktree</dt>
        <dd className="truncate">{session.worktree_path}</dd>
        <dt className="text-gray-500">PID</dt>
        <dd>{session.pty_pid ?? "N/A"}</dd>
        <dt className="text-gray-500">Autopilot</dt>
        <dd>{session.autopilot ? "ON" : "OFF"}</dd>
        <dt className="text-gray-500">Created</dt>
        <dd>{new Date(session.created_at).toLocaleString()}</dd>
      </dl>

      {isRunning && (
        <div className="flex gap-2">
          <button
            className="px-3 py-1.5 rounded bg-blue-800 hover:bg-blue-700 text-sm"
            onClick={handleHandoff}
          >
            Handoff Now
          </button>
          <button
            className="px-3 py-1.5 rounded bg-red-900 hover:bg-red-800 text-sm"
            onClick={handleKill}
          >
            Kill Session
          </button>
        </div>
      )}

      {isRunning && (
        <div>
          <h3 className="text-sm font-medium text-gray-400 mb-2">
            Live Terminal
          </h3>
          <TerminalPreview sessionId={session.id} />
        </div>
      )}

      <div>
        <h3 className="text-sm font-medium text-gray-400 mb-2">
          Recent Signals ({signals.length})
        </h3>
        <div className="space-y-1 max-h-48 overflow-auto">
          {signals
            .slice(-20)
            .reverse()
            .map((sig) => (
              <div key={sig.id} className="text-xs font-mono text-gray-400">
                <span className="text-gray-600">
                  {new Date(sig.at).toLocaleTimeString()}
                </span>{" "}
                <span className="text-blue-300">{sig.kind}</span>
                {sig.payload && (
                  <span className="text-gray-500"> {sig.payload}</span>
                )}
              </div>
            ))}
        </div>
      </div>
    </div>
  );
}
