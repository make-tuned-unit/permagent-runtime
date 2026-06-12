import { useState, useMemo } from 'react';
import { FiX, FiCopy, FiCheck } from 'react-icons/fi';
import type { EventRecord } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { TYPE_COLORS, DEFAULT_COLOR } from './EventRow';

interface EventDetailProps {
  event: EventRecord;
  onClose: () => void;
}

// Simple JSON syntax highlighter for dark theme
function highlightJson(json: string): JSX.Element[] {
  const lines = json.split('\n');
  return lines.map((line, i) => {
    const highlighted = line
      // String keys
      .replace(/"([^"]+)":/g, '<span class="text-blue-400">"$1"</span>:')
      // String values
      .replace(/: "([^"]*)"/g, ': <span class="text-emerald-400">"$1"</span>')
      // Numbers
      .replace(/: (\d+\.?\d*)/g, ': <span class="text-amber-400">$1</span>')
      // Booleans and null
      .replace(/: (true|false|null)/g, ': <span class="text-purple-400">$1</span>');

    return (
      <span key={i}>
        <span dangerouslySetInnerHTML={{ __html: highlighted }} />
        {i < lines.length - 1 ? '\n' : ''}
      </span>
    );
  });
}

export function EventDetail({ event, onClose }: EventDetailProps) {
  const { colors } = useTheme();
  const [copied, setCopied] = useState(false);
  const color = TYPE_COLORS[event.event_type] || DEFAULT_COLOR;

  const jsonString = useMemo(
    () => JSON.stringify(event.payload, null, 2),
    [event.payload],
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(jsonString);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback: noop
    }
  };

  return (
    <div
      className="max-h-[40%] overflow-y-auto"
      style={{ backgroundColor: colors.surface, borderTop: `1px solid ${colors.border}` }}
    >
      {/* Header */}
      <div
        className="flex items-center justify-between px-3 py-2"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <div className="flex items-center gap-2">
          <span
            className="text-[10px] uppercase"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            Event Detail
          </span>
          <span
            className={`rounded px-1.5 py-0.5 text-[9px] uppercase ${color.bg} ${color.text}`}
            style={{ fontFamily: font.mono }}
          >
            {event.event_type.replace(/_/g, ' ')}
          </span>
        </div>
        <button
          onClick={onClose}
          className="transition"
          style={{ color: colors.textMuted }}
          onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
          onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
        >
          <FiX size={12} />
        </button>
      </div>

      {/* Metadata fields */}
      <div className="px-3 py-2 space-y-1.5 text-[10px]" style={{ fontFamily: font.mono }}>
        <div className="flex gap-3">
          <span className="w-[70px]" style={{ color: colors.textMuted }}>ID</span>
          <span className="break-all" style={{ color: colors.text }}>{event.id}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px]" style={{ color: colors.textMuted }}>Type</span>
          <span className={color.text}>{event.event_type}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px]" style={{ color: colors.textMuted }}>Time</span>
          <span style={{ color: colors.text }}>{event.timestamp}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px]" style={{ color: colors.textMuted }}>Source</span>
          <span style={{ color: colors.text }}>{event.source}</span>
        </div>
        {event.task_id && (
          <div className="flex gap-3">
            <span className="w-[70px]" style={{ color: colors.textMuted }}>Task</span>
            <span style={{ color: colors.text }}>{event.task_id}</span>
          </div>
        )}
        {event.run_id && (
          <div className="flex gap-3">
            <span className="w-[70px]" style={{ color: colors.textMuted }}>Run</span>
            <span style={{ color: colors.text }}>{event.run_id}</span>
          </div>
        )}
      </div>

      {/* JSON payload with syntax highlighting and copy button */}
      <div className="px-3 py-2">
        <div className="flex items-center justify-between mb-1">
          <span
            className="text-[9px] uppercase"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            Payload
          </span>
          <button
            onClick={handleCopy}
            className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[9px] hover:bg-white/5 transition"
            style={{ fontFamily: font.mono, color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
          >
            {copied ? <FiCheck size={10} className="text-emerald-400" /> : <FiCopy size={10} />}
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
        <pre
          className="rounded p-2 text-[9px] overflow-x-auto max-h-[150px] overflow-y-auto"
          style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.text }}
        >
          {highlightJson(jsonString)}
        </pre>
      </div>
    </div>
  );
}
