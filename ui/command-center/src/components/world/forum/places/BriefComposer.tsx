/**
 * World W2 — the brief composer and its explicit send confirmation.
 *
 * Preparing is local: every keystroke is saved to the W1 draft store for this
 * place + project + agent, so leaving the World (unmount) and coming back
 * restores the draft WITHOUT sending it. Sending is a separate, explicit
 * step: "Review and send…" opens a confirmation layer that shows exactly what
 * will be sent; only that layer's "Send to orchestrator" button calls the
 * host's `onRequestSend`. Selecting, typing, restoring and mounting never send.
 */
import { useCallback, useEffect, useId, useRef, useState, type RefObject } from 'react';
import { Button } from '../../../common/Button';
import type { ThemeColors } from '../../../../styles/tokens';
import { composeBriefRequest, DRAFT_BOUNDS, draftKey, type PlaceDraft, type WorldDraftStore } from '../data/drafts';
import type { WorldPlaceId } from '../data/identity';
import { parseRecordLinkKey, recordLinkLabel, type WorldRecordLink } from '../data/recordLinks';
import { ageLabel } from './format';
import type { PlaceEvent, PlaceSendAck, PlaceSendRequest, SendIntent } from './integration';
import { usePanelLayer } from './layers';
import type { PlaceStyles } from './placeStyles';

const INTENT_LABEL: Record<SendIntent, string> = {
  query: 'Ask a question',
  job: 'Request work',
  capability: 'Request a capability',
};

export interface BriefComposerProps {
  open: boolean;
  onClose(): void;
  /** The control that opened the composer; focus returns here on close. */
  triggerRef: RefObject<HTMLElement | null>;
  placeId: WorldPlaceId;
  projectId: string | null;
  agentWorldId: string;
  agentName: string;
  heading: string;
  intents: readonly SendIntent[];
  /** Evidence to attach when the composer opens (record link keys). */
  seedEvidence?: readonly string[];
  drafts: WorldDraftStore;
  onRequestSend(request: PlaceSendRequest): Promise<PlaceSendAck>;
  onOpenLink(link: WorldRecordLink): void;
  onEvent?(event: PlaceEvent): void;
  now: number;
  colors: ThemeColors;
  styles: PlaceStyles;
}

function emptyDraft(placeId: WorldPlaceId, projectId: string | null, agentWorldId: string): PlaceDraft {
  return { placeId, projectId, agentWorldId, brief: '', constraints: [], acceptance: '', evidenceRefs: [], updatedAt: '', sent: null };
}

