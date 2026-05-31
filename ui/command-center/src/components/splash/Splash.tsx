import { useEffect, useState } from 'react';
import { font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { useTheme } from '../../styles/useTheme';

interface Props {
  onDone: () => void;
}

export function Splash({ onDone }: Props) {
  const { colors } = useTheme();
  const [phase, setPhase] = useState<'in' | 'out'>('in');
  const [showLine1, setShowLine1] = useState(false);
  const [showLine2, setShowLine2] = useState(false);

  useEffect(() => {
    const t1 = setTimeout(() => setShowLine1(true), 600);
    const t2 = setTimeout(() => setShowLine2(true), 2400);
    const t3 = setTimeout(() => setPhase('out'), 5000);
    return () => { clearTimeout(t1); clearTimeout(t2); clearTimeout(t3); };
  }, []);

  useEffect(() => {
    if (phase === 'out') {
      const timer = setTimeout(onDone, 400);
      return () => clearTimeout(timer);
    }
  }, [phase, onDone]);

  return (
    <div
      onClick={() => setPhase('out')}
      style={{
        position: 'fixed', inset: 0,
        background: `radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.05) 0%, ${colors.bg} 70%)`,
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        cursor: 'pointer',
        opacity: phase === 'out' ? 0 : 1,
        transition: `opacity 400ms ${ease.out}`,
      }}
    >
      <Mobius size={180} state="thinking" glow={1} />

      {/* ===== TAGLINE LOCKED =====
        * Format: "Built to grow with you. Forever."
        * - Sentence case (capital B and F only)
        * - Two periods (after "you" and after "Forever")
        * - text-transform: 'none' — NEVER 'uppercase'
        * - This has regressed three times. Do not modify the case or
        *   capitalization without explicit user instruction.
        * ========================= */}
      <p style={{
        fontFamily: font.display, fontSize: 15, fontWeight: 600,
        color: colors.textMuted, letterSpacing: '0.08em',
        textTransform: 'none',
        marginTop: 28,
        opacity: 0.7,
      }}>
        <span style={{
          opacity: showLine1 ? 1 : 0,
          transform: showLine1 ? 'translateY(0)' : 'translateY(8px)',
          transition: `opacity 800ms ${ease.out}, transform 800ms ${ease.out}`,
          display: 'inline-block',
        }}>
          Built to grow with you.
        </span>
        {' '}
        <span style={{
          opacity: showLine2 ? 1 : 0,
          transform: showLine2 ? 'translateY(0)' : 'translateY(8px)',
          transition: `opacity 800ms ${ease.spring}, transform 800ms ${ease.spring}`,
          display: 'inline-block',
          textShadow: showLine2 ? '0 0 20px rgba(0,213,255,0.3)' : 'none',
        }}>
          Forever.
        </span>
      </p>
    </div>
  );
}
