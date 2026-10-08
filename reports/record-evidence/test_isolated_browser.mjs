import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import config, { daemonPrefixes, daemonTarget, browserOrigin } from '../../ui/command-center/vite.record-review.config.mjs';

function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? files(join(directory, entry.name)) : [join(directory, entry.name)]);
}
test('every source-defined daemon route prefix proxies only to3002', () => {
  const directory = fileURLToPath(new URL('../../crates/goose-server/src/', import.meta.url));
  const prefixes = new Set();
  for (const path of files(directory).filter(path => path.endsWith('.rs'))) {
    const source = readFileSync(path, 'utf8').split('#[cfg(test)]')[0];
    for (const match of source.matchAll(/\.route\s*\(\s*"\/([^"/]+)/g)) {
      if (!match[1].startsWith('{')) prefixes.add(match[1]);
    }
  }
  const entries = Object.entries(config.server.proxy);
  assert.equal(entries.length, 1);
  const [key, proxy] = entries[0];
  assert.equal(proxy.target, 'http://127.0.0.1:3002');
  assert.equal(proxy.ws, true);
  for (const prefix of prefixes) {
    assert.ok(daemonPrefixes.includes(prefix), `missing daemon prefix ${prefix}`);
    assert.ok(new RegExp(key).test(`/${prefix}/fixture?token=synthetic`));
  }
  for (const asset of ['/ui/', '/ui/src/main.tsx', '/world/fixture.png']) {
    assert.equal(new RegExp(key).test(asset), false);
  }
});
test('browser origin and both API definitions remain fixed and loopback-only', () => {
  assert.equal(config.server.host, '127.0.0.1');
  assert.equal(config.server.port, 5302);
  assert.equal(config.server.strictPort, true);
  assert.equal(config.server.open, false);
  assert.equal(browserOrigin, 'http://127.0.0.1:5302');
  assert.equal(daemonTarget, 'http://127.0.0.1:3002');
  assert.equal(config.define['import.meta.env.VITE_DAEMON_URL'], JSON.stringify(browserOrigin));
  assert.equal(config.define['import.meta.env.VITE_API_BASE_URL'], JSON.stringify(browserOrigin));
  assert.deepEqual(config.envPrefix, []);
  assert.equal(config.server.fs.strict, true);
  assert.ok(config.server.fs.allow.every(path => !path.includes('.shared-target')));
});
test('logger never forwards error URLs or credential-bearing fragments', () => {
  const output = [];
  const info = console.info, error = console.error;
  console.info = value => output.push(value);
  console.error = value => output.push(value);
  try {
    config.customLogger.info('Open http://127.0.0.1:5302/ui/#claim=synthetic-sensitive');
    config.customLogger.error('proxy /events?token=synthetic-sensitive');
  } finally { console.info = info; console.error = error; }
  assert.equal(output.join('\n').includes('synthetic-sensitive'), false);
});
