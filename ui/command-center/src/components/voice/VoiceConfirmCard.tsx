/**
 * The on-screen tap for a spoken side-effect (D4: a voice turn never runs a
 * side-effecting tool without a spoken ask plus an on-screen confirm).
 * Mounted once at app level, so it appears wherever voice runs on desktop —
 * over the World beside the avatar, or over the chat-pane Orb.
 */
import { useEffect, useState, useSyncExternalStore } from 'react';
import { api } from '../../lib/api';
import {
  applyVoiceConfirmEvent,
  dismissVoiceConfirm,
  getVoiceConfirms,
  subscribeVoiceConfirms,
  sweepVoiceConfirms,
  type VoiceConfirm,
} from '../../lib/voiceConfirm';
import { subscribeWorldEvents } from '../world/shared/worldEvents';
import { font, radius, space, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export function secondsLeft(expiresAtMs: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAtMs - nowMs) / 1000));
}

function ConfirmRow({ card, now }: { card: VoiceConfirm; now: number }) {
  const { colors } = useTheme();
  const [failure, setFailure] = useState<string | null>(null);
  const resolve = async (decision: 'approve' | 'deny') => {
    setFailure(null);
    try {
      await api.resolveVoiceConfirm(card.confirmId, card.sessionId, decision);
      dismissVoiceConfirm(card.confirmId);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // 404 = expired or already resolved elsewhere: nothing left to tap.
      if (/404|not found|expired/i.test(msg)) dismissVoiceConfirm(card.confirmId);
      else { setFailure(msg || "That didn't go through"); return false; }
    }
    return undefined;
  };
  return (
    <div
      role="alertdialog"
      aria-label="Confirm spoken request"
      data-testid="voice-confirm-card"
      style={{
        pointerEvents: 'auto',
        maxWidth: 360,
        padding: space.xxl,
        borderRadius: radius.lg,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        color: colors.text,
        fontFamily: font.body,
        display: 'flex',
        flexDirection: 'column',
        gap: space.md,
      }}
    >
      <div style={{ ...type.label, color: colors.textMuted }}>Henry is waiting for your tap</div>
      <div style={{ ...type.body }}>{card.summary}</div>
      {card.argsPreview && (
        <div style={{ ...type.caption, color: colors.textMuted, fontFamily: font.mono, wordBreak: 'break-word' }}>
          {card.argsPreview.slice(0, 240)}
        </div>
      )}
      {failure && <div role="alert" style={{ ...type.caption, color: colors.danger }}>{failure}</div>}
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
        <Button colors={colors} variant="primary" onClick={() => resolve('approve')}>Confirm</Button>
        <Button colors={colors} variant="ghost" onClick={() => resolve('deny')}>Cancel</Button>
        <span style={{ ...type.caption, color: colors.textMuted, marginLeft: 'auto' }} aria-live="off">
          {secondsLeft(card.expiresAtMs, now)}s
        </span>
      </div>
    </div>
  );
}

export function VoiceConfirmCard() {
  const cards = useSyncExternalStore(subscribeVoiceConfirms, getVoiceConfirms, getVoiceConfirms);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => subscribeWorldEvents(evt => {
    if (evt.replayed) return;
    applyVoiceConfirmEvent(evt.type, evt.payload);
  }), []);

  useEffect(() => {
    if (cards.length === 0) return;
    const id = setInterval(() => {
      const t = Date.now();
      setNow(t);
      sweepVoiceConfirms(t);
    }, 500);
    return () => clearInterval(id);
  }, [cards.length]);

  if (cards.length === 0) return null;
  return (
    <div
      style={{
        position: 'fixed',
        left: '50%',
        transform: 'translateX(-50%)',
        bottom: space.huge * 3,
        zIndex: 9000,
        display: 'flex',
        flexDirection: 'column',
        gap: space.md,
        pointerEvents: 'none',
      }}
    >
      {cards.map(c => <ConfirmRow key={c.confirmId} card={c} now={now} />)}
    </div>
  );
}
