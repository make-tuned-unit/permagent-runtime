// ChatDock — the right-side sidebar chat (2026-07-11 UX ruling, research-
// validated: dock-first, detach-on-demand — the pattern VS Code/Cursor/Notion
// AI converged on). Chat opens here by default; a detach button promotes it to
// the standalone window (the previous behavior). Small screens get a full-width
// sheet instead of a fixed panel.
//
// Reuses ChatView (the same component the detached window renders), so the two
// modes are the same chat, not two implementations.

import { useCallback, useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { FiExternalLink, FiVolume2, FiVolumeX, FiX } from 'react-icons/fi';
import { Button } from '../common/Button';
import { hardScrollEdgeSurface } from '../common/ViewHeader';
import { ChatView } from './ChatView';
import { useCommandCenter } from '../../lib/store';
import { createChatWindow } from '../../lib/chatWindow';
import { useTheme } from '../../styles/useTheme';
import { setSpeakReplies, useSpeakReplies } from '../../lib/speakReplies';
import { duration, ease, font, radius, space } from '../../styles/tokens';

import { Tooltip } from '../common/Tooltip';
export const CHAT_DOCK_WIDTH = 384;

/**
 * The dock is draggable between these, and the range is the point.
 *
 * 384 was a fixed number with no way to argue with it: too narrow for a code
 * block, too wide on a 1280px window where it took a third of the screen. The
 * floor is the width below which a message bubble stops being readable; the
 * ceiling is where the dock starts being the app rather than a panel beside it.
 */
export const CHAT_DOCK_MIN_WIDTH = 320;
export const CHAT_DOCK_MAX_WIDTH = 560;

const WIDTH_KEY = 'permagent-chat-dock-width';

/** Keyboard resize step for the separator (Left/Right arrows). */
const WIDTH_STEP = space.md;

export function clampDockWidth(px: number): number {
  if (!Number.isFinite(px)) return CHAT_DOCK_WIDTH;
  return Math.min(CHAT_DOCK_MAX_WIDTH, Math.max(CHAT_DOCK_MIN_WIDTH, Math.round(px)));
}

/** The persisted width, clamped — a hand-edited or stale value can't wedge the
 *  dock at 40px or off the side of the window. */
export function readDockWidth(): number {
  try {
    const raw = localStorage.getItem(WIDTH_KEY);
    if (raw === null) return CHAT_DOCK_WIDTH;
    return clampDockWidth(Number(raw));
  } catch {
    return CHAT_DOCK_WIDTH;
  }
}

function writeDockWidth(px: number) {
  try { localStorage.setItem(WIDTH_KEY, String(px)); } catch { /* private mode */ }
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function ChatDock() {
  const { colors, reduceMotion } = useTheme();
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

  // ── Width, and the drag that sets it ──
  //
  // The dock's width is not published anywhere and does not need to be: wide,
  // the dock is a flex SIBLING of <main> (App.tsx:311-317), so widening it
  // narrows <main>, which narrows the browser pane's container, which fires the
  // ResizeObserver that Browser.tsx:480 hangs on that container and calls
  // syncBounds. The native webview follows the layout because it is driven by
  // the layout — see the comment at Browser.tsx:401-404, which already relies
  // on exactly this ("the container rect ALREADY excludes it").
  const [width, setWidth] = useState(readDockWidth);
  const [dragging, setDragging] = useState(false);

  // ── Open/close motion (D9: spring, under 500ms, reduce-motion honored) ──
  //
  // `entered` drives the outer clip width; the INNER panel keeps its full
  // width throughout, so the transcript never reflows during the animation —
  // it slides out from behind a shrinking clip instead of being re-laid-out
  // twenty times in 320ms. `closing` keeps the DOM alive for the exit; the
  // store flag has already flipped, so everything that reacts to
  // `chatDockOpen` (voice teardown, the launcher pill, the meeting panel's
  // dock slot) reacts immediately and only the pixels lag.
  const [entered, setEntered] = useState(false);
  const [closing, setClosing] = useState(false);
  const wasOpen = useRef(open);
  useEffect(() => {
    const was = wasOpen.current;
    wasOpen.current = open;
    if (open) {
      setClosing(false);
      if (reduceMotion) { setEntered(true); return; }
      setEntered(false);
      const id = requestAnimationFrame(() => setEntered(true));
      return () => cancelAnimationFrame(id);
    }
    setEntered(false);
    if (!was || reduceMotion) return;
    setClosing(true);
    const t = setTimeout(() => setClosing(false), duration.smooth);
    return () => clearTimeout(t);
  }, [open, reduceMotion]);

  const commitWidth = useCallback((px: number) => {
    const w = clampDockWidth(px);
    setWidth(w);
    writeDockWidth(w);
  }, []);

  if (!open && !closing) return null;

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

  return (
    <div
      style={{
        // Wide: a real flex sibling of <main>, so the content beside it shrinks
        // instead of being covered. Narrow (<640): a full-width overlay sheet,
        // where there is no room to sit alongside anything.
        ...(narrow
          ? { position: 'fixed' as const, top: 0, right: 0, bottom: 0, width: '100%', zIndex: 80 }
          : { position: 'relative' as const, width: entered ? width : 0, flexShrink: 0, height: '100%' }),
        overflow: 'hidden',
        // A width transition IS a layout animation, so it is deliberately the
        // only one, it is off while the pointer owns the width, and it is off
        // entirely under Reduce Motion.
        transition: dragging || reduceMotion
          ? 'none'
          : narrow
            ? `transform ${duration.smooth}ms ${ease.smooth}`
            : `width ${duration.smooth}ms ${ease.smooth}`,
        ...(narrow ? { transform: entered ? 'translateX(0)' : 'translateX(100%)' } : null),
      }}
    >
      <div
        style={{
          position: 'relative',
          width: narrow ? '100%' : width,
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          // Opaque, and the blur is gone rather than made real (D1). The dock is
          // a content pane, not floating chrome: wide, it is a flex sibling of
          // <main> that SHRINKS the content beside it, so there is nothing
          // behind it to refract; narrow, it is a full-width sheet, and a
          // surface that covers the screen is the most opaque case there is.
          // What it contains — a transcript of message bubbles — is content by
          // Apple's own list.
          background: colors.surface,
          // A neutral hairline, not the cyan `borderHi`: this edge separates two
          // panes, and a tinted line on a non-action is decoration standing in
          // for hierarchy (D13) as well as a second tint competing with the
          // one action allowed to carry one (D8).
          borderLeft: `1px solid ${colors.border}`,
          // Wide, the dock floats over nothing — it takes its own column, so a
          // drop shadow would be elevation with nothing to be elevated above
          // (D13). Narrow, it really is a sheet over the content, and there the
          // theme's own floating elevation is the honest answer.
          ...(narrow ? { boxShadow: colors.elevationFloating } : null),
        }}
      >
        {!narrow && (
          <ResizeEdge
            width={width}
            dragging={dragging}
            setDragging={setDragging}
            onWidth={setWidth}
            onCommit={commitWidth}
          />
        )}

        {/* Dock header — detach + close.
            D11: on macOS the boundary under pinned chrome is the HARD scroll
            edge — "a linear, nearly opaque boundary between pinned controls and
            scrolling content", not a soft gradient. `hardScrollEdgeSurface`
            (ViewHeader.tsx) is that opaque fill; the transcript scrolls under
            it, and the border below is the one hairline that completes the
            boundary — everything else here is spacing (D13). This header
            isn't built from <ViewHeader> (no title/subtitle, just a label and
            controls), so it takes the mechanic as a standalone style fragment
            rather than the whole component. */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: space.md,
            padding: `${space.lg}px ${space.xl}px`,
            borderBottom: `1px solid ${colors.border}`,
            flexShrink: 0,
            ...hardScrollEdgeSurface(colors.surface),
          }}
        >
          <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, letterSpacing: '0.08em', textTransform: 'uppercase' }}>
            Chat
          </span>
          <div style={{ flex: 1 }} />
          {/* Speak replies (#18) — the agent voices completed replies; this is
              the mute. Voice-first onboarding flips it on; the pref persists. */}
          <SpeakToggle />
          {isTauri && <DetachButton onDone={closeChatDock} />}
          <Tooltip content="Close chat">
            <Button
              colors={colors}
              onClick={closeChat}
              aria-label="Close chat"
              style={iconBtn(colors)}
            >
              <FiX size={14} />
            </Button>
          </Tooltip>
        </div>

        {/* The chat itself — same component as the detached window */}
        <div style={{ flex: 1, minHeight: 0 }}>
          <ChatView />
        </div>
      </div>
    </div>
  );
}

