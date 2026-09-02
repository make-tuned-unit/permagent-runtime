/**
 * Every setting is still reachable, and every deep link still lands.
 *
 * Consolidating nine panes into four moves things behind a different rail row,
 * and the way that breaks is silent: a `section` key the daemon has been
 * sending for months resolves to a pane that no longer exists, `PANELS[key]` is
 * `undefined`, and the user gets an empty right-hand column with no error. That
 * is the exact bug `resolveSettingsSection` was written for the first time
 * round, when `section` was dropped entirely and every deep link landed on
 * Persona.
 *
 * So the two halves are pinned from both ends:
 *   - every key the resolver accepts has a pane to land on, and
 *   - every key that names one of History's four segments opens History ON
 *     that segment, because "Settings → Spend" landing on Sessions is the same
 *     failure wearing a nicer coat.
 */

import { describe, expect, it } from 'vitest';
import { PANELS, paneForSection } from './SettingsView';
import { SETTINGS_SECTION_KEYS, resolveSettingsSection, DEFAULT_SETTINGS_SECTION } from './sections';
import { HISTORY_TAB_KEYS, isHistoryTab } from '../history/HistoryView';

describe('settings reachability', () => {
  it('gives every accepted section key a pane to land on', () => {
    const orphans = SETTINGS_SECTION_KEYS.filter(k => !PANELS[paneForSection(k)]);
    expect(orphans, 'this key resolves to a pane that does not exist — a blank right-hand column').toEqual([]);
  });

  it('lands the default on a real pane', () => {
    expect(PANELS[paneForSection(DEFAULT_SETTINGS_SECTION)]).toBeTruthy();
  });

  it('routes an unknown key to the default rather than to nothing', () => {
    // A daemon older or newer than this build, or a legacy key nobody kept.
    const pane = paneForSection(resolveSettingsSection('does-not-exist'));
    expect(pane).toBe(paneForSection(DEFAULT_SETTINGS_SECTION));
    expect(PANELS[pane]).toBeTruthy();
  });

  it("sends each of History's four record keys to History, keeping the segment", () => {
    for (const key of HISTORY_TAB_KEYS) {
      expect(SETTINGS_SECTION_KEYS as readonly string[]).toContain(key);
      expect(paneForSection(key)).toBe('history');
      // The pane reads the raw section back to pick its segment; if this
      // narrowing failed, every one of them would open on Sessions.
      expect(isHistoryTab(key)).toBe(true);
    }
  });

  it('keeps every retired pane key resolving to the pane that absorbed it', () => {
    // Written out rather than derived: these are the promises made to callers
    // outside this repo (app_conductor.rs deep links, the agent's own phrasing),
    // and a table that derives itself from the code cannot catch the code
    // changing.
    expect(paneForSection('features')).toBe('agents');
    expect(paneForSection('search')).toBe('services');
    expect(paneForSection('sources')).toBe('services');
    expect(paneForSection('data')).toBe('privacy');
    expect(paneForSection('sovereignty')).toBe('privacy');
    for (const k of ['features', 'search', 'sources', 'data', 'sovereignty']) {
      expect(resolveSettingsSection(k), `${k} must still be accepted`).toBe(k);
    }
  });

  it('has no pane nothing can reach', () => {
    const reachable = new Set(SETTINGS_SECTION_KEYS.map(paneForSection));
    const unreachable = Object.keys(PANELS).filter(p => !reachable.has(p));
    expect(unreachable, 'this pane is rendered by nothing — either wire a key to it or delete it').toEqual([]);
  });
});
