import { useState, useEffect } from 'react';
import { font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Particles } from './atoms';
import { useTheme } from '../../styles/useTheme';

const PLACEHOLDERS = [
  'Build a SaaS product from scratch...',
  'Manage my daily tasks and emails...',
  'Help me learn Rust and systems programming...',
  'Automate my development workflow...',
  'Research and write technical content...',
];

interface Props {
  intent: string;
  setIntent: (v: string) => void;
  onAdvance: () => void;
  onBack: () => void;
}

export function MomentIntent({ intent, setIntent, onAdvance }: Props) {
  const { colors } = useTheme();
  const [placeholderIdx, setPlaceholderIdx] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setPlaceholderIdx(i => (i + 1) % PLACEHOLDERS.length), 4000);
    return () => clearInterval(id);
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={16} />
      <Mobius size={140} state="idle" />

      <h1 style={{ fontFamily: font.display, fontSize: 28, fontWeight: 700, color: colors.text, margin: '28px 0 8px', letterSpacing: '-0.02em' }}>
        What will you build together?
      </h1>
      <p style={{ fontFamily: font.body, fontSize: 14, color: colors.textMuted, marginBottom: 28, textAlign: 'center', maxWidth: 380 }}>
        Tell your agent what you're working on. This helps it prepare context for your first conversation.
      </p>

      <div style={{ width: 400, marginBottom: 24 }}>
        <textarea
          value={intent}
          onChange={e => setIntent(e.target.value)}
          placeholder={PLACEHOLDERS[placeholderIdx]}
          rows={4}
          style={{
            width: '100%', fontFamily: font.body, fontSize: 14, color: colors.text,
            background: colors.inputBg,
            border: `1px solid ${colors.border}`,
            borderRadius: 12, padding: '14px 16px', outline: 'none', resize: 'none',
            lineHeight: 1.6,
            transition: `border-color 160ms ${ease.out}`,
          }}
          onFocus={e => { e.currentTarget.style.borderColor = colors.cyan; e.currentTarget.style.boxShadow = `0 0 0 3px ${colors.cyanGlow}`; }}
          onBlur={e => { e.currentTarget.style.borderColor = colors.border; e.currentTarget.style.boxShadow = 'none'; }}
        />
      </div>

      <PrimaryButton onClick={onAdvance} disabled={!intent.trim()}>
        Continue
      </PrimaryButton>
    </div>
  );
}
