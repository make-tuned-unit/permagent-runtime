import { useEffect } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';

export function ChatView() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessions = useCommandCenter(s => s.loadSessions);

  // On mount: fetch sessions, load active session messages, or create one
  useEffect(() => {
    (async () => {
      await loadSessions();
      const state = useCommandCenter.getState();

      let sessionId = state.chatSessionId;
      if (!sessionId && state.sessions.length > 0) {
        sessionId = state.sessions[0].id;
        useCommandCenter.setState({ chatSessionId: sessionId });
        try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* ignore */ }
      }

      if (sessionId) {
        try {
          const detail = await api.getSession(sessionId);
          if (detail.messages && detail.messages.length > 0) {
            const msgs = detail.messages.map((m, i) => ({
              id: `hist-${sessionId}-${i}`,
              role: (m.role as 'user' | 'assistant' | 'system') || 'assistant',
              content: m.content,
              timestamp: m.timestamp || new Date().toISOString(),
            }));
            useCommandCenter.setState({ chatMessages: msgs });
          }
        } catch {
          // Session may not exist on server yet
        }
      }
    })();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="flex h-full flex-col bg-[#0A0E17]">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">Chat</span>
        {chatSessionId && (
          <span className="text-[9px] font-mono text-dark-muted/50 truncate max-w-[200px]">
            {chatSessionId.slice(0, 12)}
          </span>
        )}
      </div>

      {/* Message list */}
      <MessageList />

      {/* Skill proposal banner */}
      <SkillPromptBanner />

      {/* Input */}
      <ChatInput />
    </div>
  );
}
