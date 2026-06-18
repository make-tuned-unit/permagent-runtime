import { useRef, useEffect, useMemo, useState, useCallback } from 'react';
import { FiChevronDown, FiMessageSquare } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { MessageBubble } from './MessageBubble';
import { StreamingIndicator } from './StreamingIndicator';
import { usePersona } from '../settings/useSettings';

export function MessageList() {
  const { colors } = useTheme();
  const chatMessages = useCommandCenter(s => s.chatMessages);
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const streamingMessageId = useCommandCenter(s => s._streamingMessageId);
  const agentName = useCommandCenter(s => s.agentName);
  // W3a: in-character opening greeting shown at conversation start. Display-only
  // — never pushed into chatMessages, so it never enters LLM context, is never
  // replayed, and vanishes the moment a real message exists.
  const { data: persona } = usePersona();
  const greeting = persona?.opening_greeting?.trim();

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

  const showStreamingIndicator = isStreaming && !streamingMessageId;

  return (
    <div className="relative flex-1 overflow-hidden">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="h-full overflow-y-auto p-4 space-y-3"
      >
        {timeline.length === 0 && !isStreaming && (
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

        {timeline.map((msg) => (
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
