import { useEffect } from 'react';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';

export function ChatView() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessions = useCommandCenter(s => s.loadSessions);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);

  // On mount: fetch sessions, load active session messages
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
        await loadSessionMessages(sessionId);
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
