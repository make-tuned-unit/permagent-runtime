/**
 * Fire-and-forget emit of an activity event onto the daemon's activity bus, via
 * the same Tauri `emit_activity` IPC the Terminal / Browser / Projects surfaces
 * already use. This is how a user-facing surface reports genuine engagement so
 * the runtime learns what the user actually does — feeding feature-usage
 * tracking (agent-led onboarding) and the ambient awareness layer. No new bus:
 * the daemon consumes this exactly like every other activity event.
 *
 * No-op outside a Tauri context (best-effort — usage tracking must never break a
 * surface). The `eventType` / `sourceSurface` strings MUST match the daemon's
 * snake_case `ActivityEventType` / `SourceSurface` serde names; those spellings
 * are locked by the `onboarding_wire_spellings_are_stable` Rust test.
 */

/**
 * Wire spellings of the engagement events the Command Center UI emits.
 * Hand-kept mirror of the snake_case serde names in
 * `crates/goose/src/events/activity.rs` (locked there by the
 * `onboarding_wire_spellings_are_stable` test) — a typo would 400 at the
 * daemon and silently drop the signal, so the union turns that class of bug
 * into a compile error. Extend it (and the Rust enum) together.
 *
 * `devices_paired` is listed for `api.ts`'s pairing-capture seam, which posts
 * straight to `/activity/emit` (a paired browser has no Tauri IPC).
 */
export type ActivityEventName =
  | 'persona_configured'
  | 'decision_resolved'
  | 'devices_paired'
  | 'pairing_link_copied'
  | 'dictation_completed'
  | 'world_view_opened'
  | 'inbox_opened'
  | 'grow_opened'
  | 'finance_opened'
  | 'brain_opened';

/** Snake_case `SourceSurface` wire names for the surfaces the UI emits from. */
export type ActivitySourceSurface =
  | 'settings'
  | 'dashboard'
  | 'world'
  | 'inbox'
  | 'grow'
  | 'finance'
  | 'brain'
  | 'voice';

export function emitActivity(
  eventType: ActivityEventName,
  sourceSurface: ActivitySourceSurface,
  payload: Record<string, unknown> = {},
): void {
  import('@tauri-apps/api/core')
    .then(({ invoke }) =>
      // Return the invoke promise into the chain: a daemon rejection (4xx/5xx,
      // missing token) must land in the catch below, not surface as an
      // unhandled promise rejection.
      invoke('emit_activity', {
        event_type: eventType,
        source_surface: sourceSurface,
        payload,
      }),
    )
    .catch((err) => {
      // Not in a Tauri context (the import failed) or the daemon rejected the
      // event. Usage tracking is best-effort: note it at debug level, never throw.
      console.debug('[activity] emit dropped:', eventType, err);
    });
}
