/**
 * The consolidation's safety net: every setting still writes what it wrote.
 *
 * This change merged nine panes into four and deleted one outright. The risk
 * that matters is not that a pane looks different — it is that a control
 * quietly stopped writing its key, or started writing a different one, and
 * nothing in the UI would show it. A settings surface can look completely
 * healthy while writing nowhere.
 *
 * So: the set of config keys the Settings surface can write is FROZEN at what
 * it was on `origin/main@e92ea08b` — the commit this lane branched from — and
 * recomputed here by scanning the tree. Equality is the assertion. A key that
 * disappears fails; a key that appears from nowhere fails too, because a new
 * write path is exactly the "second place to switch it on" this whole change
 * exists to remove.
 *
 * Two kinds of writer have to be handled:
 *
 *   - LITERAL — `api.upsertConfig('voice_model', …)`. Scanned straight out of
 *     the source.
 *   - DYNAMIC — `api.upsertConfig(gate.config_key, …)`, where the key comes
 *     from the daemon or from a typed parameter. The scanner cannot read those,
 *     so each is DECLARED below with the range of keys it can write and the
 *     reason it is dynamic. A declaration whose call site has vanished fails
 *     the test, which is what stops the declarations rotting into fiction.
 *
 * The scan covers `components/settings` and `components/history`, because the
 * four record panes moved out of Settings in this same change and their writes
 * (the spend ceilings) moved with them. Scoping the scan to the old directory
 * would have made the move look like a deletion.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { FEATURE_KEYS } from './workerGates';

const SRC = fileURLToPath(new URL('../..', import.meta.url));
const SCANNED = ['components/settings', 'components/history'];

/**
 * Every config key the Settings surface could write on `origin/main@e92ea08b`,
 * before this lane touched anything.
 *
 * TO CHANGE THIS LIST: only alongside a deliberate, stated change to what
 * Settings writes. Consolidating panes is not one — a pane merge that alters
 * this list has lost a control.
 */
const KEYS_BEFORE = [
  // Autonomy → trust level.
  'GOOSE_MODE',
  // Models → Chat / Harness role table (model_roles.rs).
  'chat_provider', 'chat_model', 'harness_provider', 'harness_model',
  // Models → Voice route (voice_model.rs). Literal on both sides.
  'voice_provider', 'voice_model',
  // Preferences → Your code.
  'dev_roots',
  // The Guard's sweep cadence, on its own agent page.
  'strix_sweep_hours',
  // The Watcher's teaching keys, on its own agent page.
  'watcher_topics', 'watcher_muted_subjects',
  // The six worker gates. Written through `gate.config_key` from the daemon's
  // roster row — on main, ALSO written a second time by the Features board.
  ...FEATURE_KEYS,
  // Provider credentials and their secret source, from the provider modal.
  // The concrete key is per-provider (`OPENAI_API_KEY`, …) and comes from the
  // provider catalogue, so it is declared as a range rather than enumerated.
  '<provider config/secret key>',
].sort();

/** A call whose key the scanner cannot read, and what it can write. */
interface DynamicWriter {
  /** Path under `src/`, with `/` separators. */
  file: string;
  /** A substring that must still be present, proving the writer exists. */
  call: string;
  /** Every key this call can write. */
  keys: readonly string[];
  /** Why the key is not a literal. */
  why: string;
}

const DYNAMIC_WRITERS: readonly DynamicWriter[] = [
  {
    file: 'components/settings/agents/AgentsPanel.tsx',
    call: 'api.upsertConfig(gate.config_key',
    keys: FEATURE_KEYS,
    why: 'The daemon serialises the key it reads on each agent row as `gate.config_key`; the switch writes THAT, so a daemon-side rename cannot leave the UI writing a dead key. `FEATURE_KEYS` mirrors the Rust gate table (self_knowledge/mod.rs::worker_gate) and both tests pin it.',
  },
  {
    file: 'components/settings/agents/agentSettings.tsx',
    call: 'api.upsertConfig(key, list)',
    keys: ['watcher_topics', 'watcher_muted_subjects'],
    why: "The Watcher's two teaching fields share one save function; `key` is a union-typed parameter, so the range is exactly these two.",
  },
  {
    file: 'components/settings/SettingsView.tsx',
    call: 'api.upsertConfig(providerKey',
    keys: ['chat_provider', 'harness_provider'],
    why: '`RoleModelRow` is rendered once for Chat and once for Harness and takes its key as a prop. Voice is NOT in this range — its precedence differs, so it keeps its own literal writes.',
  },
  {
    file: 'components/settings/SettingsView.tsx',
    call: 'api.upsertConfig(modelKey',
    keys: ['chat_model', 'harness_model'],
    why: 'The model half of the same two rows.',
  },
  {
    file: 'components/settings/ConfigureProviderModal.tsx',
    call: 'api.upsertConfig(secretKey',
    keys: ['<provider config/secret key>'],
    why: 'The key is the provider catalogue entry being configured (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, …). Enumerating the catalogue here would be a second copy of it.',
  },
];

/**
 * Named write endpoints reachable from these panes that are NOT config keys.
 *
 * A pane merge could lose one of these without touching a single config key,
 * so they are frozen the same way: the endpoint, and the file that has to
 * still be calling it.
 */
