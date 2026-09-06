// @vitest-environment jsdom
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../styles/useTheme', () => ({
  useTheme: () => ({
    reduceMotion: true,
    colors: {
      bg: '#0b1020', bgDeeper: '#070b16', text: '#fff', textMuted: '#aaa',
      textDim: '#777', cyan: '#0ff', cyanSoft: '#033', borderHi: '#555',
      danger: '#f55',
    },
  }),
}));

vi.mock('../common/Button', () => ({
  Button: ({ children, onClick, ...props }: { children?: ReactNode; onClick?: () => void }) => (
    <button onClick={onClick} {...props}>{children}</button>
  ),
}));

vi.mock('./atoms', () => ({
  BackChevron: ({ onClick }: { onClick: () => void }) => <button aria-label="Back" onClick={onClick}>Back</button>,
  ProgressDots: ({ current }: { current: number }) => <div data-testid="progress">{current}</div>,
}));

vi.mock('../../lib/api', () => ({
  api: { upsertConfig: vi.fn() },
  apiFetch: vi.fn(),
}));
vi.mock('../../lib/wizardIntent', () => ({ stashWizardIntent: vi.fn() }));

type Advance = (...args: unknown[]) => void;

vi.mock('./MomentWelcome', () => ({
  MomentWelcome: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="welcome" data-active={String(active)} onClick={() => onAdvance('ollama', '')}>Welcome</button>
  ),
}));
vi.mock('./MomentHardware', () => ({
  MomentHardware: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="hardware" data-active={String(active)} onClick={onAdvance}>Hardware</button>
  ),
}));
vi.mock('./MomentCalibration', () => ({
  MomentCalibration: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="calibration" data-active={String(active)} onClick={() => onAdvance(['precise'], 'Direct')}>Calibration</button>
  ),
}));
vi.mock('./MomentIntent', () => ({
  MomentIntent: ({ active, intent, setIntent, onAdvance }: { active: boolean; intent: string; setIntent: (value: string) => void; onAdvance: Advance }) => (
    <div data-testid="intent" data-active={String(active)}>
      <input aria-label="intent" value={intent} onChange={event => setIntent(event.target.value)} />
      <button onClick={onAdvance}>Intent next</button>
    </div>
  ),
}));
vi.mock('./MomentCode', () => ({
  MomentCode: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="code" data-active={String(active)} onClick={onAdvance}>Code</button>
  ),
}));
vi.mock('./MomentMeet', () => ({
  MomentMeet: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="meet" data-active={String(active)} onClick={onAdvance}>Meet</button>
  ),
}));
vi.mock('./MomentWebSearch', () => ({
  MomentWebSearch: ({ active, onAdvance }: { active: boolean; onAdvance: Advance }) => (
    <button data-testid="web" data-active={String(active)} onClick={onAdvance}>Web</button>
  ),
}));
vi.mock('./MomentChat', () => ({
  MomentChat: ({ active }: { active: boolean }) => <div data-testid="chat" data-active={String(active)}>Chat</div>,
}));

import { WizardShell } from './WizardShell';

let root: Root | null = null;
let container: HTMLDivElement | null = null;

afterEach(() => {
  if (root) act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

function renderShell() {
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
  act(() => root?.render(<WizardShell onComplete={vi.fn()} />));
}

function click(selector: string) {
  const element = container?.querySelector(selector) as HTMLElement | null;
  expect(element).not.toBeNull();
  act(() => element?.click());
}

describe('WizardShell lifecycle boundaries', () => {
  it('marks only the current Moment active and removes inactive steps from interaction', () => {
    renderShell();

    expect(container?.querySelector('[data-testid="welcome"]')?.getAttribute('data-active')).toBe('true');
    for (const id of ['hardware', 'calibration', 'intent', 'code', 'meet', 'web', 'chat']) {
      expect(container?.querySelector(`[data-testid="${id}"]`)?.getAttribute('data-active')).toBe('false');
    }
    const hidden = container?.querySelector('[aria-hidden="true"]');
    expect(hidden?.getAttribute('inert')).toBe('');
    expect(hidden?.getAttribute('style')).toContain('visibility: hidden');

    click('[data-testid="welcome"]');
    expect(container?.querySelector('[data-testid="hardware"]')?.getAttribute('data-active')).toBe('true');
    expect(container?.querySelector('[aria-hidden="true"] [data-testid="welcome"]')).not.toBeNull();
  });

  it('keeps non-secret intent state when navigating forward and back', () => {
    renderShell();
    click('[data-testid="welcome"]');
    click('[data-testid="hardware"]');
    click('[data-testid="calibration"]');

    const intent = container?.querySelector('input[aria-label="intent"]') as HTMLInputElement;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(intent, 'Build a useful first workflow');
      intent.dispatchEvent(new Event('input', { bubbles: true }));
      intent.dispatchEvent(new Event('change', { bubbles: true }));
    });
    // React's controlled input receives the value through its change handler in
    // the same way a keyboard edit does; advance only after that event.
    click('[data-testid="intent"] button');
    click('button[aria-label="Back"]');
    expect((container?.querySelector('input[aria-label="intent"]') as HTMLInputElement).value)
      .toBe('Build a useful first workflow');
  });
});
