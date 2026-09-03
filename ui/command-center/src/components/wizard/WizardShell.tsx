import { useState, type CSSProperties } from 'react';
import { duration, ease, radius, space, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { ProgressDots, BackChevron } from './atoms';
import { MomentWelcome } from './MomentWelcome';
import { MomentHardware } from './MomentHardware';
import { MomentCalibration } from './MomentCalibration';
import { MomentIntent } from './MomentIntent';
import { MomentCode } from './MomentCode';
import { MomentMeet } from './MomentMeet';
import { MomentWebSearch } from './MomentWebSearch';
import { MomentChat } from './MomentChat';
import { api, apiFetch } from '../../lib/api';
import { stashWizardIntent } from '../../lib/wizardIntent';
import { font, space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

import { Tooltip } from '../common/Tooltip';
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
  const { colors, reduceMotion } = useTheme();
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

  // Where the user keeps their code. Asked rather than guessed: four features
  // independently assumed `~/dev` and all four failed by finding NOTHING, which
  // is indistinguishable from a clean machine. See config::dev_roots.
  const handleCodeDone = () => {
    setStep(5);
  };

  const handleMeetDone = () => {
    setStep(6);
  };

  const handleWebSearchDone = () => {
    setStep(7);
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
      // `false` is the Button primitive's "it failed": the retry must not tick
      // success over the error banner it is standing in.
      return false;
    }
    // Hand the stated intent to the first chat composer (one-shot).
    stashWizardIntent(intent);
    setSaving(false);
    onComplete();
    return true;
  };

  const moments = [
    <MomentWelcome key="welcome" onAdvance={handleProviderDone} />,
    <MomentHardware key="hardware" onAdvance={handleHardwareDone} onBack={back} />,
    <MomentCalibration key="calibration" onAdvance={handleCalibrationDone} onBack={back} />,
    <MomentIntent key="intent" intent={intent} setIntent={setIntent} onAdvance={handleIntentDone} onBack={back} />,
    <MomentCode key="code" personaName={persona.name} onAdvance={handleCodeDone} onBack={back} />,
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
      {step > 0 && step < 7 && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: `${space.xxl}px ${space.xxxl}px` }}>
          <BackChevron onClick={back} />
          {/* Dots track the interior config steps (1–6); step 0 (welcome) and
              the final chat step show no bar. Keep `count` equal to the number
              of interior steps: an earlier version left a dot that `current`
              (max step-1) could never light. */}
          <ProgressDots count={6} current={step - 1} />
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
            transition: reduceMotion ? 'none' : `opacity ${duration.slow}ms ${ease.out}`,
          }}>
            {moment}
          </div>
        ))}
      </div>

      {/* Save failure: honest, recoverable — never a silent loss. */}
      {saveError && (
        <div style={{
          position: 'absolute', left: '50%', bottom: 28, transform: 'translateX(-50%)',
          display: 'flex', alignItems: 'center', gap: space.xl, maxWidth: 560,
          padding: `${space.lg}px ${space.xxl}px`, borderRadius: 10, zIndex: 10,
          background: colors.bgDeeper, border: `1px solid ${colors.danger}66`,
          fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
        }}>
          <span style={{ color: colors.danger }}>
            Couldn't save your setup ({saveError}).
          </span>
          <Button
            colors={colors}
            variant="ghostOn"
            type="button"
            onClick={handleComplete}
            disabled={saving}
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '4px 12px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 600,
              fontFamily: font.body, fontSize: textSize.caption, lineHeight: 1.5, flexShrink: 0,
            } as CSSProperties}
          >{saving ? 'Retrying…' : 'Retry'}</Button>
          <Tooltip content="Enter the app anyway — your persona choices may not be saved and setup may reappear next launch">
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => { stashWizardIntent(intent); onComplete(); }}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': '0',
                '--pa-btn-weight': 400,
                fontFamily: font.body, fontSize: textSize.caption, lineHeight: 1.5,
                textDecoration: 'underline', flexShrink: 0,
              } as CSSProperties}
            >Continue anyway</Button>
          </Tooltip>
        </div>
      )}
    </div>
  );
}
