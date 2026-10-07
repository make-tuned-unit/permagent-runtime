import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { TITLEBAR_HEIGHT } from '../../lib/windowChrome';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import { api, apiFetch } from '../../lib/api';
import { stashWizardIntent } from '../../lib/wizardIntent';
import { BackChevron, StepProgress } from './atoms';
import { StepWelcome } from './StepWelcome';
import { StepModel } from './StepModel';
import { StepAgent } from './StepAgent';
import { StepFolders } from './StepFolders';
import { StepExtras } from './StepExtras';
import { StepReview } from './StepReview';
import { DEFAULT_TONE_ID, FALLBACK_AGENT_NAME, effectiveName, greetingFor, toneById } from './persona';
import { clearDraft, loadDraft, saveDraft, type ModelChoice } from './wizardDraft';
import { safeWizardError } from './wizardErrors';

export const STEP_LABELS = ['Welcome', 'Model', 'Agent', 'Code folders', 'Extras', 'Review'] as const;
const STEP = { welcome: 0, model: 1, agent: 2, folders: 3, extras: 4, review: 5 } as const;

interface Props {
  onComplete: () => void;
}

interface IdentityKeep {
  first_name?: string;
  last_name: string | null;
  nickname: string | null;
  voice_id: string | null;
}

/**
 * First-run setup.
 *
 * Six steps, one mounted at a time. Choices that only the wizard holds (where
 * the person is, persona, first message) are kept in a local draft so quitting
 * resumes at the same step; everything else is written by its own step and
 * re-read by the review. Escape is Back; "Change" on the review returns to the
 * review once that step is done. Nothing advances on its own.
 */
