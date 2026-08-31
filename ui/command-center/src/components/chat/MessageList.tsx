import { useRef, useEffect, useMemo, useState, useCallback, type CSSProperties } from 'react';
import { FiChevronDown, FiMessageSquare, FiVolume2 } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { MessageBubble } from './MessageBubble';
import { StreamingIndicator } from './StreamingIndicator';
import { usePersona } from '../settings/useSettings';
import { useVoicePreview } from '../../lib/useVoices';
import { hasSpokenKey, markReplySpoken, replyDedupeKey } from '../../lib/speakReplies';
import type { ChatMessage } from '../../lib/store';

/** A contentless assistant message renders as a bare name-and-time bubble —
 *  which is exactly what the streaming placeholder is before its first token,
 *  sitting above the StreamingIndicator as a second empty bubble (reported
 *  2026-08-06). Nothing to read ⇒ nothing to draw; the indicator alone
 *  carries the in-flight state. Exported pure for the regression test. */
export function isRenderableChatMessage(msg: ChatMessage): boolean {
  return msg.role !== 'assistant'
    || !!msg.content?.trim()
    || !!msg.thinking?.trim()
    || (msg.images?.length ?? 0) > 0
    || (msg.tool_calls?.length ?? 0) > 0
    || !!msg.context_attached;
}

/** Slack, in pixels, that still counts as "reading the live end" — someone a
 *  line or two off the bottom is following the stream, not browsing history. */
const BOTTOM_SLACK_PX = 60;

/** Exported pure for the autoscroll regression test: jsdom reports every box as
 *  0x0, so the decision has to be testable apart from a real scroll container. */
export function isAtBottom(box: { scrollHeight: number; scrollTop: number; clientHeight: number }): boolean {
  return box.scrollHeight - box.scrollTop - box.clientHeight < BOTTOM_SLACK_PX;
}

/** True when a scroll event carries a position WE wrote, not one the reader
 *  chose. Exact-match works only because the pin below writes `scrollTop`
 *  directly; a `behavior: 'smooth'` scroll reports dozens of intermediate
 *  positions we never wrote, and every one of them would read as "the reader
 *  scrolled away" — which is precisely how autoscroll used to disengage
 *  mid-stream and flash the jump pill on. */
export function isSelfScroll(observedTop: number, lastWrittenTop: number | null): boolean {
  return lastWrittenTop !== null && Math.abs(observedTop - lastWrittenTop) <= 1;
}

