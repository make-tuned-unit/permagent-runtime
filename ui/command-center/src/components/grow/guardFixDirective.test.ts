import { describe, expect, it } from 'vitest';
import { guardFixDirective, guardCommitTag } from './guardFixDirective';

describe('guardFixDirective', () => {
  const text = guardFixDirective({
    projectName: 'Shop',
    projectRoot: '/dev/shop',
    projectSlug: 'shop',
    finding: {
      id: 'sarif:rule/1:src/a.ts:10',
      title: 'SQL built from user input',
      severity: 'high',
      cwe: 'CWE-89 SQL Injection',
      location: 'src/a.ts:10',
      remediation: 'Use parameterised queries.',
    },
  });

  it('carries the finding facts and the working directory', () => {
    expect(text).toContain('Working directory: /dev/shop');
    expect(text).toContain('Permagent project: shop');
    expect(text).toContain('SQL built from user input');
    expect(text).toContain('Severity: high');
    expect(text).toContain('CWE-89 SQL Injection');
    expect(text).toContain('src/a.ts:10');
    expect(text).toContain('Use parameterised queries.');
  });

  it('states the rules and the commit tag', () => {
    expect(text).toMatch(/Fix only this finding/);
    expect(text).toMatch(/Never disable, skip or weaken a check, a test/);
    expect(text).toMatch(/Add or adjust a test/);
    expect(text).toContain('Guard: sarif:rule/1:src/a.ts:10');
    expect(guardCommitTag('x:y')).toBe('Guard: x:y');
  });
});
