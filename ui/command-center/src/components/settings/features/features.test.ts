import { describe, expect, it } from 'vitest';
import {
  conciergePreconditionCopy,
  FEATURE_KEYS,
  FEATURE_ROWS,
  GMAIL_CONNECT_COMMAND,
  gmailTokenPresent,
  readFlag,
} from './features';

describe('features helpers', () => {
  it('names exactly the five daemon config keys, in display order', () => {
    expect(FEATURE_KEYS).toEqual([
      'initiative_enabled',
      'playbook_enabled',
      'concierge_enabled',
      'steward_scan_enabled',
      'strix_enabled',
    ]);
  });

  // REGRESSION. The Guard's only toggle used to live in the Models pane, three
  // groups away from every other worker switch, and a product owner looking for
  // it under Features could not find it. Before this row existed the assertion
  // below found no such key.
  it('lists the Guard among the switches', () => {
    const guard = FEATURE_ROWS.find(r => r.key === 'strix_enabled');
    expect(guard).toBeDefined();
    expect(guard!.label).toMatch(/Guard/);
  });

  // A half-added row — key in the union, no copy, or copy that promises a
  // restart the daemon does not need — renders a switch the user cannot reason
  // about. FEATURE_KEYS is derived from FEATURE_ROWS, so a duplicate key would
  // also mean two toggles writing one flag.
  it('every switch states what it does and when it takes effect', () => {
    expect(FEATURE_KEYS).toEqual(FEATURE_ROWS.map(r => r.key));
    expect(new Set(FEATURE_KEYS).size).toBe(FEATURE_ROWS.length);
    expect(FEATURE_ROWS).toHaveLength(5);
    for (const row of FEATURE_ROWS) {
      expect(row.what.length).toBeGreaterThan(0);
      expect(row.label.length).toBeGreaterThan(0);
      expect(row.effect).toContain('no restart');
      expect(`${row.what} ${row.effect}`).not.toMatch(/restart (the )?(daemon|app|Permagent)/i);
    }
  });

  it('every row says it is off by default and how soon a flip lands', () => {
    for (const row of FEATURE_ROWS) {
      expect(row.effect).toMatch(/Off by default/);
      expect(row.effect).toMatch(/no restart/);
    }
  });

  it('the Steward row is honest: proposals only, approvals in the Decision Inbox', () => {
    const steward = FEATURE_ROWS.find(r => r.key === 'steward_scan_enabled')!;
    expect(steward.what).toMatch(/proposals only/);
    expect(steward.what).toMatch(/Decision-Inbox approval/);
  });

  it('reads the Gmail token status out of the /integrations list', () => {
    expect(gmailTokenPresent(null)).toBeNull();
    expect(gmailTokenPresent([])).toBe(false);
    expect(gmailTokenPresent([{ provider: 'gmail', connected: false, token_present: false }])).toBe(false);
    expect(gmailTokenPresent([{ provider: 'gmail', connected: true, token_present: true }])).toBe(true);
  });

  it('the Concierge precondition names the CLI command when no token is stored', () => {
    expect(conciergePreconditionCopy(false)).toContain(GMAIL_CONNECT_COMMAND);
    expect(conciergePreconditionCopy(true)).not.toContain(GMAIL_CONNECT_COMMAND);
    expect(conciergePreconditionCopy(null)).toMatch(/Checking/);
  });

  it('only a literal true reads as on (bare-value /config/read contract)', () => {
    expect(readFlag(true)).toBe(true);
    expect(readFlag(null)).toBe(false);
    expect(readFlag('true')).toBe(false);
    expect(readFlag(1)).toBe(false);
    expect(readFlag(undefined)).toBe(false);
  });
});
