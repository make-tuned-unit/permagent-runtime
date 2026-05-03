import { useEffect, useRef, useState, useCallback } from 'react';
import { color, font, ease, radius } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { DropZone } from './DropZone';

const MIN_W = 320;
const MIN_H = 300;
const DEFAULT_W = 400;
const DEFAULT_H = 520;

export function ChatWidget() {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState({ x: -1, y: -1 }); // -1 = use default
  const [size, setSize] = useState({ w: DEFAULT_W, h: DEFAULT_H });

  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const chatInputRef = useRef<ChatInputHandle>(null);
  const connectedRef = useRef(false);
  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null);
  const resizeRef = useRef<{ startX: number; startY: number; origW: number; origH: number; origX: number; origY: number } | null>(null);

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  // Connect session lazily when widget opens
  useEffect(() => {
    if (!open || connectedRef.current) return;
    connectedRef.current = true;
    (async () => {
      const sessionId = await ensureSession();
      if (sessionId) {
        await loadSessionMessages(sessionId);
        connectSession(sessionId);
      }
    })();
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  // Default position: bottom-right
  const actualX = pos.x === -1 ? window.innerWidth - size.w - 24 : pos.x;
  const actualY = pos.y === -1 ? window.innerHeight - size.h - 24 : pos.y;

  // Drag handling
  const onDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startY: e.clientY, origX: actualX, origY: actualY };
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = ev.clientX - dragRef.current.startX;
      const dy = ev.clientY - dragRef.current.startY;
      setPos({ x: dragRef.current.origX + dx, y: dragRef.current.origY + dy });
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, [actualX, actualY]);

  // Resize handling (from top-left corner)
  const onResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    resizeRef.current = { startX: e.clientX, startY: e.clientY, origW: size.w, origH: size.h, origX: actualX, origY: actualY };
    const onMove = (ev: MouseEvent) => {
      if (!resizeRef.current) return;
      const dx = ev.clientX - resizeRef.current.startX;
      const dy = ev.clientY - resizeRef.current.startY;
      const newW = Math.max(MIN_W, resizeRef.current.origW - dx);
      const newH = Math.max(MIN_H, resizeRef.current.origH - dy);
      setSize({ w: newW, h: newH });
      setPos({
        x: resizeRef.current.origX + (resizeRef.current.origW - newW),
        y: resizeRef.current.origY + (resizeRef.current.origH - newH),
      });
    };
    const onUp = () => {
      resizeRef.current = null;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, [size, actualX, actualY]);

  // Collapsed: floating pill button
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

  // Expanded: draggable + resizable chat panel
  return (
    <div style={{
      position: 'fixed', left: actualX, top: actualY, zIndex: 9999,
      width: size.w, height: size.h,
      borderRadius: radius.lg,
      background: 'rgba(11,18,32,0.95)', backdropFilter: 'blur(24px)',
      border: `1px solid ${color.borderHi}`,
      boxShadow: '0 16px 48px rgba(0,0,0,0.6), 0 0 0 1px rgba(0,213,255,0.08)',
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      {/* Resize handle — top-left corner */}
      <div
        onMouseDown={onResizeStart}
        style={{
          position: 'absolute', top: 0, left: 0, width: 14, height: 14,
          cursor: 'nwse-resize', zIndex: 2,
        }}
      />

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
            width: 24, height: 24, borderRadius: 6,
            background: 'transparent', border: 'none',
            color: color.textMuted, cursor: 'pointer',
            display: 'grid', placeItems: 'center', fontSize: 16,
          }}
        >×</button>
      </div>

      {/* Messages + Input with drop zone */}
      <DropZone onDrop={handleDrop}>
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
          <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <MessageList />
          </div>
          <ChatInput ref={chatInputRef} />
        </div>
      </DropZone>
    </div>
  );
}
