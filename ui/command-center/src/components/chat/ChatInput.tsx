import { useState, useRef, useEffect } from 'react';
import { FiSend, FiLoader } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';

export function ChatInput() {
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const sendMessage = useCommandCenter(s => s.sendMessage);

  const [input, setInput] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const disabled = isStreaming;

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = '36px';
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [input]);

  const handleSend = async () => {
    const msg = input.trim();
    if (!msg || disabled) return;

    setInput('');
    await sendMessage(msg);
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
          {isStreaming ? <FiLoader size={14} className="animate-spin" /> : <FiSend size={14} />}
        </button>
      </div>
    </div>
  );
}
