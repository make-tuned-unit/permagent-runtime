import { useEffect, useRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';
import { ModelPicker } from './ModelPicker';
import { useTheme } from '../../styles/useTheme';

export function ChatView() {
  const { colors } = useTheme();
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const setAgentName = useCommandCenter(s => s.setAgentName);
  const chatInputRef = useRef<ChatInputHandle>(null);

  // Load agent identity for message labels
  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // On mount: ensure a session exists and connect SSE
  useEffect(() => {
    (async () => {
      const sessionId = await ensureSession();
      if (sessionId) {
        await loadSessionMessages(sessionId);
        connectSession(sessionId);
      }
    })();

    return () => {
      useCommandCenter.getState().disconnectSession();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="flex h-full flex-col" style={{ backgroundColor: colors.bg }}>
      {/* Header */}
      <div className="flex items-center justify-between border-b border-dark-border px-4 py-2.5">
        <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">Chat</span>
        <ModelPicker />
      </div>

      {/* Message list */}
      <MessageList />

      {/* Skill proposal banner */}
      <SkillPromptBanner />

      {/* Input */}
      <ChatInput ref={chatInputRef} />
    </div>
  );
}
