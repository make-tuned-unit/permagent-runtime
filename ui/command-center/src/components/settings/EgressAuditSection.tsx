/**
 * Privacy → Egress audit — the table of every cloud call this machine has
 * made or blocked.
 *
 * Split out of `SettingsView.tsx` (2026-09 dagK settings pass) with three
 * additions the flat newest-first list didn't have:
 *  - a 24h summary header (per provider+kind call count and blocked count),
 *    so "is anything unusual happening" doesn't require reading every row;
 *  - a small hourly traffic timeline, the same idea Little Snitch's log uses
 *    ("show the traffic, not just the rule") — rendered as one inline SVG,
 *    no new chart dependency;
 *  - consecutive identical calls collapsed into one row with a count and a
 *    time span, and the session id(s) that caused the run, so a chatty
 *    session's 40 identical calls read as one line instead of 40.
 *
 * The pure grouping/summary math lives in `egressGrouping.ts` so it has its
 * own tests independent of this render.
 */

import type { CSSProperties } from 'react';
import { concentric, font, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { EgressLogEntry } from '../../lib/api';
import { Row, Section } from './atoms';
import { Button } from '../common/Button';
import { StateBlock } from '../common/StateBlock';
import { Tooltip } from '../common/Tooltip';
import { timeAgo } from './format';
import { bucketEgressHourly, collapseConsecutiveEgress, summarizeEgress24h } from './egressGrouping';

/** A single SVG bar chart, 24 hourly buckets, allowed stacked under blocked so
 *  a blocked spike is never hidden behind a taller allowed bar. Zero traffic
 *  renders as a flat empty track (dataviz skill: zero is a state, not a gap). */
function TrafficTimeline({ log }: { log: EgressLogEntry[] }) {
  const { colors } = useTheme();
  const buckets = bucketEgressHourly(log);
  const max = Math.max(1, ...buckets.map(b => b.allowed + b.blocked));
  const w = 320;
  const h = 36;
  const barW = w / buckets.length;
  const allEmpty = buckets.every(b => b.allowed + b.blocked === 0);
  return (
    <div>
      <svg
        role="img"
        aria-label={allEmpty ? 'No cloud calls in the last 24 hours' : 'Cloud calls per hour, last 24 hours'}
        viewBox={`0 0 ${w} ${h}`}
        width={w}
        height={h}
        style={{ display: 'block' }}
      >
        {allEmpty ? (
          <line x1={0} y1={h - 1} x2={w} y2={h - 1} stroke={colors.border} strokeWidth={2} />
        ) : (
          buckets.map((b, i) => {
            const total = b.allowed + b.blocked;
            const barH = (total / max) * (h - 2);
            const blockedH = total > 0 ? (b.blocked / total) * barH : 0;
            const x = i * barW;
            return (
              <g key={b.start}>
                <title>{`${new Date(b.start).toLocaleTimeString([], { hour: '2-digit' })} — ${b.allowed} allowed, ${b.blocked} blocked`}</title>
                <rect x={x + 1} y={h - barH} width={Math.max(1, barW - 2)} height={Math.max(0, barH - blockedH)} fill={colors.textDim} />
                {blockedH > 0 && (
                  <rect x={x + 1} y={h - blockedH} width={Math.max(1, barW - 2)} height={blockedH} fill={colors.warning} />
                )}
              </g>
            );
          })
        )}
      </svg>
      <div style={{ display: 'flex', gap: space.lg, marginTop: space.xs, fontSize: textSize.micro, color: colors.textDim }}>
        <span>24h ago</span>
        <span style={{ marginLeft: 'auto' }}>now</span>
      </div>
    </div>
  );
}

function SummaryHeader({ log }: { log: EgressLogEntry[] }) {
  const { colors } = useTheme();
  const rows = summarizeEgress24h(log);
  if (rows.length === 0) {
    return (
      <div style={{ fontSize: textSize.caption, color: colors.textDim, marginBottom: space.lg }}>
        No cloud calls in the last 24 hours.
      </div>
    );
  }
  const totalBlocked = rows.reduce((n, r) => n + r.blocked, 0);
  return (
    <div style={{ marginBottom: space.lg }}>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, marginBottom: space.md }}>
        {rows.map(r => (
          <span
            key={`${r.provider}\u0000${r.kind}`}
            style={{
              ...type.label, padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.pill,
              border: `1px solid ${colors.border}`, color: colors.textMuted, fontFamily: font.body,
            }}
          >
            {r.provider} · {r.kind}: {r.count}{r.blocked > 0 && <span style={{ color: colors.warning }}> ({r.blocked} blocked)</span>}
          </span>
        ))}
      </div>
      <TrafficTimeline log={log} />
      {totalBlocked > 0 && (
        <div style={{ fontSize: textSize.micro, color: colors.warning, marginTop: space.sm }}>
          {totalBlocked} call{totalBlocked === 1 ? '' : 's'} blocked in the last 24 hours.
        </div>
      )}
    </div>
  );
}

