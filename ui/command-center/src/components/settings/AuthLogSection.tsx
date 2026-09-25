/**
 * Privacy → Auth log — every refused daemon request, and every admitted one
 * whose route can execute code, touch secrets, spend money or write data.
 *
 * The backend (`/api/security/auth-log`, `crates/goose/src/security/auth_audit.rs`)
 * has existed with no surface reading it — this is that surface. See the
 * module doc there for what the log is NOT: detection, never prevention; an
 * empty log is not proof nothing ran, only that nothing was recorded.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api, type AuthEventEntry } from '../../lib/api';
import { Row, Section } from './atoms';
import { Button } from '../common/Button';
import { StateBlock } from '../common/StateBlock';
import { Tooltip } from '../common/Tooltip';
import { timeAgo } from './format';

/** Plain-language words for the route-class enum (R3 — no internal jargon in
 *  user copy). Ordered by blast radius, matching `RouteClass` in Rust. */
const CLASS_LABEL: Record<string, string> = {
  execute: 'ran code',
  secrets: 'touched a credential',
  spend: 'cost money or bandwidth',
  mutate: 'changed your data',
  read: 'read your data',
  status: 'status check',
};

function classLabel(c: string): string {
  return CLASS_LABEL[c] ?? c;
}

export function AuthLogSection() {
  const { colors } = useTheme();
  const [log, setLog] = useState<AuthEventEntry[] | null>(null);
  const [error, setError] = useState(false);

  const refresh = useCallback(() => {
    return api.getAuthLog(100)
      .then(l => { setLog(l); setError(false); return true; })
      .catch(() => { setError(true); return false; });
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  return (
    <Section
      title="Control-plane auth log"
      sub="Every request that used your daemon's credential, admitted or refused — for actions that run code, touch secrets, spend money, or write your data. This cannot tell same-user processes apart; treat a row as evidence and an empty log as no evidence either way, not as a guarantee."
    >
      <Row label="Auth events" hint={`${log?.length ?? 0} recorded`}>
        <Button
          colors={colors}
          onClick={refresh}
          style={{
            '--pa-btn-bg': colors.surfaceHi,
            '--pa-btn-fg': colors.text,
            '--pa-btn-border': colors.border,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-bg-active': colors.surface,
            '--pa-btn-pad': '4px 10px',
            '--pa-btn-radius': `${radius.sm}px`,
            fontSize: textSize.caption,
          } as CSSProperties}
        >Refresh</Button>
      </Row>
      {error ? (
        <StateBlock tone="error" compact title="Couldn't read the auth log." detail="The record could not be read — this says nothing about whether events happened." onRetry={() => { void refresh(); }} />
      ) : log === null ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading auth log…</div>
      ) : log.length === 0 ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>No events recorded yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm, marginTop: space.md }}>
          {log.map(e => (
            <div
              key={e.id}
              data-testid="auth-log-row"
              style={{
                display: 'flex', alignItems: 'center', gap: space.lg, flexWrap: 'wrap',
                padding: `${space.md}px ${space.lg}px`, borderRadius: radius.md, background: colors.fillSubtle,
              }}
            >
              <Tooltip content={new Date(e.ts).toLocaleString()}>
                <span tabIndex={0} style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: font.mono, minWidth: 72 }}>
                  {timeAgo(e.ts) || e.ts}
                </span>
              </Tooltip>
              <span style={{
                ...type.label, fontWeight: 700, padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.pill,
                border: `1px solid ${colors.border}`,
                color: e.outcome === 'admitted' ? colors.success : colors.danger,
              }}>
                {e.outcome}
              </span>
              <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1, minWidth: 160 }}>
                {classLabel(e.class)}
                <span style={{ color: colors.textMuted }}> · {e.method} {e.path}</span>
              </span>
              {e.principal && (
                <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>{e.principal}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}
