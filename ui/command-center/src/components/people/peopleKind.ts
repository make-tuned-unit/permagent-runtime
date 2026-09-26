/**
 * What a directory row is: a person, an organisation, or the user themself.
 *
 * QA 2026-09-24: People → List mixed in "Harbourview Residents' Association",
 * "Halifax Water Customer Service", project names ("Personal", "Permagent
 * Runtime") and the user's own record twice (their first name and their full
 * name, flagged as a possible duplicate of each other). The backend is adding `kind` to the
 * directory response; until a daemon sends it every row reads as a person,
 * exactly as before — nothing here guesses from the name.
 */
import type { DirectoryPerson } from '../projects/types';

export function isOrganisation(p: Pick<DirectoryPerson, 'kind'>): boolean {
  return p.kind === 'organisation' || p.kind === 'organization';
}

export function isSelf(p: Pick<DirectoryPerson, 'kind' | 'is_self'>): boolean {
  return p.kind === 'self' || p.is_self === true;
}

/** A contact: neither an organisation nor you. */
export function isContact(p: Pick<DirectoryPerson, 'kind' | 'is_self'>): boolean {
  return !isOrganisation(p) && !isSelf(p);
}

/**
 * True when not one contact has a recorded interaction. Today only logged
 * meetings (person_meetings) and a manual last-contact field write it, so on
 * most directories every row is null — and 36 identical "never"s say nothing
 * the one banner doesn't.
 */
export function noInteractionsRecorded(people: Pick<DirectoryPerson, 'last_contact_at'>[]): boolean {
  return people.length > 0 && people.every(p => !p.last_contact_at);
}
