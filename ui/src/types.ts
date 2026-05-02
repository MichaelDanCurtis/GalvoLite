export type SessionState =
  | "starting" | "idle" | "working" | "awaiting_input"
  | "needs_attention" | "error" | "ended";

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
  // Sub-plan 4: patched via session.meta_changed event or fetched per-session
  plan_markdown?: string | null;
  seed_markdown?: string | null;
  handoff_path?: string | null;
  turn_count?: number;
  est_tokens?: number;
  last_event_kind?: string;
  waiting_on?: string[] | null;
}

/**
 * One row as returned by `GET /api/sessions/:id/events` (and shaped
 * identically to the MCP `read_events` tool). Note the field is `id`, not
 * `event_id` — matching the events table primary key.
 */
export interface EventRow {
  id: number;
  ts: string;
  source: string;
  kind: string;
  payload: unknown;
  prev_state?: string | null;
  next_state?: string | null;
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

export interface DirectoryEntry {
  name: string;
  is_dir: boolean;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  entries: DirectoryEntry[];
}

export const STATE_COLORS: Record<SessionState, string> = {
  starting: "bg-blue-400",
  idle: "bg-gray-300",
  working: "bg-green-400",
  awaiting_input: "bg-yellow-400",
  needs_attention: "bg-orange-400",
  error: "bg-red-500",
  ended: "bg-gray-400",
};

export type PlanStepStatus = "pending" | "working" | "done" | "blocked";
export interface PlanStep { hash: string; text: string; status: PlanStepStatus; note?: string }

export interface ChildResolution {
  id: string;
  role: string;
  status: "complete" | "blocked" | "error" | "ended" | "unknown";
  handoff: string | null;
}

export type Autonomy = "full" | "review" | "manual";

export interface Phase {
  id: string;
  parts: number[];
  autonomy: Autonomy;
  depends_on: string[];
}

export interface EscalationRule {
  trigger: string;
  action: string;
}

export interface Directive {
  name: string;
  plan_path: string;
  phases: Phase[];
  escalation_rules: EscalationRule[];
  notes: string;
  raw_markdown: string;
}

export type PostmortemOutcome = "shipped" | "stuck" | "escalated";

export interface Postmortem {
  plan_part: number;
  session_id: string;
  outcome: PostmortemOutcome;
  cycles: number;
  duration_seconds: number;
  written_at: string; // ISO 8601
  what_worked: string;
  gotchas: string;
  follow_ups: string;
}

export interface Decision {
  session_id: string;
  asked_at: string; // ISO 8601
  question: string;
  answer: string;
  choices: string[] | null;
}

export type DomainEvent =
  | { type: "session.state_changed"; session_id: string; prev: SessionState | null; next: SessionState; last_event_ts: string }
  | { type: "session.event_appended"; session_id: string; event_id: number; kind: string; source: string; ts: string; payload: unknown }
  | { type: "session.role_changed"; session_id: string; role: string }
  | { type: "session.meta_changed"; session_id: string; turn_count: number; est_tokens: number; plan_markdown: string | null; seed_markdown: string | null; handoff_path: string | null }
  | { type: "session.plan_replaced"; session_id: string; steps: PlanStep[] }
  | { type: "session.plan_step_changed"; session_id: string; step_hash: string; prev_status: PlanStepStatus; next_status: PlanStepStatus; note: string | null }
  | { type: "operator.prompt"; prompt_id: string; session_id: string; question: string; choices: string[] | null; timeout_secs: number | null }
  | { type: "operator.toast"; session_id: string; level: "info" | "warn" | "urgent"; message: string }
  | { type: "session.resumed"; session_id: string; last_events: EventRow[] }
  | {
      type: "session.children_resolved";
      session_id: string;
      children: ChildResolution[];
    }
  | { type: "directive.changed"; project_id: string };
