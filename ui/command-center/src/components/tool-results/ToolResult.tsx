import { useState, useCallback } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX, FiCopy } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';
import { GmailSearchResult } from './GmailSearchResult';
import { GmailReadResult } from './GmailReadResult';
import { FileReadResult } from './FileReadResult';
import { BashOutputResult } from './BashOutputResult';
import { JsonResult } from './JsonResult';

function parseResult(result: string | undefined): unknown {
  if (!result) return undefined;
  try {
    return JSON.parse(result);
  } catch {
    return result;
  }
}

function TypedResultBody({ name, data }: { name: string; data: unknown }) {
  if (data === undefined || data === null) return null;

  // Gmail search
  if (name === 'gmail__search' || name === 'gmail_search') {
    const emails = Array.isArray(data) ? data : (data as Record<string, unknown>)?.emails;
    if (Array.isArray(emails)) {
      return <GmailSearchResult emails={emails} />;
    }
  }

  // Gmail read
  if (name === 'gmail__read' || name === 'gmail_read') {
    const obj = data as Record<string, unknown>;
    if (obj && (obj.subject || obj.body || obj.from)) {
      return (
        <GmailReadResult
          subject={obj.subject as string}
          from={obj.from as string}
          to={obj.to as string}
          date={obj.date as string}
          body={obj.body as string}
        />
      );
    }
  }

  // Bash / shell output
  if (name === 'bash' || name === 'shell' || name === 'developer__shell' || name === 'run_command') {
    const obj = data as Record<string, unknown>;
    if (typeof data === 'string') {
      return <BashOutputResult stdout={data} />;
    }
    if (obj && (obj.stdout !== undefined || obj.stderr !== undefined || obj.output !== undefined)) {
      return (
        <BashOutputResult
          stdout={(obj.stdout || obj.output) as string}
          stderr={obj.stderr as string}
          exitCode={obj.exit_code as number ?? obj.exitCode as number}
        />
      );
    }
  }

  // File read
  if (name === 'file_read' || name === 'read_file' || name === 'read_attachment' || name === 'attachment_read') {
    const obj = data as Record<string, unknown>;
    if (typeof data === 'string') {
      return <FileReadResult content={data} />;
    }
    if (obj && (obj.content !== undefined || obj.filename !== undefined)) {
      return (
        <FileReadResult
          filename={obj.filename as string}
          content={obj.content as string}
          truncated={obj.truncated as boolean}
        />
      );
    }
  }

  // Fallback: JSON viewer
  if (typeof data === 'object') {
    return <JsonResult data={data} />;
  }

  // Plain string fallback
  return (
    <pre className="rounded bg-black/30 p-2 font-mono text-[10px] text-slate-300 overflow-x-auto max-h-[150px] overflow-y-auto whitespace-pre-wrap">
      {String(data)}
    </pre>
  );
}

export function ToolResult({ call }: { call: ToolCall }) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const hasResult = call.result !== undefined;
  const success = call.success !== false;
  const parsedResult = parseResult(call.result);

  const handleCopyRaw = useCallback(() => {
    const raw = call.result || JSON.stringify(call.arguments, null, 2);
    navigator.clipboard.writeText(raw).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [call]);

  return (
    <div className="mt-1.5 rounded-lg border border-dark-border bg-[#0D1424]">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] font-mono hover:bg-white/[0.03] transition"
      >
        {expanded
          ? <FiChevronDown size={12} className="text-dark-muted shrink-0" />
          : <FiChevronRight size={12} className="text-dark-muted shrink-0" />}
        <span className="rounded bg-accent/10 text-accent px-1.5 py-0.5 text-[10px] shrink-0">
          {call.name}
        </span>
        <span className="flex-1" />
        {hasResult && (
          success
            ? <FiCheck size={11} className="text-emerald-400 shrink-0" />
            : <FiX size={11} className="text-red-400 shrink-0" />
        )}
      </button>

      {expanded && (
        <div className="border-t border-dark-border/50 px-3 py-2 space-y-2">
          <div className="flex items-center justify-between">
            <div className="text-[9px] font-mono uppercase text-dark-muted">Result</div>
            <button
              onClick={handleCopyRaw}
              className="text-[9px] font-mono text-dark-muted hover:text-dark-text transition flex items-center gap-1"
            >
              {copied ? <><FiCheck size={9} className="text-emerald-400" /> Copied</> : <><FiCopy size={9} /> Raw JSON</>}
            </button>
          </div>

          {hasResult ? (
            <TypedResultBody name={call.name} data={parsedResult} />
          ) : (
            <div className="text-[10px] font-mono text-dark-muted italic">Pending...</div>
          )}
        </div>
      )}
    </div>
  );
}
