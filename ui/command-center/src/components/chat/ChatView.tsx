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
  // #629 multi-client liveness: re-read identity when `identity_changed`
  // arrives on /events (persona edited on another device).
  const identityRev = useCommandCenter(s => s.identityRev);

  useEffect(() => {
    api.getIdentity().then(id => setAgentName(id.first_name)).catch(() => {});
  }, [identityRev]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    let disposed = false;
    (async () => {
      const sessionId = await ensureSession();
      if (sessionId && !disposed) {
        await loadSessionMessages(sessionId);
        if (!disposed) void connectSession(sessionId);
      }
    })();

    return () => {
      disposed = true;
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
