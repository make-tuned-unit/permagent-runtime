/**
 * Devices → Messaging gateways (ST5, dagK settings pass).
 *
 * `/gateway/*` (crates/goose-server/src/routes/gateway.rs) lets the agent talk
 * over Telegram — start/stop, who has paired, unpair — and had no UI. The
 * remote `/tunnel/*` status (the alternative to a tailnet for reaching this
 * hub) rides along here since both answer "how else can this agent be
 * reached", which is the Devices page's question.
 *
 * Connecting a gateway (handing it a bot token) and unpairing a user are both
 * "connect/destructive" actions per the settings ruling — confirmed, not
 * fired on click.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api, type GatewayStatusInfo, type TunnelInfo } from '../../lib/api';
import { ListRow, Row, Section, TextInput } from './atoms';
import { Button } from '../common/Button';
import { ConfirmDialog } from '../common/ConfirmDialog';

const GATEWAY_LABEL: Record<string, string> = { telegram: 'Telegram' };
function gatewayLabel(t: string): string {
  return GATEWAY_LABEL[t] ?? t;
}

const TUNNEL_LABEL: Record<TunnelInfo['state'], string> = {
  Idle: 'Off', Starting: 'Starting…', Running: 'Running', Error: 'Error', Disabled: 'Disabled',
};

function TunnelStatusRow() {
  const { colors } = useTheme();
  const [info, setInfo] = useState<TunnelInfo | null>(null);
  const [unreadable, setUnreadable] = useState(false);

  useEffect(() => {
    let active = true;
    api.getTunnelInfo().then(i => { if (active) { setInfo(i); setUnreadable(false); } }).catch(() => { if (active) setUnreadable(true); });
    return () => { active = false; };
  }, []);

  return (
    <Row
      label="Remote tunnel"
      hint={
        unreadable
          ? "Couldn't read tunnel status."
          : info?.state === 'Running'
            ? `Reachable at ${info.url}. An alternative to a tailnet — this hub is reachable from outside this network while the tunnel runs.`
            : 'Off. Use a tailnet (above) or start the tunnel from the CLI to reach this hub remotely.'
      }
    >
      <span style={{ fontSize: textSize.caption, color: unreadable ? colors.danger : info?.state === 'Running' ? colors.cyan : colors.textMuted }}>
        {unreadable ? 'Unavailable' : info ? TUNNEL_LABEL[info.state] : 'Checking…'}
      </span>
    </Row>
  );
}

function GatewayRow({ gateway, onChanged }: { gateway: GatewayStatusInfo; onChanged: () => void }) {
  const { colors } = useTheme();
  const [confirmStop, setConfirmStop] = useState(false);
  const [confirmUnpair, setConfirmUnpair] = useState<{ platform: string; userId: string } | null>(null);

  const stop = async () => {
    await api.stopGateway(gateway.gateway_type);
    onChanged();
    setConfirmStop(false);
  };
  const unpair = async () => {
    if (!confirmUnpair) return;
    await api.unpairGatewayUser(confirmUnpair.platform, confirmUnpair.userId);
    onChanged();
    setConfirmUnpair(null);
  };

  return (
    <div>
      <ListRow
        title={gatewayLabel(gateway.gateway_type)}
        status={
          <span style={{ fontSize: textSize.caption, color: gateway.running ? colors.cyan : colors.textDim }}>
            {gateway.running ? 'Running' : gateway.configured ? 'Stopped' : 'Not connected'}
          </span>
        }
        detail={gateway.configured
          ? `${gateway.paired_users.length} paired user${gateway.paired_users.length === 1 ? '' : 's'}`
          : 'No bot connected yet.'}
        trailing={gateway.running && (
          <Button
            colors={colors}
            onClick={() => setConfirmStop(true)}
            style={{
              '--pa-btn-fg': colors.danger, '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': `${colors.danger}4D`, '--pa-btn-bg-hover': `${colors.danger}1A`,
              '--pa-btn-pad': '3px 10px', '--pa-btn-radius': `${radius.sm}px`,
              fontSize: textSize.micro, fontFamily: font.body,
            } as CSSProperties}
          >
            Stop
          </Button>
        )}
      />
      {gateway.paired_users.length > 0 && (
        <div style={{ marginTop: space.sm, marginLeft: space.xxl, display: 'flex', flexDirection: 'column', gap: space.xs }}>
          {gateway.paired_users.map(u => (
            <div key={`${u.platform}:${u.user_id}`} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.lg, fontSize: textSize.caption, color: colors.textMuted }}>
              <span>{u.display_name ?? u.user_id}</span>
              <Button
                colors={colors}
                onClick={() => setConfirmUnpair({ platform: u.platform, userId: u.user_id })}
                style={{
                  '--pa-btn-fg': colors.danger, '--pa-btn-pad': '2px 8px', '--pa-btn-radius': `${radius.sm}px`,
                  fontSize: textSize.micro, fontFamily: font.body,
                } as CSSProperties}
              >
                Unpair
              </Button>
            </div>
          ))}
        </div>
      )}
      {confirmStop && (
        <ConfirmDialog
          title={`Stop ${gatewayLabel(gateway.gateway_type)}?`}
          consequence="The bot stops answering immediately. Paired users stay paired — starting it again picks up where it left off."
          confirmLabel="Stop"
          failureLabel="Couldn't stop it"
          onConfirm={stop}
          onCancel={() => setConfirmStop(false)}
        />
      )}
      {confirmUnpair && (
        <ConfirmDialog
          title="Unpair this user?"
          consequence="They stop being able to reach the agent through this gateway until they pair again."
          confirmLabel="Unpair"
          failureLabel="Couldn't unpair them"
          onConfirm={unpair}
          onCancel={() => setConfirmUnpair(null)}
        />
      )}
    </div>
  );
}

function ConnectTelegram({ onConnected }: { onConnected: () => void }) {
  const { colors } = useTheme();
  const [token, setToken] = useState('');
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const connect = async () => {
    try {
      await api.startGateway('telegram', { bot_token: token.trim() });
      onConnected();
      setConfirming(false);
      setToken('');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not connect');
      throw err;
    }
  };

  return (
    <Row label="Connect Telegram" hint="Paste the bot token from @BotFather. The agent can then answer messages sent to that bot.">
      <div style={{ display: 'flex', gap: space.sm, alignItems: 'center', flexWrap: 'wrap' }}>
        <div style={{ flex: '1 1 220px', minWidth: 0 }}>
          <TextInput value={token} onChange={setToken} placeholder="123456:AA...bot token" mono />
        </div>
        <Button
          colors={colors}
          disabled={!token.trim()}
          onClick={() => setConfirming(true)}
          style={{
            '--pa-btn-bg': colors.cyanSoft, '--pa-btn-fg': colors.cyan, '--pa-btn-border': colors.borderHi,
            '--pa-btn-border-hover': colors.cyan, '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-pad': '4px 12px', '--pa-btn-radius': `${radius.sm}px`, '--pa-btn-weight': 600,
            fontSize: textSize.caption, fontFamily: font.body,
          } as CSSProperties}
        >
          Connect
        </Button>
      </div>
      {error && <div style={{ fontSize: textSize.caption, color: colors.danger, marginTop: space.xs }}>{error}</div>}
      {confirming && (
        <ConfirmDialog
          title="Connect this Telegram bot?"
          consequence="The daemon starts listening for messages sent to this bot right away, using the token you pasted. Anyone who messages the bot can reach the agent until they are unpaired."
          confirmLabel="Connect"
          failureLabel="Couldn't connect"
          onConfirm={connect}
          onCancel={() => setConfirming(false)}
        />
      )}
    </Row>
  );
}

export function MessagingGatewaysSection() {
  const { colors } = useTheme();
  const [gateways, setGateways] = useState<GatewayStatusInfo[] | null>(null);
  const [error, setError] = useState(false);

  const refresh = useCallback(() => {
    return api.getGatewayStatus()
      .then(list => { setGateways(list); setError(false); })
      .catch(() => setError(true));
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const telegram = gateways?.find(g => g.gateway_type === 'telegram') ?? null;

  return (
    <Section
      title="Messaging gateways"
      sub="Other ways to reach this agent, besides this app — a Telegram bot, and the remote tunnel that stands in for a tailnet."
    >
      {error && <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>Couldn't read gateway status.</div>}
      {gateways === null && !error && (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading gateways…</div>
      )}
      {telegram && (telegram.configured || telegram.running) ? (
        <GatewayRow gateway={telegram} onChanged={() => { void refresh(); }} />
      ) : gateways !== null ? (
        <ConnectTelegram onConnected={() => { void refresh(); }} />
      ) : null}
      <TunnelStatusRow />
    </Section>
  );
}
