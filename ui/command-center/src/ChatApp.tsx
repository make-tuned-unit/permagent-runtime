/**
 * ChatApp — standalone chat view rendered in the separate chat window.
 * Loaded when the URL contains ?view=chat.
 * All state flows through the daemon (HTTP/SSE), not the main window.
 */
import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease } from './styles/tokens';
import { useTheme } from './styles/useTheme';
import { Mobius } from './components/mobius/Mobius';
import { useCommandCenter } from './lib/store';
import { api } from './lib/api';
import { MessageList } from './components/chat/MessageList';
import { ChatInput } from './components/chat/ChatInput';
import type { ChatInputHandle } from './components/chat/ChatInput';
import { DropZone } from './components/chat/DropZone';
import { ModelPicker } from './components/chat/ModelPicker';
import { InspectionPanel } from './components/inspection/InspectionPanel';
import { AwarenessIndicator } from './components/awareness/AwarenessIndicator';
import { PreTurnPreview } from './components/awareness/PreTurnPreview';

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

export default function ChatApp() {
  const { gradient } = useTheme();
  const [agentName, setAgentName] = useState('Agent');
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [inspectionOpen, setInspectionOpen] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);

  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const sessions = useCommandCenter(s => s.sessions);
  const loadSessions = useCommandCenter(s => s.loadSessions);
  const switchToSession = useCommandCenter(s => s.switchToSession);
  const chatSessionId = useCommandCenter(s => s.chatSessionId);

  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);
  const sessionsRef = useRef<HTMLDivElement>(null);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []);

  // Connect on mount
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
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (sessionsOpen) loadSessions();
  }, [sessionsOpen, loadSessions]);

  useEffect(() => {
    if (!sessionsOpen) return;
    const handler = (e: MouseEvent) => {
      if (sessionsRef.current && !sessionsRef.current.contains(e.target as Node)) setSessionsOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [sessionsOpen]);

  const handleSelectSession = async (sessionId: string) => {
    setSessionsOpen(false);
    await switchToSession(sessionId);
  };

  const handleNewSession = async () => {
    setSessionsOpen(false);
    try {
      const session = await api.createSession();
      await switchToSession(session.id);
    } catch { /* ignore */ }
  };

  return (
    <div style={{
      width: '100vw', height: '100vh',
      background: gradient.shell,
      display: 'flex', flexDirection: 'column',
      fontFamily: font.body,
    }}>
      {/* Toolbar */}
      <div style={{
        height: 36, flexShrink: 0,
        display: 'flex', alignItems: 'center',
        padding: '0 12px', gap: 8,
        borderBottom: `1px solid ${color.border}`,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1 }}>
          <Mobius size={16} state="idle" glow={0.6} />

          {/* Session selector */}
          <div ref={sessionsRef} style={{ position: 'relative' }}>
            <button
              onClick={() => setSessionsOpen(!sessionsOpen)}
              style={{
                display: 'flex', alignItems: 'center', gap: 4,
                background: 'transparent', border: 'none', cursor: 'pointer',
                color: color.text, fontSize: 13, fontWeight: 600, fontFamily: font.body,
              }}
            >
              {agentName}
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5}><path d="M6 9l6 6 6-6" /></svg>
            </button>

            {sessionsOpen && (
              <div style={{
                position: 'absolute', top: '100%', left: 0, marginTop: 4,
                width: 260, maxHeight: 300, overflow: 'auto',
                background: gradient.dropdown, backdropFilter: 'blur(16px)',
                border: `1px solid ${color.borderHi}`, borderRadius: 8,
                boxShadow: '0 12px 40px rgba(0,0,0,0.6)', zIndex: 100,
                padding: '4px 0',
              }}>
                <button
                  onClick={handleNewSession}
                  style={{
                    width: '100%', padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 6,
                    background: 'transparent', border: 'none', cursor: 'pointer',
                    color: color.cyan, fontSize: 12, fontFamily: font.body, fontWeight: 500,
                    borderBottom: `1px solid ${color.border}`,
                  }}
                >
                  + New session
                </button>
                {sessions.map(s => (
                  <button
                    key={s.id}
                    onClick={() => handleSelectSession(s.id)}
                    style={{
                      width: '100%', padding: '6px 12px', display: 'flex', flexDirection: 'column', gap: 1,
                      background: s.id === chatSessionId ? 'rgba(0,213,255,0.06)' : 'transparent',
                      border: 'none', cursor: 'pointer', textAlign: 'left',
                    }}
                  >
                    <span style={{ fontSize: 12, color: s.id === chatSessionId ? color.cyan : color.text, fontFamily: font.body }}>
                      {s.name || `Session ${s.id}`}
                    </span>
                    {s.updated_at && (
                      <span style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono }}>
                        {s.message_count} msgs · {timeAgo(s.updated_at)}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>

          <div style={{ flex: 1 }} />

          <ModelPicker />

          <button
            onClick={() => setInspectionOpen(!inspectionOpen)}
            title="What your agent sees"
            style={{
              width: 26, height: 26, borderRadius: 6,
              background: inspectionOpen ? 'rgba(0,213,255,0.1)' : 'rgba(255,255,255,0.04)',
              border: `1px solid ${inspectionOpen ? 'rgba(0,213,255,0.3)' : color.border}`,
              color: inspectionOpen ? color.cyan : color.textMuted, cursor: 'pointer',
              display: 'grid', placeItems: 'center',
              transition: `all 150ms ${ease.out}`,
            }}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
              <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
          </button>
        </div>
      </div>

      {/* Chat body */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <DropZone onDrop={handleDrop}>
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              <MessageList />
            </div>
            <AwarenessIndicator onOpenInspection={() => setInspectionOpen(true)} />
            <PreTurnPreview visible={inputFocused} />
            <div onFocusCapture={() => setInputFocused(true)} onBlurCapture={() => setInputFocused(false)}>
              <ChatInput ref={chatInputRef} />
            </div>
          </div>
        </DropZone>
      </div>

      {inspectionOpen && <InspectionPanel onClose={() => setInspectionOpen(false)} />}
    </div>
  );
}
