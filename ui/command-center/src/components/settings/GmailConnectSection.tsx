/**
 * Services → Gmail (ST5, dagK settings pass).
 *
 * `/integrations/gmail/connect` (crates/goose-server/src/routes/integrations.rs)
 * exchanges an OAuth client id/secret for the consent URL to open — the
 * daemon runs a short local callback listener that finishes the exchange once
 * the user consents in their browser. This app only mints and opens that URL;
 * it never sees Google's redirect. `GET /integrations` already reports
 * connection status and is read elsewhere (`ConciergePrecondition`); this is
 * the one place that can actually start or end the connection.
 *
 * Connecting hands Google's consent screen an OAuth client secret, and
 * disconnecting throws away the stored token — both are confirmed.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api } from '../../lib/api';
import { Row, Section, TextInput } from './atoms';
import { Button } from '../common/Button';
import { ConfirmDialog } from '../common/ConfirmDialog';

export function GmailConnectSection() {
  const { colors } = useTheme();
  const [connected, setConnected] = useState<boolean | null>(null);
  const [unreadable, setUnreadable] = useState(false);
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [confirmConnect, setConfirmConnect] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    return api.getIntegrations()
      .then(list => {
        setConnected(list.find(i => i.provider === 'gmail')?.connected ?? false);
        setUnreadable(false);
      })
      .catch(() => setUnreadable(true));
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const connect = async () => {
    setError(null);
    try {
      const r = await api.gmailConnect(clientId.trim(), clientSecret.trim());
      // Opens the real Google consent screen in the user's own browser — this
      // app never touches the credentials Google collects there.
      window.open(r.auth_url, '_blank', 'noopener,noreferrer');
      setConfirmConnect(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not start the connection');
      throw err;
    }
  };

  const disconnect = async () => {
    await api.gmailDisconnect();
    await refresh();
    setConfirmDisconnect(false);
  };

  return (
    <Section
      title="Gmail"
      sub="Lets the Concierge read your inbox (read-only) and propose reply drafts. It never sends or changes mail."
    >
      <Row label="Status" hint={unreadable ? "Couldn't read connection status." : undefined}>
        <span style={{ fontSize: textSize.caption, color: unreadable ? colors.danger : connected ? colors.cyan : colors.textMuted }}>
          {unreadable ? 'Unavailable' : connected === null ? 'Checking…' : connected ? 'Connected' : 'Not connected'}
        </span>
      </Row>
      {connected ? (
        <Row label="Disconnect" hint="Forgets the stored Gmail token. The Concierge goes idle until reconnected.">
          <Button
            colors={colors}
            onClick={() => setConfirmDisconnect(true)}
            style={{
              '--pa-btn-fg': colors.danger, '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': `${colors.danger}4D`, '--pa-btn-bg-hover': `${colors.danger}1A`,
              '--pa-btn-pad': '4px 12px', '--pa-btn-radius': `${radius.sm}px`,
              fontSize: textSize.caption, fontFamily: font.body,
            } as CSSProperties}
          >
            Disconnect
          </Button>
        </Row>
      ) : (
        <>
          <Row label="OAuth client ID" hint="From your Google Cloud OAuth client — this app does not provide one for you.">
            <TextInput value={clientId} onChange={setClientId} placeholder="xxxx.apps.googleusercontent.com" mono />
          </Row>
          <Row label="OAuth client secret" hint="Stored locally, encrypted, never sent anywhere but Google.">
            <TextInput value={clientSecret} onChange={setClientSecret} placeholder="client secret" mono />
          </Row>
          <Row label="Connect" hint="Opens Google's consent screen in your browser to finish connecting.">
            <Button
              colors={colors}
              disabled={!clientId.trim() || !clientSecret.trim()}
              onClick={() => setConfirmConnect(true)}
              style={{
                '--pa-btn-bg': colors.cyanSoft, '--pa-btn-fg': colors.cyan, '--pa-btn-border': colors.borderHi,
                '--pa-btn-border-hover': colors.cyan, '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-pad': '4px 12px', '--pa-btn-radius': `${radius.sm}px`, '--pa-btn-weight': 600,
                fontSize: textSize.caption, fontFamily: font.body,
              } as CSSProperties}
            >
              Connect Gmail
            </Button>
          </Row>
          {error && <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '2px 0 6px' }}>{error}</div>}
        </>
      )}
      {confirmConnect && (
        <ConfirmDialog
          title="Connect Gmail?"
          consequence="Opens Google's consent screen in your default browser. Approving there lets the Concierge read your Gmail inbox (read-only) and propose reply drafts — it can never send or change mail."
          confirmLabel="Continue to Google"
          failureLabel="Couldn't start the connection"
          onConfirm={connect}
          onCancel={() => setConfirmConnect(false)}
        />
      )}
      {confirmDisconnect && (
        <ConfirmDialog
          title="Disconnect Gmail?"
          consequence="Forgets the stored token. The Concierge stops reading mail until you reconnect."
          confirmLabel="Disconnect"
          failureLabel="Couldn't disconnect it"
          onConfirm={disconnect}
          onCancel={() => setConfirmDisconnect(false)}
        />
      )}
    </Section>
  );
}
