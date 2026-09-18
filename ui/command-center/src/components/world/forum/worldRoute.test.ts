/** @vitest-environment jsdom */
import { afterEach, expect, it, vi } from 'vitest';
import { LEGACY_WORLD_KEY, shouldUseLegacyWorld } from './worldRoute';

afterEach(() => { localStorage.clear(); vi.restoreAllMocks(); });

it('ships the forum World unless the rollback flag is explicitly set to 1', () => {
  expect(shouldUseLegacyWorld()).toBe(false);
  localStorage.setItem(LEGACY_WORLD_KEY, '1');
  expect(shouldUseLegacyWorld()).toBe(true);
});

it('treats any other stored value as "not rolled back"', () => {
  for (const value of ['0', 'true', 'yes', '', ' 1', '1 ']) {
    localStorage.setItem(LEGACY_WORLD_KEY, value);
    expect(shouldUseLegacyWorld()).toBe(false);
  }
});

it('keeps the shipping route when storage throws instead of returning null', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new DOMException('The operation is insecure.', 'SecurityError');
  });
  expect(shouldUseLegacyWorld()).toBe(false);
});
