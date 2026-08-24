// Settings panes, keyed by section id. Single source of truth for which
// deep-link sections are navigable — mirrors the `PANELS` map in SettingsView.
//
// An agent "Settings → <pane>" deep-link arrives as an `app_navigate` event
// carrying a `section` (see app_conductor.rs / events::app_navigate). It is
// resolved through here so a missing or legacy key lands on the default Persona
// pane instead of a blank screen (the old bug: `section` was dropped entirely,
// so every deep-link landed on the default).

export const SETTINGS_SECTION_KEYS = [
  'agent', 'preferences', 'memory', 'autonomy', 'tools',
  'models', 'keys', 'devices', 'search', 'appearance', 'shortcuts', 'data',
  'sovereignty',
  // Console pages folded into Settings (2026-08 ruling): Sessions history,
  // Downloads inbox, and the Execution trace ('activity').
  //
  // 'spend' is deliberately NOT here any more. It moved to the Financier tab
  // (2026-08-19), which is now the one place money lives. A deep-link that
  // still says 'spend' therefore falls through `resolveSettingsSection` to the
  // default pane rather than opening a blank one — see the alias test.
  'sessions', 'inbox', 'activity',
  // Settings → Agents (Phase 2 UI over the merged /api/agents surface).
  'agents',
  // Settings → Features: the switches for the off-by-default workers
  // (Initiative, Decision Playbook, Concierge, Steward git-health).
  'features',
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
