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
export function emitActivity(
  eventType: string,
  sourceSurface: string,
  payload: Record<string, unknown> = {},
): void {
  import('@tauri-apps/api/core')
    .then(({ invoke }) => {
      invoke('emit_activity', {
        event_type: eventType,
        source_surface: sourceSurface,
        payload,
      });
    })
    .catch(() => {
      /* not in a Tauri context — nothing to report */
    });
}
