import { useState, useEffect, useRef } from 'react';
import { font } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { Particles, PrimaryButton } from './atoms';
import { useTheme } from '../../styles/useTheme';

interface Persona {
  name: string;
  traits: string[];
  tone: string;
  greeting: string;
}

interface Props {
  persona: Persona;
  onComplete: () => void;
}

export function MomentChat({ persona, onComplete }: Props) {
  const { colors, reduceMotion } = useTheme();
  const [streamed, setStreamed] = useState('');
  const [done, setDone] = useState(false);
  const streamRef = useRef<ReturnType<typeof setInterval>>();

  useEffect(() => {
    const greeting = persona.greeting || `Hello! I'm ${persona.name}. How can I help you today?`;
    // Reduce-motion: present the greeting whole rather than typing it out.
    if (reduceMotion) { setStreamed(greeting); setDone(true); return; }
    let idx = 0;
    streamRef.current = setInterval(() => {
      idx++;
      if (idx >= greeting.length) {
        setStreamed(greeting);
        setDone(true);
        clearInterval(streamRef.current);
      } else {
        setStreamed(greeting.slice(0, idx));
      }
    }, 28);
    return () => clearInterval(streamRef.current);
  }, [persona, reduceMotion]);

  const isSpeaking = !done;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', position: 'relative' }}>
      <Particles density={8} />

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 14, padding: '18px 24px',
        borderBottom: `1px solid ${colors.border}`,
      }}>
        <Mobius size={29} state={isSpeaking ? 'speaking' : 'idle'} logoMode />
        <div>
          <div style={{ fontFamily: font.display, fontSize: 16, fontWeight: 700, color: colors.text }}>
            {persona.name}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <div style={{
              width: 6, height: 6, borderRadius: '50%',
              background: isSpeaking ? colors.cyan : colors.success,
            }} />
            <span style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted }}>
              {/* "Typing", not "Speaking" — this greeting is typed on screen;
                  no audio plays here (2026-07 wiring audit). */}
              {isSpeaking ? 'Typing...' : 'Online'}
            </span>
          </div>
        </div>
      </div>

      {/* Conversation */}
      <div style={{ flex: 1, padding: '28px 24px', overflowY: 'auto' }}>
        <div style={{
          fontFamily: font.body, fontSize: 14, color: colors.text, lineHeight: 1.7,
          maxWidth: 560,
        }}>
          {streamed}
          {!done && <span style={{ animation: 'pa-caret 0.9s steps(1) infinite', borderLeft: `2px solid ${colors.cyan}`, marginLeft: 1 }}>&nbsp;</span>}
        </div>
      </div>

      {/* Footer — enter app */}
      <div style={{
        padding: '16px 24px', borderTop: `1px solid ${colors.border}`,
        display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
      }}>
        <PrimaryButton onClick={onComplete} style={{ padding: '12px 32px' }}>
          Enter Permagent
        </PrimaryButton>
        <p style={{
          fontFamily: font.body, fontSize: 12, color: colors.textMuted,
          margin: '12px 0 0', textAlign: 'center', maxWidth: 420, lineHeight: 1.5,
        }}>
          Ask for a short tour anytime — {persona.name || 'your agent'} will walk you
          through Home, Brain, Projects, and the rest, one surface at a time.
        </p>
      </div>

      <style>{`
        @keyframes pa-caret {
          0%, 50% { opacity: 1; }
          51%, 100% { opacity: 0; }
        }
      `}</style>
    </div>
  );
}
