import { describe, expect, it } from 'vitest';
import { matchPendingPerson } from './personNavigation';

const ada = { entity_uuid: 'u-ada', display_name: 'Ada Lovelace' };
const bea = { entity_uuid: 'u-bea', display_name: 'Bea' };

describe('matchPendingPerson', () => {
  it('matches a unique display name, case-insensitively', () => {
    expect(matchPendingPerson([ada, bea], { person: 'ada lovelace' })).toEqual(ada);
  });

  it('prefers entity_uuid when present', () => {
    expect(matchPendingPerson([ada, bea], { entity_uuid: 'u-bea', person: 'Ada Lovelace' })).toEqual(bea);
  });

  it('returns null when the name is missing or ambiguous', () => {
    expect(matchPendingPerson([ada, bea], {})).toBeNull();
    expect(matchPendingPerson(
      [ada, { entity_uuid: 'u-ada-2', display_name: 'Ada Lovelace' }],
      { person: 'Ada Lovelace' },
    )).toBeNull();
  });
});
