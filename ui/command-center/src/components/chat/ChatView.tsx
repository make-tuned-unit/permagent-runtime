import { useEffect, useRef } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { MessageList } from './MessageList';
import { ChatInput } from './ChatInput';
import type { ChatInputHandle } from './ChatInput';
import { SkillPromptBanner } from './SkillPromptBanner';
import { RoleRoutingPrompt } from './RoleRoutingPrompt';
import { ModelPicker } from './ModelPicker';
import { SessionPicker } from './SessionPicker';
import { VoiceOrb } from '../voice/VoiceOrb';
import { ChatPendingDecisions } from './ChatPendingDecisions';
import { DiscussNotice } from './DiscussNotice';
import { useTheme } from '../../styles/useTheme';
import { space } from '../../styles/tokens';

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
  // Files dropped on the app shell while the dock is the chat surface land in
  // this composer (see App.handleDrop). Subscribing to the field (not just
  // takeChatFiles) makes a drop that happens after mount consume too.
  const pendingChatFiles = useCommandCenter(s => s.pendingChatFiles);
  useEffect(() => {
    if (!pendingChatFiles) return;
    const files = useCommandCenter.getState().takeChatFiles();
    if (files && files.length > 0) chatInputRef.current?.addFiles(files);
  }, [pendingChatFiles]);

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

  // Conversation mode (hands-free voice): the orb takes over the whole chat
  // surface. Published by VoiceButton, which keeps running underneath — the
  // overlay is visual only, turn-taking stays with the VAD.
  const voiceConversation = useCommandCenter(s => s.voiceConversation);
  // With chat detached, THAT window's mirror orb is the conversation surface.
  // Without this guard the dock rendered a second orb simultaneously.
  const chatWindowOpen = useCommandCenter(s => s.chatWindowOpen);
  const voiceOverlay = Boolean(voiceConversation && !chatWindowOpen);

  return (
    <div className="relative flex h-full flex-col" style={{ backgroundColor: colors.bg }}>
      <div
        className="flex items-center justify-between px-4 py-2.5"
        style={{ borderBottom: `1px solid ${colors.border}` }}
      >
        {/* Keep this row unclipped: SessionPicker's panel opens below it. */}
        <SessionPicker />
        <ModelPicker />
      </div>

      <MessageList />
      <SkillPromptBanner />
      <RoleRoutingPrompt />
      {/* Meeting-recorder dock slot: MeetingRecorder portals its live panel
          here while the dock is open (state stays in the Sidebar mount).
          Zero-height when empty; content-sized, capped, scrollable when not. */}
      <div id="meeting-dock-slot" style={{ flexShrink: 0, maxHeight: '45%', overflowY: 'auto' }} />
      <DiscussNotice />
      {!voiceOverlay && <ChatPendingDecisions />}
      <ChatInput ref={chatInputRef} />

      {voiceConversation && voiceOverlay && (
        <VoiceOrb
          state={voiceConversation.state}
          getPlaybackAnalyser={voiceConversation.getPlaybackAnalyser}
          getMicAnalyser={voiceConversation.getMicAnalyser}
          onExit={voiceConversation.exit}
          wakeHint={voiceConversation.wakeHint}
          teachWord={voiceConversation.teachWord}
        />
      )}
      {voiceOverlay && (
        <div
          style={{
            position: 'absolute',
            left: space.xl,
            right: space.xl,
            bottom: space.huge,
            zIndex: 70,
          }}
        >
          <ChatPendingDecisions overlay />
        </div>
      )}
    </div>
  );
}
