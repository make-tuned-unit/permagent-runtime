/**
 * The review's single-key map (U14): Linear's verdict numbering plus
 * Superhuman's J/K/H/Z. Pure, so the whole table is testable without a DOM.
 *
 *   1…9 / 0   the verdict carrying that key (bar order)
 *   Return    the primary, only where the plan marks it reversible
 *   H         Later
 *   J / K     next / previous (arrows too)
 *   Z, ⌘Z     undo the last verdict
 *   /         focus Ask Henry
 *   ?         shortcut sheet
 *
 * Every key is inert while a text field has focus. Buttons are NOT text
 * fields: focus lands on a header button when the sheet opens, and the keys
 * must work from there. Return/Space on a focused button stay the button's.
 */

export type ReviewCommand =
  | { type: 'verdict'; key: string }
  | { type: 'default' }
  | { type: 'later' }
  | { type: 'next' }
  | { type: 'prev' }
  | { type: 'undo' }
  | { type: 'ask' }
  | { type: 'shortcuts' };

export interface KeyLike {
  key: string;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  isComposing?: boolean;
  defaultPrevented?: boolean;
  target?: EventTarget | null;
}

const TEXT_ENTRY =
  'input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"], [role="combobox"], [role="slider"], [role="menu"]';
const ACTIVATES_ON_ENTER = 'button, a[href], summary, [role="button"]';

function closest(target: EventTarget | null | undefined, selector: string): boolean {
  return typeof Element !== 'undefined' && target instanceof Element && !!target.closest(selector);
}

export function commandForKey(e: KeyLike): ReviewCommand | null {
  if (e.defaultPrevented || e.isComposing || e.altKey) return null;
  if (closest(e.target, TEXT_ENTRY)) return null;
  const mod = !!(e.metaKey || e.ctrlKey);
  if (mod) return e.key.toLowerCase() === 'z' && !e.shiftKey ? { type: 'undo' } : null;
  if (e.key === '?') return { type: 'shortcuts' };
  if (e.shiftKey) return null;
  switch (e.key) {
    case 'Enter':
      return closest(e.target, ACTIVATES_ON_ENTER) ? null : { type: 'default' };
    case 'h': case 'H': return { type: 'later' };
    case 'j': case 'ArrowRight': return { type: 'next' };
    case 'k': case 'ArrowLeft': return { type: 'prev' };
    case 'z': case 'Z': return { type: 'undo' };
    case '/': return { type: 'ask' };
    default:
      return /^[0-9]$/.test(e.key) ? { type: 'verdict', key: e.key } : null;
  }
}

/** The shortcut sheet's rows, in reading order. */
export const SHORTCUT_ROWS: ReadonlyArray<{ keys: string[]; label: string }> = [
  { keys: ['1', '2', '3'], label: 'Answer with that button' },
  { keys: ['↩'], label: 'The highlighted answer' },
  { keys: ['H'], label: 'Later' },
  { keys: ['J', 'K'], label: 'Next / previous decision' },
  { keys: ['Z'], label: 'Undo your last answer' },
  { keys: ['/'], label: 'Ask about this decision' },
  { keys: ['?'], label: 'Show these shortcuts' },
  { keys: ['esc'], label: 'Close' },
];