export function WizardShell({ onComplete }: Props) {
  const { colors } = useTheme();
  const [initial] = useState(() => loadDraft(STEP_LABELS.length));
  const [step, setStep] = useState(initial.step);
  const [name, setName] = useState(initial.name);
  const [toneId, setToneId] = useState(initial.toneId || DEFAULT_TONE_ID);
  const [firstMessage, setFirstMessage] = useState(initial.firstMessage);
  const [modelChoice, setModelChoice] = useState<ModelChoice | null>(initial.modelChoice);
  const [returnToReview, setReturnToReview] = useState(false);
  const [identity, setIdentity] = useState<IdentityKeep | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    saveDraft({ step, name, toneId, firstMessage, modelChoice });
  }, [step, name, toneId, firstMessage, modelChoice]);

  // The backend's current persona: its name is the default when the field is
  // left blank, and the fields this flow does not ask about (voice, surname,
  // nickname) are carried through instead of being wiped by the PUT.
  useEffect(() => {
    let alive = true;
    apiFetch<IdentityKeep>('/api/agent/identity')
      .then(r => { if (alive && r) setIdentity(r); })
      .catch(() => { /* fall back to the documented default name */ });
    return () => { alive = false; };
  }, []);
  const defaultName = identity?.first_name?.trim() || FALLBACK_AGENT_NAME;
  const agentName = effectiveName(name, defaultName);

  const go = useCallback((next: number) => {
    setSaveError(null);
    setStep(Math.min(Math.max(next, 0), STEP_LABELS.length - 1));
  }, []);
  const next = () => {
    if (returnToReview) { setReturnToReview(false); go(STEP.review); return; }
    go(step + 1);
  };
  const back = () => { setReturnToReview(false); go(step - 1); };
  const edit = (target: number) => { setReturnToReview(true); go(target); };

  const savingRef = useRef(false);
  const handleComplete = async () => {
    if (savingRef.current) return false;
    savingRef.current = true;
    setSaving(true);
    setSaveError(null);
    const tone = toneById(toneId);
    try {
      await apiFetch<unknown>('/api/agent/identity', {
        method: 'PUT',
        body: JSON.stringify({
          first_name: agentName,
          last_name: identity?.last_name ?? null,
          nickname: identity?.nickname ?? null,
          traits: tone.traits,
          tone: tone.tone,
          opening_greeting: greetingFor(agentName, tone.id),
          voice_id: identity?.voice_id ?? null,
        }),
      });
      await api.upsertConfig('wizard_complete', true);
    } catch (e) {
      // Never a silent loss: the banner below offers Retry or Continue anyway.
      console.error('Failed to finish setup:', safeWizardError(e, 'setup save failed'));
      setSaveError(e instanceof Error ? e.message : String(e));
      setSaving(false);
      savingRef.current = false;
      return false;
    }
    stashWizardIntent(firstMessage);
    clearDraft();
    setSaving(false);
    savingRef.current = false;
    onComplete();
    return true;
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Escape' || e.defaultPrevented || step === 0 || saving) return;
    e.preventDefault();
    back();
  };

  let body;
  switch (step) {
    case STEP.welcome:
      body = <StepWelcome onNext={next} />;
      break;
    case STEP.model:
      body = <StepModel initialChoice={modelChoice} onDone={choice => { setModelChoice(choice); next(); }} />;
      break;
    case STEP.agent:
      body = (
        <StepAgent name={name} toneId={toneId} defaultName={defaultName}
          onChange={v => { setName(v.name); setToneId(v.toneId); }} onNext={next} />
      );
      break;
    case STEP.folders:
      body = <StepFolders agentName={agentName} onNext={next} />;
      break;
    case STEP.extras:
      body = <StepExtras agentName={agentName} onNext={next} />;
      break;
    default:
      body = (
        <StepReview
          agentName={agentName}
          toneLabel={toneById(toneId).label}
          firstMessage={firstMessage}
          onFirstMessage={setFirstMessage}
          onEdit={edit}
          steps={{ model: STEP.model, agent: STEP.agent, folders: STEP.folders, extras: STEP.extras }}
          onStart={handleComplete}
          saving={saving}
        />
      );
  }

  return (
    <div
      onKeyDown={onKeyDown}
      data-testid="wizard-shell"
      style={{
        position: 'fixed', inset: 0,
        background: `radial-gradient(ellipse 80% 60% at 50% 30%, ${colors.cyanWash} 0%, ${colors.bg} 70%)`,
        display: 'flex', flexDirection: 'column',
        color: colors.text,
      }}
    >
      <div data-tauri-drag-region style={{ height: TITLEBAR_HEIGHT, flexShrink: 0 }} />
      <header style={{
        flexShrink: 0, display: 'grid', gridTemplateColumns: '1fr auto 1fr', alignItems: 'center',
        padding: `0 ${space.huge}px ${space.md}px`,
      }}>
        <div style={{ visibility: step > 0 ? 'visible' : 'hidden' }}>
          <BackChevron onClick={back} disabled={saving} />
        </div>
        <StepProgress labels={STEP_LABELS} current={step} />
        <div />
      </header>

      <main key={step} className="pa-enter" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        {body}
      </main>

      {saveError && (
        <div role="alert" style={{
          position: 'absolute', left: '50%', bottom: space.huge * 3, transform: 'translateX(-50%)',
          display: 'flex', alignItems: 'center', gap: space.xl, width: `min(600px, calc(100% - ${space.huge * 2}px))`,
          padding: `${space.lg}px ${space.xxl}px`, borderRadius: radius.lg, zIndex: 10,
          background: colors.bgDeeper, border: `1px solid ${colors.danger}`, boxShadow: colors.elevationFloating,
          fontFamily: font.body, fontSize: textSize.small, color: colors.text,
        }}>
          <span style={{ color: colors.danger, flex: 1, minWidth: 0 }}>
            Couldn't save your setup ({saveError}).
          </span>
          <Button
            colors={colors}
            variant="ghostOn"
            type="button"
            onClick={handleComplete}
            disabled={saving}
            onErrorFeedback={false}
            style={{
              '--pa-btn-pad': `${space.xs}px ${space.xl}px`,
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 600,
              fontFamily: font.body, fontSize: textSize.small, flexShrink: 0,
            } as CSSProperties}
          >{saving ? 'Retrying…' : 'Retry'}</Button>
          <Tooltip content="Open the app now. Your agent's name and tone may not be saved, and setup will show again next launch.">
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => { stashWizardIntent(firstMessage); onComplete(); }}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': '0',
                '--pa-btn-weight': 400,
                fontFamily: font.body, fontSize: textSize.small,
                textDecoration: 'underline', flexShrink: 0,
              } as CSSProperties}
            >Continue anyway</Button>
          </Tooltip>
        </div>
      )}
    </div>
  );
}
