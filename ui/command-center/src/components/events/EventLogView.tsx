import { useState, useRef, useEffect, useCallback } from 'react';
import {
  FiFilter, FiX, FiAlertTriangle, FiAlertCircle, FiInfo, FiZap,
  FiArrowDown,
} from 'react-icons/fi';
import { useCommandCenter, type EventRecord } from '../../lib/store';

const SEVERITY_CONFIG: Record<string, { color: string; icon: typeof FiInfo; bg: string }> = {
  info:     { color: 'text-slate-400',          icon: FiInfo,           bg: 'bg-slate-400/10' },
  warning:  { color: 'text-amber-400',          icon: FiAlertTriangle,  bg: 'bg-amber-400/10' },
  error:    { color: 'text-red-400',            icon: FiAlertCircle,    bg: 'bg-red-400/10' },
  critical: { color: 'text-red-500 font-bold',  icon: FiZap,            bg: 'bg-red-500/10' },
};

function formatTimeMs(ts: string): string {
  try {
    const d = new Date(ts);
    const h = String(d.getHours()).padStart(2, '0');
    const m = String(d.getMinutes()).padStart(2, '0');
    const s = String(d.getSeconds()).padStart(2, '0');
    const ms = String(d.getMilliseconds()).padStart(3, '0');
    return `${h}:${m}:${s}.${ms}`;
  } catch {
    return ts;
  }
}

function summarizeEvent(e: EventRecord): string {
  const p = e.payload;
  switch (e.event_type) {
    case 'task_created':
      return `Task ${(p.title as string) || (p.task_id as string)?.slice(0, 8) || '?'} created`;
    case 'task_started':
      return 'Task started';
    case 'task_completed':
      return 'Task completed';
    case 'task_failed':
      return `Task failed: ${p.error || '?'}`;
    case 'memory_added':
      return `Memory: ${(p.key as string) || '?'}`;
    case 'skill_proposed':
      return `Skill proposed: ${(p.name as string) || '?'}`;
    case 'skill_saved':
      return `Skill saved: ${(p.name as string) || '?'}`;
    case 'skill_triggered':
      return `Skill triggered: ${(p.name as string) || '?'}`;
    case 'message_received':
      return `Message from ${(p.role as string) || '?'}`;
    case 'daemon_started':
      return 'Daemon started';
    case 'daemon_stopped':
      return 'Daemon stopped';
    case 'integration_connected':
      return `Connected: ${(p.integration as string) || '?'}`;
    case 'integration_error':
      return `Integration error: ${(p.integration as string) || '?'}`;
    default:
      return e.event_type;
  }
}

function unique(arr: (string | null | undefined)[]): string[] {
  return [...new Set(arr.filter((v): v is string => !!v))].sort();
}

