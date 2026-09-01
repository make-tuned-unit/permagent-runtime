import type { VoiceState } from '../../hooks/useVoice';

/** Compact hands-free status without implying bare speech can open a gated turn. */
export function handsFreeStatusLabel(
  state: VoiceState,
  gatedWakePhrase?: string | null,
): string {
  if (state === 'recording') return '● listening';
  if (state === 'processing') return 'thinking…';
  if (state === 'ready' && gatedWakePhrase) return `say “${gatedWakePhrase}”`;
  return '◉ hands-free';
}
