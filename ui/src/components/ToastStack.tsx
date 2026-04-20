import { useEffect } from "react";

export interface Toast {
  id: string;
  level: "info" | "warn" | "urgent";
  message: string;
}

const AUTO_DISMISS_MS: Record<Toast["level"], number | null> = {
  info: 4000,
  warn: 8000,
  urgent: null,
};

const BORDER_COLOR: Record<Toast["level"], string> = {
  info: "border-blue-400",
  warn: "border-orange-400",
  urgent: "border-red-500",
};

interface SingleToastProps {
  toast: Toast;
  onDismiss: (id: string) => void;
}

function SingleToast({ toast, onDismiss }: SingleToastProps) {
  const duration = AUTO_DISMISS_MS[toast.level];

  useEffect(() => {
    if (duration === null) return;
    const t = setTimeout(() => onDismiss(toast.id), duration);
    return () => clearTimeout(t);
  }, [toast.id, duration, onDismiss]);

  return (
    <div
      role={toast.level === "urgent" ? "alert" : "status"}
      className={`flex items-start justify-between gap-3 min-w-[280px] max-w-[400px] bg-neutral-900 border-l-4 ${BORDER_COLOR[toast.level]} rounded-r-lg px-4 py-3 shadow-lg`}
    >
      <span className="text-sm text-neutral-100 leading-snug flex-1">
        {toast.message}
      </span>
      <button
        type="button"
        aria-label="Dismiss"
        className="text-neutral-400 hover:text-neutral-100 text-base leading-none flex-shrink-0 mt-0.5"
        onClick={() => onDismiss(toast.id)}
      >
        ×
      </button>
    </div>
  );
}

interface ToastStackProps {
  toasts: Toast[];
  onDismiss: (id: string) => void;
}

export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  if (toasts.length === 0) return null;

  // Render newest-first, up to 5; timers run for all toasts (see Option B).
  // We need timers for ALL toasts, but only render the newest 5.
  // We do this by rendering hidden timer-only instances for overflowed toasts.
  const visible = toasts.slice(-5).reverse();
  const overflow = toasts.length > 5 ? toasts.slice(0, toasts.length - 5) : [];

  return (
    <div
      role="region"
      aria-label="Notifications"
      className="fixed top-4 right-4 z-[70] flex flex-col gap-2"
    >
      {/* Hidden overflow: run their timers but don't display */}
      {overflow.map((t) => (
        <div key={t.id} className="hidden" aria-hidden="true">
          <SingleToast toast={t} onDismiss={onDismiss} />
        </div>
      ))}
      {/* Visible: newest 5 */}
      {visible.map((t) => (
        <SingleToast key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
