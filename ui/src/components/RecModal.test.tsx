import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RecModal } from "./RecModal";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    listRecs: vi.fn(),
    actRecs: vi.fn(),
    deferRec: vi.fn(),
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
});
