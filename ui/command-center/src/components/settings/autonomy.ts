/**
 * Pure helpers for the Autonomy panel's trust-level (GOOSE_MODE) controls.
 *
 * Re-enable-gate epic part B: the daemon resolves GOOSE_MODE with env-var
 * precedence (crates/goose/src/config/base.rs `get_param`), but GET /config's
 * `config` map is YAML-only. With e.g. `GOOSE_MODE=approve` in the daemon's
 * environment the panel used to highlight "Automatic", show no warning, and
 * silently write YAML that changed nothing. The daemon now also reports
 * `effective_goose_mode` (env-aware); this helper decides when the panel must
 * warn that the environment override makes the YAML selection inert.
 */

/**
 * Returns the warning to show when the daemon's EFFECTIVE mode diverges from
 * the mode this panel controls (the YAML value), or null when no warning is
 * needed.
 *
 * - `effectiveMode` — `effective_goose_mode` from GET /config; null/undefined
 *   when the daemon predates the field (no warning: nothing trustworthy to
 *   compare against).
 * - `selectedMode` — the YAML-backed selection the panel highlights; null
 *   while still loading (no warning: avoid a flash from a half-loaded state).
 */
export function trustEnvOverrideNotice(
  effectiveMode: string | null | undefined,
  selectedMode: string | null,
): string | null {
  if (!effectiveMode || selectedMode === null) return null;
  if (effectiveMode === selectedMode) return null;
  return `The daemon's environment overrides this to '${effectiveMode}' — clear the autonomy-mode override in the environment to control it here.`;
}
