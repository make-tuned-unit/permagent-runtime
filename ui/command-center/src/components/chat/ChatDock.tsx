// ChatDock — the right-side sidebar chat (2026-07-11 UX ruling, research-
// validated: dock-first, detach-on-demand — the pattern VS Code/Cursor/Notion
// AI converged on). Chat opens here by default; a detach button promotes it to
// the standalone window (the previous behavior). Small screens get a full-width
// sheet instead of a fixed panel.
//
// Reuses ChatView (the same component the detached window renders), so the two
// modes are the same chat, not two implementations.

import { useEffect, useState } from 'react';
import { ChatView } from './ChatView';
import { useCommandCenter } from '../../lib/store';
import { createChatWindow } from '../../lib/chatWindow';
import { useTheme } from '../../styles/useTheme';
import { font } from '../../styles/tokens';

export const CHAT_DOCK_WIDTH = 384;

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function ChatDock() {
  const { colors, theme } = useTheme();
  const open = useCommandCenter((s) => s.chatDockOpen);
  const closeChatDock = useCommandCenter((s) => s.closeChatDock);
  const [narrow, setNarrow] = useState(
    typeof window !== 'undefined' && window.innerWidth < 640,
  );

  useEffect(() => {
    const onResize = () => setNarrow(window.innerWidth < 640);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  if (!open) return null;

  const detach = async () => {
    closeChatDock();
    if (isTauri) {
      try {
        await createChatWindow(theme);
      } catch {
        /* fall back to keeping the dock closed; user can reopen */
      }
    }
  };

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        right: 0,
        bottom: 0,
        width: narrow ? '100%' : CHAT_DOCK_WIDTH,
        zIndex: 80,
        display: 'flex',
        flexDirection: 'column',
        background: colors.surface,
        backdropFilter: 'blur(24px) saturate(140%)',
        WebkitBackdropFilter: 'blur(24px) saturate(140%)',
        borderLeft: `1px solid ${colors.borderHi}`,
        boxShadow: '-24px 0 60px rgba(0,0,0,0.45)',
      }}
    >
      {/* Dock header — detach + close */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '10px 12px',
          borderBottom: `1px solid ${colors.border}`,
          flexShrink: 0,
        }}
      >
        <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase' }}>
          Chat
        </span>
        <div style={{ flex: 1 }} />
        {isTauri && (
          <button
            onClick={detach}
            title="Open chat in its own window"
            style={iconBtn(colors)}
          >
            {/* detach / pop-out glyph */}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
              <path d="M15 3h6v6M10 14L21 3M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6" />
            </svg>
          </button>
        )}
        <button onClick={closeChatDock} title="Close chat" style={iconBtn(colors)}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* The chat itself — same component as the detached window */}
      <div style={{ flex: 1, minHeight: 0 }}>
        <ChatView />
      </div>
    </div>
  );
}

function iconBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    width: 28,
    height: 28,
    borderRadius: 7,
    background: 'transparent',
    border: `1px solid ${colors.border}`,
    color: colors.textMuted,
    cursor: 'pointer',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  };
}
