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
import { DiscussNotice } from './DiscussNotice';
import { useTheme } from '../../styles/useTheme';
import { DropZone } from './DropZone';

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
  const worldConversationActive = useCommandCenter(s => s.worldConversationActive);
  // With chat detached, THAT window's mirror orb is the conversation surface.
  // Without this guard the dock rendered a second orb simultaneously.
  const chatWindowOpen = useCommandCenter(s => s.chatWindowOpen);
  const voiceOverlay = Boolean(voiceConversation && !chatWindowOpen && !worldConversationActive);

  return (
    <DropZone zoneId="chat-dock-drop" priority={20} onDrop={(files) => chatInputRef.current?.addFiles(files)}>
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
      {/* Pending decisions are NOT shown here. They were a strip of cards
          above the composer (and a footer inside the voice orb) carrying
          Approve / Reject / Add note / Discuss buttons; the user asked for
          them out of chat on 2026-09-20. The Decision Inbox on Home is where
          they live, and Henry still raises them in conversation. */}
      <ChatInput ref={chatInputRef} />

      {/* The decisions go INSIDE the orb, not over it. They used to be a second
          absolutely-positioned layer (`bottom: space.huge`, `zIndex: 70`) above
          the orb's own `inset: 0` takeover: nothing tied the two boxes
          together, and the orb centres its state labels in the full surface
          height, so `SAY "HEY HENRY"` and `click anywhere to end the
          conversation` rendered straight through the decision cards' text
          (reported 2026-09-18). As the orb's footer they are flex siblings of
          the orb block, so the orb gives up the height they take. */}
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
      </div>
    </DropZone>
  );
}
