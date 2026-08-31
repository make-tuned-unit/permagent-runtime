/**
 * Button — the app's one button primitive.
 *
 * Why it exists: buttons were styled with inline `CSSProperties` objects, and
 * an inline style cannot express `:hover` or `:active` at all. Pressing one
 * looked identical to not pressing one, and a disabled button looked identical
 * to an enabled one. The states live in `.pa-btn` (see index.css); this
 * component only feeds it per-theme colors through CSS custom properties, so
 * the look is unchanged and the feedback is new.
 *
 * Feedback contract:
 *  - hover / :active pressed — CSS, no JS.
 *  - pending — when `onClick` returns a promise the button spins for the round
 *    trip on its own; a form submit (where the work is on `onSubmit`) passes
 *    `pending` in instead.
 *  - success — a brief tick, but ONLY when the promise resolves to something
 *    other than `false`. Callers whose helper swallows its own errors (finance's
 *    `mutate`) resolve `false` on failure so a failed action never ticks.
 *  - disabled — dimmed and `not-allowed`, so "you can't press this" is visible.
 *
 * Motion reuses the app's `pa-spin` keyframe and rides the global
 * `prefers-reduced-motion` guard at the bottom of index.css.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ButtonHTMLAttributes, CSSProperties, MouseEvent, ReactNode } from 'react';
import { radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

/** ghost = hairline, ghostOn = hairline in accent (a selected filter),
 *  primary = flat accent fill, bare = no chrome (icon-ish affordances). */
export type ButtonVariant = 'ghost' | 'ghostOn' | 'primary' | 'bare';

/** How long the success tick stays up. Long enough to read, short enough not
 *  to be mistaken for a new resting state. */
export const SUCCESS_FLASH_MS = 900;

export interface ButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onClick' | 'style'> {
  colors: ThemeColors;
  variant?: ButtonVariant;
  /** Force the in-flight state. For form submits, where the work runs in
   *  `onSubmit` and the click handler can't be awaited. */
  pending?: boolean;
  /** Force the success tick (tests, or externally-owned confirmation). */
  success?: boolean;
  /** Returning a promise opts this button into pending + success automatically.
   *  Resolving `false` means "it failed" — no tick. */
  onClick?: (e: MouseEvent<HTMLButtonElement>) => unknown;
  /** Opt out of the tick for actions where confirmation is noise. */
  flashSuccess?: boolean;
  style?: CSSProperties;
  children?: ReactNode;
}

type Vars = CSSProperties & Record<`--${string}`, string | number>;

function variantVars(variant: ButtonVariant, colors: ThemeColors): Vars {
  const common: Vars = {
    '--pa-btn-radius': `${radius.sm}px`,
    '--pa-btn-success': colors.success,
  };
  switch (variant) {
    case 'primary':
      return {
        ...common,
        '--pa-btn-bg': colors.cyan,
        '--pa-btn-fg': colors.textOnCyan,
        '--pa-btn-border': 'transparent',
        '--pa-btn-bg-hover': colors.cyan,
        '--pa-btn-border-hover': 'transparent',
        '--pa-btn-bg-active': colors.cyan,
        '--pa-btn-pad': '7px 12px',
        '--pa-btn-weight': 600,
      };
    case 'ghostOn':
      return {
        ...common,
        '--pa-btn-bg': 'transparent',
        '--pa-btn-fg': colors.cyan,
        '--pa-btn-border': colors.cyan,
        '--pa-btn-bg-hover': colors.cyanSoft,
        '--pa-btn-border-hover': colors.cyan,
        '--pa-btn-bg-active': colors.cyanGlow,
        '--pa-btn-pad': '6px 10px',
      };
    case 'bare':
      return {
        ...common,
        '--pa-btn-bg': 'transparent',
        '--pa-btn-fg': colors.text,
        '--pa-btn-border': 'transparent',
        '--pa-btn-bg-hover': colors.surfaceHi,
        '--pa-btn-border-hover': 'transparent',
        '--pa-btn-bg-active': colors.surface,
        '--pa-btn-pad': '2px 5px',
      };
    case 'ghost':
    default:
      return {
        ...common,
        '--pa-btn-bg': 'transparent',
        '--pa-btn-fg': colors.text,
        '--pa-btn-border': colors.border,
        '--pa-btn-bg-hover': colors.surfaceHi,
        '--pa-btn-border-hover': colors.borderHi,
        '--pa-btn-bg-active': colors.surface,
        '--pa-btn-pad': '6px 10px',
      };
  }
}

function isThenable(v: unknown): v is Promise<unknown> {
  return typeof (v as { then?: unknown } | null | undefined)?.then === 'function';
}

export function Button({
  colors,
  variant = 'ghost',
  pending,
  success,
  onClick,
  flashSuccess = true,
  style,
  children,
  className,
  disabled,
  ...rest
}: ButtonProps) {
  const [selfPending, setSelfPending] = useState(false);
  const [selfSuccess, setSelfSuccess] = useState(false);
  const live = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    live.current = false;
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const handle = useCallback((e: MouseEvent<HTMLButtonElement>) => {
    if (!onClick) return;
    const result = onClick(e);
    if (!isThenable(result)) return;
    setSelfPending(true);
    void result.then(
      (value) => {
        if (!live.current) return;
        setSelfPending(false);
        // `false` is the caller's "it failed" signal — never tick on a failure.
        if (!flashSuccess || value === false) return;
        setSelfSuccess(true);
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => {
          if (live.current) setSelfSuccess(false);
        }, SUCCESS_FLASH_MS);
      },
      () => { if (live.current) setSelfPending(false); },
    );
  }, [onClick, flashSuccess]);

  const isPending = Boolean(pending ?? false) || selfPending;
  const isSuccess = !isPending && (Boolean(success ?? false) || selfSuccess);

  return (
    <button
      {...rest}
      className={['pa-btn', `pa-btn--${variant}`, className].filter(Boolean).join(' ')}
      style={{ ...variantVars(variant, colors), ...style }}
      disabled={disabled || isPending}
      aria-busy={isPending || undefined}
      data-pending={isPending ? 'true' : undefined}
      data-state={isSuccess ? 'success' : undefined}
      onClick={onClick ? handle : undefined}
    >
      {isPending && <span className="pa-btn__spinner pa-spin" aria-hidden="true" />}
      {isSuccess && <span className="pa-btn__tick" aria-hidden="true">✓</span>}
      <span className="pa-btn__label">{children}</span>
    </button>
  );
}
