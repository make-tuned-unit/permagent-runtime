import { useState } from 'react';
import { font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Particles } from './atoms';
import { useTheme } from '../../styles/useTheme';

const PRESETS = [
  { id: 'precise', label: 'Precise & Direct', traits: ['precise', 'direct', 'concise'], tone: 'Professional and to-the-point. No filler.' },
  { id: 'creative', label: 'Creative & Curious', traits: ['creative', 'curious', 'expressive'], tone: 'Imaginative and exploratory. Asks questions, proposes ideas.' },
  { id: 'warm', label: 'Warm & Supportive', traits: ['warm', 'patient', 'encouraging'], tone: 'Friendly and reassuring. Explains clearly, never rushes.' },
  { id: 'hacker', label: 'Hacker & Builder', traits: ['pragmatic', 'fast', 'resourceful'], tone: 'Ship-first mentality. Code over conversation.' },
];

interface Props {
  onAdvance: (traits: string[], tone: string) => void;
  onBack: () => void;
}

export function MomentCalibration({ onAdvance }: Props) {
  const { colors } = useTheme();
  const [selected, setSelected] = useState<string | null>(null);

  const preset = PRESETS.find(p => p.id === selected);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={16} />
      <Mobius size={140} state={selected ? 'idle' : 'calibrating'} />

      <h1 style={{ fontFamily: font.display, fontSize: 28, fontWeight: 700, color: colors.text, margin: '28px 0 8px', letterSpacing: '-0.02em' }}>
        How should your agent think?
      </h1>
      <p style={{ fontFamily: font.body, fontSize: 14, color: colors.textMuted, marginBottom: 28, textAlign: 'center', maxWidth: 380 }}>
        Pick a personality template. You can refine it in the next step.
      </p>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, width: 400, marginBottom: 24 }}>
        {PRESETS.map(p => {
          const active = selected === p.id;
          return (
            <button key={p.id} onClick={() => setSelected(p.id)}
              style={{
                padding: '16px 14px', borderRadius: 12, cursor: 'pointer',
                background: active ? 'rgba(0,213,255,0.10)' : 'rgba(255,255,255,0.02)',
                border: active ? `1px solid ${colors.cyan}` : '1px solid rgba(255,255,255,0.06)',
                boxShadow: active ? '0 0 0 3px rgba(0,213,255,0.12)' : 'none',
                textAlign: 'left', transition: `all 200ms ${ease.out}`,
              }}>
              <div style={{ fontFamily: font.body, fontSize: 14, fontWeight: 600, color: colors.text, marginBottom: 4 }}>
                {p.label}
              </div>
              <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textMuted, lineHeight: 1.4 }}>
                {p.traits.join(' · ')}
              </div>
            </button>
          );
        })}
      </div>

      <PrimaryButton disabled={!preset} onClick={() => preset && onAdvance(preset.traits, preset.tone)}>
        Continue
      </PrimaryButton>
    </div>
  );
}
