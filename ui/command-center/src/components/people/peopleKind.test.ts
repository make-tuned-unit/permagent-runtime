/**
 * QA 2026-09-24: People → List mixed organisations and the user's own record
 * in with contacts. These are the pure classification rules `PeopleDirectory`
 * uses to tell them apart — kept as plain functions so the "kind absent"
 * fallback (older daemons, before the backend sends it) is provable without a
 * live directory.
 */
import { describe, expect, it } from 'vitest';
import { isContact, isOrganisation, isSelf, noInteractionsRecorded } from './peopleKind';

describe('isOrganisation', () => {
  it('is true for both spellings', () => {
    expect(isOrganisation({ kind: 'organisation' })).toBe(true);
    expect(isOrganisation({ kind: 'organization' })).toBe(true);
  });
  it('is false for a person, self, or a daemon with no kind at all', () => {
    expect(isOrganisation({ kind: 'person' })).toBe(false);
    expect(isOrganisation({ kind: 'self' })).toBe(false);
    expect(isOrganisation({ kind: undefined })).toBe(false);
    expect(isOrganisation({ kind: null })).toBe(false);
  });
});

describe('isSelf', () => {
  it('is true from kind or the alternative is_self flag', () => {
    expect(isSelf({ kind: 'self', is_self: null })).toBe(true);
    expect(isSelf({ kind: 'person', is_self: true })).toBe(true);
  });
  it('is false for an ordinary contact', () => {
    expect(isSelf({ kind: 'person', is_self: false })).toBe(false);
    expect(isSelf({ kind: undefined, is_self: undefined })).toBe(false);
  });
});

describe('isContact', () => {
  it('excludes organisations and self, includes everyone else', () => {
    expect(isContact({ kind: 'organisation', is_self: null })).toBe(false);
    expect(isContact({ kind: 'self', is_self: null })).toBe(false);
    expect(isContact({ kind: 'person', is_self: null })).toBe(true);
    // Absent kind (older daemon) reads as a person, i.e. a contact.
    expect(isContact({ kind: undefined, is_self: undefined })).toBe(true);
  });
});

describe('noInteractionsRecorded', () => {
  it('is true when every row has no last-contact timestamp', () => {
    expect(noInteractionsRecorded([
      { last_contact_at: null }, { last_contact_at: null },
    ])).toBe(true);
  });
  it('is false as soon as one row has a real interaction', () => {
    expect(noInteractionsRecorded([
      { last_contact_at: null }, { last_contact_at: '2026-01-01T00:00:00Z' },
    ])).toBe(false);
  });
  it('is false for an empty directory — nothing to banner-ize', () => {
    expect(noInteractionsRecorded([])).toBe(false);
  });
});
