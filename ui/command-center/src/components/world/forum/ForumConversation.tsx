import { useState } from 'react';
import { useCommandCenter } from '../../../lib/store';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { MessageBubble } from '../../chat/MessageBubble';
import { ChatPendingDecisions } from '../../chat/ChatPendingDecisions';

export function ForumConversation() {
  const { colors } = useTheme();
  const messages = useCommandCenter(s=>s.chatMessages);
  const streaming = useCommandCenter(s=>s.isStreaming);
  const requestId = useCommandCenter(s=>s._activeRequestId);
  const connection = useCommandCenter(s=>s.connectionStatus);
  const [error,setError] = useState('');
  return <section className="forum-conversation" aria-label="Shared conversation">
    <p>This is your existing Permagent conversation. The selected character provides context; the orchestrator answers.</p>
    <div className="eyebrow" role="status">{connection}{streaming ? ' · reply in progress' : ''}</div>
    <div className="forum-transcript">{messages.map(m=><MessageBubble key={m.id} message={m} />)}</div>
    <ChatPendingDecisions />
    {streaming && <Button colors={colors} disabled={!requestId} flashSuccess={false} onClick={async()=>{
      setError(''); try { await useCommandCenter.getState().stopStreaming(); } catch { setError('Could not stop the reply. It may still be running.'); return false; }
    }}>Stop reply</Button>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
