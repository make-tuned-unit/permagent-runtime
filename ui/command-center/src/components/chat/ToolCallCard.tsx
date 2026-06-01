import { useState } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';

export function ToolCallCard({ call }: { call: ToolCall }) {
  const { colors } = useTheme();
  const [expanded, setExpanded] = useState(false);
  const hasResult = call.result !== undefined;
  const success = call.success !== false;

  return (
    <div className="mt-1.5 rounded-lg border border-dark-border" style={{ backgroundColor: colors.surface }}>
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] font-mono hover:bg-white/[0.03] transition"
      >
        {expanded
          ? <FiChevronDown size={12} className="text-dark-muted shrink-0" />
          : <FiChevronRight size={12} className="text-dark-muted shrink-0" />}
        <span className="text-accent truncate">{call.name}</span>
        {hasResult && (
          success
            ? <FiCheck size={11} className="ml-auto text-emerald-400 shrink-0" />
            : <FiX size={11} className="ml-auto text-red-400 shrink-0" />
        )}
      </button>

      {expanded && (
        <div className="border-t border-dark-border/50 px-3 py-2 space-y-2">
          <div>
            <div className="text-[9px] font-mono uppercase text-dark-muted mb-1">Arguments</div>
            <pre className="rounded p-2 font-mono text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto" style={{ backgroundColor: colors.bg, color: colors.text }}>
              {JSON.stringify(call.arguments, null, 2)}
            </pre>
          </div>
          {hasResult && (
            <div>
              <div className="text-[9px] font-mono uppercase text-dark-muted mb-1">Result</div>
              <pre className="rounded p-2 font-mono text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto" style={{ backgroundColor: colors.bg, color: success ? colors.text : colors.danger }}>
                {call.result}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
