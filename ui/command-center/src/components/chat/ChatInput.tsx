import { useState, useRef, useEffect } from 'react';
import { FiSend, FiLoader } from 'react-icons/fi';
import { useCommandCenter, type ChatMessage } from '../../lib/store';
import { api } from '../../lib/api';

export function ChatInput() {
  const addChatMessage = useCommandCenter(s => s.addChatMessage);
  const isStreaming = useCommandCenter(s => s.isStreaming);

  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const disabled = isStreaming || sending;

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = '36px';
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [input]);

  const handleSend = async () => {
    const msg = input.trim();
    if (!msg || disabled) return;

    const userMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      role: 'user',
      content: msg,
      timestamp: new Date().toISOString(),
    };
    addChatMessage(userMsg);
    setInput('');
    setSending(true);

    try {
      let sessionId = useCommandCenter.getState().chatSessionId;
      if (!sessionId) {
        try {
          const session = await api.createSession();
          sessionId = session.id;
          useCommandCenter.setState({ chatSessionId: sessionId });
          try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* ignore */ }
        } catch {
          sessionId = 'default';
        }
      }

      const res = await api.sendReply(sessionId, msg);

      if (res.session_id) {
        useCommandCenter.setState({ chatSessionId: res.session_id });
        try { localStorage.setItem('permagent-chat-session-id', res.session_id); } catch { /* ignore */ }
      }

      addChatMessage({
        id: `msg-${Date.now()}-reply`,
        role: 'assistant',
        content: res.reply,
        timestamp: new Date().toISOString(),
      });
    } catch (err) {
      setInput(msg);
      addChatMessage({
        id: `msg-${Date.now()}-err`,
        role: 'system',
        content: `Failed: ${err instanceof Error ? err.message : 'Unknown error'}`,
        timestamp: new Date().toISOString(),
      });
    } finally {
      setSending(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="border-t border-slate-800 bg-[#0C1019] p-3">
      <div className="flex items-end gap-2">
        <textarea
          ref={textareaRef}
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={disabled ? 'Agent is responding...' : 'Message your agent...'}
          disabled={disabled}
          rows={1}
          className="flex-1 resize-none rounded-lg border border-slate-700/60 bg-[#111827] px-4 py-2 font-mono text-[13px] text-blue-200 caret-accent outline-none focus:border-accent/50 focus:shadow-[0_0_8px_rgba(0,255,180,0.1)] placeholder:text-slate-600 transition disabled:opacity-40"
          style={{ minHeight: '36px', maxHeight: '120px' }}
        />
        <button
          onClick={handleSend}
          disabled={!input.trim() || disabled}
          className="rounded-lg bg-accent/80 px-3 py-2 text-dark-bg font-semibold transition hover:bg-accent hover:shadow-[0_0_12px_rgba(0,255,180,0.2)] disabled:opacity-30 disabled:hover:shadow-none"
        >
          {sending ? <FiLoader size={14} className="animate-spin" /> : <FiSend size={14} />}
        </button>
      </div>
    </div>
  );
}
