import { useState } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';

export function ToolCallCard({ call }: { call: ToolCall }) {
  const [expanded, setExpanded] = useState(false);
  const hasResult = call.result !== undefined;
  const success = call.success !== false;

  return (
    <div className="mt-1.5 rounded-lg border border-dark-border bg-[#0D1424]">
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
            <pre className="rounded bg-black/30 p-2 font-mono text-[10px] text-slate-300 overflow-x-auto max-h-[150px] overflow-y-auto">
              {JSON.stringify(call.arguments, null, 2)}
            </pre>
          </div>
          {hasResult && (
            <div>
              <div className="text-[9px] font-mono uppercase text-dark-muted mb-1">Result</div>
              <pre className={`rounded bg-black/30 p-2 font-mono text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto ${
                success ? 'text-slate-300' : 'text-red-300'
              }`}>
                {call.result}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
