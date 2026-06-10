import { useEffect, useRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';
import { ModelPicker } from './ModelPicker';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function ChatView() {
  const { colors } = useTheme();
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const setAgentName = useCommandCenter(s => s.setAgentName);
  const chatInputRef = useRef<ChatInputHandle>(null);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        <span
          className="text-[11px] uppercase tracking-wider"
          style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
        >
          Chat
        </span>
        <ModelPicker />
      </div>

      <MessageList />
      <SkillPromptBanner />
      <ChatInput ref={chatInputRef} />
    </div>
  );
}
