import { useState, useEffect, useCallback } from 'react';
import { ArrowLeft } from 'lucide-react';
import { useCommandCenter } from '../../lib/store';

interface IntegrationStatus {
  provider: string;
  connected: boolean;
  scopes: string | null;
  error: string | null;
}

const GMAIL_SCOPES = 'https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.send';

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

function isTauriEnv(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  // Esc key dismisses Settings
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        dismiss();
      }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [dismiss]);

  const [gmailStatus, setGmailStatus] = useState<IntegrationStatus | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);
  const [clientId, setClientId] = useState(() => localStorage.getItem('permagent_gmail_client_id') || '');
  const [clientSecret, setClientSecret] = useState(() => localStorage.getItem('permagent_gmail_client_secret') || '');
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [tauri, setTauri] = useState(false);

  const loadStatus = useCallback(async () => {
    if (!isTauriEnv()) return;
    try {
      const status = await tauriInvoke<IntegrationStatus>('get_integration_status', { provider: 'gmail' });
      setGmailStatus(status);
    } catch {
      // Not in Tauri context
    }
  }, []);

  useEffect(() => {
    setTauri(isTauriEnv());
    loadStatus();
  }, [loadStatus]);

  const saveCredentials = () => {
    localStorage.setItem('permagent_gmail_client_id', clientId);
    localStorage.setItem('permagent_gmail_client_secret', clientSecret);
    setMessage({ type: 'success', text: 'Credentials saved' });
  };

  const connectGmail = async () => {
    if (!clientId || !clientSecret) {
      setMessage({ type: 'error', text: 'Enter Client ID and Client Secret first' });
      return;
    }
    setIsConnecting(true);
    setMessage(null);
    try {
      saveCredentials();
      const result = await tauriInvoke<string>('start_oauth', {
        provider: 'gmail',
        clientId,
        clientSecret,
        scopes: GMAIL_SCOPES,
      });
      setMessage({ type: 'success', text: result });
      await loadStatus();
    } catch (e) {
      setMessage({ type: 'error', text: String(e) });
    } finally {
      setIsConnecting(false);
    }
  };

  const disconnectGmail = async () => {
    setMessage(null);
    try {
      await tauriInvoke('disconnect_integration', { provider: 'gmail' });
      setMessage({ type: 'success', text: 'Gmail disconnected' });
      await loadStatus();
    } catch (e) {
      setMessage({ type: 'error', text: String(e) });
    }
  };

  return (
    <div className="flex flex-col h-full bg-[#0B1120] text-dark-text overflow-y-auto">
      <div className="border-b border-dark-border px-6 py-4 flex items-center gap-3">
        <button
          onClick={dismiss}
          className="p-1.5 rounded hover:bg-white/5 text-dark-muted hover:text-dark-text transition-colors"
          title="Back to workspace (Esc)"
        >
          <ArrowLeft size={16} />
        </button>
        <div>
          <h1 className="text-lg font-semibold">Settings</h1>
          <p className="text-xs text-dark-muted mt-0.5">Manage integrations and connections</p>
        </div>
      </div>

      <div className="p-6 max-w-2xl space-y-6">
        {/* Gmail Integration */}
        <section className="rounded-lg border border-dark-border bg-[#111827] p-5">
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded bg-red-500/20 flex items-center justify-center text-red-400 text-sm font-bold">
                G
              </div>
              <div>
                <h2 className="font-semibold">Gmail</h2>
                <p className="text-xs text-dark-muted">Read and send email via Google OAuth 2.0</p>
              </div>
            </div>
            {gmailStatus && (
              <span className={`text-xs px-2 py-1 rounded ${
                gmailStatus.connected
                  ? 'bg-emerald-500/20 text-emerald-400'
                  : 'bg-gray-500/20 text-gray-400'
              }`}>
                {gmailStatus.connected ? 'Connected' : 'Disconnected'}
              </span>
            )}
          </div>

          {gmailStatus?.connected && gmailStatus.scopes && (
            <div className="mb-4 p-3 rounded bg-[#0B1120] text-xs">
              <span className="text-dark-muted">Scopes: </span>
              <span className="text-dark-text">{gmailStatus.scopes}</span>
            </div>
          )}

          {!tauri && (
            <div className="p-3 rounded bg-amber-500/10 border border-amber-500/20 text-xs text-amber-400 mb-4">
              OAuth requires the Permagent desktop app. Run <code className="bg-black/30 px-1 rounded">npm run tauri:dev</code> to use this feature.
            </div>
          )}

          {tauri && !gmailStatus?.connected && (
            <div className="space-y-3 mb-4">
              <div>
                <label className="block text-xs text-dark-muted mb-1">Google OAuth Client ID</label>
                <input
                  type="text"
                  value={clientId}
                  onChange={(e) => setClientId(e.target.value)}
                  placeholder="123456789.apps.googleusercontent.com"
                  className="w-full px-3 py-2 rounded bg-[#0B1120] border border-dark-border text-sm text-dark-text placeholder:text-dark-muted/50 focus:outline-none focus:border-accent"
                />
              </div>
              <div>
                <label className="block text-xs text-dark-muted mb-1">Google OAuth Client Secret</label>
                <input
                  type="password"
                  value={clientSecret}
                  onChange={(e) => setClientSecret(e.target.value)}
                  placeholder="GOCSPX-..."
                  className="w-full px-3 py-2 rounded bg-[#0B1120] border border-dark-border text-sm text-dark-text placeholder:text-dark-muted/50 focus:outline-none focus:border-accent"
                />
              </div>
            </div>
          )}

          {tauri && (
            <div className="flex gap-2">
              {gmailStatus?.connected ? (
                <button
                  type="button"
                  onClick={disconnectGmail}
                  className="px-4 py-2 text-sm rounded border border-red-500/30 text-red-400 hover:bg-red-500/10 transition-colors"
                >
                  Disconnect
                </button>
              ) : (
                <button
                  type="button"
                  onClick={connectGmail}
                  disabled={isConnecting || !clientId || !clientSecret}
                  className="px-4 py-2 text-sm rounded bg-accent text-white hover:bg-accent/80 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                >
                  {isConnecting ? 'Waiting for Google...' : 'Connect Gmail'}
                </button>
              )}
            </div>
          )}

          {message && (
            <div className={`mt-3 p-3 rounded text-xs ${
              message.type === 'success'
                ? 'bg-emerald-500/10 border border-emerald-500/20 text-emerald-400'
                : 'bg-red-500/10 border border-red-500/20 text-red-400'
            }`}>
              {message.text}
            </div>
          )}
        </section>

        {/* Info */}
        <section className="rounded-lg border border-dark-border bg-[#111827] p-5">
          <h2 className="font-semibold mb-2">OAuth Configuration</h2>
          <div className="text-xs text-dark-muted space-y-2">
            <p>To connect Gmail, create OAuth credentials in the Google Cloud Console:</p>
            <ol className="list-decimal list-inside space-y-1 ml-2">
              <li>Go to APIs &amp; Services &gt; Credentials</li>
              <li>Create OAuth 2.0 Client ID (Desktop app type)</li>
              <li>Add <code className="bg-black/30 px-1 rounded">http://127.0.0.1:9844/callback</code> as authorized redirect URI</li>
              <li>Copy the Client ID and Client Secret above</li>
            </ol>
            <p className="mt-2">
              Tokens are stored in <code className="bg-black/30 px-1 rounded">~/.permagent/secrets/gmail_token.json</code> with mode 600.
            </p>
          </div>
        </section>
      </div>
    </div>
  );
}
