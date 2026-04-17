import type {
  Project,
  Session,
  SignalRow,
  CreateProjectReq,
  CreateSessionReq,
  SendReq,
} from "./types";

const BASE = "";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`${resp.status}: ${text}`);
  }
  if (resp.status === 204) {
    return undefined as T;
  }
  const len = resp.headers.get("content-length");
  if (len === "0") {
    return undefined as T;
  }
  return resp.json() as Promise<T>;
}

export const api = {
  listProjects: () => request<Project[]>("/api/projects"),

  createProject: (req: CreateProjectReq) =>
    request<Project>("/api/projects", {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getProject: (id: string) => request<Project>(`/api/projects/${id}`),

  listSessions: () => request<Session[]>("/api/sessions"),

  createSession: (req: CreateSessionReq) =>
    request<Session>("/api/sessions", {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getSession: (id: string) => request<Session>(`/api/sessions/${id}`),

  killSession: (id: string) =>
    request<{ status: string }>(`/api/sessions/${id}`, { method: "DELETE" }),

  triggerHandoff: (id: string) =>
    request<{ status: string }>(`/api/sessions/${id}/handoff`, {
      method: "POST",
    }),

  sendToSession: (id: string, req: SendReq) =>
    request<{ status: string }>(`/api/sessions/${id}/send`, {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getSignals: (id: string) =>
    request<SignalRow[]>(`/api/sessions/${id}/signals`),

  deleteProject: (id: string) =>
    request<void>(`/api/projects/${id}`, { method: "DELETE" }),

  setAutopilot: (id: string, enabled: boolean) =>
    request<{ autopilot: boolean }>(`/api/sessions/${id}/autopilot`, {
      method: "PUT",
      body: JSON.stringify({ enabled }),
    }),

  terminalWsUrl: (id: string): string => {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/api/sessions/${id}/terminal`;
  },
};
