/**
 * @vitest-environment jsdom
 *
 * NotesPanel collapse + copy: long notes must not render their body until the
 * title row is expanded (the infinite-scroll fix), and the copy button puts
 * the full note (title + body, the note_memory_content shape) on the
 * clipboard for pasting into a coding agent.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { listProjectNotes } = vi.hoisted(() => ({
  listProjectNotes: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { listProjectNotes },
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { NotesPanel } from './NotesPanel';
import { useCommandCenter } from '../../lib/store';
import type { Project, ProjectNote } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const project = { id: 'p1', name: 'Proj' } as Project;

const note: ProjectNote = {
  id: 'n1',
  project_id: 'p1',
  title: 'Design decisions',
  body: 'A very long note body that used to stretch the panel forever.',
  memory_key: null,
  created_at: '2026-08-10T12:00:00Z',
  updated_at: '2026-08-10T12:00:00Z',
};

const untitled: ProjectNote = {
  ...note,
  id: 'n2',
  title: null,
  body: 'First line becomes the label\nrest of the body',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  listProjectNotes.mockReset().mockResolvedValue([note, untitled]);
  useCommandCenter.setState({ projectsRev: 0 });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(ui: React.ReactElement) {
  await act(async () => root.render(ui));
}

function titleRow(text: string): HTMLElement {
  const row = Array.from(container.querySelectorAll<HTMLElement>('[role="button"]'))
    .find(el => el.textContent?.includes(text));
  if (!row) throw new Error(`no title row containing "${text}"`);
  return row;
}

describe('NotesPanel collapsed notes', () => {
  it('hides the body until the title row is clicked, then hides again on re-click', async () => {
    await render(<NotesPanel project={project} />);
    expect(container.textContent).toContain('Design decisions');
    expect(container.textContent).not.toContain(note.body);

    await act(async () => { titleRow('Design decisions').click(); });
    expect(container.textContent).toContain(note.body);

    await act(async () => { titleRow('Design decisions').click(); });
    expect(container.textContent).not.toContain(note.body);
  });

  it('labels an untitled note with its first line', async () => {
    await render(<NotesPanel project={project} />);
    expect(container.textContent).toContain('First line becomes the label');
    expect(container.textContent).not.toContain('rest of the body');
  });

  it('copies title + body via the copy button without expanding the note', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });

    await render(<NotesPanel project={project} />);
    const copyBtn = Array.from(container.querySelectorAll<HTMLElement>('[aria-label="Copy note"]'))[0];
    await act(async () => { copyBtn.click(); });

    expect(writeText).toHaveBeenCalledWith(`Design decisions\n\n${note.body}`);
    expect(container.textContent).not.toContain(note.body); // still collapsed
  });
});
