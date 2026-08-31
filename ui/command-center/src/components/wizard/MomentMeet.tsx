import { useState } from 'react';
import { font, radius } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, Glass, Particles, Input, Textarea, WizardHeading, WizardSubhead, validateTrait } from './atoms';
import { useTheme } from '../../styles/useTheme';
import { VoicePicker } from '../voice/VoicePicker';

interface Persona {
  name: string;
  traits: string[];
  tone: string;
  greeting: string;
  voiceId: string | null;
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
  const [newTrait, setNewTrait] = useState('');
  const [traitError, setTraitError] = useState('');

  const updateField = <K extends keyof Persona>(key: K, value: Persona[K]) =>
    setPersona({ ...persona, [key]: value });

  const addTrait = () => {
    // Silently ignore an empty field on blur (not an error the user should see),
    // but surface a real reason for a rejected non-empty value (audit #603:
    // trait-add gave no feedback).
    if (!newTrait.trim()) { setTraitError(''); return; }
    const v = validateTrait(persona.traits, newTrait);
    if (!v.ok) { setTraitError(v.reason ?? ''); return; }
    updateField('traits', [...persona.traits, newTrait.trim()]);
    setNewTrait('');
    setTraitError('');
  };
  const removeTrait = (trait: string) =>
    updateField('traits', persona.traits.filter(t => t !== trait));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={12} />

      <WizardHeading>Meet your agent</WizardHeading>
      <WizardSubhead style={{ marginBottom: 28 }}>
        Give it a name and review its personality. You can always change these in Settings.
      </WizardSubhead>

      <Glass r={radius.lg} padding={28} style={{ width: 420, position: 'relative' }}>
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
            <div
              onClick={() => setEditName(true)}
              onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setEditName(true); } }}
              role="button"
              tabIndex={0}
              aria-label="Edit agent name"
              style={{ cursor: 'pointer', display: 'flex', alignItems: 'baseline', gap: 8 }}
            >
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
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginBottom: 8 }}>
            {persona.traits.map((trait) => (
              <span key={trait} style={{
                fontFamily: font.body, fontSize: 12, fontWeight: 500,
                color: colors.cyan, background: colors.cyanSoft,
                borderRadius: radius.pill, padding: '5px 12px',
                display: 'inline-flex', alignItems: 'center', gap: 6,
              }}>
                {trait}
                <span
                  onClick={() => removeTrait(trait)}
                  onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); removeTrait(trait); } }}
                  role="button"
                  tabIndex={0}
                  aria-label={`Remove ${trait}`}
                  style={{ cursor: 'pointer', color: colors.textDim, fontWeight: 700, lineHeight: 1 }}
                >
                  ×
                </span>
              </span>
            ))}
          </div>
          <Input
            value={newTrait}
            onChange={v => { setNewTrait(v); if (traitError) setTraitError(''); }}
            onKeyDown={e => {
              if (e.key === 'Enter' || e.key === ',') {
                e.preventDefault();
                addTrait();
              }
            }}
            onBlur={addTrait}
            ariaLabel="Add a trait"
            placeholder="Add a trait — type and press Enter"
          />
          {traitError && (
            <div role="alert" style={{ fontFamily: font.body, fontSize: 11, color: colors.danger, marginTop: 6 }}>
              {traitError}
            </div>
          )}
        </div>

        {/* Tone */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Tone
          </label>
          <Textarea
            value={persona.tone}
            onChange={v => updateField('tone', v)}
            rows={2}
          />
        </div>

        {/* Voice */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Voice
          </label>
          <VoicePicker value={persona.voiceId} onChange={v => updateField('voiceId', v)} />
        </div>

        {/* Greeting */}
        <div style={{ marginBottom: 20 }}>
          <label style={{ fontFamily: font.body, fontSize: 11, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.06em', display: 'block', marginBottom: 6 }}>
            Opening greeting
          </label>
          <Textarea
            value={persona.greeting}
            onChange={v => updateField('greeting', v)}
            rows={2}
          />
        </div>

        <PrimaryButton full onClick={onAdvance} disabled={!persona.name.trim()}>
          Looks good — let's go
        </PrimaryButton>
      </Glass>
    </div>
  );
}
