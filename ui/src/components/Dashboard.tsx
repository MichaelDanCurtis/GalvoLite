import { useCallback, useMemo, useState } from "react";
import { Plus, PlayCircle } from "lucide-react";
import { useProjects } from "../hooks/useProjects";
import { useSessions } from "../hooks/useSessions";
import { useExpandedSession } from "../hooks/useExpandedSession";
import { useSessionEvents } from "../hooks/useSessionEvents";
import { ProjectSelector } from "./ProjectSelector";
import { SessionRow } from "./SessionRow";
import { ExpandedSessionView } from "./ExpandedSessionView";
import { AddProjectModal } from "./modals/AddProjectModal";
import { RemoveProjectModal } from "./modals/RemoveProjectModal";
import { StartSessionModal } from "./modals/StartSessionModal";
import { SendModal } from "./modals/SendModal";
import { HandoffModal } from "./modals/HandoffModal";
import { EndSessionModal } from "./modals/EndSessionModal";
import type { Project, DomainEvent, Session } from "../types";

function SessionEventBridge({
  sessionId,
  onEvent,
}: {
  sessionId: string;
  onEvent: (e: DomainEvent) => void;
}) {
  useSessionEvents(sessionId, onEvent);
  return null;
}

interface DashboardProps {
  onGlobalEvent?: (e: DomainEvent) => void;
}

export function Dashboard({ onGlobalEvent }: DashboardProps = {}) {
  const { projects, createProject, removeProject } = useProjects();
  const {
    sessions,
    refresh,
    applyEvent,
    createSession,
    sendText,
    triggerHandoff,
    killSession,
    toggleAutopilot,
  } = useSessions();
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [startOpen, setStartOpen] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<Project | null>(null);
  const { expandedId, expand, collapse } = useExpandedSession();
  const [modal, setModal] = useState<null | "send" | "handoff" | "end">(null);

  const bridgeHandler = useCallback(
    (e: DomainEvent) => {
      applyEvent(e);
      onGlobalEvent?.(e);
    },
    [applyEvent, onGlobalEvent],
  );

  const filtered = selectedProject
    ? sessions.filter((s) => s.project_id === selectedProject)
    : sessions;

  const expanded = sessions.find((s) => s.id === expandedId) ?? null;

  const activeCount = filtered.filter(
    (s) => !["ended", "error"].includes(s.state)
  ).length;

  const attentionCount = filtered.filter(
    (s) => s.state === "needs_attention"
  ).length;

  const tree = useMemo(() => {
    const byParent = new Map<string | null, Session[]>();
    for (const s of filtered) {
      const k = s.parent_session_id ?? null;
      const list = byParent.get(k) ?? [];
      list.push(s);
      byParent.set(k, list);
    }
    const out: Array<{ s: Session; depth: number }> = [];
    const walk = (parentKey: string | null, depth: number) => {
      const kids = byParent.get(parentKey) ?? [];
      for (const k of kids) {
        out.push({ s: k, depth });
        walk(k.id, depth + 1);
      }
    };
    walk(null, 0);
    return out;
  }, [filtered]);

  return (
    <div className="flex flex-col h-screen">
      <header className="flex items-center justify-between px-4 py-3 border-b border-gray-800 bg-gray-900">
        <div className="flex items-center gap-4">
          <h1 className="text-lg font-bold">GalvoLite</h1>
          <ProjectSelector
            projects={projects}
            selected={selectedProject}
            onSelect={setSelectedProject}
            onRemove={setRemoveTarget}
          />
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-neutral-300 hover:text-neutral-100 border border-neutral-700 rounded px-2 py-1"
            onClick={() => setAddOpen(true)}
          >
            <Plus size={12} /> Add Project
          </button>
          {selectedProject && (
            <button
              type="button"
              className="flex items-center gap-1 text-xs text-neutral-300 hover:text-neutral-100 border border-neutral-700 rounded px-2 py-1"
              onClick={() => setStartOpen(true)}
            >
              <PlayCircle size={12} /> Start Session
            </button>
          )}
        </div>
        <div className="flex items-center gap-4 text-sm">
          <span className="text-gray-400">
            {activeCount} active
          </span>
          {attentionCount > 0 && (
            <span className="px-2 py-0.5 rounded bg-yellow-900 text-yellow-300">
              {attentionCount} need attention
            </span>
          )}
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden">
        <div className="flex-1 overflow-auto">
          {filtered.length === 0 ? (
            <div className="p-8 text-center text-gray-500">
              No sessions. Click <strong>Start Session</strong> to begin.
            </div>
          ) : (
            tree.map(({ s, depth }) => (
              <SessionRow
                key={s.id}
                session={s}
                depth={depth}
                onExpand={(sess) => expand(sess.id)}
                onRefresh={refresh}
              />
            ))
          )}
        </div>
      </div>

      <AddProjectModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onSubmit={(req) => createProject(req)}
      />
      {selectedProject && (
        <StartSessionModal
          open={startOpen}
          onClose={() => setStartOpen(false)}
          onSubmit={(req) => createSession(req)}
          projectId={selectedProject}
        />
      )}
      <RemoveProjectModal
        open={removeTarget !== null}
        projectName={removeTarget?.name ?? ""}
        sessionCount={
          removeTarget
            ? sessions.filter((s) => s.project_id === removeTarget.id).length
            : 0
        }
        onClose={() => setRemoveTarget(null)}
        onConfirm={async () => {
          if (!removeTarget) return;
          const removedId = removeTarget.id;
          await removeProject(removedId);
          if (selectedProject === removedId) setSelectedProject(null);
        }}
      />

      {expanded && (
        <ExpandedSessionView
          session={expanded}
          onClose={collapse}
          onSend={() => setModal("send")}
          onHandoff={() => setModal("handoff")}
          onEnd={() => setModal("end")}
          onToggleAutopilot={(next) => toggleAutopilot(expanded.id, next)}
        />
      )}

      {expanded && (
        <>
          <SendModal
            open={modal === "send"}
            onClose={() => setModal(null)}
            onSubmit={(text) => sendText(expanded.id, text)}
          />
          <HandoffModal
            open={modal === "handoff"}
            onClose={() => setModal(null)}
            onConfirm={() => triggerHandoff(expanded.id)}
          />
          <EndSessionModal
            open={modal === "end"}
            onClose={() => setModal(null)}
            onConfirm={async () => {
              await killSession(expanded.id);
              collapse();
            }}
            role={expanded.role}
          />
        </>
      )}

      {/* Per-session event subscriptions — renders nothing, keeps subscriptions
          alive for all sessions regardless of the current project filter. */}
      {sessions.map((s) => (
        <SessionEventBridge key={s.id} sessionId={s.id} onEvent={bridgeHandler} />
      ))}
    </div>
  );
}
