import { useRef, useEffect, useMemo, useState, useCallback } from 'react';
import { FiChevronDown, FiMessageSquare, FiVolume2 } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
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
  const bottomRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [showJump, setShowJump] = useState(false);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
    setAutoScroll(atBottom);
    setShowJump(!atBottom);
  }, []);

  const timeline = useMemo(() => {
    return [...chatMessages].sort((a, b) =>
      new Date(a.timestamp || 0).getTime() - new Date(b.timestamp || 0).getTime()
    );
  }, [chatMessages]);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [timeline.length, autoScroll, chatMessages]);

  const jumpToBottom = () => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
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
            <button
              onClick={() => void loadSessionMessages(chatSessionId)}
              className="text-[12px]"
              style={{
                color: colors.cyan, background: 'none', border: 'none', cursor: 'pointer',
                fontFamily: font.body, padding: 0, fontWeight: 600, flexShrink: 0,
              }}
            >
              Retry
            </button>
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
                  <button
                    onClick={() => void speak(persona?.voice_id, greeting)}
                    title="Hear greeting"
                    aria-label="Hear greeting"
                    className="flex items-center"
                    style={{
                      background: 'none', border: 'none', cursor: 'pointer', padding: 0,
                      color: speaking ? colors.cyan : colors.textMuted, opacity: speaking ? 1 : 0.6,
                    }}
                  >
                    <FiVolume2 size={13} />
                  </button>
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

        <div ref={bottomRef} />
      </div>

      {showJump && (
        <div className="absolute bottom-3 left-1/2 -translate-x-1/2 z-10">
          <button
            onClick={jumpToBottom}
            className="flex items-center gap-1 rounded-full shadow-lg px-3 py-1 text-[11px] transition"
            style={{
              backgroundColor: colors.surface,
              color: colors.cyan,
              fontFamily: font.mono,
              border: `1px solid ${colors.cyan}33`,
              boxShadow: colors.cardShadow,
            }}
          >
            <FiChevronDown size={12} /> Jump to latest
          </button>
        </div>
      )}
    </div>
  );
}
