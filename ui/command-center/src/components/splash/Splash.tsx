import { useEffect, useState } from 'react';
import { color, font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';

interface Props {
  onDone: () => void;
}

export function Splash({ onDone }: Props) {
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
        background: `radial-gradient(ellipse 70% 50% at 50% 45%, rgba(0,213,255,0.05) 0%, ${color.bg} 70%)`,
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        cursor: 'pointer',
        opacity: phase === 'out' ? 0 : 1,
        transition: `opacity 400ms ${ease.out}`,
      }}
    >
      <Mobius size={180} state="thinking" glow={1} />

      <p style={{
        fontFamily: font.display, fontSize: 15, fontWeight: 600,
        color: color.textMuted, letterSpacing: '0.08em',
        marginTop: 28,
        // TAGLINE TEXT IS LOCKED — sentence case, two periods, no uppercase
        // transforms. Do not change without explicit instruction.
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
