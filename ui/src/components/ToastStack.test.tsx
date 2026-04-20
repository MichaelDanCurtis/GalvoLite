import { render, screen, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { ToastStack } from "./ToastStack";

describe("ToastStack", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("renders an info toast and auto-dismisses after 4s", () => {
    const onDismiss = vi.fn();
    render(
      <ToastStack
        toasts={[{ id: "1", level: "info", message: "hi" }]}
        onDismiss={onDismiss}
      />
    );
    expect(screen.getByText("hi")).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(4100);
    });
    expect(onDismiss).toHaveBeenCalledWith("1");
  });

  it("urgent toasts do not auto-dismiss", () => {
    const onDismiss = vi.fn();
    render(
      <ToastStack
        toasts={[{ id: "1", level: "urgent", message: "!!" }]}
        onDismiss={onDismiss}
      />
    );
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("info toast calls onDismiss exactly once after 4s", () => {
    const onDismiss = vi.fn();
    render(
      <ToastStack
        toasts={[{ id: "info-1", level: "info", message: "fading away" }]}
        onDismiss={onDismiss}
      />
    );
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledWith("info-1");
  });

  it("warn toast calls onDismiss after 8s but not before", () => {
    const onDismiss = vi.fn();
    render(
      <ToastStack
        toasts={[{ id: "warn-1", level: "warn", message: "heads up" }]}
        onDismiss={onDismiss}
      />
    );
    act(() => {
      vi.advanceTimersByTime(7999);
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledWith("warn-1");
  });
});
