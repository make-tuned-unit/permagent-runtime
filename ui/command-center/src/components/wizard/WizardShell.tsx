import { useState } from 'react';
import { ease } from '../../styles/tokens';
import { ProgressDots, BackChevron } from './atoms';
import { MomentWelcome } from './MomentWelcome';
import { MomentHardware } from './MomentHardware';
import { MomentCalibration } from './MomentCalibration';
import { MomentIntent } from './MomentIntent';
import { MomentMeet } from './MomentMeet';
import { MomentWebSearch } from './MomentWebSearch';
import { MomentChat } from './MomentChat';
import { api, apiFetch } from '../../lib/api';
import { stashWizardIntent } from '../../lib/wizardIntent';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Persona {
  name: string;
  traits: string[];
  tone: string;
  greeting: string;
  voiceId: string | null;
}

interface Props {
  onComplete: () => void;
}

export function WizardShell({ onComplete }: Props) {
  const { colors } = useTheme();
  const [step, setStep] = useState(0);
  const [intent, setIntent] = useState('');
  const [persona, setPersona] = useState<Persona>({
    name: '', traits: [], tone: '', greeting: '', voiceId: null,
  });

  const back = () => setStep(s => Math.max(0, s - 1));

  const handleProviderDone = (_provider: string, _key: string) => {
    setStep(1);
  };

  // #381: hardware scan → local-model recommendation for the Librarian sits
  // between provider setup and personality calibration.
  const handleHardwareDone = () => {
    setStep(2);
  };

  const handleCalibrationDone = (traits: string[], tone: string) => {
    setPersona(p => ({
      ...p, traits, tone,
      greeting: `Hello! I'm ready to help. ${tone.split('.')[0]}.`,
    }));
    setStep(3);
  };

  const handleIntentDone = () => {
    if (!persona.name) {
      setPersona(p => ({ ...p, name: 'Aria' }));
    }
    setStep(4);
  };

  const handleMeetDone = () => {
    setStep(5);
  };

  const handleWebSearchDone = () => {
    setStep(6);
  };

  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const handleComplete = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      // Save persona to backend
      const body = {
        first_name: persona.name,
        last_name: null,
        nickname: null,
        traits: persona.traits,
        tone: persona.tone,
        opening_greeting: persona.greeting,
        voice_id: persona.voiceId,
      };
      await apiFetch<unknown>('/api/agent/identity', {
        method: 'PUT',
        body: JSON.stringify(body),
      });
      // Mark wizard complete
      await api.upsertConfig('wizard_complete', true);
    } catch (e) {
      // Surfaced (2026-07 wiring audit): the old catch logged to the console
      // and completed anyway — persona silently lost AND wizard_complete never
      // written, so the whole wizard reappeared on next launch unexplained.
      console.error('Failed to save persona:', e);
      setSaveError(e instanceof Error ? e.message : String(e));
      setSaving(false);
      return;
    }
    // Hand the stated intent to the first chat composer (one-shot).
    stashWizardIntent(intent);
    setSaving(false);
    onComplete();
  };

  const moments = [
    <MomentWelcome key="welcome" onAdvance={handleProviderDone} />,
    <MomentHardware key="hardware" onAdvance={handleHardwareDone} onBack={back} />,
    <MomentCalibration key="calibration" onAdvance={handleCalibrationDone} onBack={back} />,
    <MomentIntent key="intent" intent={intent} setIntent={setIntent} onAdvance={handleIntentDone} onBack={back} />,
    <MomentMeet key="meet" persona={persona} setPersona={setPersona} onAdvance={handleMeetDone} onBack={back} />,
    <MomentWebSearch key="websearch" personaName={persona.name} onAdvance={handleWebSearchDone} onBack={back} />,
    <MomentChat key="chat" persona={persona} onComplete={handleComplete} />,
  ];

  return (
    <div style={{
      position: 'fixed', inset: 0, paddingTop: 28,
      background: `radial-gradient(ellipse 80% 60% at 50% 40%, rgba(0,213,255,0.06) 0%, ${colors.bg} 70%)`,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      {/* Top bar: back + dots */}
      {step > 0 && step < 6 && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '16px 20px' }}>
          <BackChevron onClick={back} />
          {/* Dots track the 5 interior config steps (1–5); step 0 (welcome) and
              step 6 (chat) show no bar. count was 6 → a 6th dot that `current`
              (max step-1 = 4) could never light. */}
          <ProgressDots count={5} current={step - 1} />
          <div style={{ width: 60 }} />
        </div>
      )}

      {/* Step content with crossfade */}
      <div style={{ flex: 1, position: 'relative', minHeight: 0 }}>
        {moments.map((moment, i) => (
          <div key={i} style={{
            position: 'absolute', inset: 0,
            opacity: i === step ? 1 : 0,
            pointerEvents: i === step ? 'auto' : 'none',
            transition: `opacity 320ms ${ease.out}`,
          }}>
            {moment}
          </div>
        ))}
      </div>

      {/* Save failure: honest, recoverable — never a silent loss. */}
      {saveError && (
        <div style={{
          position: 'absolute', left: '50%', bottom: 28, transform: 'translateX(-50%)',
          display: 'flex', alignItems: 'center', gap: 12, maxWidth: 560,
          padding: '10px 16px', borderRadius: 10, zIndex: 10,
          background: colors.bgDeeper, border: `1px solid ${colors.danger}66`,
          fontFamily: font.body, fontSize: 12, color: colors.text,
        }}>
          <span style={{ color: colors.danger }}>
            Couldn't save your setup ({saveError}).
          </span>
          <button
            onClick={handleComplete}
            disabled={saving}
            style={{
              fontFamily: font.body, fontSize: 12, fontWeight: 600, color: colors.cyan,
              background: 'none', border: `1px solid ${colors.borderHi}`, borderRadius: 8,
              padding: '4px 12px', cursor: saving ? 'default' : 'pointer', flexShrink: 0,
            }}
          >{saving ? 'Retrying…' : 'Retry'}</button>
          <button
            onClick={() => { stashWizardIntent(intent); onComplete(); }}
            title="Enter the app anyway — your persona choices may not be saved and setup may reappear next launch"
            style={{
              fontFamily: font.body, fontSize: 12, color: colors.textMuted,
              background: 'none', border: 'none', cursor: 'pointer',
              textDecoration: 'underline', flexShrink: 0,
            }}
          >Continue anyway</button>
        </div>
      )}
    </div>
  );
}
