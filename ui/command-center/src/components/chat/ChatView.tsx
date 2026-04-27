import { useEffect, useCallback, useRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';
import { DropZone } from './DropZone';

export function ChatView() {
  const chatSessionId = useCommandCenter(s => s.chatSessionId);
  const loadSessionMessages = useCommandCenter(s => s.loadSessionMessages);
  const ensureSession = useCommandCenter(s => s.ensureSession);
  const connectSession = useCommandCenter(s => s.connectSession);
  const chatInputRef = useRef<ChatInputHandle>(null);

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

  const handleDrop = useCallback((files: File[]) => {
    chatInputRef.current?.addFiles(files);
  }, []);

  return (
    <DropZone onDrop={handleDrop}>
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
        <ChatInput ref={chatInputRef} />
      </div>
    </DropZone>
  );
}
