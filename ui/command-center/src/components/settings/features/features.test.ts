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
  it('names exactly the four daemon config keys, in display order', () => {
    expect(FEATURE_KEYS).toEqual([
      'initiative_enabled',
      'playbook_enabled',
      'concierge_enabled',
      'steward_scan_enabled',
    ]);
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
