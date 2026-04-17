import { useEffect, useState } from "react";
import { api } from "../api";
import type { Project, CreateProjectReq } from "../types";

export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    api
      .listProjects()
      .then((data) => {
        setProjects(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
  }, []);

  const createProject = async (req: CreateProjectReq): Promise<Project> => {
    const p = await api.createProject(req);
    refresh();
    return p;
  };

  const removeProject = async (id: string): Promise<void> => {
    await api.deleteProject(id);
    refresh();
  };

  return { projects, loading, error, refresh, createProject, removeProject };
}
