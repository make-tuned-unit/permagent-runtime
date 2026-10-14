/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const m = vi.hoisted(() => ({
  getPronunciations: vi.fn(),
  getUnknownWords: vi.fn(),
  savePronunciation: vi.fn(),
  dismissUnknownWord: vi.fn(),
  deletePronunciation: vi.fn(),
}));
vi.mock('../../lib/api', () => ({ api: m, getApiBaseUrl: () => '' }));

import { PronunciationSection } from './PronunciationSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const flush = async () => { await act(async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); }); };
const btn = (label: string) => Array.from(container.querySelectorAll('button')).find(b => b.textContent?.trim() === label)!;
const word = (over = {}) => ({
  word: 'upneen', display: 'Upneen', count: 3, first_seen_ms: 1, last_seen_ms: 2,
  sentence: 'Upneen shipped today.', substitute_ipa: null, ...over,
});

beforeEach(() => {
  Object.values(m).forEach(f => f.mockReset());
  m.getPronunciations.mockResolvedValue({});
  m.getUnknownWords.mockResolvedValue({ words: [], max_words: 200 });
  m.savePronunciation.mockResolvedValue({ saved: true, total: 1, ipa: '' });
  m.dismissUnknownWord.mockResolvedValue({ removed: true });
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('unknown words review', () => {
  it('shows plain copy when there is nothing to check', async () => {
    act(() => root.render(<PronunciationSection />)); await flush();
    expect(container.textContent).toContain('Nothing to check');
    expect(btn('Dismiss')).toBeUndefined();
  });

  it('lists each word with its count and sentence', async () => {
    m.getUnknownWords.mockResolvedValue({ words: [word(), word({ word: 'qwen', display: 'Qwen', count: 1, sentence: 'Qwen is local.' })], max_words: 200 });
    act(() => root.render(<PronunciationSection />)); await flush();
    expect(container.textContent).toContain('Upneen');
    expect(container.textContent).toContain('said 3 times');
    expect(container.textContent).toContain('Upneen shipped today.');
    expect(container.textContent).toContain('said once');
  });

  it('Dismiss calls the server with the word and reloads', async () => {
    m.getUnknownWords.mockResolvedValueOnce({ words: [word()], max_words: 200 }).mockResolvedValue({ words: [], max_words: 200 });
    act(() => root.render(<PronunciationSection />)); await flush();
    await act(async () => { btn('Dismiss').click(); }); await flush();
    expect(m.dismissUnknownWord).toHaveBeenCalledWith('upneen');
    expect(container.textContent).toContain('Nothing to check');
  });

  it('Set pronunciation saves through the existing pronunciations call', async () => {
    m.getUnknownWords.mockResolvedValueOnce({ words: [word()], max_words: 200 }).mockResolvedValue({ words: [], max_words: 200 });
    act(() => root.render(<PronunciationSection />)); await flush();
    act(() => btn('Set pronunciation').click());
    const input = Array.from(container.querySelectorAll('input')).filter(i => i.placeholder === 'sounds like').pop()!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    act(() => { setter.call(input, 'up neen'); input.dispatchEvent(new Event('input', { bubbles: true })); });
    await act(async () => { Array.from(container.querySelectorAll('button')).filter(b => b.textContent?.trim() === 'Save').pop()!.click(); }); await flush();
    expect(m.savePronunciation).toHaveBeenCalledWith('upneen', 'up neen');
  });
});
