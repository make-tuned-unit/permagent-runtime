import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { FiSend, FiLoader, FiChevronDown } from 'react-icons/fi';
import { useCommandCenter, type ChatMessage } from '../../lib/store';
import { api } from '../../lib/api';

export function ChatView() {
  const chatMessages = useCommandCenter(s => s.chatMessages);
  const addChatMessage = useCommandCenter(s => s.addChatMessage);

  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [showJump, setShowJump] = useState(false);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
    setAutoScroll(atBottom);
    setShowJump(!atBottom);
  }, []);

  const timeline = useMemo(() => {
    return [...chatMessages].sort((a, b) =>
      new Date(a.timestamp || 0).getTime() - new Date(b.timestamp || 0).getTime()
    );
  }, [chatMessages]);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [timeline.length, autoScroll]);

  const jumpToBottom = () => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    setAutoScroll(true);
    setShowJump(false);
  };

  const handleSend = async () => {
    const msg = input.trim();
    if (!msg || sending) return;

    const userMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      role: 'user',
      content: msg,
      timestamp: new Date().toISOString(),
    };
    addChatMessage(userMsg);
    setInput('');
    setSending(true);

    const pendingId = `msg-${Date.now()}-pending`;

    try {
      addChatMessage({
        id: pendingId,
        role: 'system',
        content: 'Processing...',
        timestamp: new Date().toISOString(),
      });

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

      const messages = useCommandCenter.getState().chatMessages.filter(m => m.id !== pendingId);
      useCommandCenter.setState({ chatMessages: messages });

      addChatMessage({
        id: `msg-${Date.now()}-reply`,
        role: 'assistant',
        content: res.reply,
        timestamp: new Date().toISOString(),
      });
    } catch (err) {
      const current = useCommandCenter.getState().chatMessages.filter(m => m.id !== pendingId);
      useCommandCenter.setState({ chatMessages: current });
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
    <div className="flex h-full flex-col bg-[#0A0E17]">
      {/* Header */}
      <div className="border-b border-dark-border px-4 py-2.5">
        <span className="text-[11px] font-mono uppercase tracking-wider text-dark-muted">Chat</span>
      </div>

      {/* Message list */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto p-4 space-y-3"
      >
        {timeline.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-slate-600 text-xs text-center gap-2 font-mono">
            <FiSend size={24} className="opacity-30 text-emerald-500/40" />
            <div>Send a message to start a conversation with your agent.</div>
          </div>
        )}

        {timeline.map((msg) => (
          <div key={msg.id} className={`border-l-2 pl-3 ${
            msg.role === 'user' ? 'border-l-[#22C55E]' : 'border-l-emerald-700/50'
          }`}>
            <div className={`rounded-xl px-3 py-2 ${
              msg.role === 'user'
                ? 'bg-[#22C55E]/10'
                : msg.role === 'assistant'
                ? 'bg-slate-800/60'
                : 'bg-slate-800/30'
            }`}>
              <div className="text-[10px] text-slate-500 mb-0.5 font-mono">
                {msg.role === 'user' ? 'You' : msg.role === 'assistant' ? 'Agent' : 'System'}
                {' \u00b7 '}
                {new Date(msg.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </div>
              <div className={`font-mono text-[13px] leading-relaxed ${
                msg.role === 'user'
                  ? 'text-emerald-300'
                  : msg.role === 'system'
                  ? 'text-slate-500 text-[11px]'
                  : 'text-slate-300'
              }`}>
                {msg.content}
              </div>
            </div>
          </div>
        ))}

        <div ref={bottomRef} />
      </div>

      {/* Jump to latest */}
      {showJump && (
        <div className="flex justify-center -mt-10 mb-2 relative z-10">
          <button
            onClick={jumpToBottom}
            className="flex items-center gap-1 rounded-full bg-[#0F1520] shadow-lg shadow-emerald-500/5 border border-emerald-500/20 px-3 py-1 text-[11px] text-emerald-400 font-mono hover:bg-[#151D2C] transition"
          >
            <FiChevronDown size={12} /> Jump to latest
          </button>
        </div>
      )}

      {/* Chat input */}
      <div className="border-t border-slate-800 bg-[#0C1019] p-3">
        <div className="flex items-end gap-2">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Message your agent..."
            rows={1}
            className="flex-1 resize-none rounded-full border border-slate-700/60 bg-[#111827] px-4 py-2 font-mono text-[13px] text-emerald-300 caret-emerald-400 outline-none focus:border-emerald-500/50 focus:shadow-[0_0_8px_rgba(34,197,94,0.15)] placeholder:text-slate-600 transition"
            style={{ minHeight: '36px', maxHeight: '120px' }}
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || sending}
            className="rounded-lg bg-emerald-600 px-3 py-2 text-white transition hover:bg-emerald-500 hover:shadow-[0_0_12px_rgba(34,197,94,0.3)] disabled:opacity-30 disabled:hover:shadow-none"
          >
            {sending ? <FiLoader size={14} className="animate-spin" /> : <FiSend size={14} />}
          </button>
        </div>
      </div>
    </div>
  );
}
