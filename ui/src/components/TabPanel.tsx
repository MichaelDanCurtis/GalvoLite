import type { ReactNode } from "react";

interface TabPanelProps {
  /** `data-testid` on the outer wrapper (e.g., `tab-panel-events`). */
  testId: string;
  /**
   * Whether to render the error fallback instead of `children`. Callers
   * decide the semantic — today it's `list.length === 0 && loadError` so a
   * partial-load is still shown, not hidden by a late failure. Preserve
   * that by computing the condition at the call site.
   */
  showError: boolean;
  /** `data-testid` on the error fallback (e.g., `events-error`). */
  errorTestId: string;
  /** Error message displayed inside the fallback. */
  errorText: string;
  /** Normal tab contents, rendered when `showError` is false. */
  children: ReactNode;
}

/**
 * Thin wrapper shared by the Events and Plan tab panels. Factored out to
 * collapse the duplicated error-fallback markup (Sub-plan 4 Task 14 review).
 * Keep it dumb: no condition logic lives here — the caller passes in
 * `showError` already computed.
 */
export function TabPanel({
  testId,
  showError,
  errorTestId,
  errorText,
  children,
}: TabPanelProps) {
  return (
    <div data-testid={testId} className="h-full overflow-hidden">
      {showError ? (
        <div data-testid={errorTestId} className="p-4 text-sm text-red-400">
          {errorText}
        </div>
      ) : (
        children
      )}
    </div>
  );
}
