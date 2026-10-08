/** @vitest-environment node */
import { beforeAll, describe, expect, it } from 'vitest';
import { loadConfigFromFile, type ProxyOptions } from 'vite';
import { fileURLToPath } from 'node:url';

// Resolve the same first matching prefix Vite's proxy middleware uses. These
// routes previously fell through to its /ui/ HTML handler instead of the API.
let proxy: Record<string, string | ProxyOptions> = {};
beforeAll(async () => {
  const loaded = await loadConfigFromFile(
    { command: 'serve', mode: 'development' },
    fileURLToPath(new URL('../../vite.config.ts', import.meta.url)),
  );
  expect(loaded).not.toBeNull();
  proxy = loaded!.config.server?.proxy ?? {};
});
function route(path: string) {
  const key = Object.keys(proxy).find(prefix => prefix.startsWith('^')
    ? new RegExp(prefix).test(path) : path.startsWith(prefix));
  return key ? proxy[key] : undefined;
}

describe('Settings daemon routes in the normal development server', () => {
  it.each([
    '/integrations', '/integrations/gmail/connect', '/integrations/gmail',
    '/gateway/status', '/gateway/start', '/gateway/stop', '/gateway/pair',
    '/tunnel/status',
  ])('forwards %s to the existing daemon target', path => {
    expect(route(path)).toMatchObject({ target: 'http://localhost:3001', changeOrigin: true });
  });

  it('leaves frontend and isolated-review routes outside this proxy table', () => {
    expect(route('/ui/')).toBeUndefined();
    expect(route('/src/components/settings/SettingsView.tsx')).toBeUndefined();
    expect(Object.values(proxy).some(value => typeof value === 'object' && value.target === 'http://127.0.0.1:3002')).toBe(false);
  });
});
