import { useState } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function ToolCallCard({ call }: { call: ToolCall }) {
  const { colors } = useTheme();
  const [expanded, setExpanded] = useState(false);
  const hasResult = call.result !== undefined;
  const success = call.success !== false;

  return (
    <div
      className="mt-1.5 rounded-lg"
      style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}` }}
    >
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] transition"
        style={{ fontFamily: font.mono }}
      >
        {expanded
          ? <FiChevronDown size={12} style={{ color: colors.textMuted, flexShrink: 0 }} />
          : <FiChevronRight size={12} style={{ color: colors.textMuted, flexShrink: 0 }} />}
        <span className="truncate" style={{ color: colors.cyan }}>{call.name}</span>
        {hasResult && (
          success
            ? <FiCheck size={11} className="ml-auto shrink-0" style={{ color: colors.success }} />
            : <FiX size={11} className="ml-auto shrink-0" style={{ color: colors.danger }} />
        )}
      </button>

      {expanded && (
        <div className="px-3 py-2 space-y-2" style={{ borderTop: `1px solid ${colors.border}` }}>
          <div>
            <div className="text-[9px] uppercase mb-1" style={{ fontFamily: font.mono, color: colors.textMuted }}>Arguments</div>
            <pre
              className="rounded p-2 text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto"
              style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.text }}
            >
              {JSON.stringify(call.arguments, null, 2)}
            </pre>
          </div>
          {hasResult && (
            <div>
              <div className="text-[9px] uppercase mb-1" style={{ fontFamily: font.mono, color: colors.textMuted }}>Result</div>
              <pre
                className="rounded p-2 text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto"
                style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: success ? colors.text : colors.danger }}
              >
                {call.result}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
