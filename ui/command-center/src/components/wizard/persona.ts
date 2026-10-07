/**
 * The persona the first-run setup writes: a name and one tone.
 *
 * One step, one decision. The old flow asked for a preset, then re-asked the
 * same traits and tone as free text, then stitched a greeting together from
 * the tone sentence ("Hello! I'm ready to help. Professional and
 * to-the-point."). Here each tone owns its traits, its tone sentence and a
 * greeting line written to be read aloud, so the preview is the real thing.
 */

export interface ToneOption {
  id: string;
  label: string;
  /** One line shown under the chip. */
  hint: string;
  traits: string[];
  tone: string;
  /** Second sentence of the greeting, after "Hi, I'm <name>." */
  line: string;
}

export const TONES: readonly ToneOption[] = [
  {
    id: 'balanced', label: 'Balanced', hint: 'Clear and friendly',
    traits: ['clear', 'helpful'],
    tone: 'Clear and friendly. Gets to the point without being curt.',
    line: 'What would you like to work on?',
  },
  {
    id: 'direct', label: 'Direct', hint: 'Brief, no filler',
    traits: ['direct', 'concise'],
    tone: 'Brief and to the point. No filler.',
    line: "Tell me what you need and I'll get straight to it.",
  },
  {
    id: 'warm', label: 'Warm', hint: 'Patient and encouraging',
    traits: ['warm', 'patient'],
    tone: 'Warm and encouraging. Explains things clearly and never rushes.',
    line: "Good to meet you. What's on your mind?",
  },
  {
    id: 'curious', label: 'Curious', hint: 'Asks and suggests',
    traits: ['curious', 'inventive'],
    tone: 'Curious and inventive. Asks good questions and suggests ideas.',
    line: 'What are we exploring today?',
  },
];

export const DEFAULT_TONE_ID = TONES[0].id;

/**
 * Used only when the identity route cannot be read. It mirrors the backend's
 * documented default persona name (config/agent_identity.rs), which is what
 * GET /api/agent/identity returns on a fresh install — the wizard prefers that
 * live value and never invents its own.
 */
export const FALLBACK_AGENT_NAME = 'Aria';

/** Longest name the identity route accepts (routes/identity.rs). */
export const MAX_AGENT_NAME = 40;

export function toneById(id: string): ToneOption {
  return TONES.find(t => t.id === id) ?? TONES[0];
}

/** The name that will be saved: what was typed, else the default. */
export function effectiveName(typed: string, defaultName: string): string {
  return typed.trim() || defaultName.trim() || FALLBACK_AGENT_NAME;
}

export function greetingFor(name: string, toneId: string): string {
  return `Hi, I'm ${name}. ${toneById(toneId).line}`;
}
