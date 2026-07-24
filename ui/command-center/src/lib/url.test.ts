import { describe, expect, it } from 'vitest';
import { isSafeHttpUrl } from './url';

describe('isSafeHttpUrl', () => {
  it.each([
    'http://example.com',
    'https://example.com/source',
    'HTTPS://EXAMPLE.COM/source',
  ])('accepts HTTP(S) URL %s', url => {
    expect(isSafeHttpUrl(url)).toBe(true);
  });

  it.each([
    'javascript:alert(1)',
    'data:text/html,unsafe',
    'file:///tmp/source',
    '/relative/source',
    '//example.com/source',
    'example.com/source',
  ])('rejects unsafe or relative URL %s', url => {
    expect(isSafeHttpUrl(url)).toBe(false);
  });
});