export function EgressAuditSection({ log, logError, onRefresh }: {
  log: EgressLogEntry[] | null;
  logError: boolean;
  onRefresh: () => void;
}) {
  const { colors } = useTheme();
  const groups = log ? collapseConsecutiveEgress(log) : [];

  return (
    <Section title="Egress audit" sub="Every cloud call, allowed or blocked — newest first. BLOCKED means sovereign mode refused it before anything left this machine.">
      <Row label="Cloud inference calls" hint={`${log?.length ?? 0} recorded`}>
        <Button
          colors={colors}
          onClick={onRefresh}
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
      {logError ? (
        <StateBlock
          tone="error"
          compact
          title="Couldn't read the egress log."
          detail="This says nothing about whether calls were made — only that the record could not be read. Do not take a silent panel as a guarantee."
          onRetry={onRefresh}
        />
      ) : log === null ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading audit log…</div>
      ) : log.length === 0 ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>
          Nothing has left this machine yet. Every cloud call will be recorded here.
        </div>
      ) : (
        <div style={{ marginTop: space.xl }}>
          <SummaryHeader log={log} />
          <div>
            <div className="pa-set-egress" style={{ padding: `0 ${space.lg}px ${space.md}px`, ...type.label, color: colors.textDim }}>
              <div>When</div><div>Provider · Model</div><div className="pa-set-wide-only">Kind</div><div>Result</div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
              {groups.map(g => {
                const e = g.newest;
                const span = g.count > 1 && g.oldest.ts !== g.newest.ts
                  ? `${timeAgo(g.oldest.ts) || g.oldest.ts} – ${timeAgo(g.newest.ts) || g.newest.ts}`
                  : timeAgo(e.ts) || e.ts;
                return (
                  <div key={e.id} data-testid="egress-group-row" className="pa-set-egress" style={{ padding: `${space.md}px ${space.lg}px`, borderRadius: concentric(radius.lg, space.xs), background: colors.fillSubtle }}>
                    <Tooltip content={`${new Date(g.newest.ts).toLocaleString()}${g.sessionIds.length ? ' · session ' + g.sessionIds.join(', ') : ''} · ${e.contentHash.slice(0, 12)}…`}>
                      <div tabIndex={0} style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: font.mono }}>
                        {span}
                      </div>
                    </Tooltip>
                    <Tooltip content={`${e.provider} · ${e.model} · ${e.kind}`}>
                      <div tabIndex={0} style={{ fontSize: textSize.caption, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        <span style={{ color: colors.textMuted }}>{e.provider}</span> · {e.model}
                        {g.count > 1 && <span style={{ color: colors.textDim }}> ×{g.count}</span>}
                      </div>
                    </Tooltip>
                    <div className="pa-set-wide-only" style={{ fontSize: textSize.caption, color: colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis' }}>{e.kind}</div>
                    <div>
                      <span style={{
                        ...type.label, fontWeight: 700,
                        padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.pill, border: `1px solid ${colors.border}`,
                        color: e.blocked ? colors.warning : colors.success,
                      }}>
                        {e.blocked ? 'blocked' : 'allowed'}
                      </span>
                    </div>
                    {g.sessionIds.length > 0 && (
                      <div style={{ gridColumn: '1 / -1', fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, marginTop: space.xxs }}>
                        session {g.sessionIds.join(', ')}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </Section>
  );
}
