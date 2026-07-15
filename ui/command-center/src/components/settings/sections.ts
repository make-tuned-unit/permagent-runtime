// Settings panes, keyed by section id. Single source of truth for which
// deep-link sections are navigable — mirrors the `PANELS` map in SettingsView.
//
// An agent "Settings → <pane>" deep-link arrives as an `app_navigate` event
// carrying a `section` (see app_conductor.rs / events::app_navigate). It is
// resolved through here so a missing or legacy key lands on the default Persona
// pane instead of a blank screen (the old bug: `section` was dropped entirely,
// so every deep-link landed on the default).

export const SETTINGS_SECTION_KEYS = [
  'agent', 'profile', 'preferences', 'memory', 'autonomy', 'tools',
  'models', 'keys', 'devices', 'search', 'appearance', 'shortcuts', 'data',
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