/**
 * The grab edge.
 *
 * Pointer-driven and pointer-captured, so a fast drag that outruns the cursor
 * keeps receiving moves instead of dropping the gesture the moment it leaves
 * the 8px strip. It is also a real `separator` with arrow-key steps: a resize
 * that only a mouse can perform is one a keyboard user simply does not have.
 */
function ResizeEdge({
  width,
  dragging,
  setDragging,
  onWidth,
  onCommit,
}: {
  width: number;
  dragging: boolean;
  setDragging: (v: boolean) => void;
  onWidth: (px: number) => void;
  onCommit: (px: number) => void;
}) {
  const { colors, reduceMotion } = useTheme();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const drag = useRef<{ id: number; startX: number; startWidth: number } | null>(null);

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    drag.current = { id: e.pointerId, startX: e.clientX, startWidth: width };
    e.currentTarget.setPointerCapture?.(e.pointerId);
    setDragging(true);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    // Dragging LEFT widens the dock — it lives on the right edge of the window.
    onWidth(clampDockWidth(d.startWidth + (d.startX - e.clientX)));
  };

  const endDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    setDragging(false);
    onCommit(clampDockWidth(d.startWidth + (d.startX - e.clientX)));
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const delta = e.key === 'ArrowLeft' ? WIDTH_STEP : e.key === 'ArrowRight' ? -WIDTH_STEP : 0;
    if (!delta) return;
    e.preventDefault();
    onCommit(width + delta);
  };

  // A pointer user gets the rule on hover; a keyboard user gets the same rule
  // on focus, because a resize that can only be SEEN with a mouse is the same
  // omission as one that can only be performed with one.
  const fill = dragging ? colors.fillActive : (hovered || focused) ? colors.fillHover : 'transparent';
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the chat panel"
      aria-valuenow={width}
      aria-valuemin={CHAT_DOCK_MIN_WIDTH}
      aria-valuemax={CHAT_DOCK_MAX_WIDTH}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onKeyDown}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        position: 'absolute',
        left: 0,
        top: 0,
        bottom: 0,
        width: space.md,
        cursor: 'col-resize',
        zIndex: 2,
        // The hit area is 8px; what LIGHTS is a 2px rule on the seam, so the
        // affordance reads as the edge itself rather than as a stripe beside it.
        display: 'flex',
        justifyContent: 'flex-start',
        touchAction: 'none',
        outline: 'none',
      }}
    >
      <div
        aria-hidden
        style={{
          width: space.xs / 2,
          height: '100%',
          borderRadius: radius.pill,
          background: fill,
          transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.smooth}`,
        }}
      />
    </div>
  );
}

function DetachButton({ onDone }: { onDone: () => void }) {
  const { colors, theme } = useTheme();
  return (
    <Tooltip content="Open chat in its own window">
      <Button
        colors={colors}
        onClick={async () => {
          onDone();
          if (!isTauri) return true;
          try {
            await createChatWindow(theme);
          } catch {
            /* fall back to keeping the dock closed; user can reopen */
            return false;
          }
          return true;
        }}
        aria-label="Open chat in its own window"
        style={iconBtn(colors)}
      >
        {/* detach / pop-out glyph */}
        <FiExternalLink size={14} />
      </Button>
    </Tooltip>
  );
}

function SpeakToggle() {
  const { colors } = useTheme();
  const speaking = useSpeakReplies();
  const label = speaking ? 'Mute — replies go back to text only' : 'Speak replies aloud';
  return (
    <Tooltip content={label}>
      <Button
        colors={colors}
        onClick={() => setSpeakReplies(!speaking)}
        aria-label={label}
        style={{ ...iconBtn(colors), '--pa-btn-fg': speaking ? colors.cyan : colors.textMuted } as CSSProperties}
      >
        {speaking ? (
          <FiVolume2 size={14} />
        ) : (
          <FiVolumeX size={14} />
        )}
      </Button>
    </Tooltip>
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
    '--pa-btn-radius': `${radius.sm}px`,
    width: 28,
    height: 28,
  } as CSSProperties;
}
