import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiChevronDown } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

export function SessionPicker() {
  const { gradient, colors } = useTheme();
  const agentName = useCommandCenter(s => s.agentName);
  const sessions = useCommandCenter(s => s.sessions);
  const loadSessions = useCommandCenter(s => s.loadSessions);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const [open, setOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) loadSessions();
  }, [open, loadSessions]);

  useEffect(() => {
    if (!open) return;
    const handler = (event: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const handleSelectSession = async (sessionId: string) => {
    setOpen(false);
    await switchToSession(sessionId);
  };

  // Resolves false when the create fails, so nothing can confirm a session that
  // was never made. (Surfacing that failure is a separate concern — see report.)
  const handleNewSession = async () => {
    setOpen(false);
    try {
      const session = await api.createSession();
      await switchToSession(session.id);
      return true;
    } catch { /* ignore */ }
    return false;
  };

  return (
    <div ref={pickerRef} data-testid="session-picker" style={{ position: 'relative', minWidth: 0 }}>
      {/* Disclosure toggle (aria-expanded/-controls pairing is what describes
          it), so it keeps the element and takes only the shared `.pa-btn`
          interaction rules — there is nothing to await here, and the pending
          floor and success tick of the Button primitive would both be wrong. */}
      <button
        type="button"
        className="pa-btn"
        aria-label="Choose chat session"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        style={{
          '--pa-btn-fg': colors.text,
          '--pa-btn-fg-hover': colors.cyan,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
          '--pa-btn-weight': 600,
          display: 'flex', justifyContent: 'flex-start', gap: 4, maxWidth: '100%',
          // `.pa-btn` normalises to 11px/14px; this control is the agent name at
          // 13px on the app's own leading, and its height is pure line-height.
          fontSize: textSize.small, lineHeight: 1.5, fontFamily: font.body,
        } as CSSProperties}
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
          {agentName}
        </span>
        <FiChevronDown size={10} style={{ flexShrink: 0 }} />
      </button>

      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, marginTop: 4,
          width: 260, maxHeight: 300, overflow: 'auto',
          background: gradient.dropdown, backdropFilter: 'blur(16px)',
          border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
          boxShadow: '0 12px 40px rgba(0,0,0,0.6)', zIndex: 100,
          padding: '4px 0',
        }}>
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={handleNewSession}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-pad': '8px 12px',
              '--pa-btn-radius': '0',
              width: '100%', justifyContent: 'flex-start', gap: 6,
              fontSize: textSize.caption, fontFamily: font.body,
              // Only the bottom edge is drawn — `.pa-btn`'s `border` shorthand
              // paints all four, so this one longhand has to stay inline.
              borderBottom: `1px solid ${colors.border}`,
            } as CSSProperties}
          >
            + New session
          </Button>
          {sessions.map(session => {
            const isCurrent = session.id === chatSessionId;
            return (
              <Button
                colors={colors}
                variant="bare"
                type="button"
                key={session.id}
                onClick={() => handleSelectSession(session.id)}
                style={{
                  '--pa-btn-bg': isCurrent ? colors.cyanSoft : 'transparent',
                  '--pa-btn-bg-hover': isCurrent ? colors.cyanSoft : colors.surfaceHi,
                  '--pa-btn-pad': '6px 12px',
                  '--pa-btn-radius': '0',
                  width: '100%', justifyContent: 'flex-start',
                  textAlign: 'left', lineHeight: 1.5,
                } as CSSProperties}
              >
                {/* The two lines stack inside the label, not on the button:
                    `Button` folds its children into one span, so a column laid
                    out on the button itself would put the name and the message
                    count back on the same line. */}
                <span style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 1 }}>
                  <span style={{ fontSize: textSize.caption, color: isCurrent ? colors.cyan : colors.text, fontFamily: font.body }}>
                    {session.name || `Session ${session.id}`}
                  </span>
                  {session.updated_at && (
                    <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
                      {session.message_count} msgs · {timeAgo(session.updated_at)}
                    </span>
                  )}
                </span>
              </Button>
            );
          })}
        </div>
      )}
    </div>
  );
}
