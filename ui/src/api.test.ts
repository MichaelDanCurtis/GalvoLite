import { describe, test, expect, vi, beforeEach, afterEach } from "vitest";
import { api } from "./api";
import type { Directive } from "./types";

const SAMPLE_DIRECTIVE: Directive = {
  name: "Test",
  plan_path: "./plan.md",
  phases: [{ id: "P1", parts: [1], autonomy: "full", depends_on: [] }],
  escalation_rules: [],
  notes: "",
  raw_markdown: "# Directive: Test\n",
};

describe("api.getDirective", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  test("parses 404 as null", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("", { status: 404 }),
    );
    const result = await api.getDirective("p1");
    expect(result).toBeNull();
  });

  test("parses 200 as Directive", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response(JSON.stringify(SAMPLE_DIRECTIVE), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const result = await api.getDirective("p1");
    expect(result).toEqual(SAMPLE_DIRECTIVE);
  });

  test("throws on 500 with body in message", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("boom", { status: 500 }),
    );
    await expect(api.getDirective("p1")).rejects.toThrow("500: boom");
  });
});

describe("api.putDirective", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  test("resolves on 200 (empty body)", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("", { status: 200 }),
    );
    await expect(api.putDirective("p1", "# ok")).resolves.toBeUndefined();
  });

  test("surfaces 400 message", async () => {
    (globalThis.fetch as ReturnType<typeof vi.fn>).mockResolvedValueOnce(
      new Response("missing required section: Phases", { status: 400 }),
    );
    await expect(api.putDirective("p1", "broken")).rejects.toThrow(
      "400: missing required section: Phases",
    );
  });

  test("sends markdown in JSON body", async () => {
    const fetchMock = globalThis.fetch as ReturnType<typeof vi.fn>;
    fetchMock.mockResolvedValueOnce(new Response("", { status: 200 }));
    await api.putDirective("p1", "# Directive: X");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/p1/directive",
      expect.objectContaining({
        method: "PUT",
        headers: expect.objectContaining({ "Content-Type": "application/json" }),
        body: JSON.stringify({ markdown: "# Directive: X" }),
      }),
    );
  });
});
