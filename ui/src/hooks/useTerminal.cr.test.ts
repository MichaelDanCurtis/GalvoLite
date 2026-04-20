import { describe, it, expect } from "vitest";
import { mapKeystrokeForPty } from "./useTerminal";

describe("mapKeystrokeForPty (Finding 2)", () => {
  it("translates a bare LF to CR", () => {
    expect(mapKeystrokeForPty("\n")).toBe("\r");
  });
  it("preserves printable text", () => {
    expect(mapKeystrokeForPty("ls")).toBe("ls");
  });
  it("translates trailing LF in multi-char input", () => {
    expect(mapKeystrokeForPty("ls\n")).toBe("ls\r");
  });
  it("leaves CR alone", () => {
    expect(mapKeystrokeForPty("\r")).toBe("\r");
  });
});
