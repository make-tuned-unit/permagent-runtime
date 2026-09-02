/**
 * Toggle — the app's one settings switch.
 *
 * Why it moved here from `settings/atoms.tsx`: the switch it used to be had a
 * track, a thumb and nothing else. A toggle bound to a server-persisted
 * setting is a promise-returning action wearing a switch's clothes, and the
 * old one could express neither half of that — no busy phase while the write
 * was in flight, and no failure path at all. So six call sites in Settings had
 * each hand-written their own optimistic flip and their own revert-on-catch
 * (`saveStrix` being the one that got it exactly right), and the call sites
 * that had not simply showed the new position regardless: the switch lied
 * about what the daemon had stored.
 *
 * Contract (U3 §1.2):
 *  - optimistic — the switch moves to the requested position immediately,
 *    because that is what the user asked for and the app expects it to happen;
 *  - pending — while the write is in flight the switch is busy, not
 *    unavailable (`cursor: progress`), and will not accept a second flip;
 *  - success — the optimistic position is handed back to `on`, so the daemon,
 *    not the switch's own optimism, has the last word;
 *  - failure — the switch returns to where it was and an adjacent message says
 *    what happened. A rejection, or a handler that resolves `false` (Button's
 *    own "it failed" signal), both count as failure;
 *  - disabled — dimmed and `not-allowed`, and `disabledReason` puts the reason
 *    on the control when the row does not already make it obvious.
 *
 * A handler that returns nothing is a purely local setting (a preference in
 * localStorage, a theme flag): there is nothing to await, so there is no
 * pending phase and nothing to revert.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { ease, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Tooltip } from './Tooltip';

/** `false` means "it failed" — the same signal `Button` takes. */
export type ToggleOutcome = void | boolean;

export interface ToggleProps {
  on: boolean;
  onChange?: (next: boolean) => ToggleOutcome | Promise<ToggleOutcome>;
  disabled?: boolean;
  /** Why this switch cannot be pressed. Shown via the shared Tooltip; required
   *  reading whenever the disabling condition is not visible on the row. */
  disabledReason?: string;
  /** A message the caller owns, rendered where the switch puts its own — for
   *  the failures only the caller can name ("Saved, but could not re-read it"). */
  error?: string | null;
  /** Accessible name. A bare switch with no adjacent label needs one. */
  label?: string;
}

function isThenable(v: unknown): v is Promise<unknown> {
  return typeof (v as { then?: unknown } | null | undefined)?.then === 'function';
}

export function Toggle({ on, onChange, disabled = false, disabledReason, label, error }: ToggleProps) {
  const { colors } = useTheme();
  /** The position the user asked for, held only until the write settles. */
  const [desired, setDesired] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const live = useRef(true);

  useEffect(() => () => { live.current = false; }, []);

  const flip = useCallback(() => {
    if (disabled || busy || !onChange) return;
    const next = !(desired ?? on);
    setFailure(null);
    const result = onChange(next);
    if (!isThenable(result)) {
      // Local setting: `on` has already changed under us, or it hasn't and the
      // caller meant it not to. Either way there is nothing in flight.
      if (result === false) setFailure("Couldn't save that setting.");
      return;
    }
    setDesired(next);
    setBusy(true);
    void result.then(
      (value) => {
        if (!live.current) return;
        setBusy(false);
        setDesired(null);
        if (value === false) setFailure("Couldn't save that setting.");
      },
      (err: unknown) => {
        if (!live.current) return;
        setBusy(false);
        setDesired(null);
        setFailure(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
      },
    );
  }, [disabled, busy, onChange, desired, on]);

  const shown = desired ?? on;
  const message = error ?? failure;

  const switchEl = (
    <button
      type="button"
      role="switch"
      aria-checked={shown}
      aria-label={label}
      aria-busy={busy || undefined}
      data-pending={busy ? 'true' : undefined}
      disabled={disabled}
      onClick={flip}
      style={{
        width: 36, height: 22, borderRadius: radius.pill, padding: 2,
        background: shown ? colors.cyan : colors.surfaceHi,
        border: 'none',
        cursor: disabled ? 'not-allowed' : busy ? 'progress' : 'pointer',
        position: 'relative',
        opacity: disabled ? 0.55 : 1,
        transition: `background 160ms ${ease.out}`,
        boxShadow: shown ? `0 0 8px ${colors.cyanGlow}` : 'none',
      }}
    >
      {/* The thumb carries the busy phase rather than a badge beside the
          track: the switch is 36px wide, and anything alongside it would
          move the row. Reuses the app's one spin keyframe, so it rides the
          global prefers-reduced-motion guard. */}
      <div
        style={{
          width: 18, height: 18, borderRadius: '50%', background: '#fff',
          transform: shown ? 'translateX(14px)' : 'translateX(0)',
          transition: 'transform 180ms cubic-bezier(0.34, 1.56, 0.64, 1)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}
      >
        {busy && (
          <span
            className="pa-spin"
            aria-hidden="true"
            style={{
              width: 10, height: 10, borderRadius: '50%',
              border: `1.5px solid ${shown ? colors.cyan : colors.textMuted}`,
              borderTopColor: 'transparent',
            } as CSSProperties}
          />
        )}
      </div>
    </button>
  );

  return (
    <span style={{ display: 'inline-flex', flexDirection: 'column', alignItems: 'flex-start', gap: 6 }}>
      {disabled && disabledReason ? (
        // Disabled buttons are not focusable and often skip pointer events;
        // wrap so the reason stays reachable from keyboard and hover.
        <Tooltip content={disabledReason}>
          <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
            {switchEl}
          </span>
        </Tooltip>
      ) : switchEl}
      {message && (
        <span style={{ fontSize: textSize.caption, color: colors.danger, lineHeight: 1.4, maxWidth: 420 }}>
          {message}
        </span>
      )}
    </span>
  );
}
