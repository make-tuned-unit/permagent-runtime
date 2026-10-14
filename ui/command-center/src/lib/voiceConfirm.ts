/**
 * Pending spoken side-effects waiting for an on-screen tap (lane L2 contract).
 *
 * The daemon emits, on /events:
 *   voice_confirm_requested {confirm_id, session_id, tool, summary, args_preview, expires_at}
 *   voice_confirm_resolved  {session_id, confirm_id, outcome: approved|denied|expired}
 * The server never emits `expired` itself in practice, so the card lapses
 * client-side at `expires_at`.
 */

export interface VoiceConfirm {
  confirmId: string;
  sessionId: string;
  tool: string;
  summary: string;
  argsPreview: string;
  /** Epoch ms. */
  expiresAtMs: number;
}

let pending: VoiceConfirm[] = [];
const listeners = new Set<() => void>();
const emit = () => listeners.forEach(l => l());

function toMs(v: unknown): number {
  if (typeof v === 'number' && Number.isFinite(v)) return v < 1e12 ? v * 1000 : v;
  if (typeof v === 'string') {
    const t = Date.parse(v);
    if (Number.isFinite(t)) return t;
    const n = Number(v);
    if (Number.isFinite(n)) return n < 1e12 ? n * 1000 : n;
  }
  return NaN;
}

function previewText(v: unknown): string {
  if (typeof v === 'string') return v;
  if (v === null || v === undefined) return '';
  try { return JSON.stringify(v); } catch { return ''; }
}

/** Feed one /events frame. Returns true when it changed the pending list. */
export function applyVoiceConfirmEvent(type: string, payload: Record<string, unknown>, nowMs: number = Date.now()): boolean {
  if (type === 'voice_confirm_requested') {
    const confirmId = typeof payload.confirm_id === 'string' ? payload.confirm_id : '';
    const sessionId = typeof payload.session_id === 'string' ? payload.session_id : '';
    const expiresAtMs = toMs(payload.expires_at);
    if (!confirmId || !sessionId || !Number.isFinite(expiresAtMs) || expiresAtMs <= nowMs) return false;
    const card: VoiceConfirm = {
      confirmId,
      sessionId,
      tool: typeof payload.tool === 'string' ? payload.tool : '',
      summary: typeof payload.summary === 'string' ? payload.summary : 'Confirm this action',
      argsPreview: previewText(payload.args_preview),
      expiresAtMs,
    };
    pending = [...pending.filter(p => p.confirmId !== confirmId), card];
    emit();
    return true;
  }
  if (type === 'voice_confirm_resolved') {
    const id = typeof payload.confirm_id === 'string' ? payload.confirm_id : '';
    return dismissVoiceConfirm(id);
  }
  return false;
}

export function dismissVoiceConfirm(confirmId: string): boolean {
  const next = pending.filter(p => p.confirmId !== confirmId);
  if (next.length === pending.length) return false;
  pending = next;
  emit();
  return true;
}

/** Drop cards past their deadline. */
export function sweepVoiceConfirms(nowMs: number = Date.now()): boolean {
  const next = pending.filter(p => p.expiresAtMs > nowMs);
  if (next.length === pending.length) return false;
  pending = next;
  emit();
  return true;
}

export function getVoiceConfirms(): VoiceConfirm[] {
  return pending;
}

export function subscribeVoiceConfirms(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function resetVoiceConfirms(): void {
  pending = [];
  emit();
}
