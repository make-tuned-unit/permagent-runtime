import { useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

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

  const handleNewSession = async () => {
    setOpen(false);
    try {
      const session = await api.createSession();
      await switchToSession(session.id);
    } catch { /* ignore */ }
  };

  return (
    <div ref={pickerRef} data-testid="session-picker" style={{ position: 'relative', minWidth: 0 }}>
      <button
        type="button"
        aria-label="Choose chat session"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        style={{
          display: 'flex', alignItems: 'center', gap: 4, maxWidth: '100%',
          background: 'transparent', border: 'none', cursor: 'pointer',
          color: colors.text, fontSize: 13, fontWeight: 600, fontFamily: font.body,
        }}
      >
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
          {agentName}
        </span>
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} style={{ flexShrink: 0 }}><path d="M6 9l6 6 6-6" /></svg>
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
          <button
            type="button"
            onClick={handleNewSession}
            style={{
              width: '100%', padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 6,
              background: 'transparent', border: 'none', cursor: 'pointer',
              color: colors.cyan, fontSize: 12, fontFamily: font.body, fontWeight: 500,
              borderBottom: `1px solid ${colors.border}`,
            }}
          >
            + New session
          </button>
          {sessions.map(session => (
            <button
              type="button"
              key={session.id}
              onClick={() => handleSelectSession(session.id)}
              style={{
                width: '100%', padding: '6px 12px', display: 'flex', flexDirection: 'column', gap: 1,
                background: session.id === chatSessionId ? colors.cyanSoft : 'transparent',
                border: 'none', cursor: 'pointer', textAlign: 'left',
              }}
            >
              <span style={{ fontSize: 12, color: session.id === chatSessionId ? colors.cyan : colors.text, fontFamily: font.body }}>
                {session.name || `Session ${session.id}`}
              </span>
              {session.updated_at && (
                <span style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>
                  {session.message_count} msgs · {timeAgo(session.updated_at)}
                </span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
