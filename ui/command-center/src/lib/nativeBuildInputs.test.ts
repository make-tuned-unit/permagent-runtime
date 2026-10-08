import { describe, expect, it } from 'vitest';
import { validateNativeBuildInputs } from './nativeBuildInputs';

describe('native build daemon target', () => {
  it('accepts clean production defaults and production loopback URLs', () => {
    for (const env of [{}, { VITE_DAEMON_URL: '' }, { VITE_DAEMON_URL: 'http://127.0.0.1:3001/' }, { VITE_API_BASE_URL: 'http://localhost:3001' }]) {
      expect(() => validateNativeBuildInputs(env, {})).not.toThrow();
    }
  });
  it.each(['http://127.0.0.1:3002', 'https://example.test', 'http://user:pass@localhost:3001', 'http://localhost:3001/api', 'http://localhost:3001?token=secret'])('rejects stale or non-native shipping target %s without echoing it', value => {
    for (const key of ['VITE_DAEMON_URL', 'VITE_API_BASE_URL']) {
      expect(() => validateNativeBuildInputs({ [key]: value }, {})).toThrow('does not match');
      try { validateNativeBuildInputs({ [key]: value }, {}); } catch (e) {
        expect(String(e)).not.toContain(value);
      }
    }
  });
  it('checks the fallback alias even when the primary URL is correct', () => {
    expect(() => validateNativeBuildInputs({ VITE_DAEMON_URL: 'http://127.0.0.1:3001', VITE_API_BASE_URL: 'http://127.0.0.1:3002' }, {})).toThrow();
  });
  it('allows coherent review inputs and rejects incomplete or mismatched inputs', () => {
    const review = { root: '/tmp/record-qa-runtime', port: '3002' };
    expect(() => validateNativeBuildInputs({ VITE_DAEMON_URL: 'http://127.0.0.1:3002' }, review)).not.toThrow();
    for (const input of [{}, { VITE_DAEMON_URL: 'http://127.0.0.1:3001' }]) expect(() => validateNativeBuildInputs(input, review)).toThrow();
    expect(() => validateNativeBuildInputs({}, { root: review.root })).toThrow();
    for (const port of ['0', '80', '3001', '65536', '3002oops']) expect(() => validateNativeBuildInputs({}, { ...review, port })).toThrow();
  });
});
