import { useId, useState } from 'react';
import { font, radius, space, textSize, duration, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { FieldLabel, Input, Panel } from './atoms';
import { StepFrame } from './StepFrame';
import { TONES, MAX_AGENT_NAME, effectiveName, greetingFor, toneById } from './persona';

interface Props {
  name: string;
  toneId: string;
  /** The name saved when the field is left blank (the backend's default). */
  defaultName: string;
  onChange: (next: { name: string; toneId: string }) => void;
  onNext: () => void;
}

/** One step for the persona: a name and a tone, with the real greeting. */
export function StepAgent({ name, toneId, defaultName, onChange, onNext }: Props) {
  const { colors } = useTheme();
  const nameId = useId();
  const tone = toneById(toneId);
  const shownName = effectiveName(name, defaultName);
  const tooLong = name.trim().length > MAX_AGENT_NAME;
  const [focused, setFocused] = useState<string | null>(null);

  return (
    <StepFrame
      title="Name your agent"
      why="Pick a name and a tone. You can change both, and give it a voice, in Settings."
      primary={{ label: 'Continue', disabled: tooLong }}
      onPrimary={onNext}
    >
      <div>
        <FieldLabel htmlFor={nameId}>Name</FieldLabel>
        <Input id={nameId} value={name} onChange={v => onChange({ name: v, toneId: tone.id })}
          placeholder={defaultName} maxLength={MAX_AGENT_NAME + 10} invalid={tooLong} autoFocus />
        <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: tooLong ? colors.danger : colors.textMuted, marginTop: space.sm }}>
          {tooLong ? `Keep it under ${MAX_AGENT_NAME} characters.` : `Leave it blank to use ${defaultName}.`}
        </div>
      </div>

      <div>
        <FieldLabel id={`${nameId}-tone`}>Tone</FieldLabel>
        <div role="radiogroup" aria-labelledby={`${nameId}-tone`} style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(118px, 1fr))', gap: space.md }}>
          {TONES.map(t => {
            const on = t.id === tone.id;
            return (
              <label key={t.id} style={{
                display: 'flex', flexDirection: 'column', gap: space.xxs, cursor: 'pointer',
                padding: `${space.lg}px ${space.xl}px`, borderRadius: radius.md,
                background: on ? colors.cyanSoft : colors.inputBg,
                border: `1px solid ${on ? colors.cyan : colors.border}`,
                boxShadow: focused === t.id ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
                transition: `background ${duration.fast}ms ${ease.out}, border-color ${duration.fast}ms ${ease.out}`,
                position: 'relative',
              }}>
                <input type="radio" name="agent-tone" value={t.id} checked={on}
                  onChange={() => onChange({ name, toneId: t.id })}
                  onFocus={() => setFocused(t.id)} onBlur={() => setFocused(null)}
                  style={{ position: 'absolute', opacity: 0, pointerEvents: 'none' }} />
                <span style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{t.label}</span>
                <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted }}>{t.hint}</span>
              </label>
            );
          })}
        </div>
      </div>

      <Panel tone="accent">
        <FieldLabel>How {shownName} will greet you</FieldLabel>
        <p style={{ margin: 0, fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.55, color: colors.text }}>
          “{greetingFor(shownName, tone.id)}”
        </p>
      </Panel>
    </StepFrame>
  );
}
