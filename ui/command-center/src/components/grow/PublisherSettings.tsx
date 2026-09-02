/**
 * Grow's publishing and media SOURCES: the Postiz key, this project's
 * per-network logins, and the Higgsfield keys the Reel animator needs.
 *
 * Split out of GrowView.tsx (R9). All three are settings rows on the calendar
 * lens — they say what is connected and offer the one control that changes it.
 */

import { useCallback, useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { apiFetch } from '../../lib/api';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import { growAccent, growLink } from './growStyles';
import { FIELD_CLASS, LINK_CLASS, growField } from './growChrome';

type ChannelBinding = {
  integrationId?: string;
  identifier?: string;
  name?: string;
  profile?: string;
};

type PublisherSnap = {
  configured: boolean;
  baseUrl?: string;
  channels?: Record<string, ChannelBinding>;
  pending?: { channel: string } | null;
};

const NETWORKS: { id: string; label: string }[] = [
  { id: 'ig', label: 'Instagram' },
  { id: 'li', label: 'LinkedIn' },
  { id: 'x', label: 'X' },
];

export function PostizConnect({ colors }: { colors: ThemeColors }) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [open, setOpen] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    apiFetch<PublisherSnap>('/api/grow/postiz')
      .then((s) => setConfigured(!!s && !Array.isArray(s) && s.configured))
      .catch(() => setConfigured(false));
  }, []);
  const field = growField(colors, { mono: true });
  const save = async () => {
    setBusy(true);
    try {
      const s = await apiFetch<PublisherSnap>('/api/grow/postiz', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ apiKey, baseUrl: baseUrl.trim() || undefined }),
      });
      setConfigured(s.configured);
      setOpen(false);
      setApiKey('');
    } finally { setBusy(false); }
  };
  return (
    <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.md }}>
      Posting uses your Postiz account (Cloud by default). {configured ? 'API key saved.' : 'Not connected — Approve stays on this calendar until you save a key and log in to a network for this project.'}
      {' '}
      <Button colors={colors} variant="bare" type="button" onClick={() => setOpen((v) => !v)} style={growLink(colors)}>
        {configured ? 'Replace key' : 'Save API key'}
      </Button>
      {open && (
        <div style={{ display: 'flex', gap: space.sm, marginTop: space.md, flexWrap: 'wrap', alignItems: 'center' }}>
          <input value={apiKey} onChange={(e) => setApiKey(e.target.value)} placeholder="Postiz API key" type="password" aria-label="Postiz API key" className={FIELD_CLASS} style={field} />
          <input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://api.postiz.com/public/v1" aria-label="Postiz base URL" className={FIELD_CLASS} style={{ ...field, minWidth: 220 }} />
          <Button colors={colors} type="button" disabled={busy || !apiKey.trim()} onClick={() => save()} style={growAccent(colors, '4px 10px')}>Save</Button>
        </div>
      )}
    </div>
  );
}

