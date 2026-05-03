import { useEffect, useState } from "react";
import { Bell } from "lucide-react";
import { api } from "../api";

interface Props { projectId: string; onOpen: () => void; degraded: boolean; }

export function RecBell({ projectId, onOpen, degraded }: Props) {
  const [count, setCount] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api.listRecs(projectId, "pending")
      .then((rs) => !cancelled && setCount(rs.length))
      .catch(() => !cancelled && setCount(0));
    return () => { cancelled = true; };
  }, [projectId]);

  const color = degraded ? "text-yellow-400" : "text-neutral-300";
  return (
    <button
      type="button"
      aria-label="recommendations"
      onClick={onOpen}
      className="relative px-2 py-1 hover:bg-neutral-800 rounded"
    >
      <Bell size={16} className={color} />
      {count > 0 && (
        <span className="absolute -top-1 -right-1 bg-red-600 text-white text-[10px] rounded-full w-4 h-4 flex items-center justify-center">
          {count}
        </span>
      )}
    </button>
  );
}
