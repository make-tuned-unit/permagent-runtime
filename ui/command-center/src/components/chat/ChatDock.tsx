// ChatDock — the right-side sidebar chat (2026-07-11 UX ruling, research-
// validated: dock-first, detach-on-demand — the pattern VS Code/Cursor/Notion
// AI converged on). Chat opens here by default; a detach button promotes it to
// the standalone window (the previous behavior). Small screens get a full-width
// sheet instead of a fixed panel.
//
// Reuses ChatView (the same component the detached window renders), so the two
// modes are the same chat, not two implementations.

import { useEffect, useState, type CSSProperties } from 'react';
import { FiExternalLink, FiVolume2, FiVolumeX, FiX } from 'react-icons/fi';
import { Button } from '../common/Button';
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

  // Closing the dock closes the CONVERSATION too — but only when the dock is
  // the LAST chat surface. The popped-out window is also a surface (ChatView
  // already yields to it with the same predicate); killing the conversation
  // while it is open muted voice in the window the user was still using.
  // With the last surface gone there is nowhere honest to show the orb, and
  // leaving the mic hot behind a closed UI is worse than ending cleanly.
  const closeChat = () => {
    const { voiceConversation, chatWindowOpen } = useCommandCenter.getState();
    if (!chatWindowOpen) voiceConversation?.exit();
    closeChatDock();
  };

  const detach = async () => {
    closeChatDock();
    if (isTauri) {
      try {
        await createChatWindow(theme);
      } catch {
        /* fall back to keeping the dock closed; user can reopen */
        return false;
      }
    }
    return true;
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
        // Opaque, and the blur is gone rather than made real (D1). The dock is
        // a content pane, not floating chrome: wide, it is a flex sibling of
        // <main> that SHRINKS the content beside it, so there is nothing
        // behind it to refract; narrow, it is a full-width sheet, and a
        // surface that covers the screen is the most opaque case there is.
        // What it contains — a transcript of message bubbles — is content by
        // Apple's own list. The filter here blurred nothing (it sat over this
        // same opaque fill) and cost a compositing pass on the app's most
        // persistent surface.
        background: colors.surface,
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
          <Button
            colors={colors}
            onClick={detach}
            title="Open chat in its own window"
            aria-label="Open chat in its own window"
            style={iconBtn(colors)}
          >
            {/* detach / pop-out glyph */}
            <FiExternalLink size={14} />
          </Button>
        )}
        <Button
          colors={colors}
          onClick={closeChat}
          title="Close chat"
          aria-label="Close chat"
          style={iconBtn(colors)}
        >
          <FiX size={14} />
        </Button>
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
  const label = speaking ? 'Mute — replies go back to text only' : 'Speak replies aloud';
  return (
    <Button
      colors={colors}
      onClick={() => setSpeakReplies(!speaking)}
      title={label}
      aria-label={label}
      style={{ ...iconBtn(colors), '--pa-btn-fg': speaking ? colors.cyan : colors.textMuted } as CSSProperties}
    >
      {speaking ? (
        <FiVolume2 size={14} />
      ) : (
        <FiVolumeX size={14} />
      )}
    </Button>
  );
}

/** The dock header's square icon affordances. The look rides the `--pa-btn-*`
 *  custom properties rather than inline `color`/`background`/`border`, because
 *  an inline declaration outranks `.pa-btn:hover` and would silently kill the
 *  hover/press states the primitive exists to provide. */
function iconBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-pad': '0',
    '--pa-btn-radius': '7px',
    width: 28,
    height: 28,
  } as CSSProperties;
}
