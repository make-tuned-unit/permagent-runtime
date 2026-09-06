import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '../..');
const inventoryPath = path.join(root, 'docs/orchestrator/UI_POLISH_SURFACE_INVENTORY_2026-09-05.md');
const masterPath = path.join(root, 'docs/orchestrator/UI_POLISH_MASTER_PROGRAM_DAG.yaml');
const apiPath = path.join(root, 'ui/command-center/src/lib/api.ts');
const storePath = path.join(root, 'ui/command-center/src/lib/store.ts');
const sectionsPath = path.join(root, 'ui/command-center/src/components/settings/sections.ts');
const commandCenterSourcePath = path.join(root, 'ui/command-center/src');

const inventory = fs.readFileSync(inventoryPath, 'utf8');
const master = fs.readFileSync(masterPath, 'utf8');
const apiSource = fs.readFileSync(apiPath, 'utf8');
const store = fs.readFileSync(storePath, 'utf8');
const sections = fs.readFileSync(sectionsPath, 'utf8');

function parseToolType(source) {
  const declaration = source.match(/export type ToolType = ([^;]+);/);
  assert.ok(declaration, 'ToolType declaration missing or changed shape');
  const names = [...declaration[1].matchAll(/'([^']+)'/g)].map(([, name]) => name);
  assert.ok(names.length > 0, 'ToolType declaration has no route names');
  return names;
}

function parseSettingsKeys(source) {
  const declaration = source.match(/SETTINGS_SECTION_KEYS = \[([\s\S]*?)\] as const/);
  assert.ok(declaration, 'SETTINGS_SECTION_KEYS declaration missing or changed shape');
  const names = [...declaration[1].matchAll(/'([^']+)'/g)].map(([, name]) => name);
  assert.ok(names.length > 0, 'SETTINGS_SECTION_KEYS declaration has no keys');
  return names;
}

function parseApiMethods(source) {
  const start = source.indexOf('export const api = {');
  assert.ok(start >= 0, 'top-level api object declaration missing');
  const end = source.indexOf('\n};', start);
  assert.ok(end > start, 'top-level api object terminator missing');
  const body = source.slice(start, end);
  const names = [...body.matchAll(/^  ([A-Za-z_$][\w$]*):/gm)].map(([, name]) => name);
  assert.ok(names.length > 0, 'top-level api object has no methods');
  return names;
}

function sourceFiles(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const child = path.join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(child);
    return /\.(ts|tsx)$/.test(entry.name) && !/\.(test|spec)\./.test(entry.name) ? [child] : [];
  });
}

function firstCallArgument(source, openParen) {
  let quote = null;
  let escaped = false;
  let depth = 0;
  let value = '';
  for (let index = openParen + 1; index < source.length; index += 1) {
    const char = source[index];
    if (quote) {
      value += char;
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === quote) quote = null;
      continue;
    }
    if (char === "'" || char === '"' || char === '`') {
      quote = char;
      value += char;
    } else if ('([{'.includes(char)) {
      depth += 1;
      value += char;
    } else if (char === ')' && depth === 0) {
      return value.trim();
    } else if (char === ',' && depth === 0) {
      return value.trim();
    } else {
      if (char === ')') depth -= 1;
      value += char;
    }
  }
  return value.trim();
}

