import { FiX } from 'react-icons/fi';
import type { PermagentEventType } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// All known event types for the multi-select
const ALL_EVENT_TYPES: PermagentEventType[] = [
  'daemon_started', 'daemon_stopped',
  'task_created', 'task_started', 'task_completed', 'task_failed',
  'memory_added', 'memory_recalled',
  'entity_added', 'entity_updated',
  'decision_created', 'decision_resolved',
  'agent_state_changed',
  'skill_proposed', 'skill_saved', 'skill_triggered',
  'message_received', 'stream_chunk',
  'integration_connected', 'integration_error',
];

type DatePreset = 'all' | 'last_hour' | 'today' | 'last_24h';

const DATE_PRESETS: { value: DatePreset; label: string }[] = [
  { value: 'all', label: 'All time' },
  { value: 'last_hour', label: 'Last hour' },
  { value: 'today', label: 'Today' },
  { value: 'last_24h', label: 'Last 24h' },
];

export function getDateCutoff(preset: DatePreset): string | undefined {
  const now = new Date();
  switch (preset) {
    case 'last_hour':
      return new Date(now.getTime() - 60 * 60 * 1000).toISOString();
    case 'today': {
      const start = new Date(now);
      start.setHours(0, 0, 0, 0);
      return start.toISOString();
    }
    case 'last_24h':
      return new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
    default:
      return undefined;
  }
}

interface EventFilterProps {
  selectedTypes: PermagentEventType[];
  onToggleType: (type: PermagentEventType) => void;
  datePreset: DatePreset;
  onDatePresetChange: (preset: DatePreset) => void;
  onClearAll: () => void;
}

export function EventFilter({
  selectedTypes,
  onToggleType,
  datePreset,
  onDatePresetChange,
  onClearAll,
}: EventFilterProps) {
  const { colors } = useTheme();
  const hasFilters = selectedTypes.length > 0 || datePreset !== 'all';

  return (
    <div
      className="px-3 py-2 space-y-2"
      style={{ backgroundColor: colors.bgDeeper, borderBottom: `1px solid ${colors.border}` }}
    >
      {/* Date range presets */}
      <div className="flex items-center gap-1.5">
        <span
          className="text-[9px] uppercase mr-1"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Range
        </span>
        {DATE_PRESETS.map(p => {
          const active = datePreset === p.value;
          return (
            <button
              key={p.value}
              onClick={() => onDatePresetChange(p.value)}
              className={`rounded px-2 py-0.5 text-[10px] transition ${active ? '' : 'hover:bg-white/5'}`}
              style={{
                fontFamily: font.mono,
                backgroundColor: active ? colors.cyanSoft : undefined,
                color: active ? colors.cyan : colors.textMuted,
              }}
              onMouseEnter={e => { if (!active) e.currentTarget.style.color = colors.text; }}
              onMouseLeave={e => { if (!active) e.currentTarget.style.color = colors.textMuted; }}
            >
              {p.label}
            </button>
          );
        })}
      </div>

      {/* Event type chips (multi-select) */}
      <div className="flex flex-wrap items-center gap-1">
        <span
          className="text-[9px] uppercase mr-1"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Type
        </span>
        {ALL_EVENT_TYPES.map(type => {
          const active = selectedTypes.includes(type);
          return (
            <button
              key={type}
              onClick={() => onToggleType(type)}
              className={`rounded-full px-2 py-0.5 text-[9px] transition ${active ? '' : 'bg-white/5 hover:bg-white/10'}`}
              style={{
                fontFamily: font.mono,
                backgroundColor: active ? colors.cyanSoft : undefined,
                color: active ? colors.cyan : colors.textMuted,
              }}
              onMouseEnter={e => { if (!active) e.currentTarget.style.color = colors.text; }}
              onMouseLeave={e => { if (!active) e.currentTarget.style.color = colors.textMuted; }}
            >
              {type.replace(/_/g, ' ')}
            </button>
          );
        })}
      </div>

      {/* Clear all */}
      {hasFilters && (
        <div className="flex justify-end">
          <button
            onClick={onClearAll}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] hover:bg-white/5 transition"
            style={{ fontFamily: font.mono, color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
          >
            <FiX size={10} /> Clear filters
          </button>
        </div>
      )}
    </div>
  );
}

export type { DatePreset };
