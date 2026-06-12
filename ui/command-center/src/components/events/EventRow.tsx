import { useMemo } from 'react';
import type { EventRecord } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// Color-coded type badges per spec:
// green = success/completed, blue = info/started, red = errors/failed, yellow = warnings
const TYPE_COLORS: Record<string, { bg: string; text: string }> = {
  task_completed:         { bg: 'bg-emerald-500/15', text: 'text-emerald-400' },
  skill_saved:            { bg: 'bg-emerald-500/15', text: 'text-emerald-400' },
  integration_connected:  { bg: 'bg-emerald-500/15', text: 'text-emerald-400' },
  daemon_started:         { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  task_created:           { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  task_started:           { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  message_received:       { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  stream_chunk:           { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  memory_added:           { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  skill_proposed:         { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  skill_triggered:        { bg: 'bg-blue-500/15',    text: 'text-blue-400' },
  task_failed:            { bg: 'bg-red-500/15',     text: 'text-red-400' },
  integration_error:      { bg: 'bg-red-500/15',     text: 'text-red-400' },
  daemon_stopped:         { bg: 'bg-amber-500/15',   text: 'text-amber-400' },
};

const DEFAULT_COLOR = { bg: 'bg-slate-500/15', text: 'text-slate-400' };

function relativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 0) return 'just now';
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function summarizePayload(event: EventRecord): string {
  const p = event.payload;
  switch (event.event_type) {
    case 'task_created':
      return (p.title as string) || (p.task_id as string)?.slice(0, 8) || 'New task';
    case 'task_started':
      return (p.title as string) || 'Task started';
    case 'task_completed':
      return (p.title as string) || 'Task completed';
    case 'task_failed':
      return `Failed: ${(p.error as string) || '?'}`;
    case 'memory_added':
      return `Memory: ${(p.key as string) || '?'}`;
    case 'skill_proposed':
      return `Proposed: ${(p.name as string) || (p.description as string) || '?'}`;
    case 'skill_saved':
      return `Saved: ${(p.name as string) || '?'}`;
    case 'skill_triggered':
      return `Triggered: ${(p.name as string) || '?'}`;
    case 'message_received':
      return `From ${(p.role as string) || '?'}`;
    case 'daemon_started':
      return 'Daemon started';
    case 'daemon_stopped':
      return 'Daemon stopped';
    case 'integration_connected':
      return `Connected: ${(p.integration as string) || '?'}`;
    case 'integration_error':
      return `Error: ${(p.integration as string) || '?'}`;
    default:
      return event.event_type;
  }
}

interface EventRowProps {
  event: EventRecord;
  isSelected: boolean;
  onClick: () => void;
}

export function EventRow({ event, isSelected, onClick }: EventRowProps) {
  const { colors } = useTheme();
  const color = TYPE_COLORS[event.event_type] || DEFAULT_COLOR;
  const relative = useMemo(() => relativeTime(event.timestamp), [event.timestamp]);
  const summary = useMemo(() => summarizePayload(event), [event]);

  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-2 px-3 py-1.5 text-left transition hover:bg-white/[0.03] ${
        isSelected ? 'bg-white/[0.05]' : ''
      }`}
      style={{ borderBottom: `1px solid ${colors.border}` }}
    >
      {/* Relative timestamp with full ISO on hover */}
      <span
        className="shrink-0 text-[10px] w-[52px] text-right"
        style={{ fontFamily: font.mono, color: colors.success, opacity: 0.7 }}
        title={event.timestamp}
      >
        {relative}
      </span>

      {/* Color-coded type badge */}
      <span
        className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] uppercase tracking-wide ${color.bg} ${color.text} min-w-[80px] text-center truncate`}
        style={{ fontFamily: font.mono }}
      >
        {event.event_type.replace(/_/g, ' ')}
      </span>

      {/* Payload summary */}
      <span className="flex-1 truncate text-[11px]" style={{ fontFamily: font.mono, color: colors.text }}>
        {summary}
      </span>
    </button>
  );
}

export { TYPE_COLORS, DEFAULT_COLOR };
