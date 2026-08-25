/**
 * Face helpers for the People graph (and the detail-panel preview).
 *
 * photo_url is an http(s) image the Enricher (or the user) stored. A
 * javascript: or file: value is never used as an img src.
 */

export function personInitials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return `${parts[0][0] ?? ''}${parts[parts.length - 1][0] ?? ''}`.toUpperCase();
}

export function safePhotoUrl(url: string | null | undefined): string | null {
  if (!url) return null;
  const trimmed = url.trim();
  return /^https?:\/\//i.test(trimmed) ? trimmed : null;
}

/**
 * Translucent tint from a solid color, for the active-face ring glow.
 * Handles #rgb and #rrggbb; anything else (already rgba()/rgb(), or not a
 * color at all) passes through unchanged. Same shape as the withAlpha()
 * duplicated in xtermTheme.ts / AutomateView.tsx — kept local here rather
 * than shared, matching how those two already each keep their own copy.
 */
export function withAlpha(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  let h = m[1];
  if (h.length === 3) h = h.split('').map(c => c + c).join('');
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Whether a person's name pill shows over the graph. The ego node ("You")
 * is always labeled; everyone else only while hovered, keyboard-focused, or
 * the currently selected person (their detail modal is open) — being a
 * bridge between projects no longer earns a permanent label on its own.
 */
export function shouldShowLabel({ isYou, hovered, focused, selected }: {
  isYou: boolean;
  hovered: boolean;
  focused: boolean;
  selected: boolean;
}): boolean {
  return isYou || hovered || focused || selected;
}

export interface FaceVisuals {
  opacity: number;
  boxShadow: string;
  transform: string;
  transition: string;
}

/**
 * Visual treatment for a person's face disc. "active" = hovered, keyboard-
 * focused, or selected: it overrides the isQuiet dimming and adds a ring
 * glow + slight scale, echoing the name-pill highlight. Reduced motion keeps
 * the opacity and ring-glow change (color/opacity, not motion) but drops the
 * scale and its transition.
 */
export function faceVisuals({ active, dimmed, accent, reducedMotion }: {
  active: boolean;
  dimmed: boolean;
  accent: string;
  reducedMotion: boolean;
}): FaceVisuals {
  const baseShadow = '0 6px 16px rgba(0,0,0,0.35)';
  return {
    opacity: active ? 1 : dimmed ? 0.42 : 1,
    boxShadow: active ? `0 0 0 3px ${withAlpha(accent, 0.28)}, ${baseShadow}` : baseShadow,
    transform: active && !reducedMotion ? 'scale(1.08)' : 'none',
    transition: reducedMotion
      ? 'box-shadow 140ms ease, opacity 140ms ease'
      : 'transform 140ms ease, box-shadow 140ms ease, opacity 140ms ease',
  };
}
