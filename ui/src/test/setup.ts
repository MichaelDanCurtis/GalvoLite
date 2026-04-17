import "@testing-library/jest-dom/vitest";
import React from "react";
import { vi } from "vitest";

// Framer Motion animations are not meaningful in jsdom.
// Mock `motion.*` to regular DOM elements so markup queries still work.
vi.mock("framer-motion", async () => {
  const actual = await vi.importActual<typeof import("framer-motion")>(
    "framer-motion",
  );
  return {
    ...actual,
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    motion: new Proxy(
      {},
      {
        get: (_t, tag: string) => {
          const Component = (props: Record<string, unknown>) => {
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
          return Component;
        },
      },
    ),
  };
});
