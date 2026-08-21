/**
 * Sidebar glyphs.
 *
 * Three of these were replaced on 2026-08-03 because their names lied about
 * what they drew, and the drawings were interchangeable:
 *
 *   - `code` (Build) drew four rectangles — a dashboard mosaic, not code.
 *   - `layout-dashboard` (Automate) drew three horizontal lines — a hamburger.
 *   - `columns` (Projects) drew three equal full-height bars — reads "layout".
 *
 * Three abstract geometric shapes in a vertical rail are not distinguishable at
 * 18px, which is why Build, Automate and Projects were the ones people got
 * wrong. The replacements each borrow an established convention instead:
 * `</>` for code, a lightning bolt for automation, and a folder for projects.
 * All three were rendered at the real 18px and compared before being picked —
 * path data that looks fine at 72px routinely turns to mush in the rail.
 */
export const ICON_PATHS: Record<string, string> = {
  home: 'M3 11l9-8 9 8v9a2 2 0 01-2 2h-4v-7h-6v7H5a2 2 0 01-2-2v-9z',
  globe: 'M12 2a10 10 0 100 20 10 10 0 000-20zM2 12h20M12 2a15 15 0 014 10 15 15 0 01-4 10M12 2a15 15 0 00-4 10 15 15 0 004 10',
  'trending-up': 'M3 17l6-6 4 4 8-8M17 7h4v4',
  /** Finance — a coin. Distinct from Grow's trend line at 18px. */
  coin: 'M12 2a10 10 0 100 20 10 10 0 000-20zM12 6a6 6 0 100 12 6 6 0 000-12z',
  brain: 'M9 4a4 4 0 00-4 4 3 3 0 00-1 5.5A3 3 0 005 18a4 4 0 004 3M15 4a4 4 0 014 4 3 3 0 011 5.5A3 3 0 0119 18a4 4 0 01-4 3M9 4a3 3 0 013 3v14M15 4a3 3 0 00-3 3',

  /** Projects — a folder. Chosen over a kanban-board glyph after rendering
   *  both at the real 18px: a framed rectangle with inner column strokes turns
   *  to mush at that size, and it stays inside the same abstract-rectangle
   *  family that made these tabs indistinguishable in the first place. */
  folder: 'M3 7a2 2 0 012-2h4l2 2.5h8a2 2 0 012 2V18a2 2 0 01-2 2H5a2 2 0 01-2-2z',
  /** Build — `</>`, the one glyph everybody already reads as code. */
  brackets: 'M8 6l-5 6 5 6M16 6l5 6-5 6M13.5 4l-3 16',
  /** Automate — a lightning bolt, the convention shared by every automation
   *  tool the user has met (Shortcuts, Zapier, IFTTT). */
  bolt: 'M13 2L4 14h6l-1 8 9-12h-6z',
  /** People — two figures. The one glyph that reads as a directory of people
   *  at 18px without collapsing into the same rectangle-family as the rest. */
  users: 'M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M9 11a4 4 0 100-8 4 4 0 000 8zM23 21v-2a4 4 0 00-3-3.87M16 3.13a4 4 0 010 7.75',

  // ── Legacy keys ──────────────────────────────────────────────────────────
  // Still present in existing databases (the workspaces.icon column persists
  // whatever was seeded). Kept so an installed profile renders SOMETHING
  // rather than falling through to the home glyph. Canonical workspaces are
  // resolved by name first — see resolveIconPath — so these are only reached
  // by a custom workspace that happens to carry an old key.
  'layout-dashboard': 'M4 7h16M4 12h16M4 17h10',
  code: 'M4 4h7v7H4zM13 4h7v4h-7zM13 10h7v10h-7zM4 13h7v7H4z',
  columns: 'M4 3h4v18H4zM10 3h4v18h-4zM16 3h4v18h-4z',
};

/**
 * The glyph each canonical workspace should show, keyed by name.
 *
 * Resolving by NAME rather than by the stored icon key is what lets the new
 * glyphs reach existing installs with no migration: the `workspaces.icon`
 * column still says `code` on a profile seeded months ago, and rewriting those
 * rows would mean a migration that can only run once and silently does nothing
 * if it fails. It also avoids aliasing `layout-dashboard` globally — that key
 * is the column DEFAULT, so a user-created workspace would inherit whatever it
 * was aliased to.
 */
const CANONICAL_ICON_BY_NAME: Record<string, string> = {
  Home: 'home',
  Projects: 'folder',
  People: 'users',
  Build: 'brackets',
  Grow: 'trending-up',
  Finance: 'coin',
  Automate: 'bolt',
  World: 'globe',
  Brain: 'brain',
};

/**
 * Pick a workspace's glyph: canonical name wins, then the stored icon key,
 * then the home glyph as a last resort.
 */
export function resolveIconPath(name: string, iconKey: string): string {
  const canonical = CANONICAL_ICON_BY_NAME[name];
  if (canonical && ICON_PATHS[canonical]) return ICON_PATHS[canonical];
  return ICON_PATHS[iconKey] ?? ICON_PATHS.home;
}
