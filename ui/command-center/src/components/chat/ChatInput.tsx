import { useState, useRef, useEffect, useCallback, useImperativeHandle, forwardRef } from 'react';
import { FiSend, FiLoader, FiPaperclip } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { AttachmentChip } from './AttachmentChip';

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50 MB

export interface ChatInputHandle {
  addFiles: (files: File[]) => void;
}

export const ChatInput = forwardRef<ChatInputHandle>(function ChatInput(_props, ref) {
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const sendMessage = useCommandCenter(s => s.sendMessage);

  const [input, setInput] = useState('');
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const disabled = isStreaming;

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = '36px';
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [input]);

  const addFiles = useCallback((files: File[]) => {
    const valid = files.filter(f => {
      if (f.size > MAX_FILE_SIZE) {
        console.warn(`File too large: ${f.name} (${f.size} bytes)`);
        return false;
      }
      return true;
    });
    setPendingFiles(prev => [...prev, ...valid]);
  }, []);

  useImperativeHandle(ref, () => ({ addFiles }), [addFiles]);

  const removeFile = useCallback((index: number) => {
    setPendingFiles(prev => prev.filter((_, i) => i !== index));
  }, []);

  const handleSend = async () => {
    const msg = input.trim();
    if ((!msg && pendingFiles.length === 0) || disabled) return;

    const files = [...pendingFiles];
    setInput('');
    setPendingFiles([]);
    await sendMessage(msg || '(file upload)', files.length > 0 ? files : undefined);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const items = e.clipboardData.items;
    const files: File[] = [];
    for (let i = 0; i < items.length; i++) {
      if (items[i].kind === 'file') {
        const file = items[i].getAsFile();
        if (file) files.push(file);
      }
    }
    if (files.length > 0) {
      e.preventDefault();
      addFiles(files);
    }
  }, [addFiles]);

  return (
    <div className="border-t border-slate-800 bg-[#0C1019] p-3">
      {pendingFiles.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mb-2">
          {pendingFiles.map((f, i) => (
            <AttachmentChip key={`${f.name}-${i}`} filename={f.name} onRemove={() => removeFile(i)} />
          ))}
        </div>
      )}
      <div className="flex items-end gap-2">
        <button
          onClick={() => fileInputRef.current?.click()}
          disabled={disabled}
          className="rounded-lg border border-slate-700/60 bg-[#111827] px-2 py-2 text-dark-muted hover:text-dark-text transition disabled:opacity-30"
          title="Attach files"
        >
          <FiPaperclip size={14} />
        </button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={e => {
            if (e.target.files) {
              addFiles(Array.from(e.target.files));
              e.target.value = '';
            }
          }}
        />
        <textarea
          ref={textareaRef}
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          placeholder={disabled ? 'Agent is responding...' : 'Message your agent...'}
          disabled={disabled}
          rows={1}
          className="flex-1 resize-none rounded-lg border border-slate-700/60 bg-[#111827] px-4 py-2 font-mono text-[13px] text-blue-200 caret-accent outline-none focus:border-accent/50 focus:shadow-[0_0_8px_rgba(0,255,180,0.1)] placeholder:text-slate-600 transition disabled:opacity-40"
          style={{ minHeight: '36px', maxHeight: '120px' }}
        />
        <button
          onClick={handleSend}
          disabled={(!input.trim() && pendingFiles.length === 0) || disabled}
          className="rounded-lg bg-accent/80 px-3 py-2 text-dark-bg font-semibold transition hover:bg-accent hover:shadow-[0_0_12px_rgba(0,255,180,0.2)] disabled:opacity-30 disabled:hover:shadow-none"
        >
          {isStreaming ? <FiLoader size={14} className="animate-spin" /> : <FiSend size={14} />}
        </button>
      </div>
    </div>
  );
});
