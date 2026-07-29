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
import { setSpeakReplies, useSpeakReplies } from '../../lib/speakReplies';
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

  // Closing the dock closes the CONVERSATION too. The orb belongs to a chat
  // surface; with the last surface gone there is nowhere honest to show it,
  // and leaving the mic hot behind a closed UI is worse than ending cleanly.
  const closeChat = () => {
    const { voiceConversation } = useCommandCenter.getState();
    voiceConversation?.exit();
    closeChatDock();
  };

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
        // Wide: a real flex sibling of <main>, so the content beside it shrinks
        // instead of being covered. Narrow (<640): a full-width overlay sheet,
        // where there is no room to sit alongside anything.
        ...(narrow
          ? { position: 'fixed' as const, top: 0, right: 0, bottom: 0, width: '100%', zIndex: 80 }
          : { position: 'relative' as const, width: CHAT_DOCK_WIDTH, flexShrink: 0, height: '100%' }),
        display: 'flex',
        flexDirection: 'column',
        background: colors.surface,
        backdropFilter: 'blur(24px) saturate(140%)',
        WebkitBackdropFilter: 'blur(24px) saturate(140%)',
        borderLeft: `1px solid ${colors.borderHi}`,
        // Directional left-edge lift, theme-aware: a deep shadow reads on the
        // void, a soft cool one on silver (a black glow is invisible on light).
        boxShadow: theme === 'silver' ? '-24px 0 48px rgba(30,37,48,0.16)' : '-24px 0 60px rgba(0,0,0,0.45)',
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
        {/* Speak replies (#18) — the agent voices completed replies; this is
            the mute. Voice-first onboarding flips it on; the pref persists. */}
        <SpeakToggle />
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
        <button onClick={closeChat} title="Close chat" style={iconBtn(colors)}>
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

function SpeakToggle() {
  const { colors } = useTheme();
  const speaking = useSpeakReplies();
  return (
    <button
      onClick={() => setSpeakReplies(!speaking)}
      title={speaking ? 'Mute — replies go back to text only' : 'Speak replies aloud'}
      style={{ ...iconBtn(colors), color: speaking ? colors.cyan : colors.textMuted }}
    >
      {speaking ? (
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <path d="M11 5L6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 010 7M19 5a9 9 0 010 14" />
        </svg>
      ) : (
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <path d="M11 5L6 9H2v6h4l5 4V5zM23 9l-6 6M17 9l6 6" />
        </svg>
      )}
    </button>
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
