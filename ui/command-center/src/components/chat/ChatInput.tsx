import { useState, useRef, useEffect, useCallback, useImperativeHandle, forwardRef, type CSSProperties } from 'react';
import { FiSend, FiLoader, FiPaperclip } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { CHAT_SETUP_INTENT_EVENT, CHAT_SETUP_INTENT_KEY, adoptChatSetupIntentRecord, discardChatSetupIntent, peekChatSetupIntentRecord, peekWizardIntent, takeChatSetupIntent, takeWizardIntent, type ChatSetupIntentRecord } from '../../lib/wizardIntent';
import { ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { AttachmentChip } from './AttachmentChip';
import { Button } from '../common/Button';
import { VoiceButton } from '../voice/VoiceButton';
import { useOrchestratorName } from '../world/shared/useOrchestratorName';
import { openChatModelSetup } from './ChatSetupCard';
import type { ModelSetupReadiness } from './chatSetupState';
import { CHAT_DRAFT_SYNC, encodeDraftFiles } from '../../lib/chatDraftHandoff';

import { Tooltip } from '../common/Tooltip';
const MAX_FILE_SIZE = 50 * 1024 * 1024;

/** The composer's controls are square and share the text field's height, so the
 *  row reads as one control rather than three sizes stacked beside a field. */
const COMPOSER_CONTROL = 36;

export interface ChatInputHandle {
  addFiles: (files: File[]) => void;
  restoreDraft: (text: string, files: File[], pendingSetup?: ChatSetupIntentRecord | null) => void;
  freezeDraft: () => { text: string; files: File[]; source: string; revision: number } | null;
  releaseDraft: () => void;
}

interface ChatInputProps {
  modelReadiness?: ModelSetupReadiness;
}

export const ChatInput = forwardRef<ChatInputHandle, ChatInputProps>(function ChatInput({ modelReadiness }, ref) {
  const { colors } = useTheme();
  const isStreaming = useCommandCenter(s => s.isStreaming);
  const chatWindowOpen = useCommandCenter(s => s.chatWindowOpen);
  const ownsSetupIntent = new URLSearchParams(window.location.search).get('view') === 'chat' || !chatWindowOpen;
  // The configured name, then the store's — which defaults to the literal
  // "Agent", so "Message Agent" is exactly what must not reach the screen.
  const configuredName = useOrchestratorName();
  const storeName = useCommandCenter(s => s.agentName);
  const agentName = configuredName?.trim() || (storeName && storeName !== 'Agent' ? storeName : 'your agent');
  const sendMessage = useCommandCenter(s => s.sendMessage);
  const stopStreaming = useCommandCenter(s => s.stopStreaming);

  const detached = new URLSearchParams(window.location.search).get('view') === 'chat';
  const waitsForHandoff = detached && new URLSearchParams(window.location.search).has('draft_handoff');
  const [handoffReady, setHandoffReady] = useState(() => !waitsForHandoff);
  const [input, setInput] = useState(() => waitsForHandoff ? '' : (useCommandCenter.getState().chatDraft?.text || peekWizardIntent() || ''));
  const [pendingSetup, setPendingSetup] = useState(() => peekChatSetupIntentRecord());
  const [pendingFiles, setPendingFiles] = useState<File[]>(() => waitsForHandoff ? [] : (useCommandCenter.getState().chatDraft?.files ?? []));
  // Local "cancel requested" state: set on Stop click, cleared when the turn
  // actually settles (isStreaming → false via the Finish event). Kept out of the
  // global store — it's pure per-input affordance state.
  const [stopping, setStopping] = useState(false);
  // A cancel POST that itself failed used to be a console line and a silently
  // re-armed button — indistinguishable from never having pressed Stop, on the
  // highest-traffic control in the app.
  const [stopFailed, setStopFailed] = useState(false);
  const [needsModelNotice, setNeedsModelNotice] = useState(false);
  const [closeFrozen, setCloseFrozen] = useState(false);
  const closeFrozenRef = useRef(false);
  const fileSyncRevision = useRef(0);
  const draftRevision = useRef(0);
  const draftSyncSource = useRef(globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`);
  const latestDraft = useRef({ text: input, files: pendingFiles });
  latestDraft.current = { text: input, files: pendingFiles };
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const disabled = isStreaming || !handoffReady || closeFrozen || (!detached && chatWindowOpen);
  /** One line, drawn over the field — see the placeholder span below. */
  const placeholder = !handoffReady ? 'Restoring your draft…' : isStreaming ? `${agentName} is replying…` : `Message ${agentName}`;

  useEffect(() => {
    const seed = peekWizardIntent();
    if (seed && textareaRef.current?.value === seed) takeWizardIntent();
  }, []);

  useEffect(() => {
    useCommandCenter.getState().setChatDraft?.({ text: input, files: pendingFiles });
  }, [input, pendingFiles]);

  // Native webviews have separate stores. Sync edits back to the main window
  // after the acknowledged handoff, so closing the pop-out restores its draft.
  useEffect(() => {
    if (!detached || !handoffReady || !('__TAURI_INTERNALS__' in window)) return;
    const revision = draftRevision.current;
    void import('@tauri-apps/api/event').then(({ emitTo }) => emitTo('main', CHAT_DRAFT_SYNC, { text: input, source: draftSyncSource.current, revision })).catch(() => {});
  }, [detached, handoffReady, input]);
  useEffect(() => {
    if (!detached || !handoffReady || !('__TAURI_INTERNALS__' in window)) return;
    let active = true;
    const revision = ++fileSyncRevision.current;
    const draftVersion = draftRevision.current;
    void encodeDraftFiles(pendingFiles)
      .then(async files => {
        if (!active) return;
        const { emitTo } = await import('@tauri-apps/api/event');
        if (active) await emitTo('main', CHAT_DRAFT_SYNC, { files, source: draftSyncSource.current, revision: draftVersion, fileRevision: revision });
      })
      .catch(() => {});
    return () => { active = false; };
  }, [detached, handoffReady, pendingFiles]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = '36px';
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [input]);

  // ChatDock and ChatApp apply native payloads to their own storage. This
  // composer observes only local changes and never assumes WK storage shares.
  useEffect(() => {
    const refresh = () => setPendingSetup(peekChatSetupIntentRecord());
    const onStorage = (event: StorageEvent) => {
      if (event.key === CHAT_SETUP_INTENT_KEY) refresh();
    };
    window.addEventListener(CHAT_SETUP_INTENT_EVENT, refresh);
    window.addEventListener('storage', onStorage);
    refresh();
    return () => {
      window.removeEventListener(CHAT_SETUP_INTENT_EVENT, refresh);
      window.removeEventListener('storage', onStorage);
    };
  }, []);

  // Never replace a draft already in the composer. Once it is sent or cleared,
  // promote the setup request into the field for review and editing.
  useEffect(() => {
    if (!pendingSetup || !ownsSetupIntent || !handoffReady || closeFrozenRef.current || closeFrozen) return;
    if (input.trim() === pendingSetup.text.trim()) {
      takeChatSetupIntent(pendingSetup.id);
      setPendingSetup(peekChatSetupIntentRecord());
    } else if (!input && pendingFiles.length === 0) {
      const intent = takeChatSetupIntent(pendingSetup.id);
      if (intent) setInput(intent);
      setPendingSetup(peekChatSetupIntentRecord());
    }
  }, [input, pendingFiles.length, pendingSetup, ownsSetupIntent, handoffReady, closeFrozen]);

  useEffect(() => {
    if (modelReadiness !== 'needs-model') setNeedsModelNotice(false);
  }, [modelReadiness]);

  // Once the turn settles (Finish/Error flips isStreaming off), drop the
  // transient "stopping" state so the composer returns to its idle Send button.
  useEffect(() => {
    if (!isStreaming) {
      setStopping(false);
      setStopFailed(false);
    }
  }, [isStreaming]);

  const addFiles = useCallback((files: File[]) => {
    if (closeFrozenRef.current || (!detached && useCommandCenter.getState().chatWindowOpen)) return;
    const valid = files.filter(f => f.size <= MAX_FILE_SIZE);
    const next = [...latestDraft.current.files, ...valid];
    draftRevision.current += 1;
    latestDraft.current = { ...latestDraft.current, files: next };
    setPendingFiles(next);
  }, [detached]);

  useImperativeHandle(ref, () => ({
    addFiles,
    restoreDraft: (text, files, setup) => {
      draftRevision.current += 1;
      latestDraft.current = { text, files: [...files, ...latestDraft.current.files] };
      setInput(text);
      setPendingFiles(latestDraft.current.files);
      adoptChatSetupIntentRecord(setup ?? null);
      setHandoffReady(true);
    },
    freezeDraft: () => {
      if (!handoffReady || closeFrozenRef.current) return null;
      closeFrozenRef.current = true;
      setCloseFrozen(true);
      return { ...latestDraft.current, files: [...latestDraft.current.files], source: draftSyncSource.current, revision: draftRevision.current };
    },
    releaseDraft: () => { closeFrozenRef.current = false; setCloseFrozen(false); },
  }), [addFiles, handoffReady]);

  const removeFile = useCallback((index: number) => {
    if (closeFrozenRef.current || (!detached && useCommandCenter.getState().chatWindowOpen)) return;
    const next = latestDraft.current.files.filter((_, i) => i !== index);
    draftRevision.current += 1;
    latestDraft.current = { ...latestDraft.current, files: next };
    setPendingFiles(next);
  }, [detached]);


  /** Something to send: typed text, or files waiting to go with it. */
  const canSend = Boolean(input.trim()) || pendingFiles.length > 0;

  const handleSend = async () => {
    const msg = input.trim();
    if ((!msg && pendingFiles.length === 0) || disabled || closeFrozenRef.current) return;
    if (modelReadiness === 'needs-model') {
      setNeedsModelNotice(true);
      return;
    }

    const files = [...pendingFiles];
    draftRevision.current += 1;
    latestDraft.current = { text: '', files: [] };
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
      {!handoffReady && <div role="status" style={{ marginBottom: space.sm, color: colors.textMuted, fontFamily: font.body, fontSize: textSize.caption }}>Restoring your unsent draft…</div>}
      {pendingSetup && (input || pendingFiles.length > 0) && input.trim() !== pendingSetup.text.trim() && (
        <details style={{ marginBottom: space.md, color: colors.textMuted, fontFamily: font.body, fontSize: textSize.caption }}>
          <summary style={{ cursor: 'pointer' }}>Tailscale setup request is ready. Your current draft is unchanged.</summary>
          <p style={{ margin: `${space.sm}px 0 0`, whiteSpace: 'pre-wrap' }}>{pendingSetup.text}</p>
          <p style={{ margin: `${space.sm}px 0 0` }}>Send or clear your current draft to edit this request in the composer.</p>
          <Button colors={colors} variant="bare" type="button" disabled={disabled} onClick={() => { if (discardChatSetupIntent(pendingSetup.id)) setPendingSetup(peekChatSetupIntentRecord()); }}>Dismiss setup request</Button>
        </details>
      )}
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
      {needsModelNotice && modelReadiness === 'needs-model' && (
        <div role="alert" style={{ marginBottom: space.md, color: colors.warning, fontFamily: font.body, fontSize: textSize.caption }}>
          No model is configured, so this draft has not been sent. Set up a model, then send it.
          <Button colors={colors} variant="bare" type="button" onClick={openChatModelSetup}>Open model setup</Button>
        </div>
      )}
      <div className="flex items-end" style={{ gap: space.md }}>
        <Tooltip content="Attach files">
          <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
            <Button
              colors={colors}
              onClick={() => fileInputRef.current?.click()}
              disabled={disabled}
              aria-label="Attach files"
              style={{
                '--pa-btn-bg': colors.inputBg,
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-border': colors.border,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-pad': '0',
                '--pa-btn-radius': `${radius.md}px`,
                width: COMPOSER_CONTROL, height: COMPOSER_CONTROL, flexShrink: 0,
              } as CSSProperties}
            >
              <FiPaperclip size={13} style={{ display: 'block' }} />
            </Button>
          </span>
        </Tooltip>
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
        {/* The placeholder is DRAWN, not set on the textarea.
            A textarea's native placeholder wraps, and in the docked composer
            "Message your agent..." wrapped to a second line that then spilled
            out of the field's 36px box — reported twice (QA 2026-09-20). No
            width, no agent name and no translation can wrap this one: it is a
            single line that truncates with an ellipsis. The textarea keeps the
            accessible name, so a screen reader is unaffected. */}
        <div style={{ position: 'relative', flex: 1, minWidth: 0, display: 'flex' }}>
          {!input && (
            <span
              aria-hidden="true"
              style={{
                // The field's own text inset: its px-3 padding plus its 1px border,
                // so the drawn placeholder starts exactly where typing does.
                position: 'absolute', left: `calc(${space.xl}px + 1px)`,
                right: `calc(${space.xl}px + 1px)`, top: '50%',
                transform: 'translateY(-50%)', pointerEvents: 'none',
                whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.2,
                color: colors.textDim,
              }}
            >{placeholder}</span>
          )}
        <textarea
          ref={textareaRef}
          value={input}
          onChange={e => {
            if (closeFrozenRef.current) return;
            draftRevision.current += 1;
            latestDraft.current = { ...latestDraft.current, text: e.target.value };
            setInput(e.target.value);
          }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          aria-label={placeholder}
          disabled={disabled}
          rows={1}
          className="flex-1 resize-none rounded-lg px-3 py-2 text-[14px] outline-none transition disabled:opacity-40"
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
        </div>
        {isStreaming ? (
          // Every-turn escape hatch: while a turn streams, the Send button
          // becomes a Stop button that cancels the in-flight request. Danger-
          // outlined so it reads as an interrupt and stays legible on every
          // theme (dark's coral + silver's red both contrast on the inset).
          <Tooltip content={stopFailed ? "Couldn't stop the reply — try again" : 'Stop generating'}>
            <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
              <Button
                key="stop"
                colors={colors}
                // Stop keeps its OWN in-flight state rather than handing the click's
                // promise to the primitive. `stopping` is not "the click is in
                // flight": it means a cancel was issued and we are waiting for the
                // server's terminal Finish, and a cancel POST that fails clears it
                // again on purpose so the turn stays cancellable. The primitive's
                // pending floor would hold the button disabled past that re-arm,
                // and its spinner would sit on top of the one already rendered
                // below. `flashSuccess` is off for the same reason a tick would be
                // wrong here at all: Stop is confirmed by the reply stopping.
                onClick={() => { void handleStop(); }}
                flashSuccess={false}
                disabled={stopping}
                aria-label="Stop generating"
                style={{
                  // The failed attempt stays visible on the control itself until
                  // the next press, not just in the line above it.
                  '--pa-btn-bg': stopFailed ? `${colors.danger}26` : colors.inputBg,
                  '--pa-btn-fg': colors.danger,
                  '--pa-btn-border': colors.danger,
                  '--pa-btn-bg-hover': `${colors.danger}33`,
                  '--pa-btn-border-hover': colors.danger,
                  '--pa-btn-bg-active': `${colors.danger}40`,
                  '--pa-btn-pad': '0',
                  '--pa-btn-radius': `${radius.md}px`,
                  width: COMPOSER_CONTROL, height: COMPOSER_CONTROL, flexShrink: 0,
                } as CSSProperties}
              >
                {stopping
                  ? <FiLoader size={12} className="animate-spin" style={{ display: 'block' }} />
                  : <span style={{ width: 9, height: 9, borderRadius: 2, backgroundColor: colors.danger, display: 'block' }} />}
              </Button>
            </span>
          </Tooltip>
        ) : (
          <Tooltip content="Send message">
            <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
              <Button
                // Distinct key from Stop: same component type in the same slot, so
                // without one React would carry the Send button's in-flight state
                // straight into the Stop button that replaces it mid-turn.
                key="send"
                colors={colors}
                variant="primary"
                onClick={handleSend}
                // The reply is the confirmation: by the time a tick could land, this
                // button has already become Stop and the answer is streaming in.
                flashSuccess={false}
                disabled={!canSend}
                aria-label="Send message"
                // Flat accent, not the brand ribbon: the gradient belongs to
                // the mark, and on a control it read as a sticker pasted onto
                // the composer — with hover and press set to the SAME gradient
                // it also never answered a press. Cyan is the app's one "do
                // it" fill, it squares up with the field's height and radius,
                // and it goes quiet when there is nothing to send.
                style={{
                  '--pa-btn-bg': canSend ? colors.cyan : colors.inputBg,
                  '--pa-btn-fg': canSend ? colors.textOnCyan : colors.textDim,
                  '--pa-btn-border': canSend ? 'transparent' : colors.border,
                  '--pa-btn-bg-hover': canSend ? colors.cyan : colors.inputBg,
                  '--pa-btn-fg-hover': canSend ? colors.textOnCyan : colors.textDim,
                  '--pa-btn-border-hover': canSend ? 'transparent' : colors.border,
                  '--pa-btn-bg-active': canSend ? colors.cyanGlow : colors.inputBg,
                  '--pa-btn-pad': '0',
                  '--pa-btn-radius': `${radius.md}px`,
                  // `sendMessage` creates/verifies the session before the turn
                  // starts, so the spinner is real time, not decoration — and it
                  // shares the box with the glyph, which the default 5px gap
                  // would overflow.
                  width: COMPOSER_CONTROL, height: COMPOSER_CONTROL, flexShrink: 0, gap: 0,
                  boxShadow: canSend ? `0 0 10px ${colors.cyanSoft}` : 'none',
                  transition: `background 160ms ${ease.out}, box-shadow 160ms ${ease.out}`,
                } as CSSProperties}
              >
                {/* The paper plane's mass sits to its right, so it reads
                    off-centre in a square box until it is nudged back. */}
                <FiSend size={14} style={{ display: 'block', transform: 'translateX(-0.5px)' }} />
              </Button>
            </span>
          </Tooltip>
        )}
      </div>
    </div>
  );
});
