import { useState, useCallback } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX, FiCopy } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
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
  const { colors } = useTheme();
  if (data === undefined || data === null) return null;

  if (name === 'gmail__search' || name === 'gmail_search') {
    const emails = Array.isArray(data) ? data : (data as Record<string, unknown>)?.emails;
    if (Array.isArray(emails)) return <GmailSearchResult emails={emails} />;
  }

  if (name === 'gmail__read' || name === 'gmail_read') {
    const obj = data as Record<string, unknown>;
    if (obj && (obj.subject || obj.body || obj.from)) {
      return <GmailReadResult subject={obj.subject as string} from={obj.from as string} to={obj.to as string} date={obj.date as string} body={obj.body as string} />;
    }
  }

  if (name === 'bash' || name === 'shell' || name === 'developer__shell' || name === 'run_command') {
    const obj = data as Record<string, unknown>;
    if (typeof data === 'string') return <BashOutputResult stdout={data} />;
    if (obj && (obj.stdout !== undefined || obj.stderr !== undefined || obj.output !== undefined)) {
      return <BashOutputResult stdout={(obj.stdout || obj.output) as string} stderr={obj.stderr as string} exitCode={obj.exit_code as number ?? obj.exitCode as number} />;
    }
  }

  if (name === 'file_read' || name === 'read_file' || name === 'read_attachment' || name === 'attachment_read') {
    const obj = data as Record<string, unknown>;
    if (typeof data === 'string') return <FileReadResult content={data} />;
    if (obj && (obj.content !== undefined || obj.filename !== undefined)) {
      return <FileReadResult filename={obj.filename as string} content={obj.content as string} truncated={obj.truncated as boolean} />;
    }
  }

  if (typeof data === 'object') return <JsonResult data={data} />;

  return (
    <pre
      className="rounded p-2 text-[10px] overflow-x-auto max-h-[150px] overflow-y-auto whitespace-pre-wrap"
      style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.text }}
    >
      {String(data)}
    </pre>
  );
}

export function ToolResult({ call }: { call: ToolCall }) {
  const { colors } = useTheme();
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
        <span
          className="rounded px-1.5 py-0.5 text-[10px] shrink-0"
          style={{ backgroundColor: `${colors.cyan}1A`, color: colors.cyan }}
        >
          {call.name}
        </span>
        <span className="flex-1" />
        {hasResult && (
          success
            ? <FiCheck size={11} className="shrink-0" style={{ color: colors.success }} />
            : <FiX size={11} className="shrink-0" style={{ color: colors.danger }} />
        )}
      </button>

      {expanded && (
        <div className="px-3 py-2 space-y-2" style={{ borderTop: `1px solid ${colors.border}` }}>
          <div className="flex items-center justify-between">
            <div className="text-[9px] uppercase" style={{ fontFamily: font.mono, color: colors.textMuted }}>Result</div>
            <button
              onClick={handleCopyRaw}
              className="text-[9px] transition flex items-center gap-1"
              style={{ fontFamily: font.mono, color: colors.textMuted }}
            >
              {copied ? <><FiCheck size={9} style={{ color: colors.success }} /> Copied</> : <><FiCopy size={9} /> Raw JSON</>}
            </button>
          </div>

          {hasResult ? (
            <TypedResultBody name={call.name} data={parsedResult} />
          ) : (
            <div className="text-[10px] italic" style={{ fontFamily: font.mono, color: colors.textMuted }}>Pending...</div>
          )}
        </div>
      )}
    </div>
  );
}
