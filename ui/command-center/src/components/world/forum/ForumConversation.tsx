import { useState } from 'react';
import { useCommandCenter } from '../../../lib/store';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { MessageBubble } from '../../chat/MessageBubble';

export function ForumConversation() {
  const { colors } = useTheme();
  const messages = useCommandCenter(s=>s.chatMessages);
  const streaming = useCommandCenter(s=>s.isStreaming);
  const requestId = useCommandCenter(s=>s._activeRequestId);
  const connection = useCommandCenter(s=>s.connectionStatus);
  const [error,setError] = useState('');
  const connectionLabel = {
    connected: 'Shared chat connected',
    connecting: 'Connecting to shared chat…',
    disconnected: 'Shared chat is not connected. Open it to continue your conversation.',
    error: 'Shared chat could not connect. Open it to check the connection and try again.',
  }[connection];
  return <section className="forum-conversation" aria-label="Shared conversation">
    <p>This is your shared conversation with the orchestrator. Open shared chat to read and reply in the current conversation.</p>
    <Button colors={colors} variant="primary" flashSuccess={false} onClick={() => useCommandCenter.getState().openChatDock()}>Open shared chat</Button>
    <p role="status">{connectionLabel}{streaming ? ' · Reply in progress' : ''}</p>
    <div className="forum-transcript">{messages.length > 0
      ? messages.map(m=><MessageBubble key={m.id} message={m} />)
      : <p>No messages to show here yet. Open shared chat to load your conversation or write your first message.</p>}</div>
    {streaming && <Button colors={colors} disabled={!requestId} flashSuccess={false} onClick={async()=>{
      setError(''); try { await useCommandCenter.getState().stopStreaming(); } catch { setError('Could not stop the reply. It may still be running.'); return false; }
    }}>Stop reply</Button>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
