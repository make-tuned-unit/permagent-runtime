/** Native authentication/supervision uses 3001 except for the explicit review
 * target. Do not embed a developer's backend URL in a downloadable app. */
export function validateNativeBuildInputs(
  frontend: Record<string, unknown>,
  review: { root?: string; port?: string },
): void {
  if (!!review.root !== !!review.port) {
    throw new Error('Native review root and port must be supplied together.');
  }
  const reviewMode = !!review.root;
  const port = review.port;
  if (reviewMode && (!/^\d+$/.test(port!) || Number(port) < 1024 || Number(port) > 65535 || Number(port) === 3001)) {
    throw new Error('Native review port must be an unprivileged non-production port.');
  }
  const expected = `http://127.0.0.1:${reviewMode ? Number(port) : 3001}`;
  for (const key of ['VITE_DAEMON_URL', 'VITE_API_BASE_URL']) {
    const value = frontend[key];
    if (value === undefined || value === '') continue;
    const normalized = typeof value === 'string' ? value.replace(/\/$/, '') : '';
    // Production CSP also permits localhost, but never credentials or paths.
    if (normalized !== expected && !(!reviewMode && normalized === 'http://localhost:3001')) {
      throw new Error(`${key} does not match the native daemon target. Clear the override or build the explicit review target.`);
    }
  }
  if (reviewMode && frontend.VITE_DAEMON_URL !== expected) {
    throw new Error('Native review frontend must explicitly match its daemon target.');
  }
}
