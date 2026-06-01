import { FiX } from 'react-icons/fi';
import type { PermagentEventType } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';

// All known event types for the multi-select
const ALL_EVENT_TYPES: PermagentEventType[] = [
  'daemon_started', 'daemon_stopped',
  'task_created', 'task_started', 'task_completed', 'task_failed',
  'memory_added',
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
    <div className="border-b border-dark-border px-3 py-2 space-y-2" style={{ backgroundColor: colors.bgDeeper }}>
      {/* Date range presets */}
      <div className="flex items-center gap-1.5">
        <span className="text-[9px] font-mono uppercase text-dark-muted mr-1">Range</span>
        {DATE_PRESETS.map(p => (
          <button
            key={p.value}
            onClick={() => onDatePresetChange(p.value)}
            className={`rounded px-2 py-0.5 text-[10px] font-mono transition ${
              datePreset === p.value
                ? 'bg-accent/20 text-accent'
                : 'text-dark-muted hover:text-dark-text hover:bg-white/5'
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>

      {/* Event type chips (multi-select) */}
      <div className="flex flex-wrap items-center gap-1">
        <span className="text-[9px] font-mono uppercase text-dark-muted mr-1">Type</span>
        {ALL_EVENT_TYPES.map(type => {
          const active = selectedTypes.includes(type);
          return (
            <button
              key={type}
              onClick={() => onToggleType(type)}
              className={`rounded-full px-2 py-0.5 text-[9px] font-mono transition ${
                active
                  ? 'bg-accent/20 text-accent'
                  : 'bg-white/5 text-dark-muted hover:text-dark-text hover:bg-white/10'
              }`}
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
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-mono text-dark-muted hover:bg-white/5 hover:text-dark-text transition"
          >
            <FiX size={10} /> Clear filters
          </button>
        </div>
      )}
    </div>
  );
}

export type { DatePreset };
