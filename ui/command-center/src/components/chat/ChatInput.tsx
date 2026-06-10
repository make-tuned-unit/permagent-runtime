import { useState, useRef, useEffect, useCallback, useImperativeHandle, forwardRef } from 'react';
import { FiSend, FiLoader, FiPaperclip } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AttachmentChip } from './AttachmentChip';
import { VoiceButton } from '../voice/VoiceButton';

const MAX_FILE_SIZE = 50 * 1024 * 1024;

export interface ChatInputHandle {
  addFiles: (files: File[]) => void;
}

export const ChatInput = forwardRef<ChatInputHandle>(function ChatInput(_props, ref) {
  const { colors } = useTheme();
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
    const valid = files.filter(f => f.size <= MAX_FILE_SIZE);
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
    <div className="p-3" style={{ borderTop: `1px solid ${colors.border}`, backgroundColor: colors.surface }}>
      {pendingFiles.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mb-2">
          {pendingFiles.map((f, i) => (
            <AttachmentChip key={`${f.name}-${i}`} filename={f.name} onRemove={() => removeFile(i)} />
          ))}
        </div>
      )}
      <div className="flex items-end" style={{ gap: 8 }}>
        <button
          onClick={() => fileInputRef.current?.click()}
          disabled={disabled}
          title="Attach files"
          className="transition disabled:opacity-30"
          style={{
            width: 28, height: 28, borderRadius: 6, flexShrink: 0,
            display: 'grid', placeItems: 'center',
            border: `1px solid ${colors.border}`,
            backgroundColor: colors.inputBg,
            color: colors.textMuted,
          }}
        >
          <FiPaperclip size={12} style={{ display: 'block' }} />
        </button>
        <VoiceButton />
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
          className="flex-1 resize-none rounded-lg px-4 py-2 text-[14px] outline-none transition disabled:opacity-40"
          style={{
            fontFamily: font.body,
            color: colors.text,
            backgroundColor: colors.inputBg,
            border: `1px solid ${colors.border}`,
            caretColor: colors.cyan,
            minHeight: '36px',
            maxHeight: '120px',
            transition: `border-color 150ms ${ease.out}, box-shadow 150ms ${ease.out}`,
          }}
          onFocus={e => {
            e.currentTarget.style.borderColor = colors.borderHi;
            e.currentTarget.style.boxShadow = `0 0 8px ${colors.cyanGlow}`;
          }}
          onBlur={e => {
            e.currentTarget.style.borderColor = colors.border;
            e.currentTarget.style.boxShadow = 'none';
          }}
        />
        <button
          onClick={handleSend}
          disabled={(!input.trim() && pendingFiles.length === 0) || disabled}
          className="transition disabled:opacity-30"
          style={{
            width: 28, height: 28, borderRadius: 6, flexShrink: 0,
            display: 'grid', placeItems: 'center',
            background: colors.ribbonGradient,
            color: colors.textOnAccent,
            fontWeight: 600,
          }}
        >
          {isStreaming ? <FiLoader size={12} className="animate-spin" /> : <FiSend size={12} />}
        </button>
      </div>
    </div>
  );
});
