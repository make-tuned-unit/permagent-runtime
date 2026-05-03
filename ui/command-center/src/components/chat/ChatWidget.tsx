import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease, radius } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { DropZone } from './DropZone';

const MIN_W = 320;
const MIN_H = 280;
const DEFAULT_W = 400;
const DEFAULT_H = 520;
const EDGE = 6; // resize handle thickness

export function ChatWidget() {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState({ x: -1, y: -1 });
  const [size, setSize] = useState({ w: DEFAULT_W, h: DEFAULT_H });

  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
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

  const getPos = () => ({
    x: pos.x === -1 ? window.innerWidth - size.w - 24 : pos.x,
    y: pos.y === -1 ? window.innerHeight - size.h - 24 : pos.y,
  });

  // ── Drag (header) ──
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

  // ── Resize (edges) ──
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
      <button onClick={() => setOpen(true)} style={{
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
        Chat
      </button>
    );
  }

  const p = getPos();

  return (
    <div style={{
      position: 'fixed', left: p.x, top: p.y, zIndex: 9999,
      width: size.w, height: size.h,
      borderRadius: radius.lg,
      background: 'rgba(11,18,32,0.95)', backdropFilter: 'blur(24px)',
      border: `1px solid ${color.borderHi}`,
      boxShadow: '0 16px 48px rgba(0,0,0,0.6), 0 0 0 1px rgba(0,213,255,0.08)',
      display: 'flex', flexDirection: 'column',
    }}>
      {/* Resize handles — edges */}
      <div onMouseDown={e => onEdgeStart(e, { top: true })} style={{ position: 'absolute', top: -EDGE/2, left: EDGE, right: EDGE, height: EDGE, cursor: 'ns-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true })} style={{ position: 'absolute', bottom: -EDGE/2, left: EDGE, right: EDGE, height: EDGE, cursor: 'ns-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { left: true })} style={{ position: 'absolute', top: EDGE, bottom: EDGE, left: -EDGE/2, width: EDGE, cursor: 'ew-resize', zIndex: 3 }} />
      <div onMouseDown={e => onEdgeStart(e, { right: true })} style={{ position: 'absolute', top: EDGE, bottom: EDGE, right: -EDGE/2, width: EDGE, cursor: 'ew-resize', zIndex: 3 }} />
      {/* Corners */}
      <div onMouseDown={e => onEdgeStart(e, { top: true, left: true })} style={{ position: 'absolute', top: -EDGE/2, left: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nwse-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { top: true, right: true })} style={{ position: 'absolute', top: -EDGE/2, right: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nesw-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true, left: true })} style={{ position: 'absolute', bottom: -EDGE/2, left: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nesw-resize', zIndex: 4 }} />
      <div onMouseDown={e => onEdgeStart(e, { bottom: true, right: true })} style={{ position: 'absolute', bottom: -EDGE/2, right: -EDGE/2, width: EDGE*2, height: EDGE*2, cursor: 'nwse-resize', zIndex: 4 }} />

      {/* Draggable header */}
      <div
        onMouseDown={onDragStart}
        style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '10px 16px',
          borderBottom: `1px solid ${color.border}`,
          flexShrink: 0, cursor: 'grab', userSelect: 'none',
        }}
      >
        <Mobius size={20} state="idle" glow={0.5} />
        <span style={{ fontFamily: font.display, fontSize: 13, fontWeight: 600, color: color.text, flex: 1 }}>
          Chat
        </span>
        <button
          onClick={() => setOpen(false)}
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
            <ChatInput ref={chatInputRef} />
          </div>
        </DropZone>
      </div>
    </div>
  );
}
