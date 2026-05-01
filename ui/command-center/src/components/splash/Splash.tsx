import { useEffect, useState } from 'react';
import { color, font, ease } from '../../styles/tokens';

interface Props {
  onDone: () => void;
}

export function Splash({ onDone }: Props) {
  const [phase, setPhase] = useState<'in' | 'out'>('in');

  useEffect(() => {
    const timer = setTimeout(() => setPhase('out'), 1800);
    return () => clearTimeout(timer);
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
      {/* Logo */}
      <img
        src="/mobius/logo.webp"
        alt="Permagent"
        draggable={false}
        style={{
          width: 420, height: 'auto',
          filter: 'drop-shadow(0 0 24px rgba(0,213,255,0.35))',
          animation: 'splash-pulse 2.4s ease-in-out infinite',
        }}
      />

      {/* Tagline */}
      <p style={{
        fontFamily: font.display, fontSize: 15, fontWeight: 600,
        color: color.textMuted, letterSpacing: '0.08em',
        textTransform: 'uppercase', marginTop: 28,
        opacity: 0.7,
      }}>
        Persistent Intelligence
      </p>

      <style>{`
        @keyframes splash-pulse {
          0%, 100% { filter: drop-shadow(0 0 24px rgba(0,213,255,0.35)); }
          50% { filter: drop-shadow(0 0 40px rgba(0,213,255,0.55)); }
        }
      `}</style>
    </div>
  );
}
