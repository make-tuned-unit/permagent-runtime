import { useCallback, useEffect, useRef, useState } from 'react';
import { useForumReducedMotion } from './useForumReducedMotion';

/** How long the HUD stays up after the last input, in milliseconds. */
export const HUD_REST_DELAY_MS = 4200;
/** Walking is the mode you most want an unobstructed view in, so it rests sooner. */
export const HUD_REST_DELAY_WALKING_MS = 2400;

/**
 * Whether the forum HUD should be showing right now.
 *
 * The user: "the key data on the screen blocks too much and needs to be hidden and
 * responsive coming in to view and away again so i can enjoy the world."
 *
 * So the HUD is present when you are doing something and steps aside when you
 * are not. Any pointer move, key, wheel or touch inside the world wakes it; a
 * quiet interval rests it again. Two rules keep this from ever trapping
 * someone: hovering a HUD surface holds it awake indefinitely (you cannot have
 * a menu fade out from under the cursor mid-reach), and focusing anything
 * inside it — tabbing to a chip — does the same, so keyboard users are never
 * left aiming at something invisible.
 *
 * `prefers-reduced-motion` keeps the HUD up permanently: the fade is the
 * effect, and someone who has asked for less motion should not have the
 * interface moving on its own.
 */
export function useHudPresence({
  walking,
  enabled = true,
}: {
  walking: boolean;
  enabled?: boolean;
}) {
  const [awake, setAwake] = useState(true);
  const holdRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const reduceMotion = useForumReducedMotion();

  const clear = () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const rest = useCallback(() => {
    clear();
    if (!enabled || reduceMotion || holdRef.current) return;
    timerRef.current = setTimeout(
      () => setAwake(false),
      walking ? HUD_REST_DELAY_WALKING_MS : HUD_REST_DELAY_MS,
    );
  }, [enabled, reduceMotion, walking]);

  const wake = useCallback(() => {
    setAwake(true);
    rest();
  }, [rest]);

  /** Hold the HUD open while a surface is hovered or focused, and release after. */
  const hold = useCallback(
    (held: boolean) => {
      holdRef.current = held;
      if (held) {
        clear();
        setAwake(true);
      } else {
        rest();
      }
    },
    [rest],
  );

  useEffect(() => {
    if (!enabled || reduceMotion) {
      setAwake(true);
      clear();
      return;
    }
    rest();
    return clear;
  }, [enabled, reduceMotion, rest]);

  return { awake: awake || !enabled || reduceMotion, wake, hold };
}
