/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { copyText } from './clipboard';

function setClipboard(value: unknown) {
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value });
}

afterEach(() => {
  setClipboard(undefined);
  delete (document as unknown as Record<string, unknown>).execCommand;
});

describe('copyText', () => {
  it('uses the async Clipboard API when the context is secure enough to have one', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText });
    expect(await copyText('payload')).toBe(true);
    expect(writeText).toHaveBeenCalledWith('payload');
  });

  it('falls back to a selection copy when navigator.clipboard is absent', async () => {
    // The paired-device case: the same bundle served over plain HTTP on the LAN
    // has no navigator.clipboard at all, and every direct
    // navigator.clipboard.writeText call in the app throws into a dead promise.
    setClipboard(undefined);
    const execCommand = vi.fn().mockReturnValue(true);
    (document as unknown as Record<string, unknown>).execCommand = execCommand;

    expect(await copyText('payload')).toBe(true);
    expect(execCommand).toHaveBeenCalledWith('copy');
    // No stray textarea left behind.
    expect(document.querySelectorAll('textarea')).toHaveLength(0);
  });

  it('falls back when writeText rejects (denied permission, unfocused document)', async () => {
    setClipboard({ writeText: vi.fn().mockRejectedValue(new Error('NotAllowedError')) });
    (document as unknown as Record<string, unknown>).execCommand = vi.fn().mockReturnValue(true);
    expect(await copyText('payload')).toBe(true);
  });

  it('reports failure instead of pretending when nothing works', async () => {
    setClipboard(undefined);
    (document as unknown as Record<string, unknown>).execCommand = vi.fn().mockReturnValue(false);
    expect(await copyText('payload')).toBe(false);
  });

  it('reports failure rather than throwing when execCommand itself blows up', async () => {
    setClipboard(undefined);
    (document as unknown as Record<string, unknown>).execCommand = vi.fn(() => { throw new Error('nope'); });
    await expect(copyText('payload')).resolves.toBe(false);
  });

  it('refuses an empty copy — a silent no-op that looks like success is the bug', async () => {
    const writeText = vi.fn();
    setClipboard({ writeText });
    expect(await copyText('')).toBe(false);
    expect(writeText).not.toHaveBeenCalled();
  });
});
