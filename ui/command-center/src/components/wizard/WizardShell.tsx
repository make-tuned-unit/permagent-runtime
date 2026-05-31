import { useState } from 'react';
import { ease } from '../../styles/tokens';
import { ProgressDots, BackChevron } from './atoms';
import { MomentWelcome } from './MomentWelcome';
import { MomentCalibration } from './MomentCalibration';
import { MomentIntent } from './MomentIntent';
import { MomentMeet } from './MomentMeet';
import { MomentChat } from './MomentChat';
import { api, apiFetch } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';

interface Persona {
  name: string;
  traits: string[];
  tone: string;
  greeting: string;
}

interface Props {
  onComplete: () => void;
}

export function WizardShell({ onComplete }: Props) {
  const { colors } = useTheme();
  const [step, setStep] = useState(0);
  const [intent, setIntent] = useState('');
  const [persona, setPersona] = useState<Persona>({
    name: '', traits: [], tone: '', greeting: '',
  });

  const back = () => setStep(s => Math.max(0, s - 1));

  const handleProviderDone = (_provider: string, _key: string) => {
    setStep(1);
  };

  const handleCalibrationDone = (traits: string[], tone: string) => {
    setPersona(p => ({
      ...p, traits, tone,
      greeting: `Hello! I'm ready to help. ${tone.split('.')[0]}.`,
    }));
    setStep(2);
  };

  const handleIntentDone = () => {
    if (!persona.name) {
      setPersona(p => ({ ...p, name: 'Henry' }));
    }
    setStep(3);
  };

  const handleMeetDone = () => {
    setStep(4);
  };

  const handleComplete = async () => {
    try {
      // Save persona to backend
      const body = {
        first_name: persona.name,
        last_name: null,
        nickname: null,
        traits: persona.traits,
        tone: persona.tone,
        opening_greeting: persona.greeting,
        voice_id: null,
      };
      await apiFetch<unknown>('/api/agent/identity', {
        method: 'PUT',
        body: JSON.stringify(body),
      });
      // Mark wizard complete
      await api.upsertConfig('wizard_complete', true);
    } catch (e) {
      console.error('Failed to save persona:', e);
    }
    onComplete();
  };

  const moments = [
    <MomentWelcome key="welcome" onAdvance={handleProviderDone} />,
    <MomentCalibration key="calibration" onAdvance={handleCalibrationDone} onBack={back} />,
    <MomentIntent key="intent" intent={intent} setIntent={setIntent} onAdvance={handleIntentDone} onBack={back} />,
    <MomentMeet key="meet" persona={persona} setPersona={setPersona} onAdvance={handleMeetDone} onBack={back} />,
    <MomentChat key="chat" persona={persona} onComplete={handleComplete} />,
  ];

  return (
    <div style={{
      position: 'fixed', inset: 0,
      background: `radial-gradient(ellipse 80% 60% at 50% 40%, rgba(0,213,255,0.06) 0%, ${colors.bg} 70%)`,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      {/* Top bar: back + dots */}
      {step > 0 && step < 4 && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '16px 20px' }}>
          <BackChevron onClick={back} />
          <ProgressDots count={4} current={step - 1} />
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
    </div>
  );
}
