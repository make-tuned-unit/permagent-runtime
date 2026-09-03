import { useState, useCallback, useId, type CSSProperties } from 'react';
import { FiChevronRight, FiChevronDown, FiCheck, FiX, FiCopy } from 'react-icons/fi';
import type { ToolCall } from '../../lib/store';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
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
  const bodyId = useId();
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
      {/* A disclosure toggle, not an action: it opens the result body below and
          there is nothing to await, so the pending floor and the success tick
          are both the wrong signal. It takes the shared `.pa-btn` interaction
          rules directly and keeps the aria pairing that describes it. The
          hover fill follows the card's own rounding so it cannot poke out of
          the corners. */}
      <button
        type="button"
        className="pa-btn"
        aria-expanded={expanded}
        aria-controls={bodyId}
        onClick={() => setExpanded(!expanded)}
        style={{
          '--pa-btn-bg': 'transparent',
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-pad': '6px 12px',
          '--pa-btn-radius': expanded
            ? `${radius.md}px ${radius.md}px 0 0`
            : `${radius.md}px`,
          display: 'flex', width: '100%', justifyContent: 'flex-start',
          textAlign: 'left', gap: space.md, borderWidth: 0,
          fontFamily: font.mono, fontSize: textSize.micro,
        } as CSSProperties}
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
        <div id={bodyId} className="px-3 py-2 space-y-2" style={{ borderTop: `1px solid ${colors.border}` }}>
          <div className="flex items-center justify-between">
            <div className="text-[9px] uppercase" style={{ fontFamily: font.mono, color: colors.textMuted }}>Result</div>
            <Button
              colors={colors}
              variant="bare"
              onClick={handleCopyRaw}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                fontFamily: font.mono,
                fontSize: 9,
                gap: space.xs,
              } as CSSProperties}
            >
              {copied ? <><FiCheck size={9} style={{ color: colors.success }} /> Copied</> : <><FiCopy size={9} /> Raw JSON</>}
            </Button>
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
