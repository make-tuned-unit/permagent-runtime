import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * The user: "i also want to see User counts because a few of my projects are
 * starting to get signed up users."
 *
 * The daemon counts distinct `user_id`s the site sends on its own events, and
 * deliberately returns `null` when the site sends no identity at all — because
 * "we cannot see this" and "nobody signed up" are different facts. The one way
 * to get this wrong is to collapse them, either by defaulting null to 0 or by
 * falling back to a device or session count, both of which count browsers
 * rather than people. These assertions exist to keep that from happening.
 */
const SOURCE = readFileSync(new URL('./FirstPartyAnalyticsPanel.tsx', import.meta.url), 'utf8');

describe('the Users tile', () => {
  it('is on the analytics panel', () => {
    expect(SOURCE).toContain("'Users'");
    expect(SOURCE).toContain('signedUpUsers');
  });

  it('distinguishes "no identity sent" from a real zero', () => {
    // A null-check, not a truthiness check: `stats.signedUpUsers || '—'` would
    // turn a genuine zero into a dash and hide a real answer.
    expect(SOURCE).toMatch(/stats\.signedUpUsers\s*!=\s*null/);
    expect(SOURCE).not.toMatch(/signedUpUsers\s*\|\|/);
    expect(SOURCE).not.toMatch(/signedUpUsers\s*\?\?\s*0/);
  });

  it('never substitutes devices or sessions for people', () => {
    // Just this tile: from its label to the start of the next one.
    const start = SOURCE.indexOf("'Users'");
    const next = SOURCE.indexOf("['", start);
    const tile = SOURCE.slice(start, next > start ? next : start + 400);
    expect(tile).not.toContain('deviceSignatures');
    expect(tile).not.toContain('stats.sessions');
  });

  it('names where the count came from rather than asserting a bare "users"', () => {
    expect(SOURCE).toContain('signupEventName');
  });
});