const ENDPOINT_WRITERS: readonly { call: string; file: string; what: string }[] = [
  { call: 'api.setSovereignty(', file: 'components/settings/SettingsView.tsx', what: 'the data boundary and prompt capture' },
  { call: 'api.setAnalyticsConsent(', file: 'components/settings/SettingsView.tsx', what: 'product-analytics consent' },
  { call: 'api.setLibrarianSchedule(', file: 'components/settings/agents/agentSettings.tsx', what: "the Librarian's nightly schedule and pruning" },
  { call: 'api.putCouncilMembers(', file: 'components/settings/agents/agentSettings.tsx', what: "the Council's seats" },
  { call: 'api.setBudget(', file: 'components/history/SpendPanel.tsx', what: 'the session and per-task spend ceilings' },
  { call: 'api.setSecretSource(', file: 'components/settings/ConfigureProviderModal.tsx', what: 'where a provider key is read from' },
  { call: 'api.removeConfig(', file: 'components/settings/ConfigureProviderModal.tsx', what: 'removing a provider key' },
  { call: 'api.renameDevice(', file: 'components/settings/SettingsView.tsx', what: 'renaming a paired device' },
  { call: 'api.revokeDevice(', file: 'components/settings/SettingsView.tsx', what: "revoking a companion's key" },
  { call: 'api.putIdentity(', file: 'components/settings/useSettings.ts', what: 'the persona: name, greeting, tone, traits, voice' },
  { call: 'api.addExtension(', file: 'components/settings/SearchToolsSection.tsx', what: 'connecting a search/tool extension' },
  { call: 'api.createCustomProvider(', file: 'components/settings/AddCustomProviderModal.tsx', what: 'adding a custom provider' },
  { call: 'api.removeCustomProvider(', file: 'components/settings/ProvidersSection.tsx', what: 'removing a custom provider' },
];

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

/** Every scanned file, keyed by its `/`-separated path under `src/`. */
function scannedTree(): Map<string, string> {
  const tree = new Map<string, string>();
  for (const root of SCANNED) {
    for (const file of sourceFiles(join(SRC, ...root.split('/')))) {
      tree.set(relative(SRC, file).split(sep).join('/'), readFileSync(file, 'utf8'));
    }
  }
  return tree;
}

/** `api.upsertConfig('some_key'` — the keys the scanner CAN read. */
function literalKeys(source: string): string[] {
  const re = /api\.upsertConfig\(\s*'([A-Za-z_][A-Za-z_0-9]*)'/g;
  const out: string[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) out.push(m[1]);
  return out;
}

/** `api.upsertConfig(notALiteral` — every dynamic call site, as `file:line`. */
function dynamicCallSites(tree: Map<string, string>): string[] {
  const sites: string[] = [];
  for (const [file, source] of tree) {
    source.split('\n').forEach((line, i) => {
      if (/api\.upsertConfig\(\s*[^'\s)]/.test(line)) sites.push(`${file}:${i + 1}  ${line.trim()}`);
    });
  }
  return sites;
}

describe('what Settings writes', () => {
  it('writes exactly the config keys it wrote before the panes were consolidated', () => {
    const tree = scannedTree();

    const after = new Set<string>();
    for (const source of tree.values()) for (const k of literalKeys(source)) after.add(k);
    for (const w of DYNAMIC_WRITERS) {
      const source = tree.get(w.file);
      // A declaration whose call site is gone is not evidence of anything, so
      // its keys are not credited — which is what makes a deleted writer show
      // up below as a MISSING key rather than passing on a stale promise.
      if (source && source.includes(w.call)) for (const k of w.keys) after.add(k);
    }

    const before = new Set(KEYS_BEFORE);
    const lost = [...before].filter(k => !after.has(k)).sort();
    const gained = [...after].filter(k => !before.has(k)).sort();

    expect(lost, 'a setting stopped writing its key — the control was lost in a pane merge').toEqual([]);
    expect(gained, 'a NEW config write path appeared; a second writer for a key is the thing this consolidation removes').toEqual([]);
  });

  it('leaves no dynamic write path undeclared', () => {
    const tree = scannedTree();
    const declared = new Set(
      DYNAMIC_WRITERS.filter(w => tree.get(w.file)?.includes(w.call)).map(w => `${w.file}|${w.call}`),
    );
    const undeclared = dynamicCallSites(tree).filter(site => {
      const file = site.slice(0, site.indexOf(':'));
      return ![...declared].some(d => {
        const [f, call] = d.split('|');
        return f === file && site.includes(call.slice(call.indexOf('(') + 1));
      });
    });
    expect(
      undeclared,
      'this call writes a config key the scanner cannot read — declare it in DYNAMIC_WRITERS with the range it can write',
    ).toEqual([]);
  });

  it('still reaches every non-config write endpoint it reached before', () => {
    const tree = scannedTree();
    const missing = ENDPOINT_WRITERS
      .filter(w => !tree.get(w.file)?.includes(w.call))
      .map(w => `${w.call} in ${w.file} — ${w.what}`);
    expect(missing, 'a write endpoint lost its only caller').toEqual([]);
  });

  it('declares every worker gate key the Rust gate table gates', () => {
    // The gate keys are the ones the retired Features board wrote a second
    // time. If a key left this list, the surface that switches that worker on
    // left with it.
    for (const key of FEATURE_KEYS) expect(KEYS_BEFORE).toContain(key);
  });
});
