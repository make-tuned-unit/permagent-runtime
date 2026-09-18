/**
 * The Grow panel's two poll cadences.
 *
 * Their own module because `GrowActions` is the only caller and the numbers are
 * a claim about the server, not about the view: one is a progress check on a
 * job the user started, the other is the backstop for a nightly sweep with no
 * event of its own. Naming them here keeps that reasoning attached to the
 * numbers rather than to whichever component happened to hold them.
 */

/** How often the Grow panel re-reads while a review is running on the server.
 *  Only ticks while `generating` is true (see `GrowActions`), so this is a
 *  progress check on a job the user started, not a background poll. */
export const GENERATION_POLL_MS = 4000;

/** How often the Actions and Results lenses re-read while they are the surface
 *  on screen. Deliberately slow: this is the backstop for the nightly sweep's
 *  missing event (R1.4), not a live wire, and a judged 7-day window is not a
 *  fact that needs to land inside a second. When `growth_sweep` learns to emit
 *  `project_changed`, `projectsRev` becomes the fast path and this stays as the
 *  belt — the same shape the review poll already has. */
export const VERDICT_POLL_MS = 120_000;
