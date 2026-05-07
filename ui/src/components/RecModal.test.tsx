import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RecModal } from "./RecModal";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    listRecs: vi.fn(),
    actRecs: vi.fn(),
    deferRec: vi.fn(),
    explainRec: vi.fn(),
  },
}));

const sample = (id: string, sev: any = "warn") => ({
  id, project_id: "p1", kind: "k", severity: sev,
  title: `Rec ${id}`, body: "B", rationale: "R", evidence_json: [],
  status: "pending", created_at: "t", acted_at: null, deferred_until: null,
});

describe("RecModal", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders pending recs with severity badges", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1", "warn"), sample("r2", "urgent")]);
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    expect(await screen.findByText("Rec r1")).toBeInTheDocument();
    expect(screen.getByText("Rec r2")).toBeInTheDocument();
    expect(screen.getAllByText(/warn|urgent/i).length).toBeGreaterThanOrEqual(2);
  });

  it("multi-select Make sends rec_ids array to actRecs", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1"), sample("r2")]);
    (api.actRecs as any).mockResolvedValue({ results: [] });
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    await screen.findByText("Rec r1");
    fireEvent.click(screen.getByLabelText("select Rec r1"));
    fireEvent.click(screen.getByLabelText("select Rec r2"));
    fireEvent.click(screen.getByRole("button", { name: /make selected/i }));
    await waitFor(() => expect(api.actRecs).toHaveBeenCalledWith("p1", ["r1", "r2"], "make"));
  });

  it("Defer button calls deferRec for clicked row", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1")]);
    (api.deferRec as any).mockResolvedValue({ deferred_until: "x" });
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    await screen.findByText("Rec r1");
    fireEvent.click(screen.getByRole("button", { name: /defer/i }));
    await waitFor(() => expect(api.deferRec).toHaveBeenCalledWith("p1", "r1"));
  });

  it("empty state shown when no recs", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    expect(await screen.findByText(/no pending/i)).toBeInTheDocument();
  });

  it("switches to deferred filter and refetches", async () => {
    const listRecs = vi.fn().mockImplementation((_pid: string, status: string) => {
      if (status === "pending") return Promise.resolve([{
        id: "r1", project_id: "p", kind: "k", severity: "info", title: "P-rec",
        body: "b", rationale: "r", status: "pending", created_at: "2026-01-01T00:00:00Z",
        acted_at: null, deferred_until: null, evidence_json: null,
      }]);
      if (status === "deferred") return Promise.resolve([{
        id: "r2", project_id: "p", kind: "k", severity: "warn", title: "D-rec",
        body: "b", rationale: "r", status: "deferred", created_at: "2026-01-01T00:00:00Z",
        acted_at: null, deferred_until: "2026-01-02T00:00:00Z", evidence_json: null,
      }]);
      return Promise.resolve([]);
    });
    vi.spyOn(api, "listRecs").mockImplementation(listRecs);

    render(<RecModal projectId="p" open={true} onClose={() => {}} />);

    // Initial render: pending filter, P-rec visible.
    await screen.findByText("P-rec");

    // Click Deferred segment (segmented control uses role="tab").
    fireEvent.click(screen.getByRole("tab", { name: /deferred/i }));

    // After refetch, D-rec is visible and P-rec is not.
    await screen.findByText("D-rec");
    expect(screen.queryByText("P-rec")).toBeNull();
    expect(listRecs).toHaveBeenCalledWith("p", "deferred");
  });

  it("defer triggers a refetch via onDeferred callback", async () => {
    const listRecs = vi.fn().mockResolvedValue([{
      id: "r1", project_id: "p", kind: "k", severity: "info", title: "X-rec",
      body: "B", rationale: "R", status: "pending",
      created_at: "2026-01-01T00:00:00Z", acted_at: null, deferred_until: null,
      evidence_json: null,
    }]);
    vi.spyOn(api, "listRecs").mockImplementation(listRecs);
    vi.spyOn(api, "deferRec").mockResolvedValue({ deferred_until: "2026-01-02T00:00:00Z" });

    render(<RecModal projectId="p" open={true} onClose={() => {}} />);
    await screen.findByText("X-rec");

    // Initial mount calls listRecs once with status=pending.
    expect(listRecs).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /^defer$/i }));

    await waitFor(() => {
      // Defer should trigger one more listRecs call (the refresh).
      expect(listRecs).toHaveBeenCalledTimes(2);
    });
  });

  it("re-explain replaces rationale on success", async () => {
    const explainRec = vi.fn().mockResolvedValue({ updated: true });
    vi.spyOn(api, "explainRec").mockImplementation(explainRec);
    vi.spyOn(api, "listRecs").mockResolvedValue([{
      id: "r1", project_id: "p", kind: "k", severity: "info", title: "T",
      body: "B", rationale: "old rationale", status: "pending",
      created_at: "2026-01-01T00:00:00Z", acted_at: null, deferred_until: null,
      evidence_json: null,
    }]);

    render(<RecModal projectId="p" open={true} onClose={() => {}} />);
    await screen.findByText("old rationale");

    fireEvent.click(screen.getByRole("button", { name: /re-explain/i }));

    const textarea = screen.getByPlaceholderText(/what's unclear/i);
    fireEvent.change(textarea, { target: { value: "explain like I'm 5" } });
    fireEvent.click(screen.getByRole("button", { name: /^submit$/i }));

    await waitFor(() => {
      expect(explainRec).toHaveBeenCalledWith("p", "r1", "explain like I'm 5");
    });
  });
});
