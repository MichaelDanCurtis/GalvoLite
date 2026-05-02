import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Decision, Postmortem } from "../types";
import { PostmortemsPanel } from "./PostmortemsPanel";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    listPostmortems: vi.fn(),
    listDecisions: vi.fn(),
  },
}));

const mockListPm = vi.mocked(api.listPostmortems);
const mockListDec = vi.mocked(api.listDecisions);

const stubPm: Postmortem = {
  plan_part: 5,
  session_id: "00000000-0000-0000-0000-000000000001",
  outcome: "stuck",
  cycles: 4,
  duration_seconds: 3600,
  written_at: "2026-05-01T12:00:00Z",
  what_worked: "Repro nailed",
  gotchas: "DB lock contention",
  follow_ups: "Audit migrations",
};

const stubDec: Decision = {
  session_id: "00000000-0000-0000-0000-000000000002",
  asked_at: "2026-05-01T13:00:00Z",
  question: "Use SQLite?",
  answer: "Yes",
  choices: null,
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("PostmortemsPanel", () => {
  it("renders postmortem and decision counts", async () => {
    mockListPm.mockResolvedValueOnce([stubPm]);
    mockListDec.mockResolvedValueOnce([stubDec]);
    render(<PostmortemsPanel projectId="p1" />);
    await waitFor(() => expect(screen.getByText("Postmortems")).toBeInTheDocument());
    // Each section has a count badge with "1"
    const counts = screen.getAllByText("1");
    expect(counts.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("part 5")).toBeInTheDocument();
    expect(screen.getByText("Use SQLite?")).toBeInTheDocument();
  });

  it("expands postmortem body on click", async () => {
    mockListPm.mockResolvedValueOnce([stubPm]);
    mockListDec.mockResolvedValueOnce([]);
    render(<PostmortemsPanel projectId="p1" />);
    await waitFor(() => expect(screen.getByText("part 5")).toBeInTheDocument());
    fireEvent.click(screen.getByText("part 5"));
    await waitFor(() => expect(screen.getByText(/DB lock contention/)).toBeInTheDocument());
  });

  it("expands decision body on click", async () => {
    mockListPm.mockResolvedValueOnce([]);
    mockListDec.mockResolvedValueOnce([stubDec]);
    render(<PostmortemsPanel projectId="p1" />);
    await waitFor(() => expect(screen.getByText("Use SQLite?")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Use SQLite?"));
    await waitFor(() => {
      const matches = screen.getAllByText(/Yes/);
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows error state when listPostmortems rejects", async () => {
    mockListPm.mockRejectedValueOnce(new Error("502"));
    mockListDec.mockResolvedValueOnce([]);
    render(<PostmortemsPanel projectId="p1" />);
    await waitFor(() => expect(screen.getByText(/Error: 502/)).toBeInTheDocument());
  });

  it("shows empty state when no postmortems and no decisions", async () => {
    mockListPm.mockResolvedValueOnce([]);
    mockListDec.mockResolvedValueOnce([]);
    render(<PostmortemsPanel projectId="p1" />);
    await waitFor(() => expect(screen.getByText(/no postmortems yet/i)).toBeInTheDocument());
    expect(screen.getByText(/no decisions yet/i)).toBeInTheDocument();
  });
});
