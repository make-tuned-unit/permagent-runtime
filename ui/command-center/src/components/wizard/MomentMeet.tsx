import { useState } from 'react';
import { font } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Glass, Particles } from './atoms';
import { useTheme } from '../../styles/useTheme';

interface Persona {
  name: string;
  traits: string[];
  tone: string;
  greeting: string;
}

interface Props {
  persona: Persona;
  setPersona: (p: Persona) => void;
  onAdvance: () => void;
  onBack: () => void;
}

export function MomentMeet({ persona, setPersona, onAdvance }: Props) {
  const { colors } = useTheme();
  const [editName, setEditName] = useState(false);

  const updateField = <K extends keyof Persona>(key: K, value: Persona[K]) =>
    setPersona({ ...persona, [key]: value });

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={12} />

      <h1 style={{ fontFamily: font.display, fontSize: 28, fontWeight: 700, color: colors.text, margin: '0 0 8px', letterSpacing: '-0.02em' }}>
        Meet your agent
      </h1>
      <p style={{ fontFamily: font.body, fontSize: 14, color: colors.textMuted, marginBottom: 28, textAlign: 'center', maxWidth: 380 }}>
        Give it a name and review its personality. You can always change these in Settings.
      </p>

      <Glass r={16} padding={28} style={{ width: 420, position: 'relative' }}>
        <div style={{ position: 'absolute', top: 16, right: 16 }}>
          <Mobius size={48} state="idle" logoMode />
        </div>

        {/* Name */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Name
          </label>
          {editName ? (
            <input
              value={persona.name}
              onChange={e => updateField('name', e.target.value)}
              onBlur={() => setEditName(false)}
              onKeyDown={e => e.key === 'Enter' && setEditName(false)}
              autoFocus
              style={{
                fontFamily: font.display, fontSize: 22, fontWeight: 700, color: colors.text,
                background: 'transparent', border: 'none', borderBottom: `1px solid ${colors.cyan}`,
                outline: 'none', width: '100%', padding: '2px 0',
              }}
            />
          ) : (
            <div onClick={() => setEditName(true)} style={{ cursor: 'pointer', display: 'flex', alignItems: 'baseline', gap: 8 }}>
              <span style={{ fontFamily: font.display, fontSize: 22, fontWeight: 700, color: colors.text }}>
                {persona.name || 'Click to name...'}
              </span>
              <span style={{ fontSize: 11, color: colors.textDim }}>edit</span>
            </div>
          )}
        </div>

        {/* Traits */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 8 }}>
            Traits
          </label>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {persona.traits.map((trait, i) => (
              <span key={i} style={{
                fontFamily: font.body, fontSize: 12, fontWeight: 500,
                color: colors.cyan, background: colors.cyanSoft,
                borderRadius: 999, padding: '5px 12px',
              }}>
                {trait}
              </span>
            ))}
          </div>
        </div>

        {/* Tone */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Tone
          </label>
          <textarea
            value={persona.tone}
            onChange={e => updateField('tone', e.target.value)}
            rows={2}
            style={{
              width: '100%', fontFamily: font.body, fontSize: 13, color: colors.text,
              background: 'rgba(255,255,255,0.02)', border: '1px solid rgba(255,255,255,0.06)',
              borderRadius: 8, padding: '10px 12px', outline: 'none', resize: 'none', lineHeight: 1.5,
            }}
          />
        </div>

        {/* Greeting */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Opening greeting
          </label>
          <textarea
            value={persona.greeting}
            onChange={e => updateField('greeting', e.target.value)}
            rows={2}
            style={{
              width: '100%', fontFamily: font.body, fontSize: 13, color: colors.text,
              background: 'rgba(255,255,255,0.02)', border: '1px solid rgba(255,255,255,0.06)',
              borderRadius: 8, padding: '10px 12px', outline: 'none', resize: 'none', lineHeight: 1.5,
            }}
          />
        </div>

        <PrimaryButton full onClick={onAdvance} disabled={!persona.name.trim()}>
          Looks good — let's go
        </PrimaryButton>
      </Glass>
    </div>
  );
}
