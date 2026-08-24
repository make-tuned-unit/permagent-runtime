import { describe, it, expect } from 'vitest';
import {
  resolveSettingsSection,
  DEFAULT_SETTINGS_SECTION,
  SETTINGS_SECTION_KEYS,
} from './sections';

// Proves the #5 fix: an agent "Settings → <pane>" deep-link (app_navigate with a
// `section`) is honored rather than dropped. useAppNavigate forwards `section`
// into pendingSettingsSection; SettingsView resolves it through this helper on
// mount. Before the fix `section` was ignored and every deep-link landed on the
// default Persona pane.
describe('resolveSettingsSection', () => {
  it('lands Settings → Devices on the Devices pane (not the default)', () => {
    expect(resolveSettingsSection('devices')).toBe('devices');
    expect(resolveSettingsSection('devices')).not.toBe(DEFAULT_SETTINGS_SECTION);
  });

  it('lands Settings → agent (Persona) on the agent pane', () => {
    expect(resolveSettingsSection('agent')).toBe('agent');
  });

  it('falls back to the default pane when no section is supplied', () => {
    expect(resolveSettingsSection(null)).toBe(DEFAULT_SETTINGS_SECTION);
    expect(resolveSettingsSection(undefined)).toBe(DEFAULT_SETTINGS_SECTION);
    expect(resolveSettingsSection('')).toBe(DEFAULT_SETTINGS_SECTION);
  });

  it('falls back to the default for an unknown/legacy key (e.g. the old "identity")', () => {
    // agent_identity.rs used to point at "identity", which is not a real pane —
    // it is fixed to "agent", but an unknown key must still degrade gracefully.
    expect(resolveSettingsSection('identity')).toBe(DEFAULT_SETTINGS_SECTION);
    expect(resolveSettingsSection('does-not-exist')).toBe(DEFAULT_SETTINGS_SECTION);
  });

  it('degrades the retired "spend" deep-link instead of opening a dead pane', () => {
    // Spend moved to the Financier tab (2026-08-19) and is no longer a Settings
    // section. A stale deep-link — from an older catalog, a saved link, or a
    // model that remembers the old layout — must land somewhere real. Landing
    // on a pane key with no component behind it is a blank screen.
    expect(SETTINGS_SECTION_KEYS).not.toContain('spend');
    expect(resolveSettingsSection('spend')).toBe(DEFAULT_SETTINGS_SECTION);
  });

  it('every declared pane key resolves to itself', () => {
    for (const key of SETTINGS_SECTION_KEYS) {
      expect(resolveSettingsSection(key)).toBe(key);
    }
  });
});
