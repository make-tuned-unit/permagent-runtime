import { useState } from 'react';
import { font, ease, duration, radius, textSize } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Particles, WizardHeading, WizardSubhead } from './atoms';
import { useTheme } from '../../styles/useTheme';

const PRESETS = [
  { id: 'precise', label: 'Precise & Direct', traits: ['precise', 'direct', 'concise'], tone: 'Professional and to-the-point. No filler.' },
  { id: 'creative', label: 'Creative & Curious', traits: ['creative', 'curious', 'expressive'], tone: 'Imaginative and exploratory. Asks questions, proposes ideas.' },
  { id: 'warm', label: 'Warm & Supportive', traits: ['warm', 'patient', 'encouraging'], tone: 'Friendly and reassuring. Explains clearly, never rushes.' },
  { id: 'hacker', label: 'Hacker & Builder', traits: ['pragmatic', 'fast', 'resourceful'], tone: 'Ship-first mentality. Code over conversation.' },
];

interface Props {
  active?: boolean;
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

      <WizardHeading style={{ marginTop: 28 }}>How should your agent think?</WizardHeading>
      <WizardSubhead style={{ marginBottom: 28 }}>
        Pick a personality template. You can refine it in the next step.
      </WizardSubhead>

      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, width: 400, marginBottom: 24 }}>
        {PRESETS.map(p => {
          const active = selected === p.id;
          return (
            <button key={p.id} onClick={() => setSelected(p.id)}
              aria-pressed={active}
              style={{
                padding: '16px 14px', borderRadius: radius.lg, cursor: 'pointer',
                background: active ? colors.cyanSoft : colors.inputBg,
                border: active ? `1px solid ${colors.cyan}` : `1px solid ${colors.border}`,
                boxShadow: active ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
                textAlign: 'left', transition: `all ${duration.base}ms ${ease.out}`,
              }}>
              <div style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text, marginBottom: 4 }}>
                {p.label}
              </div>
              <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.4 }}>
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
