import "@testing-library/jest-dom/vitest";
import React from "react";
import { vi } from "vitest";

// xterm queries window.matchMedia at construction time; jsdom doesn't provide it.
if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

// useTerminal uses ResizeObserver for fit-on-resize; jsdom lacks it.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}

// Framer Motion animations are not meaningful in jsdom.
// Mock `motion.*` to regular DOM elements so markup queries still work.
vi.mock("framer-motion", async () => {
  const actual = await vi.importActual<typeof import("framer-motion")>(
    "framer-motion",
  );
  // Cache Components per tag so React sees a stable component type across
  // re-renders (otherwise every render unmounts/remounts the subtree, which
  // resets controlled-input state and focus).
  const componentCache = new Map<string, React.ComponentType<Record<string, unknown>>>();
  return {
    ...actual,
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    motion: new Proxy(
      {},
      {
        get: (_t, tag: string) => {
          let Component = componentCache.get(tag);
          if (!Component) {
            Component = (props: Record<string, unknown>) => {
              // Strip motion-only props that React complains about.
              const {
                initial,
                animate,
                exit,
                transition,
                variants,
                whileHover,
                whileTap,
                layoutId,
                ...rest
              } = props;
              return React.createElement(tag, rest, (props as any).children);
            };
            componentCache.set(tag, Component);
          }
          return Component;
        },
      },
    ),
  };
});