export function EventLogView() {
  const events = useCommandCenter(s => s.events);
  const eventTypeFilter = useCommandCenter(s => s.eventTypeFilter);
  const setEventTypeFilter = useCommandCenter(s => s.setEventTypeFilter);

  const [showFilters, setShowFilters] = useState(false);
  const [severityFilter, setSeverityFilter] = useState('');
  const [inspectedEvent, setInspectedEvent] = useState<EventRecord | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const [showJumpBtn, setShowJumpBtn] = useState(false);
  const prevEventsLenRef = useRef(events.length);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    setIsAtBottom(atBottom);
    setShowJumpBtn(!atBottom);
  }, []);

  useEffect(() => {
    if (isAtBottom && events.length > prevEventsLenRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
    prevEventsLenRef.current = events.length;
  }, [events.length, isAtBottom]);

  const jumpToLatest = useCallback(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
      setIsAtBottom(true);
      setShowJumpBtn(false);
    }
  }, []);

  const displayed = events.filter(e => {
    if (severityFilter && e.severity !== severityFilter) return false;
    if (eventTypeFilter && e.event_type !== eventTypeFilter) return false;
    return true;
  });

  const chronological = [...displayed].reverse();
  const uniqueTypes = unique(events.map(e => e.event_type));
  const hasActiveFilters = severityFilter || eventTypeFilter;

  const clearAllFilters = () => {
    setSeverityFilter('');
    setEventTypeFilter('');
  };

  return (
    <div className="flex h-full flex-col bg-[#0A0E17] text-gray-300">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">
            Event Log
          </span>
          <span className="rounded bg-dark-surface px-1.5 py-0.5 text-[10px] font-mono text-dark-muted">
            {displayed.length}
          </span>
        </div>
        <button
          onClick={() => setShowFilters(f => !f)}
          className={`rounded p-1 transition ${
            showFilters || hasActiveFilters
              ? 'bg-accent/10 text-accent'
              : 'text-dark-muted hover:text-dark-text'
          }`}
        >
          <FiFilter size={14} />
        </button>
      </div>

      {/* Filter bar */}
      {showFilters && (
        <div className="flex flex-wrap items-center gap-1.5 border-b border-dark-border bg-[#080d18] px-3 py-2">
          <select
            value={eventTypeFilter}
            onChange={e => setEventTypeFilter(e.target.value)}
            className="appearance-none rounded border border-dark-border bg-[#080d18] pl-2 pr-6 py-1 text-[10px] text-dark-text outline-none"
          >
            <option value="">Event type</option>
            {uniqueTypes.map(o => (
              <option key={o} value={o}>{o}</option>
            ))}
          </select>
          <select
            value={severityFilter}
            onChange={e => setSeverityFilter(e.target.value)}
            className="appearance-none rounded border border-dark-border bg-[#080d18] pl-2 pr-6 py-1 text-[10px] text-dark-text outline-none"
          >
            <option value="">Severity</option>
            {['info', 'warning', 'error', 'critical'].map(o => (
              <option key={o} value={o}>{o}</option>
            ))}
          </select>
          {hasActiveFilters && (
            <button
              onClick={clearAllFilters}
              className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-dark-muted hover:bg-white/5 hover:text-dark-text transition"
            >
              <FiX size={10} /> Clear
            </button>
          )}
        </div>
      )}

      {/* Event stream */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="relative flex-1 overflow-y-auto font-mono text-[11px] leading-relaxed"
      >
        {chronological.map(event => {
          const sev = SEVERITY_CONFIG[event.severity] || SEVERITY_CONFIG.info;
          const SevIcon = sev.icon;

          return (
            <button
              key={event.id}
              onClick={() => setInspectedEvent(inspectedEvent?.id === event.id ? null : event)}
              className={`flex w-full items-start gap-2 border-b border-dark-border/20 px-3 py-1.5 text-left transition hover:bg-white/[0.03] ${
                inspectedEvent?.id === event.id ? 'bg-white/[0.05]' : ''
              }`}
            >
              <span className="shrink-0 pt-0.5 text-[#00ffb4]/70 w-[78px]">
                {formatTimeMs(event.timestamp)}
              </span>
              <span className={`shrink-0 pt-0.5 ${sev.color}`}>
                <SevIcon size={12} />
              </span>
              <span className={`shrink-0 rounded px-1 py-0.5 text-[9px] uppercase ${sev.bg} ${sev.color} w-[40px] text-center`}>
                {event.event_type.split('_')[0]?.slice(0, 5)}
              </span>
              <span className="shrink-0 text-dark-muted w-[60px] truncate">{event.source}</span>
              <span className="flex-1 truncate text-dark-text">{summarizeEvent(event)}</span>
            </button>
          );
        })}

        {chronological.length === 0 && (
          <div className="flex items-center justify-center p-8 text-xs text-dark-muted font-mono">
            {events.length === 0 ? 'Waiting for events...' : 'No events match filters'}
          </div>
        )}

        {showJumpBtn && chronological.length > 0 && (
          <button
            onClick={jumpToLatest}
            className="sticky bottom-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-1.5 rounded-full bg-accent/20 px-3 py-1.5 text-[10px] font-mono text-accent backdrop-blur transition hover:bg-accent/30"
          >
            <FiArrowDown size={12} />
            Jump to latest
          </button>
        )}
      </div>

      {/* Event detail */}
      {inspectedEvent && (
        <div className="border-t border-dark-border bg-[#0D1424] max-h-[40%] overflow-y-auto">
          <div className="flex items-center justify-between px-3 py-2 border-b border-dark-border/50">
            <span className="text-[10px] font-mono uppercase text-dark-muted">Event Detail</span>
            <button onClick={() => setInspectedEvent(null)} className="text-dark-muted hover:text-dark-text">
              <FiX size={12} />
            </button>
          </div>
          <div className="px-3 py-2 space-y-1.5 font-mono text-[10px]">
            <div className="flex gap-3"><span className="w-[70px] text-dark-muted">ID</span><span className="text-dark-text break-all">{inspectedEvent.id}</span></div>
            <div className="flex gap-3"><span className="w-[70px] text-dark-muted">Type</span><span className="text-accent">{inspectedEvent.event_type}</span></div>
            <div className="flex gap-3"><span className="w-[70px] text-dark-muted">Time</span><span className="text-dark-text">{inspectedEvent.timestamp}</span></div>
            <div className="flex gap-3"><span className="w-[70px] text-dark-muted">Source</span><span className="text-dark-text">{inspectedEvent.source}</span></div>
            {inspectedEvent.task_id && <div className="flex gap-3"><span className="w-[70px] text-dark-muted">Task</span><span className="text-dark-text">{inspectedEvent.task_id}</span></div>}
            {inspectedEvent.run_id && <div className="flex gap-3"><span className="w-[70px] text-dark-muted">Run</span><span className="text-dark-text">{inspectedEvent.run_id}</span></div>}
          </div>
          <div className="px-3 py-2">
            <div className="text-[9px] font-mono uppercase text-dark-muted mb-1">Payload</div>
            <pre className="rounded bg-black/30 p-2 font-mono text-[9px] text-dark-text overflow-x-auto max-h-[150px] overflow-y-auto">
              {JSON.stringify(inspectedEvent.payload, null, 2)}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
