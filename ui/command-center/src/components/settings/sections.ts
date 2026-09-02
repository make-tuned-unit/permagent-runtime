// Settings panes, keyed by section id. Single source of truth for which
// deep-link sections are navigable — mirrors the `PANELS` map in SettingsView.
//
// An agent "Settings → <pane>" deep-link arrives as an `app_navigate` event
// carrying a `section` (see app_conductor.rs / events::app_navigate). It is
// resolved through here so a missing or legacy key lands on the default Persona
// pane instead of a blank screen (the old bug: `section` was dropped entirely,
// so every deep-link landed on the default).

export const SETTINGS_SECTION_KEYS = [
  // ── Panes ──
  'agent', 'preferences', 'memory', 'autonomy', 'agents',
  'tools', 'models', 'keys', 'devices', 'services',
  'appearance', 'shortcuts', 'privacy',
  'history',
  // ── Legacy keys, still accepted ──
  // Deep links are written by the daemon (app_conductor.rs), by the agent's own
  // phrasing, and by this app's internal `goto()` calls, and none of them are
  // versioned. A pane consolidation must not make a live caller land on the
  // wrong page, so the OLD key still resolves — `SECTION_HOME` in SettingsView
  // maps it to the pane that owns it now, and History reads the key itself to
  // open on the segment that was asked for.
  //   sessions/inbox/activity/spend → History (four segments)
  //   features                      → Agents (its six toggles were a second
  //                                   writer of the roster's gate keys)
  //   search/sources                → Services
  //   data/sovereignty              → Privacy & data
  'sessions', 'inbox', 'activity', 'spend',
  'features',
  'search', 'sources',
  'data', 'sovereignty',
] as const;

export type SettingsSectionKey = (typeof SETTINGS_SECTION_KEYS)[number];

/** Default Settings pane (Persona). */
export const DEFAULT_SETTINGS_SECTION: SettingsSectionKey = 'agent';

/**
 * Resolve an incoming deep-link section to a real Settings pane key, falling
 * back to the default when the section is absent or unrecognized.
 */
export function resolveSettingsSection(
  section: string | null | undefined,
): SettingsSectionKey {
  if (section && (SETTINGS_SECTION_KEYS as readonly string[]).includes(section)) {
    return section as SettingsSectionKey;
  }
  return DEFAULT_SETTINGS_SECTION;
}
