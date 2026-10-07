/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('../shared/agentStatus', () => ({ useAgentRuntimeStates: () => [{ id: 'henry', source: 'sim', hudState: 'idle' }] }));
vi.mock('../shared/worldEvents', () => ({ subscribeWorldEvents: () => () => {} }));
vi.mock('../../../lib/store', () => ({ useCommandCenter: (selector: (state: { daemonAuthError: boolean }) => unknown) => selector({ daemonAuthError: false }) }));
vi.mock('../../../styles/useTheme', () => ({ useTheme: () => ({ colors: { cyan: '#9fe', borderHi: '#9fe', cyanSoft: '#123', textMuted: '#789' } }) }));
vi.mock('./HeroAgentPortrait', () => ({ HeroAgentPortrait: ({ id, size }: { id: string; size: string }) => <span data-testid="dock-live-portrait">{id}:{size}</span> }));
vi.mock('./habitatMotion', () => ({ commandHabitatAgent: vi.fn(() => true), stopHabitatAgent: vi.fn(), useHabitatTravel: () => null }));

import { HabitatAgentDock } from './HabitatAgentDock';

let root: Root;
let host: HTMLDivElement;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

it('uses the current hero portrait renderer for the 28 px collaborator emblem', () => {
  act(() => root.render(<HabitatAgentDock
    selected="henry"
    name="Your orchestrator"
    directing={false}
    following={false}
    onSelect={vi.fn()}
    onDirect={vi.fn()}
    onFollow={vi.fn()}
    onInspect={vi.fn()}
    onBrief={vi.fn()}
    onConversation={vi.fn()}
  />));
  expect(host.querySelector('.habitat-agent-emblem [data-testid="dock-live-portrait"]')?.textContent).toBe('henry:dock');
});
