import { useState, useMemo } from 'react';
import { FiX, FiCopy, FiCheck } from 'react-icons/fi';
import type { EventRecord } from '../../lib/store';
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
    <div className="border-t border-dark-border max-h-[40%] overflow-y-auto" style={{ backgroundColor: colors.surface }}>
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-dark-border/50">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-mono uppercase text-dark-muted">Event Detail</span>
          <span className={`rounded px-1.5 py-0.5 text-[9px] font-mono uppercase ${color.bg} ${color.text}`}>
            {event.event_type.replace(/_/g, ' ')}
          </span>
        </div>
        <button onClick={onClose} className="text-dark-muted hover:text-dark-text transition">
          <FiX size={12} />
        </button>
      </div>

      {/* Metadata fields */}
      <div className="px-3 py-2 space-y-1.5 font-mono text-[10px]">
        <div className="flex gap-3">
          <span className="w-[70px] text-dark-muted">ID</span>
          <span className="text-dark-text break-all">{event.id}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px] text-dark-muted">Type</span>
          <span className={color.text}>{event.event_type}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px] text-dark-muted">Time</span>
          <span className="text-dark-text">{event.timestamp}</span>
        </div>
        <div className="flex gap-3">
          <span className="w-[70px] text-dark-muted">Source</span>
          <span className="text-dark-text">{event.source}</span>
        </div>
        {event.task_id && (
          <div className="flex gap-3">
            <span className="w-[70px] text-dark-muted">Task</span>
            <span className="text-dark-text">{event.task_id}</span>
          </div>
        )}
        {event.run_id && (
          <div className="flex gap-3">
            <span className="w-[70px] text-dark-muted">Run</span>
            <span className="text-dark-text">{event.run_id}</span>
          </div>
        )}
      </div>

      {/* JSON payload with syntax highlighting and copy button */}
      <div className="px-3 py-2">
        <div className="flex items-center justify-between mb-1">
          <span className="text-[9px] font-mono uppercase text-dark-muted">Payload</span>
          <button
            onClick={handleCopy}
            className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[9px] font-mono text-dark-muted hover:text-dark-text hover:bg-white/5 transition"
          >
            {copied ? <FiCheck size={10} className="text-emerald-400" /> : <FiCopy size={10} />}
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
        <pre className="rounded bg-black/30 p-2 font-mono text-[9px] text-dark-text overflow-x-auto max-h-[150px] overflow-y-auto">
          {highlightJson(jsonString)}
        </pre>
      </div>
    </div>
  );
}