export function BriefComposer(props: BriefComposerProps) {
  const { open, onClose, triggerRef, placeId, projectId, agentWorldId, agentName, heading, intents, seedEvidence, drafts, onRequestSend, onOpenLink, onEvent, now, colors, styles } = props;
  const key = draftKey(placeId, projectId, agentWorldId);
  const containerRef = useRef<HTMLDivElement>(null);
  const reviewRef = useRef<HTMLButtonElement>(null);
  const briefRef = useRef<HTMLTextAreaElement>(null);
  const confirmRef = useRef<HTMLDivElement>(null);
  const confirmHeadingRef = useRef<HTMLHeadingElement>(null);
  const ids = { brief: useId(), constraints: useId(), acceptance: useId(), confirm: useId() };

  const [draft, setDraft] = useState<PlaceDraft>(() => drafts.restore(placeId, projectId, agentWorldId) ?? emptyDraft(placeId, projectId, agentWorldId));
  const [constraintText, setConstraintText] = useState(() => draft.constraints.join('\n'));
  const [intent, setIntent] = useState<SendIntent>(intents[0] ?? 'query');
  const [confirming, setConfirming] = useState(false);
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [ack, setAck] = useState<PlaceSendAck | null>(null);
  const [persistence, setPersistence] = useState<'saved' | 'not_saved'>(() => drafts.persistence?.() ?? 'saved');

  // A different context (project / agent) is a different draft: restore it.
  const loadedKey = useRef(key);
  useEffect(() => {
    if (loadedKey.current === key) return;
    loadedKey.current = key;
    const restored = drafts.restore(placeId, projectId, agentWorldId) ?? emptyDraft(placeId, projectId, agentWorldId);
    setDraft(restored);
    setConstraintText(restored.constraints.join('\n'));
    setAck(null);
    setConfirming(false);
  }, [key, drafts, placeId, projectId, agentWorldId]);

  const edit = useCallback((change: Parameters<WorldDraftStore['update']>[3]) => {
    const next = drafts.update(placeId, projectId, agentWorldId, change);
    setDraft(next);
    setPersistence(drafts.persistence?.() ?? 'saved');
    setAck(null);
    onEvent?.({ type: 'draft_changed', placeId, draftKey: key });
  }, [drafts, placeId, projectId, agentWorldId, onEvent, key]);

  // Attach seeded evidence once per opening (a local draft edit, not a send).
  const seeded = useRef<string>('');
  useEffect(() => {
    if (!open || !seedEvidence?.length) return;
    const sig = seedEvidence.join('|');
    if (seeded.current === sig) return;
    seeded.current = sig;
    const merged = Array.from(new Set([...draft.evidenceRefs, ...seedEvidence])).slice(0, DRAFT_BOUNDS.evidenceRefs);
    if (merged.length !== draft.evidenceRefs.length) edit({ evidenceRefs: merged });
  }, [open, seedEvidence, draft.evidenceRefs, edit]);

  // Opening the composer puts the caret in the outcome field (keyboard-first).
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open && !wasOpen.current) briefRef.current?.focus({ preventScroll: true });
    wasOpen.current = open;
  }, [open]);

  usePanelLayer(open, onClose, triggerRef, containerRef);
  usePanelLayer(open && confirming, () => setConfirming(false), reviewRef, confirmRef);

  useEffect(() => {
    if (confirming) confirmHeadingRef.current?.focus({ preventScroll: true });
  }, [confirming]);

  if (!open) return null;

  const composed = composeBriefRequest(draft);
  const hasText = draft.brief.trim().length > 0;
  const stage = draft.sent ? `Sent ${ageLabel(draft.sent.at, now)} — ${draft.sent.acknowledgement}` : hasText ? `Draft — not sent. Saved on this device${draft.updatedAt ? ` ${ageLabel(draft.updatedAt, now)}` : ''}.` : 'Empty draft.';

  const send = async () => {
    if (sending || !hasText) return;
    setSending(true);
    setSendError(null);
    onEvent?.({ type: 'send_requested', placeId, draftKey: key });
    try {
      const result = await onRequestSend({ placeId, projectId, agentWorldId, agentName, intent, question: composed.question, criteria: composed.criteria, draftKey: key });
      const marked = drafts.markSent(placeId, projectId, agentWorldId, result.acknowledgement);
      if (marked) setDraft(marked);
      setAck(result);
      setConfirming(false);
      onEvent?.({ type: 'send_acknowledged', placeId, draftKey: key, ack: result });
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      setSendError(reason);
      onEvent?.({ type: 'send_failed', placeId, draftKey: key, reason });
    } finally {
      setSending(false);
    }
  };

  return (
    <div ref={containerRef} style={styles.layer} data-layer="composer" aria-label={heading} role="group">
      <h3 style={{ ...styles.strong, margin: 0 }}>{heading}</h3>
      <p style={styles.note} data-draft-stage={draft.sent ? 'sent' : hasText ? 'draft' : 'empty'}>{stage}</p>
      {persistence === 'not_saved' && (
        <p style={styles.note} role="status" data-testid="draft-not-saved">Not saved to disk: this draft is only kept while the World is open.</p>
      )}
      <div role="group" aria-label="What kind of request" style={styles.actions}>
        {intents.map((i) => (
          <Button key={i} colors={colors} variant={intent === i ? 'ghostOn' : 'ghost'} flashSuccess={false} aria-pressed={intent === i} onClick={() => setIntent(i)}>{INTENT_LABEL[i]}</Button>
        ))}
      </div>
      <div style={styles.field}>
        <label htmlFor={ids.brief} style={styles.label}>Outcome and context — what you want, and what the orchestrator should know</label>
        <textarea id={ids.brief} ref={briefRef} rows={3} maxLength={DRAFT_BOUNDS.briefChars} style={styles.input} value={draft.brief} onChange={(e) => edit({ brief: e.target.value })} />
      </div>
      <div style={styles.field}>
        <label htmlFor={ids.constraints} style={styles.label}>Constraints — one per line (what must not change, limits, deadlines)</label>
        <textarea
          id={ids.constraints}
          rows={2}
          style={styles.input}
          value={constraintText}
          onChange={(e) => {
            setConstraintText(e.target.value);
            edit({ constraints: e.target.value.split('\n').map((c) => c.trim()).filter(Boolean) });
          }}
        />
      </div>
      <div style={styles.field}>
        <label htmlFor={ids.acceptance} style={styles.label}>Acceptance — what would count as done</label>
        <textarea id={ids.acceptance} rows={2} maxLength={DRAFT_BOUNDS.acceptanceChars} style={styles.input} value={draft.acceptance} onChange={(e) => edit({ acceptance: e.target.value })} />
      </div>
      {draft.evidenceRefs.length > 0 && (
        <div style={styles.field}>
          <span style={styles.label}>Evidence attached ({draft.evidenceRefs.length})</span>
          <ul style={styles.list}>
            {draft.evidenceRefs.map((ref) => {
              const link = parseRecordLinkKey(ref);
              return (
                <li key={ref} style={styles.actions}>
                  <span style={styles.note}>{link ? recordLinkLabel(link).replace(/^Open (in )?/, '') : 'Record'} · <code>{ref}</code></span>
                  <Button colors={colors} variant="bare" flashSuccess={false} aria-label={`Remove evidence ${ref}`} onClick={() => edit({ evidenceRefs: draft.evidenceRefs.filter((r) => r !== ref) })}>Remove</Button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      <div style={styles.actions}>
        <Button ref={reviewRef} colors={colors} variant="ghostOn" flashSuccess={false} data-action="review_send" aria-expanded={confirming} aria-controls={ids.confirm} disabled={!hasText} onClick={() => { setSendError(null); setConfirming(true); }}>Review and send…</Button>
        <Button colors={colors} variant="ghost" flashSuccess={false} onClick={onClose}>Keep as draft</Button>
        {!hasText && <span style={styles.note}>Write the outcome first. Nothing is sent until you confirm.</span>}
      </div>
      {confirming && (
        <div id={ids.confirm} ref={confirmRef} style={styles.layer} data-layer="confirm-send" role="group" aria-labelledby={`${ids.confirm}-h`}>
          <h4 id={`${ids.confirm}-h`} ref={confirmHeadingRef} tabIndex={-1} style={{ ...styles.strong, margin: 0 }}>Send this to the orchestrator?</h4>
          <p style={styles.note}>
            {INTENT_LABEL[intent]} in the shared conversation. {agentName} is named as context; the orchestrator decides who does the work.
            {!composed.criteria && ' No acceptance criterion yet — the orchestrator will have to guess what done means.'}
          </p>
          <pre style={styles.preview} data-testid="send-preview">{composed.question}{composed.criteria ? `\n\nUseful answer criteria: ${composed.criteria}` : ''}</pre>
          {sendError && <p style={styles.warn} role="alert">Not sent: {sendError} Your draft is still here.</p>}
          <div style={styles.actions}>
            <Button colors={colors} variant="primary" flashSuccess={false} minPendingMs={0} pending={sending} data-action="confirm_send" onClick={() => { void send(); }}>Send to orchestrator</Button>
            <Button colors={colors} variant="ghost" flashSuccess={false} disabled={sending} onClick={() => setConfirming(false)}>Not yet</Button>
          </div>
        </div>
      )}
      {ack && (
        <div role="status" style={styles.slot} data-send-ack="true">
          <p style={styles.strong}>Sent. {ack.acknowledgement}</p>
          <div style={styles.actions}>
            {ack.conversation && <Button colors={colors} flashSuccess={false} onClick={() => onOpenLink({ to: 'conversation', sessionId: ack.conversation!.sessionId })}>Open the conversation</Button>}
            {ack.goal
              ? <Button colors={colors} flashSuccess={false} onClick={() => onOpenLink({ to: 'goal', projectId: ack.goal!.projectId, goalId: ack.goal!.goalId })}>Open the tracked goal</Button>
              : <span style={styles.note}>No tracked goal is linked yet. A reply without a tracked job stays a reply; if a goal is created it appears in goals in flight.</span>}
          </div>
        </div>
      )}
    </div>
  );
}
