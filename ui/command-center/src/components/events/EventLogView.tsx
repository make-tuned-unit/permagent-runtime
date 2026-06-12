import { useState, useRef, useEffect, useCallback } from 'react';
import { FiFilter, FiArrowDown } from 'react-icons/fi';
import { useCommandCenter, type EventRecord, type PermagentEventType } from '../../lib/store';
import { EventRow } from './EventRow';
import { EventFilter, getDateCutoff, type DatePreset } from './EventFilter';
import { EventDetail } from './EventDetail';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function EventLogView() {
  const events = useCommandCenter(s => s.events);

  const [showFilters, setShowFilters] = useState(false);
  const [selectedTypes, setSelectedTypes] = useState<PermagentEventType[]>([]);
  const [datePreset, setDatePreset] = useState<DatePreset>('all');
  const [inspectedEvent, setInspectedEvent] = useState<EventRecord | null>(null);

  // Auto-scroll state
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [showJumpBtn, setShowJumpBtn] = useState(false);
  const prevCountRef = useRef(events.length);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    setAutoScroll(atBottom);
    setShowJumpBtn(!atBottom);
  }, []);

  useEffect(() => {
    if (autoScroll && events.length > prevCountRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
    prevCountRef.current = events.length;
  }, [events.length, autoScroll]);

  const jumpToLatest = useCallback(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
      setAutoScroll(true);
      setShowJumpBtn(false);
    }
  }, []);

  // Filter events
  const dateCutoff = getDateCutoff(datePreset);
  const displayed = events.filter(e => {
    if (selectedTypes.length > 0 && !selectedTypes.includes(e.event_type as PermagentEventType)) {
      return false;
    }
    if (dateCutoff && e.timestamp < dateCutoff) return false;
    return true;
  });

  // Newest at top (events already come newest-first from store)
  const hasActiveFilters = selectedTypes.length > 0 || datePreset !== 'all';

  const toggleType = (type: PermagentEventType) => {
    setSelectedTypes(prev =>
      prev.includes(type) ? prev.filter(t => t !== type) : [...prev, type],
    );
  };

  const clearAllFilters = () => {
    setSelectedTypes([]);
    setDatePreset('all');
  };

  const { colors } = useTheme();

  return (
    <div className="flex h-full flex-col" style={{ backgroundColor: colors.bg, color: colors.text }}>
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <div className="flex items-center gap-2">
          <span
            className="text-[11px] uppercase tracking-wider"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            Event Log
          </span>
          <span
            className="rounded px-1.5 py-0.5 text-[10px]"
            style={{ fontFamily: font.mono, backgroundColor: colors.surface, color: colors.textMuted }}
          >
            {displayed.length}
          </span>
        </div>
        <button
          onClick={() => setShowFilters(f => !f)}
          className="rounded p-1 transition"
          style={{
            backgroundColor: showFilters || hasActiveFilters ? colors.cyanSoft : undefined,
            color: showFilters || hasActiveFilters ? colors.cyan : colors.textMuted,
          }}
          onMouseEnter={e => { if (!(showFilters || hasActiveFilters)) e.currentTarget.style.color = colors.text; }}
          onMouseLeave={e => { if (!(showFilters || hasActiveFilters)) e.currentTarget.style.color = colors.textMuted; }}
        >
          <FiFilter size={14} />
        </button>
      </div>

      {/* Filter controls */}
      {showFilters && (
        <EventFilter
          selectedTypes={selectedTypes}
          onToggleType={toggleType}
          datePreset={datePreset}
          onDatePresetChange={setDatePreset}
          onClearAll={clearAllFilters}
        />
      )}

      {/* Event stream */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="relative flex-1 overflow-y-auto text-[11px] leading-relaxed"
        style={{ fontFamily: font.mono }}
      >
        {displayed.map(event => (
          <EventRow
            key={event.id}
            event={event}
            isSelected={inspectedEvent?.id === event.id}
            onClick={() =>
              setInspectedEvent(inspectedEvent?.id === event.id ? null : event)
            }
          />
        ))}

        {displayed.length === 0 && (
          <div
            className="flex items-center justify-center p-8 text-xs"
            style={{ fontFamily: font.mono, color: colors.textMuted }}
          >
            {events.length === 0 ? 'Waiting for events...' : 'No events match filters'}
          </div>
        )}

        {showJumpBtn && displayed.length > 0 && (
          <button
            onClick={jumpToLatest}
            className="sticky bottom-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-1.5 rounded-full px-3 py-1.5 text-[10px] backdrop-blur transition"
            style={{ fontFamily: font.mono, backgroundColor: colors.cyanSoft, color: colors.cyan }}
            onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.cyan}4D`; }}
            onMouseLeave={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
          >
            <FiArrowDown size={12} />
            Jump to latest
          </button>
        )}
      </div>

      {/* Expandable event detail panel */}
      {inspectedEvent && (
        <EventDetail
          event={inspectedEvent}
          onClose={() => setInspectedEvent(null)}
        />
      )}
    </div>
  );
}
