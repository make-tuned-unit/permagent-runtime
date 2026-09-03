import { useState, useEffect } from 'react';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Particles, Textarea, WizardHeading, WizardSubhead } from './atoms';
import { useTheme } from '../../styles/useTheme';
import { space } from '../../styles/tokens';

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
  const { reduceMotion } = useTheme();
  const [placeholderIdx, setPlaceholderIdx] = useState(0);

  // Rotate the example placeholder — but hold still if the user asked to reduce
  // motion (the cycling text is decorative, not information the user needs).
  useEffect(() => {
    if (reduceMotion) return;
    const id = setInterval(() => setPlaceholderIdx(i => (i + 1) % PLACEHOLDERS.length), 4000);
    return () => clearInterval(id);
  }, [reduceMotion]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={16} />
      <Mobius size={140} state="idle" />

      <WizardHeading style={{ marginTop: 28 }}>What will you build together?</WizardHeading>
      <WizardSubhead style={{ marginBottom: 28 }}>
        Tell your agent what you're working on. It'll be waiting as your first
        chat message — edit it or send it as-is.
      </WizardSubhead>

      <div style={{ width: 400, marginBottom: space.huge }}>
        <Textarea
          value={intent}
          onChange={setIntent}
          placeholder={PLACEHOLDERS[placeholderIdx]}
          rows={4}
        />
      </div>

      <PrimaryButton onClick={onAdvance} disabled={!intent.trim()}>
        Continue
      </PrimaryButton>
    </div>
  );
}
