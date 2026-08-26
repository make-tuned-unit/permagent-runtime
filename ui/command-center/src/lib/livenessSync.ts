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
 *
 * A SECOND lane, {@link APPLY_BY_TYPE}, handles frames whose payload IS the
 * point rather than a "go refetch" trigger — currently the Build tab's coding
 * harness spend (`session_spend_changed`, the daemon's per-turn announcement
 * of the CLI harness's own session ledger — see costMeter.ts's `CodingSpend`
 * for why this is a different account from the chat session's `liveTokens`).
 * Debouncing a spend frame would mean the Build meter sits on a stale number
 * for up to 250ms after every turn for no benefit (the daemon already coalesces
 * to at most one frame per turn), so this lane applies its handler to the
 * store IMMEDIATELY — still gated by the same {@link frameReplayed} check,
 * because a reconnect replay burst must not stomp a live total with a stale
 * historical one.
 */

import { useCommandCenter } from './store';
import { wireEventType } from './wireEvent';
import { frameReplayed } from '../components/world/shared/worldEvents';
import type { CodingSpend } from './costMeter';

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

/** Coerce one wire field to a finite number, else 0 — this is untrusted wire
 *  data, and a `NaN`/`undefined` spend must never propagate into a `$` render. */
function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : 0;
}

/** Coerce one wire field to a string, else null — mirrors `num` for the
 *  optional string fields (`provider`/`model`/`working_dir` may be null). */
function str(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

/** snake_case wire payload → camelCase {@link CodingSpend}, defensively — the
 *  rest of the UI's API types are camelCase, so the conversion happens once,
 *  here, at the /events boundary rather than leaking snake_case downstream. */
function toCodingSpend(payload: unknown): CodingSpend {
  const p = (payload && typeof payload === 'object' ? payload : {}) as Record<string, unknown>;
  return {
    sessionId: str(p.session_id) ?? '',
    turnUsd: num(p.turn_usd),
    sessionUsd: num(p.session_usd),
    todayUsd: num(p.today_usd),
    totalTokens: num(p.total_tokens),
    provider: str(p.provider),
    model: str(p.model),
    workingDir: str(p.working_dir),
    estimated: p.estimated === true,
    finalTurn: p.final_turn === true,
  };
}

/** The liveness event kinds whose PAYLOAD is applied directly to the store,
 *  with no debounce — see the module doc comment for why this lane exists. */
const APPLY_BY_TYPE: Record<string, (payload: unknown) => void> = {
  session_spend_changed: (payload) => {
    useCommandCenter.getState().setCodingSpend(toCodingSpend(payload));
  },
};

const pending = new Map<string, ReturnType<typeof setTimeout>>();

/** Test seam: cancel all pending debounced refreshes. */
export function _resetLivenessSync(): void {
  for (const t of pending.values()) clearTimeout(t);
  pending.clear();
}

/**
 * Apply one parsed /events frame to the liveness map. Non-liveness frames and
 * replayed frames are ignored. A frame matching {@link APPLY_BY_TYPE} applies
 * its payload to the store immediately; a frame matching {@link REFRESH_BY_TYPE}
 * schedules a trailing-debounced store refresh keyed by event kind.
 */
export function applyLivenessFrame(event: unknown, connectionEpoch: number): void {
  const type = wireEventType(event);

  const apply = APPLY_BY_TYPE[type];
  if (apply) {
    if (frameReplayed(event, connectionEpoch)) return;
    const payload = (event && typeof event === 'object' ? (event as { payload?: unknown }).payload : undefined);
    apply(payload);
    return;
  }

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
