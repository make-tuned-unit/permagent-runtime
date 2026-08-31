/**
 * Every term the interface borrows from a domain gets defined where it is
 * first used. The definitions live in one file; this pins that each one is
 * actually reachable from the surface it belongs to, because a gloss nobody
 * imports is a gloss nobody reads.
 *
 * Source-level on purpose: several of these surfaces are heavy 3D or
 * daemon-backed views the runner cannot mount, and the failure being guarded
 * against is a term losing its definition, which is visible in the source.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { GLOSSARY, type GlossaryKey } from './vocabulary';

const SRC = fileURLToPath(new URL('..', import.meta.url));

/** Which surface owes each borrowed term its definition. */
const OWNERS: Record<GlossaryKey, string> = {
  icir: 'components/finance/FinanceView.tsx',
  halfLife: 'components/finance/FinanceView.tsx',
  financierApproved: 'components/finance/FinanceView.tsx',
  impactConfidence: 'components/grow/GrowView.tsx',
  cleanRuns: 'components/projects/VerificationApprovalPanel.tsx',
  costMeter: 'components/build/CostStatusline.tsx',
};

describe('borrowed terms are defined where they are used', () => {
  it.each(Object.keys(GLOSSARY) as GlossaryKey[])('%s is glossed on its own surface', key => {
    const source = readFileSync(SRC + OWNERS[key], 'utf8');
    expect(source, `${OWNERS[key]} should render GLOSSARY.${key}`)
      .toContain(`GLOSSARY.${key}`);
  });

  it('names an owner for every gloss, so a new term cannot land unplaced', () => {
    expect(Object.keys(OWNERS).sort()).toEqual(Object.keys(GLOSSARY).sort());
  });
});
