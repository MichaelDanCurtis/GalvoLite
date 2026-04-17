import type { Project } from "../types";

interface Props {
  projects: Project[];
  selected: string | null;
  onSelect: (id: string | null) => void;
}

export function ProjectSelector({ projects, selected, onSelect }: Props) {
  return (
    <div className="flex items-center gap-3">
      <label className="text-sm text-gray-400">Project:</label>
      <select
        className="bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-blue-500"
        value={selected ?? ""}
        onChange={(e) => onSelect(e.target.value || null)}
      >
        <option value="">All projects</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
    </div>
  );
}
