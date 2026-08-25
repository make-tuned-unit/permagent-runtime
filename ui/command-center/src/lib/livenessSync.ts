/**
 * Multi-client liveness (#629): the /events → surface-invalidation seam.
 *
 * When two devices are open, a write on one emits a domain event on the daemon
 * bus (`workspace_changed`, `project_changed`, `person_changed`,
 * `identity_changed`, `session_changed` — all emitted only on REAL mutations,
 * Rust-side). This module maps each of those events to the store refresh that
 * makes the corresponding surface refetch:
 *
 *   workspace_changed → refreshWorkspaces()   (layouts update, active preserved)
 *   project_changed   → bumpProjects()        (projects list + Documents/
 *                                              Memories/Notes panels refetch)
 *   person_changed    → bumpPeople()          (People panel refetches)
 *   person_merged     → bumpPeople(), and if the open person-detail IS the
 *                       duplicate that just got absorbed, close it (its id no
 *                       longer resolves) — see the handler below for why this
 *                       closes rather than re-opens on the survivor.
 *   identity_changed  → refreshIdentity()     (agentName + identityRev bump →
 *                                              chat header / nameplate / settings)
 *   session_changed   → loadSessions()        (sessions overlay re-reads)
 *
 * Wired into {@link routeGlobalFrame} (hooks/useAppNavigate.ts) — the ONE
 * global /events subscription in the main window — so no surface opens its own
 * socket for this.
 *
 * Replay honesty: the daemon replays its buffer (≤1000 frames) on every
 * (re)connect. Refetching current state is harmless once, but a reconnect
 * burst would fire hundreds of refetches — so replayed frames are dropped with
 * the same shared classifier the action frames use ({@link frameReplayed}).
 *
 * Debounce honesty: this is NOT a poll — nothing fires without a real event.
 * The trailing debounce only coalesces a burst of events of the same kind
 * (e.g. a multi-file document upload) into one refetch.
 */

import { useCommandCenter } from './store';
import { wireEventType } from './wireEvent';
import { frameReplayed } from '../components/world/shared/worldEvents';

/** Trailing-debounce window per event kind (ms). */
export const LIVENESS_DEBOUNCE_MS = 250;

/** The liveness event kinds and the store refresh each one drives. Handlers
 *  that need the frame's payload (currently only `person_merged`) accept it;
 *  the rest ignore the argument. */
const REFRESH_BY_TYPE: Record<string, (event: unknown) => void> = {
  workspace_changed: () => { void useCommandCenter.getState().refreshWorkspaces(); },
  project_changed: () => useCommandCenter.getState().bumpProjects(),
  person_changed: () => useCommandCenter.getState().bumpPeople(),
  // #1090: a merge on another client makes the duplicate's id stop resolving.
  // The directory/graph refetch via bumpPeople() either way; additionally, if
  // THIS client has the duplicate's detail open, close it rather than silently
  // re-pointing to the survivor — re-opening would need the survivor's full
  // Person record, which this event does not carry (only the two uuids +
  // merge_id) and `openPersonDetail` has no id-only fetch path. Closing is the
  // honest move: no stale panel left pointing at an id that no longer exists.
  person_merged: (event: unknown) => {
    const state = useCommandCenter.getState();
    state.bumpPeople();
    const payload = (event as { payload?: Record<string, unknown> } | null)?.payload;
    const duplicateUuid = typeof payload?.duplicate_uuid === 'string' ? payload.duplicate_uuid : null;
    if (duplicateUuid && state.personDetail?.person.entity_uuid === duplicateUuid) {
      state.closePersonDetail();
    }
  },
  identity_changed: () => { void useCommandCenter.getState().refreshIdentity(); },
  session_changed: () => { void useCommandCenter.getState().loadSessions(); },
};

const pending = new Map<string, ReturnType<typeof setTimeout>>();

/** Test seam: cancel all pending debounced refreshes. */
export function _resetLivenessSync(): void {
  for (const t of pending.values()) clearTimeout(t);
  pending.clear();
}

/**
 * Apply one parsed /events frame to the liveness map. Non-liveness frames and
 * replayed frames are ignored. Live frames schedule a trailing-debounced store
 * refresh keyed by event kind.
 */
export function applyLivenessFrame(event: unknown, connectionEpoch: number): void {
  const type = wireEventType(event);
  const refresh = REFRESH_BY_TYPE[type];
  if (!refresh) return;
  if (frameReplayed(event, connectionEpoch)) return;

  const existing = pending.get(type);
  if (existing) clearTimeout(existing);
  pending.set(
    type,
    setTimeout(() => {
      pending.delete(type);
      refresh(event);
    }, LIVENESS_DEBOUNCE_MS),
  );
}
