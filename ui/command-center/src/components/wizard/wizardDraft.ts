/**
 * Resume state for first-run setup.
 *
 * Settings the steps already wrote (provider, key, folders, search, memory
 * model) live in the backend and are re-read on entry. What only the wizard
 * holds until the end — where the person was, the persona and the first
 * message — is kept here so quitting halfway resumes at the same step instead
 * of starting over. Cleared when setup completes.
 */

export const DRAFT_KEY = 'permagent-wizard-draft-v1';

export type ModelChoice = 'hosted' | 'local' | 'later';

export interface WizardDraft {
  step: number;
  name: string;
  toneId: string;
  firstMessage: string;
  modelChoice: ModelChoice | null;
}

export const EMPTY_DRAFT: WizardDraft = {
  step: 0, name: '', toneId: '', firstMessage: '', modelChoice: null,
};

const CHOICES: readonly ModelChoice[] = ['hosted', 'local', 'later'];

/** Read the draft; anything malformed is ignored field by field. */
export function loadDraft(stepCount: number): WizardDraft {
  let raw: unknown = null;
  try {
    const text = localStorage.getItem(DRAFT_KEY);
    raw = text ? JSON.parse(text) : null;
  } catch {
    raw = null;
  }
  if (!raw || typeof raw !== 'object') return { ...EMPTY_DRAFT };
  const r = raw as Record<string, unknown>;
  const step = typeof r.step === 'number' && Number.isInteger(r.step)
    ? Math.min(Math.max(r.step, 0), stepCount - 1)
    : 0;
  return {
    step,
    name: typeof r.name === 'string' ? r.name : '',
    toneId: typeof r.toneId === 'string' ? r.toneId : '',
    firstMessage: typeof r.firstMessage === 'string' ? r.firstMessage : '',
    modelChoice: CHOICES.includes(r.modelChoice as ModelChoice) ? r.modelChoice as ModelChoice : null,
  };
}

export function saveDraft(draft: WizardDraft): void {
  try {
    localStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
  } catch {
    /* storage unavailable — resume is a convenience, setup still works */
  }
}

export function clearDraft(): void {
  try {
    localStorage.removeItem(DRAFT_KEY);
  } catch {
    /* nothing to clear */
  }
}
