import { getApiBaseUrl, loadDaemonToken } from './api';

const shortLivedStreamTokenEnabled =
  import.meta.env.PERMAGENT_SHORTLIVED_STREAM_TOKEN === '1';

/** Return the credential to put on a browser stream URL.
 *
 * The feature is build-time gated and defaults off. Minting is deliberately
 * best-effort: every error falls back to the established daemon/device token
 * so enabling the defense-in-depth feature can never break a stream.
 */
export async function getStreamToken(): Promise<string> {
  const daemonToken = await loadDaemonToken();
  if (!daemonToken || !shortLivedStreamTokenEnabled) return daemonToken ?? '';

  try {
    const response = await fetch(`${getApiBaseUrl()}/sse-token`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${daemonToken}` },
    });
    if (!response.ok) return daemonToken;
    const body = (await response.json()) as { token?: unknown };
    return typeof body.token === 'string' && body.token ? body.token : daemonToken;
  } catch {
    return daemonToken;
  }
}
