import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { DropZone } from './DropZone';
import { ModelPicker } from './ModelPicker';
import { InspectionPanel } from '../inspection/InspectionPanel';
import { AwarenessIndicator } from '../awareness/AwarenessIndicator';
import { PreTurnPreview } from '../awareness/PreTurnPreview';

const MIN_W = 320;
const MIN_H = 280;
const DEFAULT_W = 400;
const DEFAULT_H = 520;
const EDGE = 6;

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

export function ChatWidget() {
  const { gradient } = useTheme();
  const open = useCommandCenter(s => s.chatOpen);
  const setChatOpen = useCommandCenter(s => s.setChatOpen);
  const [pos, setPos] = useState({ x: -1, y: -1 });
  const [size, setSize] = useState({ w: DEFAULT_W, h: DEFAULT_H });
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

  useEffect(() => {
    if (!open || connectedRef.current) return;
    connectedRef.current = true;
    (async () => {
      const sid = await ensureSession();
      if (sid) {
        await loadSessionMessages(sid);
        connectSession(sid);
      }
    })();
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  // Load sessions when dropdown opens
  useEffect(() => {
    if (sessionsOpen) loadSessions();
  }, [sessionsOpen, loadSessions]);

  // Close sessions dropdown on outside click
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

  const getPos = () => ({
    x: pos.x === -1 ? window.innerWidth - size.w - 24 : pos.x,
    y: pos.y === -1 ? window.innerHeight - size.h - 24 : pos.y,
  });

  const onDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    const p = getPos();
    const startX = e.clientX, startY = e.clientY;
    const origX = p.x, origY = p.y;
    const onMove = (ev: MouseEvent) => {
      setPos({ x: origX + ev.clientX - startX, y: origY + ev.clientY - startY });
    };
    const onUp = () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp); };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, [pos, size]); // eslint-disable-line react-hooks/exhaustive-deps

  const onEdgeStart = useCallback((e: React.MouseEvent, edges: { top?: boolean; bottom?: boolean; left?: boolean; right?: boolean }) => {
    e.preventDefault();
    e.stopPropagation();
    const p = getPos();
    const startX = e.clientX, startY = e.clientY;
    const origW = size.w, origH = size.h, origX = p.x, origY = p.y;
    const onMove = (ev: MouseEvent) => {
      let w = origW, h = origH, x = origX, y = origY;
      if (edges.right) w = Math.max(MIN_W, origW + ev.clientX - startX);
      if (edges.bottom) h = Math.max(MIN_H, origH + ev.clientY - startY);
      if (edges.left) { w = Math.max(MIN_W, origW - (ev.clientX - startX)); x = origX + origW - w; }
      if (edges.top) { h = Math.max(MIN_H, origH - (ev.clientY - startY)); y = origY + origH - h; }
      setSize({ w, h });
      setPos({ x, y });
    };
    const onUp = () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp); };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, [pos, size]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!open) {
    return (
      <button onClick={() => setChatOpen(true)} style={{
        position: 'fixed', bottom: 20, right: 20, zIndex: 9999,
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '12px 20px', borderRadius: 999,
        background: 'rgba(20,28,48,0.85)', backdropFilter: 'blur(16px)',
        border: `1px solid ${color.borderHi}`,
        color: color.cyan, cursor: 'pointer',
        fontFamily: font.body, fontSize: 13, fontWeight: 600,
        boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
        transition: `all 200ms ${ease.out}`,
      }}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2v10z" />
        </svg>
        Chat with {agentName}
      </button>
    );
  }

  const p = getPos();

  return (
    <div style={{
      position: 'fixed', left: p.x, top: p.y, zIndex: 9999,
      width: size.w, height: size.h,
      borderRadius: radius.lg,
      background: gradient.dropdown, backdropFilter: 'blur(24px)',
      border: `1px solid ${color.borderHi}`,
      boxShadow: '0 16px 48px rgba(0,0,0,0.6), 0 0 0 1px rgba(0,213,255,0.08)',
      display: 'flex', flexDirection: 'column',
    }}>
      {/* Resize handles */}
      <div onMouseDown={e => onEdgeStart(e, { top: true })} style={{ position: 'absolute', top: -EDGE/2, left: EDGE, right: EDGE, height: EDGE, cursor: 'ns-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true })} style={{ position: 'absolute', bottom: -EDGE/2, left: EDGE, right: EDGE, height: EDGE, cursor: 'ns-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { left: true })} style={{ position: 'absolute', top: EDGE, bottom: EDGE, left: -EDGE/2, width: EDGE, cursor: 'ew-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { right: true })} style={{ position: 'absolute', top: EDGE, bottom: EDGE, right: -EDGE/2, width: EDGE, cursor: 'ew-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { top: true, left: true })} style={{ position: 'absolute', top: -EDGE/2, left: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nwse-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { top: true, right: true })} style={{ position: 'absolute', top: -EDGE/2, right: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nesw-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true, left: true })} style={{ position: 'absolute', bottom: -EDGE/2, left: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nesw-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true, right: true })} style={{ position: 'absolute', bottom: -EDGE/2, right: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nwse-resize', zIndex: 4 }} />

      {/* Draggable header */}
      <div
        onMouseDown={onDragStart}
        style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '10px 12px 10px 16px',
          borderBottom: `1px solid ${color.border}`,
          flexShrink: 0, cursor: 'grab', userSelect: 'none',
          position: 'relative',
        }}
      >
        {/* Möbius + name — click toggles sessions */}
        <div
          ref={sessionsRef}
          onMouseDown={e => e.stopPropagation()}
          style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', flex: 1 }}
          onClick={() => setSessionsOpen(!sessionsOpen)}
        >
          <Mobius size={20} state="idle" glow={0.5} />
          <span style={{ fontFamily: font.display, fontSize: 13, fontWeight: 600, color: color.text }}>
            {agentName}
          </span>
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke={color.textMuted} strokeWidth={2.5} strokeLinecap="round"
            style={{ transition: `transform 150ms ${ease.out}`, transform: sessionsOpen ? 'rotate(180deg)' : 'rotate(0deg)' }}>
            <path d="M6 9l6 6 6-6" />
          </svg>

          {/* Sessions dropdown */}
          {sessionsOpen && (
            <div
              onClick={e => e.stopPropagation()}
              style={{
                position: 'absolute', top: '100%', left: 0, right: 60, zIndex: 50,
                maxHeight: 320, overflowY: 'auto',
                background: gradient.dropdown, backdropFilter: 'blur(16px)',
                border: `1px solid ${color.borderHi}`,
                borderRadius: radius.md,
                boxShadow: '0 12px 40px rgba(0,0,0,0.5)',
              }}
            >
              {/* New session button */}
              <button onClick={handleNewSession} style={{
                width: '100%', display: 'flex', alignItems: 'center', gap: 8,
                padding: '10px 14px',
                borderBottom: `1px solid ${color.border}`,
                background: 'transparent', border: 'none', borderBottomStyle: 'solid',
                color: color.cyan, cursor: 'pointer',
                fontFamily: font.body, fontSize: 12, fontWeight: 600,
              }}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
                  <path d="M12 5v14M5 12h14" />
                </svg>
                New session
              </button>

              {sessions.length === 0 && (
                <div style={{ padding: '16px 14px', fontSize: 11, color: color.textDim, textAlign: 'center', fontFamily: font.mono }}>
                  No sessions yet
                </div>
              )}

              {sessions.map(s => {
                const isActive = s.id === chatSessionId;
                return (
                  <button
                    key={s.id}
                    onClick={() => handleSelectSession(s.id)}
                    style={{
                      width: '100%', display: 'flex', alignItems: 'center', gap: 10,
                      padding: '10px 14px', textAlign: 'left',
                      background: isActive ? 'rgba(0,213,255,0.06)' : 'transparent',
                      borderLeft: isActive ? `2px solid ${color.cyan}` : '2px solid transparent',
                      border: 'none', borderBottom: `1px solid ${color.border}`,
                      borderLeftWidth: 2, borderLeftStyle: 'solid',
                      borderLeftColor: isActive ? color.cyan : 'transparent',
                      cursor: 'pointer',
                      transition: `background 100ms ${ease.out}`,
                    }}
                    onMouseEnter={e => { if (!isActive) e.currentTarget.style.background = 'rgba(255,255,255,0.03)'; }}
                    onMouseLeave={e => { if (!isActive) e.currentTarget.style.background = 'transparent'; }}
                  >
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{
                        fontFamily: font.body, fontSize: 12, fontWeight: 500,
                        color: isActive ? color.text : color.textMuted,
                        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                      }}>
                        {s.name || s.id.slice(0, 12)}
                      </div>
                      <div style={{ fontFamily: font.mono, fontSize: 10, color: color.textDim, marginTop: 2 }}>
                        {s.message_count} msg{s.message_count !== 1 ? 's' : ''}
                        {s.updated_at && ` · ${timeAgo(s.updated_at)}`}
                      </div>
                    </div>
                    {isActive && (
                      <div style={{ width: 5, height: 5, borderRadius: '50%', background: color.cyan, flexShrink: 0 }} />
                    )}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        {/* Model picker */}
        <div onMouseDown={e => e.stopPropagation()}>
          <ModelPicker />
        </div>
        {/* Inspection panel toggle */}
        <button
          onClick={() => setInspectionOpen(!inspectionOpen)}
          onMouseDown={e => e.stopPropagation()}
          title="What your agent sees"
          style={{
            width: 28, height: 28, borderRadius: 6,
            background: inspectionOpen ? 'rgba(0,213,255,0.1)' : 'rgba(255,255,255,0.04)',
            border: `1px solid ${inspectionOpen ? 'rgba(0,213,255,0.3)' : color.border}`,
            color: inspectionOpen ? color.cyan : color.textMuted, cursor: 'pointer',
            display: 'grid', placeItems: 'center',
            transition: `all 150ms ${ease.out}`,
          }}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
            <circle cx="12" cy="12" r="3" />
          </svg>
        </button>
        {/* Close */}
        <button
          onClick={() => setChatOpen(false)}
          onMouseDown={e => e.stopPropagation()}
          style={{
            width: 28, height: 28, borderRadius: 6,
            background: 'rgba(255,255,255,0.04)', border: `1px solid ${color.border}`,
            color: color.textMuted, cursor: 'pointer',
            display: 'grid', placeItems: 'center',
            transition: `all 150ms ${ease.out}`,
          }}
          onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.08)'; e.currentTarget.style.color = color.text; }}
          onMouseLeave={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.04)'; e.currentTarget.style.color = color.textMuted; }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
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

      {/* Inspection panel slide-over */}
      {inspectionOpen && <InspectionPanel onClose={() => setInspectionOpen(false)} />}
    </div>
  );
}
