/**
 * Tests for drop-to-CC-terminal path formatting (#557). Pins that dropped
 * paths are injected shell-safely (spaces/metacharacters quoted) with a
 * trailing space and no newline (path is inserted, prompt not submitted).
 */

import { describe, expect, it } from 'vitest';
import { shellQuotePath, formatDroppedPathsForInput, resolvePtyInjection } from './terminalDrop';

describe('shellQuotePath', () => {
  it('leaves a plain path unquoted', () => {
    expect(shellQuotePath('/Users/jesse/dev/app/main.rs')).toBe('/Users/jesse/dev/app/main.rs');
  });

  it('single-quotes a path containing spaces', () => {
    expect(shellQuotePath('/Users/jesse/My Documents/notes.md'))
      .toBe(`'/Users/jesse/My Documents/notes.md'`);
  });

  it('escapes embedded single quotes the POSIX way', () => {
    expect(shellQuotePath("/tmp/it's a file.txt")).toBe(`'/tmp/it'\\''s a file.txt'`);
  });

  it('quotes shell metacharacters so they are not interpreted', () => {
    expect(shellQuotePath('/tmp/$(rm -rf ~).txt')).toBe(`'/tmp/$(rm -rf ~).txt'`);
    expect(shellQuotePath('/tmp/a b`c`.txt')).toBe(`'/tmp/a b\`c\`.txt'`);
  });
});

describe('formatDroppedPathsForInput', () => {
  it('appends a trailing space and no newline for a single path', () => {
    const out = formatDroppedPathsForInput(['/tmp/a.txt']);
    expect(out).toBe('/tmp/a.txt ');
    expect(out.endsWith('\n')).toBe(false);
  });

  it('space-separates multiple paths, each independently quoted', () => {
    expect(formatDroppedPathsForInput(['/tmp/a.txt', '/tmp/b c.txt']))
      .toBe(`/tmp/a.txt '/tmp/b c.txt' `);
  });

  it('returns empty string for no paths', () => {
    expect(formatDroppedPathsForInput([])).toBe('');
    expect(formatDroppedPathsForInput([''])).toBe('');
  });
});

describe('resolvePtyInjection', () => {
  const tabs = [
    { id: 'a', sessionId: 'sess-a' },
    { id: 'b', sessionId: 'sess-b' },
    { id: 'c', sessionId: null },
  ];

  it('injects into the ACTIVE tab session, ignoring the others', () => {
    expect(resolvePtyInjection(tabs, 'b', ['/tmp/x.txt']))
      .toEqual({ sessionId: 'sess-b', data: '/tmp/x.txt ' });
  });

  it('is a no-op when the active tab has no spawned session yet', () => {
    expect(resolvePtyInjection(tabs, 'c', ['/tmp/x.txt'])).toBeNull();
  });

  it('is a no-op when the active tab id is unknown', () => {
    expect(resolvePtyInjection(tabs, 'missing', ['/tmp/x.txt'])).toBeNull();
  });

  it('is a no-op when there are no paths to inject', () => {
    expect(resolvePtyInjection(tabs, 'a', [])).toBeNull();
  });

  it('quotes a spaced path in the resolved data', () => {
    expect(resolvePtyInjection(tabs, 'a', ['/tmp/a b.txt']))
      .toEqual({ sessionId: 'sess-a', data: `'/tmp/a b.txt' ` });
  });
});
