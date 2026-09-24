import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  PolicyError,
  runShippedContentCheck,
  StalenessError,
  validatePolicy,
} from '../../../scripts/shipped-content-check.mjs';

const REPO_ROOT = join(import.meta.dirname, '../../..');

function basePolicy(overrides: Record<string, unknown> = {}) {
  return {
    version: 1,
    scope: {
      roots: ['scan-root'],
      extensions: ['.md'],
      skipDirectories: ['node_modules'],
    },
    rules: [
      {
        id: 'operational-home-root',
        kind: 'operationalLiteral',
        include: ['**/*'],
        match: { type: 'regex', pattern: '~/dev', flags: '' },
        message: 'Resolve operational roots from configuration or discovery.',
      },
      {
        id: 'unsupported-machine-topology',
        kind: 'unsupportedClaim',
        include: ['**/*'],
        match: {
          type: 'context',
          anchor: {
            type: 'regex',
            pattern: '\\b(?:your\\s+Mac\\s+mini|your\\s+tailnet)\\b',
            flags: 'i',
          },
          window: { linesBefore: 1, linesAfter: 1 },
          requireAll: [
            { type: 'regex', pattern: '\\b(?:written|processed|running|hosted)\\b', flags: 'i' },
          ],
          rejectAny: [],
        },
        message: 'Hardware ownership and network provenance require runtime evidence.',
      },
    ],
    exceptions: [] as Array<Record<string, unknown>>,
    ...overrides,
  };
}

function writeScanTree(repoRoot: string, relPath: string, body: string) {
  const abs = join(repoRoot, 'scan-root', relPath);
  mkdirSync(join(abs, '..'), { recursive: true });
  writeFileSync(abs, body, 'utf8');
}

describe('shipped-content policy validation', () => {
  it('rejects unknown top-level fields', () => {
    const policy = { ...basePolicy(), extra: true };
    expect(() => validatePolicy(policy)).toThrow(PolicyError);
    expect(() => validatePolicy(policy)).toThrow(/unknown field/);
  });

  it('rejects invalid regex patterns', () => {
    const policy = basePolicy();
    policy.rules[0].match = { type: 'regex', pattern: '(', flags: '' };
    expect(() => validatePolicy(policy)).toThrow(PolicyError);
  });

  it('rejects regexes that match empty input', () => {
    const policy = basePolicy();
    policy.rules[0].match = { type: 'regex', pattern: '(?:)', flags: '' };
    expect(() => validatePolicy(policy)).toThrow(/must not match empty/);
  });

  it('fails when a scope root pattern matches nothing', () => {
    const dir = mkdtempSync(join(tmpdir(), 'shipped-content-'));
    const policy = basePolicy();
    expect(() => validatePolicy(policy, { repoRoot: dir })).toThrow(/matched no directories/);
  });

  it('fails when an exception exactLine no longer matches the file', () => {
    const dir = mkdtempSync(join(tmpdir(), 'shipped-content-'));
    mkdirSync(join(dir, 'scan-root'), { recursive: true });
    writeFileSync(join(dir, 'scan-root', 'note.md'), 'still ~/dev here\n', 'utf8');
    const policy = basePolicy({
      exceptions: [
        {
          id: 'stale-line',
          ruleId: 'operational-home-root',
          path: 'scan-root/note.md',
          line: 1,
          exactLine: 'old ~/dev text',
          contextBefore: [],
          contextAfter: [],
          matchedText: '~/dev',
          expectedMatches: 1,
          reasonCode: 'testFixture',
          reason: 'unit test fixture',
          reviewBy: '2099-01-01',
        },
      ],
    });
    expect(() => validatePolicy(policy, { repoRoot: dir, today: '2026-01-01' })).toThrow(
      StalenessError,
    );
    expect(() => validatePolicy(policy, { repoRoot: dir, today: '2026-01-01' })).toThrow(
      /exactLine mismatch/,
    );
  });

  it('fails when reviewBy is expired', () => {
    const policy = basePolicy({
      exceptions: [
        {
          id: 'expired',
          ruleId: 'operational-home-root',
          path: 'scan-root/x.md',
          line: 1,
          exactLine: 'x',
          contextBefore: [],
          contextAfter: [],
          matchedText: 'x',
          expectedMatches: 1,
          reasonCode: 'testFixture',
          reason: 'unit test fixture',
          reviewBy: '2020-01-01',
        },
      ],
    });
    expect(() => validatePolicy(policy, { today: '2026-09-22' })).toThrow(StalenessError);
    expect(() => validatePolicy(policy, { today: '2026-09-22' })).toThrow(/expired/);
  });
});

