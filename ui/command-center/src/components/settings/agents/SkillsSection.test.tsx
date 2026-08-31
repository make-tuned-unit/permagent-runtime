/**
 * @vitest-environment jsdom
 *
 * The Skills Library's front door (J4). Pins the two things that make it a
 * door rather than a decoration: it opens the Library, and it says what is
 * behind it — including when the answer is "nothing yet", which must read as
 * an explanation rather than as a failed load.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const store = vi.hoisted(() => ({
  skills: [] as Array<{ id: string; name: string; description: string }>,
  skillsLoading: false,
  loadSkills: vi.fn(),
  setActivePanel: vi.fn(),
}));

vi.mock('../../../lib/store', () => {
  const state = () => ({
    skills: store.skills,
    skillsLoading: store.skillsLoading,
    loadSkills: store.loadSkills,
    setActivePanel: store.setActivePanel,
  });
  return {
    useCommandCenter: Object.assign(
      (selector: (s: Record<string, unknown>) => unknown) => selector(state()),
      { getState: state },
    ),
  };
});

import { SkillsSection } from './SkillsSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;

function render() {
  act(() => { root.render(<SkillsSection />); });
}

beforeEach(() => {
  store.skills = [];
  store.skillsLoading = false;
  store.loadSkills.mockClear();
  store.setActivePanel.mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('Settings → Agents → Skills', () => {
  it('opens the Skills Library', () => {
    render();
    const btn = host.querySelector<HTMLButtonElement>('[data-testid="open-skills-library"]');
    expect(btn).not.toBeNull();
    act(() => { btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(store.setActivePanel).toHaveBeenCalledWith('skills');
  });

  it('loads the library so the count it states is a real one', () => {
    render();
    expect(store.loadSkills).toHaveBeenCalled();
  });

  it('states the count when there are skills', () => {
    store.skills = [
      { id: 'a', name: 'A', description: '' },
      { id: 'b', name: 'B', description: '' },
    ];
    render();
    expect(host.querySelector('[data-testid="skills-count"]')!.textContent)
      .toContain('2 learned skills');
  });

  it('explains an empty library instead of leaving a bare door', () => {
    render();
    const text = host.querySelector('[data-testid="skills-count"]')!.textContent ?? '';
    expect(text).toContain('Nothing learned yet');
    // Empty is not a failure, and it is not silence either: it says how the
    // library gets filled.
    expect(text.toLowerCase()).toContain('proposed');
  });
});
