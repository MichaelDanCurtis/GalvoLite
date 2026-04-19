interface Props {
  role: string;
}

const STYLES: Record<string, { bg: string; fg: string; label: string }> = {
  orchestrator: { bg: "bg-purple-900", fg: "text-purple-200", label: "ORCH" },
  brainstorm: { bg: "bg-sky-900", fg: "text-sky-200", label: "BRAIN" },
  implementation: { bg: "bg-emerald-900", fg: "text-emerald-200", label: "IMPL" },
  scout: { bg: "bg-amber-900", fg: "text-amber-200", label: "SCOUT" },
};

export function RoleBadge({ role }: Props) {
  const style = STYLES[role.toLowerCase()] ?? {
    bg: "bg-neutral-800",
    fg: "text-neutral-300",
    label: role.toUpperCase(),
  };
  return (
    <span
      className={`inline-block text-[10px] font-bold tracking-wider px-1.5 py-0.5 rounded ${style.bg} ${style.fg}`}
    >
      {style.label}
    </span>
  );
}
