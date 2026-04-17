import { useState } from "react";
import { useProjects } from "../hooks/useProjects";
import { useSessions } from "../hooks/useSessions";
import { ProjectSelector } from "./ProjectSelector";
import { SessionRow } from "./SessionRow";
import { SessionDetail } from "./SessionDetail";

export function Dashboard() {
  const { projects } = useProjects();
  const { sessions, refresh } = useSessions(2000);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);

  const filtered = selectedProject
    ? sessions.filter((s) => s.project_id === selectedProject)
    : sessions;

  const detail = filtered.find((s) => s.id === selectedSession);

  const activeCount = filtered.filter(
    (s) => !["complete", "killed", "error"].includes(s.state)
  ).length;

  const attentionCount = filtered.filter(
    (s) => s.state === "attention_needed"
  ).length;

  return (
    <div className="flex flex-col h-screen">
      <header className="flex items-center justify-between px-4 py-3 border-b border-gray-800 bg-gray-900">
        <div className="flex items-center gap-4">
          <h1 className="text-lg font-bold">GalvoLite</h1>
          <ProjectSelector
            projects={projects}
            selected={selectedProject}
            onSelect={setSelectedProject}
          />
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
              No sessions. Start one via the API or CLI.
            </div>
          ) : (
            filtered.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                selected={s.id === selectedSession}
                onSelect={setSelectedSession}
                onRefresh={refresh}
              />
            ))
          )}
        </div>

        {detail && (
          <div className="w-[480px] shrink-0">
            <SessionDetail session={detail} onRefresh={refresh} />
          </div>
        )}
      </div>
    </div>
  );
}