function parseApiFetchCallSites() {
  const calls = [];
  for (const file of sourceFiles(commandCenterSourcePath)) {
    if (file.endsWith('/lib/api.ts')) continue;
    const source = fs.readFileSync(file, 'utf8');
    for (const match of source.matchAll(/apiFetch(?:<[^>]*>)?\s*\(/g)) {
      const argument = firstCallArgument(source, match.index + match[0].length - 1);
      calls.push({
        file: path.relative(root, file),
        line: source.slice(0, match.index).split('\n').length,
        argument,
      });
    }
  }
  return calls;
}

function parseNativeCommands() {
  const commands = new Set();
  for (const file of sourceFiles(commandCenterSourcePath)) {
    const source = fs.readFileSync(file, 'utf8');
    for (const [, command] of source.matchAll(/(?:\b(?:core\.invoke|api\.invoke|inv\.invoke|invoke))\s*(?:<[^>]*>)?\s*\(\s*['"]([^'"]+)['"]/g)) {
      commands.add(command);
    }
  }
  return [...commands].sort();
}

function parseAnchors(markdown, kind) {
  const rows = [...markdown.matchAll(new RegExp(
    `^ui-polish-route ${kind}="([^"]+)" surface="([^"]+)"$`, 'gm',
  ))];
  assert.ok(rows.length > 0, `no ${kind} coverage anchors`);
  const anchors = new Map();
  for (const [, name, surface] of rows) {
    assert.ok(!anchors.has(name), `duplicate ${kind} coverage anchor: ${name}`);
    anchors.set(name, surface);
  }
  return anchors;
}

function validateCoverage({ toolNames, settingNames, toolAnchors, settingAnchors, surfaceIds }) {
  assert.ok(toolNames.length > 0, 'no source ToolType routes to validate');
  assert.ok(settingNames.length > 0, 'no source settings keys to validate');
  for (const name of toolNames) {
    const surface = toolAnchors.get(name);
    assert.ok(surface, `unmapped source tool route: ${name}`);
    assert.ok(surfaceIds.has(surface), `tool ${name} points at missing surface ${surface}`);
  }
  for (const name of settingNames) {
    const surface = settingAnchors.get(name);
    assert.ok(surface, `unmapped source settings route: ${name}`);
    assert.ok(surfaceIds.has(surface), `setting ${name} points at missing surface ${surface}`);
  }
  for (const name of toolAnchors.keys()) assert.ok(toolNames.includes(name), `stale tool anchor: ${name}`);
  for (const name of settingAnchors.keys()) assert.ok(settingNames.includes(name), `stale setting anchor: ${name}`);
}

test('surface IDs are unique and point at real UI-polish nodes', () => {
  const rows = [...inventory.matchAll(/^\| ([A-Z][A-Z0-9-]+) · (u\d+) · /gm)];
  assert.ok(rows.length >= 60, `inventory unexpectedly shrank: ${rows.length} rows`);
  const ids = rows.map(([, id]) => id);
  assert.equal(new Set(ids).size, ids.length, 'duplicate surface ID');
  const nodes = new Set([...master.matchAll(/^  - id: (u\d+)/gm)].map(([, id]) => id));
  for (const [, , node] of rows) assert.ok(nodes.has(node), `unknown owning node ${node}`);
});

test('every source-defined tool and settings key has an explicit inventory anchor', () => {
  const rows = [...inventory.matchAll(/^\| ([A-Z][A-Z0-9-]+) · u\d+ · /gm)];
  const surfaceIds = new Set(rows.map(([, id]) => id));
  validateCoverage({
    toolNames: parseToolType(store),
    settingNames: parseSettingsKeys(sections),
    toolAnchors: parseAnchors(inventory, 'tool'),
    settingAnchors: parseAnchors(inventory, 'setting'),
    surfaceIds,
  });
});

test('every top-level api method has exactly one disposition row', () => {
  const section = inventory.match(/## Top-level `api` method disposition[\s\S]*?(?=\n## Direct source denominator|\n## Unresolved coverage)/)?.[0];
  assert.ok(section, 'API disposition table section missing');
  const rows = [...section.matchAll(/^\| `([^`]+)` \|/gm)].map(([, name]) => name);
  const methods = parseApiMethods(apiSource);
  assert.equal(rows.length, methods.length, `API denominator mismatch: ${rows.length} rows for ${methods.length} methods`);
  assert.equal(new Set(rows).size, rows.length, 'duplicate API disposition row');
  assert.deepEqual(rows, methods, 'API disposition order differs from api.ts; review the denominator table');
});

test('every dynamic apiFetch call site is listed in the direct-source denominator', () => {
  const section = inventory.match(/### Dynamic first-argument `apiFetch` call sites[\s\S]*?(?=\n## Native command denominator|\n## Portal and actionable-host reverse join|\n### Direct `apiFetch` and native command coverage)/)?.[0];
  assert.ok(section, 'dynamic apiFetch call-site table missing');
  const listed = [...section.matchAll(/^\| `([^`]+)` \| `([^`]+)` \|/gm)].map(([, location, argument]) => `${location}::${argument}`);
  const calls = parseApiFetchCallSites().filter(({ argument }) => !/^['"`]/.test(argument));
  const actual = calls.map(({ file, line, argument }) => `${file}:${line}::${argument}`);
  assert.equal(actual.length, 12, `dynamic apiFetch call-site denominator changed: ${actual.length}`);
  assert.deepEqual(listed, actual, 'dynamic apiFetch call-site table differs from source order');
});

test('direct apiFetch source denominator remains finite and explicitly counted', () => {
  const section = inventory.match(/## Direct source denominator:[\s\S]*?(?=\n### Dynamic first-argument `apiFetch` call sites)/)?.[0];
  assert.ok(section, 'literal/template apiFetch route table missing');
  const routes = [...section.matchAll(/^\| `([^`]+)` \|/gm)].map(([, route]) => route);
  assert.equal(routes.length, 118, `literal/template route denominator changed: ${routes.length}`);
  assert.equal(new Set(routes).size, routes.length, 'duplicate direct route row');
  const calls = parseApiFetchCallSites();
  assert.equal(calls.length, 180, `direct apiFetch call-site denominator changed: ${calls.length}`);
  assert.equal(calls.filter(({ argument }) => !/^['"`]/.test(argument)).length, 12, 'dynamic call-site count changed');
});

test('native invoke command denominator is exact and every command has a disposition', () => {
  const section = inventory.match(/## Native command denominator:[\s\S]*?(?=\n## Portal and actionable-host reverse join|\n### Direct `apiFetch` and native command coverage)/)?.[0];
  assert.ok(section, 'native command denominator table missing');
  const listed = [...section.matchAll(/^\| `([^`]+)` \|/gm)].map(([, command]) => command);
  const actual = parseNativeCommands();
  assert.equal(listed.length, actual.length, `native command denominator mismatch: ${listed.length} rows for ${actual.length} commands`);
  assert.equal(new Set(listed).size, listed.length, 'duplicate native command disposition row');
  assert.deepEqual([...listed].sort(), actual, 'native command denominator differs from source');
});

test('coverage parser fails closed when a source declaration disappears', () => {
  assert.throws(() => parseToolType('export const ToolType = [];'), /ToolType declaration/);
  assert.throws(() => parseSettingsKeys('export const SETTINGS_SECTION_KEYS = [];'), /SETTINGS_SECTION_KEYS declaration/);
});

test('a newly introduced source route cannot pass without a ledger anchor', () => {
  const anchors = new Map([['chat', 'CC-CHAT']]);
  assert.throws(() => validateCoverage({
    toolNames: ['chat'],
    settingNames: ['agent', 'history'],
    toolAnchors: anchors,
    settingAnchors: new Map([['agent', 'SET-AGENT-IDENTITY']]),
    surfaceIds: new Set(['CC-CHAT', 'SET-AGENT-IDENTITY']),
  }), /history/);
});
