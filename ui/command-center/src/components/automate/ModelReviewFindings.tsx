import { useCallback, useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { Button } from '../common/Button';
import { withAlpha } from '../dashboard/decisions/format';
import {
  SNOOZE_OPTIONS, actOnModelFindings, candidateLabel, groupModelFindings, listModelFindings,
  snoozeUntil, tierLabel, unknownNote,
  type ModelFindingAction, type ModelFindingRow,
} from './modelFindingLedger';

// W1b: the Model Review run's findings, from the one model findings ledger.
// Open findings can be dismissed or snoozed (singly or as a selection);
// deferred and dismissed ones can be reopened. Every action goes through the
// one actions endpoint, and the list is refetched after it — the server's
// effective state is the only state shown.

function actionVars(colors: ThemeColors): CSSProperties {
  return {
    '--pa-btn-pad': '3px 8px',
    '--pa-btn-radius': `${radius.sm}px`,
    '--pa-btn-bg': colors.border,
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border-hover': colors.borderHi,
    fontFamily: font.body,
    fontSize: textSize.micro,
  } as CSSProperties;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

export function ModelReviewFindings() {
  const { colors } = useTheme();
  const openDecisionInbox = useCommandCenter(s => s.openDecisionInbox);
  const [rows, setRows] = useState<ModelFindingRow[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [unknownCount, setUnknownCount] = useState(0);
  const [linkNote, setLinkNote] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  /** Which snooze picker is open: a row's key, 'bulk', or none. */
  const [snoozeFor, setSnoozeFor] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await listModelFindings();
      setRows(data);
      setLoadError(null);
      // A selection only ever holds findings that are still open.
      const open = new Set(data.filter(r => r.state === 'open').map(r => r.key));
      setSelected(prev => new Set([...prev].filter(k => open.has(k))));
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const act = useCallback(async (keys: string[], action: ModelFindingAction, deferUntil?: string): Promise<boolean> => {
    if (keys.length === 0) return false;
    setBusy(true);
    setActionError(null);
    setUnknownCount(0);
    let ok = true;
    try {
      const res = await actOnModelFindings(keys, action, deferUntil);
      setUnknownCount(res.unknown?.length ?? 0);
      setSnoozeFor(null);
    } catch (err) {
      ok = false;
      setActionError(err instanceof Error ? err.message : String(err));
    }
    await load();
    setBusy(false);
    return ok;
  }, [load]);

  const toggle = (key: string) => setSelected(prev => {
    const next = new Set(prev);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });

  const openInReview = () => {
    // Opens Home's Decision Review (J3). It cannot focus one decision yet:
    // the review queue has no deep-link by id.
    if (!openDecisionInbox()) setLinkNote('Decision Review is on Home, and no workspace holds Home.');
  };

  if (rows === null && loadError === null) {
    return <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Loading results...</div>;
  }

  const groups = groupModelFindings(rows ?? []);
  const selectedKeys = (rows ?? []).filter(r => r.state === 'open' && selected.has(r.key)).map(r => r.key);

  const snoozePicker = (keys: string[]) => (
    <div role="group" aria-label="Snooze for" style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.xs }}>
      <span style={{ fontSize: textSize.micro, color: colors.textDim }}>Snooze for</span>
      {SNOOZE_OPTIONS.map(opt => (
        <Button key={opt.days} colors={colors} disabled={busy} style={actionVars(colors)}
          onClick={() => act(keys, 'defer', snoozeUntil(opt.days))}>{opt.label}</Button>
      ))}
      <Button colors={colors} variant="bare" disabled={busy} flashSuccess={false}
        style={{ fontSize: textSize.micro, '--pa-btn-fg': colors.textDim, '--pa-btn-pad': '3px 6px' } as CSSProperties}
        onClick={() => setSnoozeFor(null)}>Cancel</Button>
    </div>
  );

  return (
    <div data-testid="model-review-findings" style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
      <div style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.text }}>Model findings</div>

      {loadError && (
        <div role="alert" style={{ fontSize: textSize.caption, color: colors.danger, overflowWrap: 'anywhere' }}>
          Couldn&apos;t load model findings: {loadError}
          {' '}
          <Button colors={colors} variant="bare" flashSuccess={false}
            style={{ fontSize: textSize.caption, '--pa-btn-fg': colors.cyan, '--pa-btn-pad': '0' } as CSSProperties}
            onClick={() => load()}>Retry</Button>
        </div>
      )}
      {actionError && (
        <div role="alert" style={{ fontSize: textSize.caption, color: colors.danger, overflowWrap: 'anywhere' }}>{actionError}</div>
      )}
      {unknownCount > 0 && (
        <div role="status" style={{ fontSize: textSize.caption, color: colors.warning }}>{unknownNote(unknownCount)}</div>
      )}
      {linkNote && (
        <div role="status" style={{ fontSize: textSize.caption, color: colors.textMuted }}>{linkNote}</div>
      )}

      {rows && rows.length === 0 && (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '4px 0' }}>No model findings.</div>
      )}

      {selectedKeys.length > 0 && (
        <div data-testid="model-findings-bulk" style={{
          display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.md,
          padding: `${space.md}px ${space.lg}px`, borderRadius: radius.sm,
          background: colors.surface, border: `1px solid ${colors.borderHi}`,
        }}>
          <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>{selectedKeys.length} selected</span>
          <Button colors={colors} disabled={busy} style={actionVars(colors)}
            onClick={() => act(selectedKeys, 'dismiss')}>Dismiss selected</Button>
          {snoozeFor === 'bulk' ? snoozePicker(selectedKeys) : (
            <Button colors={colors} disabled={busy} flashSuccess={false} style={actionVars(colors)}
              onClick={() => setSnoozeFor('bulk')}>Snooze selected</Button>
          )}
        </div>
      )}

      {groups.map(group => (
        <section key={group.state} aria-label={group.label}>
          <div style={{
            fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase',
            color: colors.textDim, marginBottom: space.xs,
          }}>
            {group.label} &middot; {group.rows.length}
          </div>
          {group.rows.map(row => {
            const d = row.finding.discovery;
            const isOpen = row.state === 'open';
            return (
              <div key={row.key} data-testid="model-finding-row" data-state={row.state} style={{
                display: 'flex', gap: space.md, padding: `${space.md}px ${space.lg}px`, marginTop: space.xs,
                borderRadius: radius.sm, background: colors.surface, border: `1px solid ${colors.border}`,
                opacity: row.state === 'dismissed' ? 0.7 : 1,
              }}>
                {isOpen && (
                  <input type="checkbox" checked={selected.has(row.key)} onChange={() => toggle(row.key)}
                    aria-label={`Select ${row.finding.consumer} finding`} style={{ marginTop: space.xs, flexShrink: 0 }} />
                )}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.sm }}>
                    <span style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.text }}>{row.finding.consumer}</span>
                    <span style={{
                      fontSize: textSize.micro, padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.xs,
                      color: colors.cyan, background: withAlpha(colors.cyan, 0.1),
                    }}>{tierLabel(row.finding.tier)}</span>
                    {d.stale && (
                      <span style={{
                        fontSize: textSize.micro, padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.xs,
                        color: colors.stale, background: colors.staleSoft,
                      }}>stale</span>
                    )}
                  </div>
                  <div style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono, marginTop: space.xxs, overflowWrap: 'anywhere' }}>
                    {d.incumbent_identity} &rarr; {candidateLabel(d)}
                  </div>
                  <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xxs, overflowWrap: 'anywhere' }}>
                    Compatibility: {d.compatibility}
                  </div>
                  {d.uncertainty && (
                    <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xxs, overflowWrap: 'anywhere' }}>
                      {d.uncertainty}
                    </div>
                  )}
                  {row.state === 'deferred' && row.defer_until && (
                    <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
                      Snoozed until <time dateTime={row.defer_until}>{formatDate(row.defer_until)}</time>
                    </div>
                  )}
                  {isOpen && snoozeFor === row.key && <div style={{ marginTop: space.sm }}>{snoozePicker([row.key])}</div>}
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', justifyContent: 'flex-end', gap: space.xs, flexShrink: 0 }}>
                  {row.decision_id !== null && (
                    <Button colors={colors} flashSuccess={false} style={actionVars(colors)} onClick={openInReview}>
                      Open in Decision Review
                    </Button>
                  )}
                  {isOpen ? (
                    <>
                      <Button colors={colors} disabled={busy} style={actionVars(colors)}
                        onClick={() => act([row.key], 'dismiss')}>Dismiss</Button>
                      {snoozeFor !== row.key && (
                        <Button colors={colors} disabled={busy} flashSuccess={false} style={actionVars(colors)}
                          onClick={() => setSnoozeFor(row.key)}>Snooze</Button>
                      )}
                    </>
                  ) : (
                    <Button colors={colors} disabled={busy} style={actionVars(colors)}
                      onClick={() => act([row.key], 'reopen')}>Reopen</Button>
                  )}
                </div>
              </div>
            );
          })}
        </section>
      ))}
    </div>
  );
}
