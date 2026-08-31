import { useState, useRef, useEffect, useCallback, useImperativeHandle, forwardRef } from 'react';
import { FiSend, FiLoader, FiPaperclip } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { takeWizardIntent } from '../../lib/wizardIntent';
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
  const stopStreaming = useCommandCenter(s => s.stopStreaming);

  const [input, setInput] = useState('');
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  // Local "cancel requested" state: set on Stop click, cleared when the turn
  // actually settles (isStreaming → false via the Finish event). Kept out of the
  // global store — it's pure per-input affordance state.
  const [stopping, setStopping] = useState(false);
  // A cancel POST that itself failed used to be a console line and a silently
  // re-armed button — indistinguishable from never having pressed Stop, on the
  // highest-traffic control in the app.
  const [stopFailed, setStopFailed] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const disabled = isStreaming;

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = '36px';
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [input]);

  // First-conversation hand-off: the wizard's intent step pre-fills the
  // composer (one-shot) so the user can edit or send it as-is — this is the
  // "prepare context for your first conversation" the wizard promises.
  useEffect(() => {
    const seed = takeWizardIntent();
    if (seed) setInput(prev => (prev ? prev : seed));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Once the turn settles (Finish/Error flips isStreaming off), drop the
  // transient "stopping" state so the composer returns to its idle Send button.
  useEffect(() => {
    if (!isStreaming) {
      setStopping(false);
      setStopFailed(false);
    }
  }, [isStreaming]);

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

  const handleStop = async () => {
    if (stopping) return;
    setStopping(true);
    setStopFailed(false);
    try {
      // Server emits a terminal Finish on cancel, which settles the UI and frees
      // the request slot — so we wait for it rather than optimistically resetting.
      // If nothing was cancelled (the request_id hasn't landed yet), re-enable
      // Stop so the click isn't swallowed and the turn stays cancellable.
      const issued = await stopStreaming();
      if (!issued) setStopping(false);
    } catch (err) {
      // Cancel POST failed (e.g. turn already ended, or network): the agent may
      // still be alive, so re-enable Stop instead of pretending it stopped —
      // and say so, because a re-armed button is not feedback.
      console.error('[chat] stop failed:', err);
      setStopping(false);
      setStopFailed(true);
    }
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
      {stopFailed && (
        <div
          role="alert"
          className="mb-2 text-[11px]"
          style={{ fontFamily: font.body, color: colors.danger }}
        >
          Couldn't stop the reply — the agent may still be running. Try again.
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
        {isStreaming ? (
          // Every-turn escape hatch: while a turn streams, the Send button
          // becomes a Stop button that cancels the in-flight request. Danger-
          // outlined so it reads as an interrupt and stays legible on every
          // theme (dark's coral + silver's red both contrast on the inset).
          <button
            onClick={handleStop}
            disabled={stopping}
            title={stopFailed ? "Couldn't stop the reply — try again" : 'Stop generating'}
            aria-label="Stop generating"
            className="transition disabled:opacity-50"
            style={{
              width: 28, height: 28, borderRadius: 6, flexShrink: 0,
              display: 'grid', placeItems: 'center',
              // The failed attempt stays visible on the control itself until
              // the next press, not just in the line above it.
              border: `1px solid ${colors.danger}`,
              backgroundColor: stopFailed ? `${colors.danger}26` : colors.inputBg,
              color: colors.danger,
            }}
          >
            {stopping
              ? <FiLoader size={12} className="animate-spin" style={{ display: 'block' }} />
              : <span style={{ width: 9, height: 9, borderRadius: 2, backgroundColor: colors.danger, display: 'block' }} />}
          </button>
        ) : (
          <button
            onClick={handleSend}
            disabled={!input.trim() && pendingFiles.length === 0}
            title="Send message"
            aria-label="Send message"
            className="transition disabled:opacity-30"
            style={{
              width: 28, height: 28, borderRadius: 6, flexShrink: 0,
              display: 'grid', placeItems: 'center',
              background: colors.ribbonGradient,
              color: colors.textOnAccent,
              fontWeight: 600,
            }}
          >
            <FiSend size={12} />
          </button>
        )}
      </div>
    </div>
  );
});