describe('shipped-content checker behaviour', () => {
  it('fails when an exception suppresses nothing', () => {
    const dir = mkdtempSync(join(tmpdir(), 'shipped-content-'));
    writeScanTree(dir, 'hit.md', 'leak ~/dev here\n');
    writeScanTree(dir, 'decoy.md', 'also ~/dev on this line\n');
    const policy = basePolicy();
    policy.rules[0].include = ['scan-root/hit.md'];
    policy.exceptions = [
        {
          id: 'orphan',
          ruleId: 'operational-home-root',
          path: 'scan-root/decoy.md',
          line: 1,
          exactLine: 'also ~/dev on this line',
          contextBefore: [],
          contextAfter: [],
          matchedText: '~/dev',
          expectedMatches: 1,
          reasonCode: 'testFixture',
          reason: 'unit test fixture',
          reviewBy: '2099-01-01',
        },
      ];
    expect(() => runShippedContentCheck({ repoRoot: dir, policy, today: '2026-01-01' })).toThrow(
      /suppressed nothing/,
    );
  });

  it('scans only files matched by a rule include glob, resolved from the repo root', () => {
    const dir = mkdtempSync(join(tmpdir(), 'shipped-content-'));
    writeScanTree(dir, 'prompts/in.md', 'leak ~/dev here\n');
    writeScanTree(dir, 'other/out.md', 'leak ~/dev here\n');
    const policy = basePolicy();
    policy.rules[0].include = ['scan-root/prompts/**'];
    const { findings } = runShippedContentCheck({ repoRoot: dir, policy, today: '2026-01-01' });
    const paths = findings.filter((f) => f.ruleId === 'operational-home-root').map((f) => f.path);
    expect(paths).toEqual(['scan-root/prompts/in.md']);
  });

  it('fires the context matcher on "Written on your Mac mini" but not on bare "Mac mini"', () => {
    const dir = mkdtempSync(join(tmpdir(), 'shipped-content-'));
    writeScanTree(
      dir,
      'copy.md',
      'A generic Mac mini sits on the desk.\nWritten on your Mac mini over your tailnet.\n',
    );
    const { findings } = runShippedContentCheck({
      repoRoot: dir,
      policy: basePolicy(),
      today: '2026-01-01',
    });
    const topology = findings.filter((f) => f.ruleId === 'unsupported-machine-topology');
    expect(topology.some((f) => f.line === 2)).toBe(true);
    expect(topology.some((f) => f.line === 1)).toBe(false);
  });
});

describe('shipped-content convention (repo)', () => {
  const MUST_REPORT = [
    'crates/goose-server/src/automation/health_review.yaml:134',
    'crates/goose/src/prompts/system.md:93',
    'crates/goose/src/projects.rs:119',
    'crates/goose-server/src/automation/steward.yaml:10',
    'crates/goose-server/src/automation/steward.yaml:15',
    'crates/goose/src/meeting_writeup.rs:85',
  ];

  const MUST_NOT = [
    'crates/goose/src/privacy.rs:24',
    'crates/goose/src/privacy.rs:25',
    'ui/command-center/src/components/wizard/MomentCode.tsx:42',
    'crates/goose-server/src/automation/workspace_snapshot.yaml:35',
    'ui/command-center/src/components/settings/DevRootsSection.tsx:164',
    'ui/command-center/src/components/wizard/MomentCode.tsx:218',
    'crates/goose/src/prompts/session_name.md:5',
  ];

  it('reports known environment leaks and spares reviewed non-findings', () => {
    const { lines, ok } = runShippedContentCheck({ repoRoot: REPO_ROOT, today: '2026-09-22' });
    expect(ok).toBe(false);
    for (const needle of MUST_REPORT) {
      expect(lines.some((l) => l.includes(needle))).toBe(true);
    }
    for (const needle of MUST_NOT) {
      expect(lines.some((l) => l.includes(needle))).toBe(false);
    }
  });
});
