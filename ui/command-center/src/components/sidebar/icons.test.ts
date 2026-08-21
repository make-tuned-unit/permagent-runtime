import { describe, it, expect } from 'vitest';
import { ICON_PATHS, resolveIconPath } from './icons';

/**
 * The point of resolving by name is that EXISTING installs — whose
 * `workspaces.icon` column still holds the old keys — get the corrected
 * glyphs without a migration. If that ever regresses, the sidebar silently
 * goes back to three interchangeable rectangles, which is not the kind of
 * failure anyone files a bug for. So it is pinned.
 */
describe('resolveIconPath', () => {
  it('gives canonical workspaces their new glyph even when the DB holds the OLD key', () => {
    expect(resolveIconPath('Build', 'code')).toBe(ICON_PATHS.brackets);
    expect(resolveIconPath('Automate', 'layout-dashboard')).toBe(ICON_PATHS.bolt);
    expect(resolveIconPath('Projects', 'columns')).toBe(ICON_PATHS.folder);
  });

  it('gives canonical workspaces the same glyph when the DB holds the NEW key', () => {
    expect(resolveIconPath('Build', 'brackets')).toBe(ICON_PATHS.brackets);
    expect(resolveIconPath('Automate', 'bolt')).toBe(ICON_PATHS.bolt);
    expect(resolveIconPath('Projects', 'folder')).toBe(ICON_PATHS.folder);
  });

  it('leaves the other canonical workspaces alone', () => {
    expect(resolveIconPath('Home', 'home')).toBe(ICON_PATHS.home);
    expect(resolveIconPath('World', 'globe')).toBe(ICON_PATHS.globe);
    expect(resolveIconPath('Brain', 'brain')).toBe(ICON_PATHS.brain);
    expect(resolveIconPath('Grow', 'trending-up')).toBe(ICON_PATHS['trending-up']);
    expect(resolveIconPath('Finance', 'coin')).toBe(ICON_PATHS.coin);
    expect(resolveIconPath('People', 'users')).toBe(ICON_PATHS.users);
  });

  it('honours a custom workspace\'s own icon key', () => {
    expect(resolveIconPath('My Workspace', 'globe')).toBe(ICON_PATHS.globe);
  });

  it('does NOT give a custom workspace the bolt just because it carries the column default', () => {
    // `layout-dashboard` is the schema default for workspaces.icon. Aliasing
    // that key globally to the Automate bolt would brand every user-created
    // workspace as an automation tab.
    expect(resolveIconPath('My Workspace', 'layout-dashboard'))
      .toBe(ICON_PATHS['layout-dashboard']);
    expect(resolveIconPath('My Workspace', 'layout-dashboard'))
      .not.toBe(ICON_PATHS.bolt);
  });

  it('falls back to home for an unknown key rather than rendering nothing', () => {
    expect(resolveIconPath('Whatever', 'no-such-icon')).toBe(ICON_PATHS.home);
  });

  it('every glyph is non-empty, valid-looking path data', () => {
    for (const [key, d] of Object.entries(ICON_PATHS)) {
      expect(d.length, `${key} is empty`).toBeGreaterThan(0);
      expect(d, `${key} does not start with a move command`).toMatch(/^M/);
      expect(d, `${key} contains NaN`).not.toContain('NaN');
    }
  });

  it('the three reworked glyphs are distinct from each other and from the old ones', () => {
    const reworked = [ICON_PATHS.folder, ICON_PATHS.brackets, ICON_PATHS.bolt];
    expect(new Set(reworked).size).toBe(3);
    const legacy = [ICON_PATHS.columns, ICON_PATHS.code, ICON_PATHS['layout-dashboard']];
    for (const glyph of reworked) expect(legacy).not.toContain(glyph);
  });
});
