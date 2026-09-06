/** Keep diagnostic logs useful without echoing credentials or local paths. */
export function safeWizardError(error: unknown, fallback: string): string {
  const raw = error instanceof Error && error.message ? error.message : fallback;
  return raw
    .replace(/\b(?:api[_-]?key|token|secret|password)\b\s*[:=]\s*[^\s,;)'"`]+/gi, '[credential redacted]')
    .replace(/\bBearer\s+[^\s,;)'"`]+/gi, 'Bearer [credential redacted]')
    .replace(/\b(?:sk|pk|ghp|github_pat|xox[baprs])-[-_A-Za-z0-9]+\b/gi, '[credential redacted]')
    .replace(/(?:\/Users|\/home|\/private\/var|\/tmp)\/[^\s,;)'"`]+/g, '[local path]')
    .slice(0, 240);
}
