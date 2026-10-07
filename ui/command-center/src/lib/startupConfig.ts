export type StartupConfigResult = 'app' | 'wizard' | 'malformed';

/**
 * `/config` is a typed object response. A missing wizard flag is a legitimate
 * first-run config (the backend omits unset keys); a non-object config or a
 * non-boolean flag is a broken response and must not silently open onboarding.
 *
 * The one classifier for startup: BootScreen (which owns the connect loop) and
 * the tests both use it, so "wizard or app" is decided in one place.
 */
export function classifyStartupConfig(payload: unknown): StartupConfigResult {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return 'malformed';
  const config = (payload as { config?: unknown }).config;
  if (!config || typeof config !== 'object' || Array.isArray(config)) return 'malformed';
  const wizardComplete = (config as Record<string, unknown>).wizard_complete;
  if (wizardComplete !== undefined && typeof wizardComplete !== 'boolean') return 'malformed';
  return wizardComplete === true ? 'app' : 'wizard';
}
