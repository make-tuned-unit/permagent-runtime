import { useEffect, useState } from 'react';
import { duration, ease, font, textSize } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';

interface Props {
  onDone: () => void;
}

export function Splash({ onDone }: Props) {
  const { colors, reduceMotion } = useTheme();
  const [phase, setPhase] = useState<'in' | 'out'>('in');
  const [showLine1, setShowLine1] = useState(false);
  const [showLine2, setShowLine2] = useState(false);

  // The splash is timed to ONE complete pass of the Möbius loop — that is why
  // it was 5000ms: 151 frames at 30fps. Halved by stepping two source frames
  // per tick (below) rather than by cutting the animation off mid-turn, so the
  // logo still completes its full revolution: 151/2 steps at 30fps ≈ 2520ms.
  // These are stage delays (when text appears), not transition durations —
  // D9's <500ms ceiling applies to the transitions themselves, below.
  useEffect(() => {
    const t1 = setTimeout(() => setShowLine1(true), 300);
    const t2 = setTimeout(() => setShowLine2(true), 1200);
    const t3 = setTimeout(() => setPhase('out'), 2500);
    return () => { clearTimeout(t1); clearTimeout(t2); clearTimeout(t3); };
  }, []);

  useEffect(() => {
    if (phase === 'out') {
      const timer = setTimeout(onDone, reduceMotion ? 0 : duration.smooth);
      return () => clearTimeout(timer);
    }
  }, [phase, onDone, reduceMotion]);

  // D9: every transition is a spring token under 500ms. Reduce Motion collapses
  // all of them to an instant cut — no transform, no eased opacity ramp — per
  // D14/the Mobius reduce-motion note this lane's brief calls out.
  const fadeOut = reduceMotion
    ? 'none'
    : `opacity ${duration.smooth}ms ${ease.smooth}`;
  const line1Transition = reduceMotion
    ? 'none'
    : `opacity ${duration.smooth}ms ${ease.smooth}, transform ${duration.smooth}ms ${ease.smooth}`;
  // The one delight moment D9 reserves `bouncy` for: "Forever." landing.
  const line2Transition = reduceMotion
    ? 'none'
    : `opacity ${duration.bouncy}ms ${ease.bouncy}, transform ${duration.bouncy}ms ${ease.bouncy}`;
  const lineOffset = (shown: boolean) => (reduceMotion || shown ? 'translateY(0)' : 'translateY(8px)');

  return (
    <div
      onClick={() => setPhase('out')}
      style={{
        position: 'fixed', inset: 0, paddingTop: TITLEBAR_HEIGHT,
        background: `radial-gradient(ellipse 70% 50% at 50% 45%, ${colors.cyanWash} 0%, ${colors.bg} 70%)`,
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        cursor: 'pointer',
        opacity: phase === 'out' ? 0 : 1,
        transition: fadeOut,
      }}
    >
      {/* Mobius freezes every state under Reduce Motion at the source, so the
       *  'thinking' loop is safe to use directly here regardless of the
       *  setting — see the `motionDisabled` gate in Mobius.tsx. */}
      <Mobius size={180} state="thinking" glow={1} frameStep={2} />

      {/* ===== TAGLINE LOCKED =====
        * Format: "Built to grow with you. Forever."
        * - Sentence case (capital B and F only)
        * - Two periods (after "you" and after "Forever")
        * - text-transform: 'none' — NEVER 'uppercase'
        * - This has regressed three times. Do not modify the case or
        *   capitalization without explicit user instruction.
        * ========================= */}
      <p style={{
        fontFamily: font.display, fontSize: textSize.body, fontWeight: 600,
        color: colors.textMuted, letterSpacing: '0.08em',
        textTransform: 'none',
        marginTop: 28,
        opacity: 0.7,
      }}>
        <span style={{
          opacity: showLine1 ? 1 : 0,
          transform: lineOffset(showLine1),
          transition: line1Transition,
          display: 'inline-block',
        }}>
          Built to grow with you.
        </span>
        {' '}
        <span style={{
          opacity: showLine2 ? 1 : 0,
          transform: lineOffset(showLine2),
          transition: line2Transition,
          display: 'inline-block',
          textShadow: showLine2 ? `0 0 20px ${colors.cyanGlow}` : 'none',
        }}>
          Forever.
        </span>
      </p>
    </div>
  );
}
