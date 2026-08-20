/** Agent/voice deep-link into a person on the People tab.
 *
 * `observe_app` redacts UUIDs, so the agent opens someone by display name.
 * `entity_uuid` is still accepted when the agent already has it (create_person).
 */

export type PendingPersonNavigation = {
  entity_uuid?: string;
  person?: string;
};

export function matchPendingPerson<T extends { entity_uuid: string; display_name: string }>(
  people: T[],
  pending: PendingPersonNavigation,
): T | null {
  if (pending.entity_uuid) {
    return people.find(p => p.entity_uuid === pending.entity_uuid) ?? null;
  }
  const name = pending.person?.trim();
  if (!name) return null;
  const exact = people.filter(p => p.display_name.localeCompare(name, undefined, { sensitivity: 'base' }) === 0);
  return exact.length === 1 ? exact[0] : null;
}