export function MessageList() {
  const { colors } = useTheme();
  const chatMessages = useCommandCenter(s => s.chatMessages);
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const agentName = useCommandCenter(s => s.agentName);
  // C8 / #568 lesson: a transient history-load failure surfaces inline with a
  // retry — never a silent catch that leaves an inexplicably empty chat.
  const sessionLoadError = useCommandCenter(s => s.sessionLoadError);
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  // W3a: in-character opening greeting shown at conversation start. Display-only
  // — never pushed into chatMessages, so it never enters LLM context, is never
  // replayed, and vanishes the moment a real message exists.
  const { data: persona } = usePersona();
  const greeting = persona?.opening_greeting?.trim();
  // W3b: speak the opening greeting in the persona's chosen voice at
  // conversation start. Auto-attempts once per distinct greeting (degrades
  // silently when voice assets are absent or autoplay is blocked); the speaker
  // button on the bubble is the gesture-driven replay/fallback.
  const { preview: speak, playingId: speaking } = useVoicePreview();
  // Greeting gating (pop-out regression, 2026-08-05): the history fetch races
  // the (much cheaper) identity fetch, so a freshly popped-out window showed —
  // and SPOKE — the greeting over an existing conversation every time.
  const chatHistoryLoaded = useCommandCenter(s => s.chatHistoryLoaded);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [showJump, setShowJump] = useState(false);
  // Where our own last scroll write landed, so the scroll event it provokes is
  // not mistaken for the reader scrolling away. Null ⇒ the next event is theirs.
  const selfScrollTop = useRef<number | null>(null);
  const pinFrame = useRef<number | null>(null);

  const pinToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight - el.clientHeight;
    selfScrollTop.current = el.scrollTop; // read back: the assignment clamps
  }, []);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (isSelfScroll(el.scrollTop, selfScrollTop.current)) {
      selfScrollTop.current = null; // consumed — the next event is the reader's
      return;
    }
    selfScrollTop.current = null;
    const atBottom = isAtBottom(el);
    setAutoScroll(atBottom);
    setShowJump(!atBottom);
  }, []);

  const timeline = useMemo(() => {
    return [...chatMessages].sort((a, b) =>
      new Date(a.timestamp || 0).getTime() - new Date(b.timestamp || 0).getTime()
    );
  }, [chatMessages]);

  // Keep the live end in view while the reply streams.
  //
  // This used to call `scrollIntoView({ behavior: 'smooth' })` on every store
  // change — measured at one call per streamed delta. Each call cancels the
  // in-flight smooth animation and restarts it from wherever it had got to, so
  // at delta rates the scroll never settles: that restart-per-token IS the
  // reported stutter. Two changes fix it: coalesce to at most one write per
  // animation frame (deltas can land several times a frame), and write
  // `scrollTop` directly so there is no animation to restart. The pin lands on
  // the same frame the new text paints, so nothing is delayed to buy this.
  useEffect(() => {
    if (!autoScroll) return;
    if (pinFrame.current !== null) return; // a write is already queued this frame
    pinFrame.current = requestAnimationFrame(() => {
      pinFrame.current = null;
      pinToBottom();
    });
  }, [chatMessages, autoScroll, isStreaming, pinToBottom]);

  useEffect(() => () => {
    if (pinFrame.current !== null) cancelAnimationFrame(pinFrame.current);
  }, []);

  const jumpToBottom = () => {
    // Instant, not smooth, for the same reason as the pin: a smooth animation
    // would report intermediate positions through onScroll and immediately
    // flip the pill it was pressed to dismiss back on.
    pinToBottom();
    setAutoScroll(true);
    setShowJump(false);
  };

  // Auto-speak the greeting once when the empty chat first shows it. Gated on
  // history having SETTLED (not "list is momentarily empty"), and deduped via
  // the cross-window spoken-reply ring — a per-mount ref is structurally
  // incapable of surviving a pop-out, so the new window re-spoke it.
  useEffect(() => {
    if (timeline.length === 0 && !isStreaming && chatHistoryLoaded && greeting) {
      const key = replyDedupeKey(chatSessionId, greeting);
      if (!hasSpokenKey(key)) {
        markReplySpoken(chatSessionId, greeting);
        void speak(persona?.voice_id, greeting);
      }
    }
  }, [timeline.length, isStreaming, chatHistoryLoaded, greeting, chatSessionId, persona?.voice_id, speak]);

  // Visible for the WHOLE in-flight turn — not just before the first token —
  // so mid-turn tool-use silences still read as alive ("Thinking…" escalates
  // with elapsed time; see StreamingIndicator.stageLabel).
  const showStreamingIndicator = isStreaming;

  return (
    <div className="relative flex-1 overflow-hidden">
      <div
        ref={scrollRef}
        data-testid="message-scroller"
        onScroll={handleScroll}
        // overflow-x-hidden is load-bearing: with only overflow-y-auto set,
        // CSS promotes overflow-x from visible to auto, so any wide child gave
        // the whole message list a horizontal scrollbar. Wide content (code,
        // tables) scrolls inside its own block instead.
        className="h-full overflow-y-auto overflow-x-hidden p-4 space-y-3"
      >
        {sessionLoadError && chatSessionId && (
          <div
            className="flex items-center gap-2.5 rounded-lg px-3 py-2"
            style={{ border: `1px solid ${colors.danger}44`, backgroundColor: colors.surface }}
          >
            <span className="text-[12px]" style={{ color: colors.danger, fontFamily: font.body }}>
              Couldn't load this conversation: {sessionLoadError}
            </span>
            <Button
              colors={colors}
              variant="bare"
              className="shrink-0 hover:underline"
              onClick={() => void loadSessionMessages(chatSessionId)}
              style={{
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-weight': 600,
                fontFamily: font.body,
                fontSize: 12,
              } as CSSProperties}
            >
              Retry
            </Button>
          </div>
        )}

        {timeline.length === 0 && !isStreaming && chatHistoryLoaded && !sessionLoadError && (
          greeting ? (
            // Agent's in-character opening, rendered as a top-left assistant
            // bubble (matches MessageBubble's assistant styling) — the identity
            // moment. Display-only; not part of the conversation timeline.
            <div className="flex justify-start">
              <div
                className="max-w-[85%] rounded-xl px-3.5 py-2.5"
                style={{
                  backgroundColor: colors.surface,
                  boxShadow: colors.cardHighlight ? `${colors.cardShadow}, ${colors.cardHighlight}` : colors.cardShadow,
                  overflowWrap: 'break-word', wordBreak: 'break-word', minWidth: 0,
                }}
              >
                <div className="flex items-center gap-2 mb-1">
                  <span className="text-[11px]" style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}>
                    {agentName}
                  </span>
                  <Button
                    colors={colors}
                    variant="bare"
                    onClick={() => void speak(persona?.voice_id, greeting)}
                    title="Hear greeting"
                    aria-label="Hear greeting"
                    style={{
                      '--pa-btn-fg': speaking ? colors.cyan : colors.textMuted,
                      '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '0',
                      opacity: speaking ? 1 : 0.6,
                    } as CSSProperties}
                  >
                    <FiVolume2 size={13} />
                  </Button>
                </div>
                <div className="text-[13px] leading-relaxed whitespace-pre-wrap" style={{ fontFamily: font.body, color: colors.text }}>
                  {greeting}
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-xs text-center gap-2" style={{ color: colors.textMuted, fontFamily: font.body }}>
              <FiMessageSquare size={24} style={{ opacity: 0.3, color: colors.cyan }} />
              <div>Send a message to start a conversation with your agent.</div>
            </div>
          )
        )}

        {timeline.filter(isRenderableChatMessage).map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}

        {showStreamingIndicator && <StreamingIndicator />}
      </div>

      {showJump && (
        <div className="absolute bottom-3 left-1/2 -translate-x-1/2 z-10">
          <Button
            colors={colors}
            onClick={jumpToBottom}
            style={{
              '--pa-btn-bg': colors.surface,
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-border': `${colors.cyan}33`,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '4px 12px',
              '--pa-btn-radius': `${radius.pill}px`,
              fontFamily: font.mono,
              fontSize: 11,
              gap: 4,
              boxShadow: colors.cardShadow,
            } as CSSProperties}
          >
            <FiChevronDown size={12} /> Jump to latest
          </Button>
        </div>
      )}
    </div>
  );
}
