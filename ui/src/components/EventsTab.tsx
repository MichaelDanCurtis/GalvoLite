import type { EventRow } from "../types";

interface Props {
  events: EventRow[];
}

/**
 * Reverse-chronological events feed for the "Events" tab of
 * ExpandedSessionView. The parent is responsible for ordering: rows must
 * arrive newest-first (the backend returns ORDER BY id DESC, and push-channel
 * appenders must prepend).
 */
export function EventsTab({ events }: Props) {
  if (events.length === 0) {
    return (
      <div
        data-testid="events-empty"
        className="p-4 text-sm text-neutral-500"
      >
        No events yet.
      </div>
    );
  }
  return (
    <ul className="overflow-y-auto h-full divide-y divide-neutral-900">
      {events.map((e) => (
        <li
          key={e.id}
          data-testid="event-row"
          className="flex items-center gap-3 px-4 py-1.5 text-xs font-mono"
        >
          <span
            className="px-1.5 py-0.5 rounded bg-neutral-800 text-neutral-300 uppercase shrink-0"
            data-testid="event-source"
          >
            {e.source}
          </span>
          <span className="text-neutral-100 truncate flex-1 min-w-0">{e.kind}</span>
          <span className="text-neutral-500 shrink-0">{e.ts}</span>
        </li>
      ))}
    </ul>
  );
}
