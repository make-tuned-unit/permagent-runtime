import { useEffect, useRef } from 'react';
import { color, font, radius } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from '../dashboard/useDashboard';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from '../chat/MessageList';
import { ChatInput } from '../chat/ChatInput';
import type { ChatInputHandle } from '../chat/ChatInput';
import { TerminalManager } from '../terminal/TerminalManager';
import { Browser } from '../browser';

// ── Atoms ────────────────────────────────────────────────────────────

const ghostBtn: React.CSSProperties = {
  height: 30, padding: '0 12px', borderRadius: 8,
  background: 'transparent', border: `1px solid ${color.border}`,
  fontFamily: font.body, fontSize: 12, fontWeight: 500,
  color: color.text, cursor: 'pointer',
  display: 'inline-flex', alignItems: 'center', gap: 6,
};

const primaryBtn: React.CSSProperties = {
  height: 30, padding: '0 14px', borderRadius: 8,
  background: color.cyan, color: '#0B1220', border: 'none',
  fontFamily: font.body, fontSize: 12, fontWeight: 600,
  cursor: 'pointer', boxShadow: `0 0 14px ${color.cyanGlow}`,
};

// ── Build View ───────────────────────────────────────────────────────

export function BuildView() {
  const { data } = useDashboard();
  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);

  const agentName = data?.agent.name ?? 'Agent';
  const hasActive = (data?.in_flight.length ?? 0) > 0;
  const activeTask = hasActive ? data!.in_flight[0] : null;
  const mobiusState = hasActive ? 'thinking' : 'idle';

  // Connect session for chat
  useEffect(() => {
    if (connectedRef.current) return;
    connectedRef.current = true;
    (async () => {
      const sid = await ensureSession();
      if (sid) {
        await loadSessionMessages(sid);
        connectSession(sid);
      }
    })();
    return () => {
      useCommandCenter.getState().disconnectSession();
      connectedRef.current = false;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
      background: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 60%)',
      color: color.text, fontFamily: font.body,
    }}>
      {/* Title strip */}
      <div style={{
        padding: '14px 24px', display: 'flex', alignItems: 'center', gap: 14,
        borderBottom: `1px solid ${color.border}`, flexShrink: 0,
      }}>
        <Mobius size={36} state={mobiusState as any} glow={0.9} />
        <div style={{ minWidth: 0 }}>
          <div style={{ fontFamily: font.display, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em' }}>
            {activeTask ? activeTask.title : 'Build'}
          </div>
          <div style={{ fontSize: 11, color: color.textMuted, marginTop: 2, display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{
              width: 5, height: 5, borderRadius: '50%',
              background: hasActive ? color.cyan : color.textDim,
              boxShadow: hasActive ? '0 0 6px rgba(0,213,255,0.7)' : 'none',
            }} />
            {agentName} · {hasActive ? 'thinking' : 'idle'}
          </div>
        </div>
        <div style={{ flex: 1 }} />

        {/* Progress rail */}
        <div style={{ display: 'flex', gap: 6 }}>
          {[1, 2, 3, 4, 5].map(n => {
            const step = hasActive ? 3 : 0;
            return (
              <div key={n} style={{
                width: 26, height: 4, borderRadius: 2,
                background: n < step ? '#5BD17F' : n === step ? color.cyan : 'rgba(255,255,255,0.08)',
                boxShadow: n === step ? `0 0 6px ${color.cyanGlow}` : 'none',
              }} />
            );
          })}
        </div>

        {hasActive && (
          <>
            <button style={ghostBtn}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
                <rect x="6" y="4" width="4" height="16" rx="1" /><rect x="14" y="4" width="4" height="16" rx="1" /></svg>
              Pause
            </button>
            <button style={primaryBtn}>Take over</button>
          </>
        )}
      </div>

      {/* Top row: terminal + browser */}
      <div style={{
        padding: '18px 24px 12px',
        display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14,
        height: 360, flexShrink: 0,
      }}>
        <div style={{ borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${color.border}` }}>
          <TerminalManager />
        </div>
        <div style={{ borderRadius: radius.md, overflow: 'hidden', border: `1px solid ${color.border}` }}>
          <Browser />
        </div>
      </div>

      {/* Chat row */}
      <div style={{
        flex: 1, margin: '6px 24px 18px',
        borderRadius: radius.md,
        background: 'rgba(20,28,48,0.45)',
        border: `1px solid ${color.border}`,
        display: 'flex', flexDirection: 'column', minHeight: 0,
        backdropFilter: 'blur(10px)', WebkitBackdropFilter: 'blur(10px)',
        overflow: 'hidden',
      }}>
        {/* Chat header */}
        <div style={{
          padding: '10px 18px', display: 'flex', alignItems: 'center', gap: 10,
          borderBottom: `1px solid ${color.border}`, flexShrink: 0,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600 }}>Conversation</div>
          <div style={{ flex: 1 }} />
        </div>

        {/* Messages */}
        <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          <MessageList />
        </div>

        {/* Composer */}
        <ChatInput ref={chatInputRef} />
      </div>
    </div>
  );
}
