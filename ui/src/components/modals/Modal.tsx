import { useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X } from "lucide-react";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
  originRect?: DOMRect | null;
}

export function Modal({ open, onClose, title, children, originRect }: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const spring = originRect
    ? {
        initial: {
          opacity: 0,
          scale: 0.2,
          x: originRect.left - window.innerWidth / 2,
          y: originRect.top - window.innerHeight / 2,
        },
        animate: { opacity: 1, scale: 1, x: 0, y: 0 },
        exit: { opacity: 0, scale: 0.5 },
      }
    : {
        initial: { opacity: 0, scale: 0.9 },
        animate: { opacity: 1, scale: 1 },
        exit: { opacity: 0, scale: 0.9 },
      };

  return (
    <AnimatePresence>
      {open ? (
        <motion.div
          key="backdrop"
          data-testid="modal-backdrop"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={onClose}
        >
          <motion.div
            key="panel"
            className="relative bg-neutral-900 border border-neutral-700 rounded-lg min-w-[360px] max-w-[560px] w-full mx-4 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
            {...spring}
            transition={{ type: "spring", stiffness: 260, damping: 22 }}
          >
            <header className="flex items-center justify-between px-4 py-3 border-b border-neutral-800">
              <h2 className="text-sm font-semibold text-neutral-100">{title}</h2>
              <button
                aria-label="Close"
                className="text-neutral-400 hover:text-neutral-100"
                onClick={onClose}
              >
                <X size={16} />
              </button>
            </header>
            <div className="p-4">{children}</div>
          </motion.div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
