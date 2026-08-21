/** @vitest-environment jsdom */
/**
 * Run enrichment must actually run: send a message to the live session with
 * the person's entity_uuid, never copy a prompt to the clipboard.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { buildEnrichmentMessage, PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { Person } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const person: Person = {
  entity_uuid: 'uuid-ex',
  canonical_id: 'person:example-person',
  display_name: 'Example Person',
  role: 'Director of Sales',
  company: 'Example Coworking',
  email: null,
  phone: null,
  notes: null,
  last_contact_at: null,
  birthday: null,
  relationship_strength: null,
  how_met: null,
  linkedin: null,
  x_handle: null,
  facebook: null,
  instagram: null,
  personal_site: null,
  photo_url: null,
  find_online_hints: 'Halifax coworking, director of sales',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

let container: HTMLDivElement;
let root: Root;
const sendMessage = vi.fn(async () => {});
const openChatDock = vi.fn();
const writeText = vi.fn();

beforeEach(() => {
  apiFetch.mockReset().mockResolvedValue([]);
  sendMessage.mockClear();
  openChatDock.mockClear();
  writeText.mockReset();
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  useCommandCenter.setState({ sendMessage, openChatDock });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('buildEnrichmentMessage', () => {
  it('asks the agent to run by entity_uuid, including find-online hints', () => {
    const msg = buildEnrichmentMessage(person);
    expect(msg).toContain('uuid-ex');
    expect(msg).toContain('Halifax coworking, director of sales');
    expect(msg).toContain('run it now');
    expect(msg.toLowerCase()).not.toContain('paste');
    expect(msg.toLowerCase()).not.toContain('clipboard');
  });
});

describe('PersonDetailModal run enrichment', () => {
  it('sends the enrichment to chat instead of copying a prompt', async () => {
    await act(async () => root.render(
      <PersonDetailModal projectId={null} person={person} onClose={() => {}} />,
    ));
    const btn = [...container.querySelectorAll('button')].find(b => b.textContent === 'Run enrichment');
    expect(btn).toBeTruthy();
    expect(container.textContent).not.toContain('Prompt copied');
    await act(async () => btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(writeText).not.toHaveBeenCalled();
    expect(openChatDock).toHaveBeenCalled();
    expect(sendMessage).toHaveBeenCalledTimes(1);
    expect(sendMessage).toHaveBeenCalledWith(buildEnrichmentMessage(person));
  });
});