export function ProjectChannels({ projectId, colors }: { projectId: string; colors: ThemeColors }) {
  const [snap, setSnap] = useState<PublisherSnap | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loginUrl, setLoginUrl] = useState<string | null>(null);
  const load = useCallback(() => {
    apiFetch<PublisherSnap>(`/api/projects/${encodeURIComponent(projectId)}/publisher`)
      .then((s) => {
        if (!s || Array.isArray(s)) return;
        setSnap(s);
        if (!s.pending) setLoginUrl(null);
      })
      .catch(() => setSnap({ configured: false, channels: {}, pending: null }));
  }, [projectId]);
  useEffect(() => { load(); }, [load]);
  useEffect(() => {
    if (!snap?.pending) return;
    const t = window.setInterval(load, 2000);
    return () => window.clearInterval(t);
  }, [snap?.pending, load]);

  const connect = async (channel: string) => {
    setBusy(channel);
    setError(null);
    try {
      const start = await apiFetch<{ url: string; channel: string; label: string }>(
        `/api/projects/${encodeURIComponent(projectId)}/publisher/connect`,
        { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ channel }) },
      );
      setLoginUrl(start.url);
      if (start.url) {
        // permagentd is a background process; webbrowser::open from it is a
        // no-op on macOS. The webview must open the login itself.
        window.open(start.url, '_blank', 'noopener,noreferrer');
      }
      load();
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally { setBusy(null); }
  };
  const disconnect = async (channel: string) => {
    setBusy(channel);
    setError(null);
    try {
      const next = await apiFetch<PublisherSnap>(
        `/api/projects/${encodeURIComponent(projectId)}/publisher/${encodeURIComponent(channel)}`,
        { method: 'DELETE' },
      );
      setSnap(next);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    } finally { setBusy(null); }
  };

  const channels = snap?.channels ?? {};
  const pending = snap?.pending?.channel;
  const configured = !!snap?.configured;
  const anyBound = NETWORKS.some((n) => channels[n.id]?.integrationId);

  return (
    <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.xl }}>
      <div style={{ marginBottom: space.md }}>
        {anyBound
          ? 'Approve schedules this post on the connected account for that channel.'
          : configured
            ? 'Connect Instagram, LinkedIn, or X for this project. A login window opens; after you sign in, that account is ready to post to.'
            : 'Approve parks a draft on this calendar until this project has a connected account.'}
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
        {NETWORKS.map((n) => {
          const bound = channels[n.id];
          const waiting = pending === n.id;
          const label = bound?.name || bound?.profile
            ? `${n.label} · ${bound.name || bound.profile}`
            : n.label;
          return (
            <div key={n.id} style={{ display: 'flex', alignItems: 'center', gap: space.sm, border: `1px solid ${colors.border}`, borderRadius: radius.md, padding: `${space.xs}px ${space.md}px` }}>
              <span style={{ color: bound ? colors.text : colors.textDim }}>{waiting ? `Waiting for ${n.label} login…` : label}</span>
              {bound ? (
                <Button colors={colors} variant="bare" type="button" disabled={busy === n.id} onClick={() => disconnect(n.id)} style={growLink(colors)}>
                  Disconnect
                </Button>
              ) : (
                <Tooltip content={configured ? `Connect ${n.label}` : 'Save a Postiz API key above first'}>
                <Button
                  colors={colors}
                  variant="bare"
                  type="button"
                  disabled={busy === n.id}
                  onClick={() => connect(n.id)}
                  style={{
                    ...growLink(colors),
                    // Muted until a key exists, so hover must not promise the
                    // accent this control does not yet earn.
                    '--pa-btn-fg': configured ? colors.cyan : colors.textDim,
                    '--pa-btn-fg-hover': configured ? colors.cyan : colors.text,
                  } as CSSProperties}
                >
                  {waiting ? 'Open login again' : `Connect ${n.label}`}
                </Button>
                </Tooltip>
              )}
            </div>
          );
        })}
      </div>
      {loginUrl && (
        <div style={{ marginTop: space.sm }}>
          If a browser window did not open,{' '}
          <a href={loginUrl} className={LINK_CLASS} target="_blank" rel="noreferrer" style={{ color: colors.cyan }}>open the {pending ? NETWORKS.find((n) => n.id === pending)?.label ?? '' : ''} login</a>.
        </div>
      )}
      {error && <div role="alert" style={{ color: colors.danger, marginTop: space.sm }}>{error}</div>}
    </div>
  );
}

export function HiggsfieldConnect({ colors }: { colors: ThemeColors }) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [open, setOpen] = useState(false);
  const [keyId, setKeyId] = useState('');
  const [secret, setSecret] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    apiFetch<{ configured: boolean }>('/api/grow/higgsfield')
      .then((s) => setConfigured(s.configured))
      .catch(() => setConfigured(false));
  }, []);
  const field = growField(colors, { mono: true });
  const save = async () => {
    setBusy(true);
    try {
      const s = await apiFetch<{ configured: boolean }>('/api/grow/higgsfield', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ keyId, secret }),
      });
      setConfigured(s.configured);
      setOpen(false);
      setSecret('');
    } finally { setBusy(false); }
  };
  return (
    <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.xl }}>
      Reels use your Higgsfield account. {configured ? 'Connected.' : 'Not connected — stills still generate locally.'}
      {' '}
      <Button colors={colors} variant="bare" type="button" onClick={() => setOpen((v) => !v)} style={growLink(colors)}>
        {configured ? 'Replace keys' : 'Connect'}
      </Button>
      {open && (
        <div style={{ display: 'flex', gap: space.sm, marginTop: space.md, flexWrap: 'wrap' }}>
          <input value={keyId} onChange={(e) => setKeyId(e.target.value)} placeholder="Key ID" aria-label="Higgsfield key id" className={FIELD_CLASS} style={field} />
          <input value={secret} onChange={(e) => setSecret(e.target.value)} placeholder="Secret" type="password" aria-label="Higgsfield secret" className={FIELD_CLASS} style={field} />
          <Button colors={colors} type="button" disabled={busy} onClick={() => save()} style={growAccent(colors, '4px 10px')}>Save</Button>
        </div>
      )}
    </div>
  );
}
