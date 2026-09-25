/**
 * pressable — turns a non-button element with an `onClick` into a real
 * keyboard-operable control, and opts it into the app's global press feedback
 * (`.pa-pressable` in index.css).
 *
 * R8/R4: ~550 `onClick` handlers in this app live on `div`/`li`/`tr` — a list
 * row that opens on click, a card that navigates, a chip-like tag. None of
 * them get a press state, a focus ring, a role a screen reader announces, or
 * Enter/Space activation, because a `div` is not a control until you tell the
 * platform it is one (WCAG 4.1.2, R4). Converting the ~550 call sites is out
 * of scope for one lane sharing a codebase with nine others mid-edit (see
 * dagK LANE_RULES); this is the shared helper each of those lanes converges
 * on as they touch their own rows, so the fix lands once per surface and
 * never as a hand-rolled `role`/`tabIndex`/`onKeyDown` triple.
 *
 * Usage: spread the result onto the element that currently owns `onClick`.
 *   <div {...pressable(() => openRow(item.id))} ...>
 * `role` defaults to `'button'`; pass `'row'`/`'option'`/etc. for a list
 * item inside a `listbox`/`grid`/`table` semantic the surface already has.
 */
import type { KeyboardEvent, MouseEvent } from 'react';

export interface PressableOptions {
  /** ARIA role to announce. Default 'button' — the common case for a card or
   *  row that simply opens something. */
  role?: string;
  /** Extra class names to merge with `pa-pressable`. */
  className?: string;
  /** Set false only when the element is already disabled/inert and should not
   *  be reachable by keyboard right now (matches the native `disabled` idea
   *  for a `div`, which has no such attribute of its own). */
  enabled?: boolean;
  /** Accessible name, when the element's text content doesn't already say it
   *  (an icon-only row, a card whose visible text is metadata). */
  label?: string;
}

export interface PressableProps {
  role: string;
  tabIndex: number;
  className: string;
  'aria-label'?: string;
  'aria-disabled'?: true;
  onClick?: (e: MouseEvent<HTMLElement>) => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}

/**
 * Build the props to spread onto a clickable non-button element.
 *
 * Space is prevented on `keydown` (native button behaviour: Space should not
 * also scroll the page) and both Enter and Space activate on their own phase
 * rather than waiting for `keyup`, matching how a real `<button>` fires Enter
 * immediately and Space on release — close enough for a synthetic control
 * that Enter-on-down is the one most people's muscle memory expects here.
 */
export function pressable(onActivate: () => void, opts: PressableOptions = {}): PressableProps {
  const { role = 'button', className, enabled = true, label } = opts;
  return {
    role,
    tabIndex: enabled ? 0 : -1,
    className: ['pa-pressable', className].filter(Boolean).join(' '),
    ...(label ? { 'aria-label': label } : {}),
    ...(enabled ? {} : { 'aria-disabled': true as const }),
    onClick: enabled ? (e) => { e.preventDefault(); onActivate(); } : undefined,
    onKeyDown: (e) => {
      if (!enabled) return;
      if (e.key === 'Enter') {
        e.preventDefault();
        onActivate();
      } else if (e.key === ' ' || e.key === 'Spacebar') {
        // Prevent the page scroll a bare Space would otherwise trigger on a
        // focused, non-form element; activate on down like Enter, since this
        // is a synthetic control and there is no native "release" semantic to
        // preserve.
        e.preventDefault();
        onActivate();
      }
    },
  };
}

/** Enter-only variant, for a row that also nests its own interactive children
 *  (a chip, a secondary button) where Space would be ambiguous. */
export function pressableEnterOnly(onActivate: () => void, opts: PressableOptions = {}): PressableProps {
  const base = pressable(onActivate, opts);
  return {
    ...base,
    onKeyDown: (e) => {
      if (!opts.enabled && opts.enabled !== undefined) return;
      if (e.key === 'Enter') {
        e.preventDefault();
        onActivate();
      }
    },
  };
}
