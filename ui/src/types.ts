export type SessionState =
  | "spawning"
  | "working"
  | "attention_needed"
  | "handing_off"
  | "restoring"
  | "complete"
  | "killed"
  | "error";

export interface Project {
  id: string;
  name: string;
  path: string;
  remote_url: string | null;
  created_at: string;
}

export interface Session {
  id: string;
  project_id: string;
  role: string;
  branch: string | null;
  worktree_path: string;
  pty_pid: number | null;
  state: SessionState;
  autopilot: boolean;
  parent_session_id: string | null;
  task_summary: string;
  created_at: string;
  ended_at: string | null;
}

export interface SignalRow {
  // i64 server-side; JS number is safe to 2^53.
  id: number;
  session_id: string;
  cycle_id: string | null;
  kind: string;
  payload: string | null;
  at: string;
}

export interface CreateSessionReq {
  project_id: string;
  task: string;
  role?: string;
  claude_cmd?: string;
  args?: string[];
}

export interface CreateProjectReq {
  name: string;
  path: string;
  remote_url?: string;
}

export interface SendReq {
  text: string;
  newline?: boolean;
}

export const STATE_COLORS: Record<SessionState, string> = {
  spawning: "bg-blue-400",
  working: "bg-green-400",
  attention_needed: "bg-yellow-400",
  handing_off: "bg-blue-400",
  restoring: "bg-blue-400",
  complete: "bg-gray-400",
  killed: "bg-red-400",
  error: "bg-red-500",
};
