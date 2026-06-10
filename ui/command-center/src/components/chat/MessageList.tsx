import { useRef, useEffect, useMemo, useState, useCallback } from 'react';
import { FiChevronDown, FiMessageSquare } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { MessageBubble } from './MessageBubble';
import { StreamingIndicator } from './StreamingIndicator';

export function MessageList() {
  const { colors } = useTheme();
  const chatMessages = useCommandCenter(s => s.chatMessages);
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const streamingMessageId = useCommandCenter(s => s._streamingMessageId);

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
          <div className="flex flex-col items-center justify-center h-full text-xs text-center gap-2" style={{ color: colors.textMuted, fontFamily: font.body }}>
            <FiMessageSquare size={24} style={{ opacity: 0.3, color: colors.cyan }} />
            <div>Send a message to start a conversation with your agent.</div>
          </div>
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
