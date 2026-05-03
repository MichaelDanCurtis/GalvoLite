import type {
  Project,
  Session,
  SignalRow,
  EventRow,
  PlanStep,
  CreateProjectReq,
  CreateSessionReq,
  SendReq,
  DirectoryListing,
  Directive,
  Postmortem,
  Decision,
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

  getSessionEvents: (id: string, limit: number = 100) =>
    request<EventRow[]>(`/api/sessions/${id}/events?limit=${limit}`),

  // No limit/cursor — plan_steps per session is bounded (~dozens at most).
  getSessionPlanSteps: (id: string) =>
    request<PlanStep[]>(`/api/sessions/${id}/plan_steps`),

  deleteProject: (id: string) =>
    request<void>(`/api/projects/${id}`, { method: "DELETE" }),

  setAutopilot: (id: string, enabled: boolean) =>
    request<{ autopilot: boolean }>(`/api/sessions/${id}/autopilot`, {
      method: "PUT",
      body: JSON.stringify({ enabled }),
    }),

  listDirectory: (path: string) =>
    request<DirectoryListing>(
      `/api/fs/list?path=${encodeURIComponent(path)}`,
    ),

  answerOperatorPrompt: (promptId: string, answer: string) =>
    request<void>(`/api/operator/prompt/${encodeURIComponent(promptId)}/answer`, {
      method: "POST",
      body: JSON.stringify({ answer }),
    }),

  /**
   * Fetch the plan.md markdown for a project. Returns `null` when no plan.md
   * has been written yet (server returns 404). Any other non-2xx is thrown.
   */
  getPlan: async (projectId: string): Promise<{ markdown: string } | null> => {
    const resp = await fetch(`/api/projects/${projectId}/plan`);
    if (resp.status === 404) return null;
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${resp.status}: ${text}`);
    }
    return resp.json() as Promise<{ markdown: string }>;
  },

  /**
   * Fetch the directive for a project. Returns `null` when no directive
   * has been written yet (server returns 404). Any other non-2xx is thrown.
   *
   * Bypasses the shared `request<T>` helper because 404 must map to `null`
   * rather than throw, which `request<T>`'s signature can't express.
   */
  getDirective: async (projectId: string): Promise<Directive | null> => {
    const resp = await fetch(`/api/projects/${projectId}/directive`);
    if (resp.status === 404) return null;
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${resp.status}: ${text}`);
    }
    return resp.json() as Promise<Directive>;
  },

  /**
   * Write the directive markdown for a project. The server parses + atomically
   * writes; on parse failure the server returns 400 with the parse error in the
   * body, which we surface as the thrown Error message.
   *
   * Bypasses the shared `request<T>` helper because the success response is
   * 200 with an empty body and no `content-length`; `request<T>` would hand
   * that to `resp.json()` and throw a parse error.
   */
  putDirective: async (projectId: string, markdown: string): Promise<void> => {
    const resp = await fetch(`/api/projects/${projectId}/directive`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ markdown }),
    });
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${resp.status}: ${text}`);
    }
  },

  listPostmortems: async (projectId: string): Promise<Postmortem[]> => {
    const resp = await fetch(`/api/projects/${projectId}/postmortems`);
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${resp.status}: ${text}`);
    }
    return resp.json() as Promise<Postmortem[]>;
  },

  listDecisions: async (projectId: string): Promise<Decision[]> => {
    const resp = await fetch(`/api/projects/${projectId}/decisions`);
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${resp.status}: ${text}`);
    }
    return resp.json() as Promise<Decision[]>;
  },

  terminalWsUrl: (id: string): string => {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/api/sessions/${id}/terminal`;
  },

  executePlan: (projectId: string) =>
    request<{ spawned: string[] }>(
      `/api/projects/${projectId}/execute-plan`,
      { method: "POST", body: "{}" },
    ),

  approvePhase: (projectId: string, phaseId: string) =>
    request<{ approved: string }>(
      `/api/projects/${projectId}/phases/${encodeURIComponent(phaseId)}/approve`,
      { method: "POST", body: "{}" },
    ),
};
