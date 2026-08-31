/**
 * Home card for the latest Council of LLMs weekly report.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { api, type CouncilLatest } from '../../../lib/api';
import { Button } from '../../common/Button';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';

export function CouncilCard() {
  const { colors } = useTheme();
  const [data, setData] = useState<CouncilLatest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openTakes, setOpenTakes] = useState(false);
  const [inboxOpen, setInboxOpen] = useState(false);
  const inbox = useDecisions();
  const live = useRef(true);

  useEffect(() => () => { live.current = false; }, []);

  // A daemon that can't be reached is not a Council with nothing to say, so
  // the failure names itself and hands back a way to try again rather than
  // printing a raw exception and stopping there.
  const load = useCallback(async () => {
    try {
      const d = await api.getCouncilLatest();
      if (!live.current) return true;
      setData(d);
      setError(null);
      return true;
    } catch (e) {
      if (!live.current) return false;
      setError(e instanceof Error ? e.message : String(e));
      return false;
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const report = data?.report;
  const session = data?.session;
  const positions = (data?.positions ?? []).filter(p => p.round === 1);
  const actions = data?.openActions ?? 0;

  return (
    <>
      <div
        data-testid="council-card"
        style={{
          height: '100%', boxSizing: 'border-box',
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
          padding: '18px 20px',
          display: 'flex', flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        <div style={{
          fontFamily: font.body, fontSize: 11, fontWeight: 600,
          letterSpacing: '0.10em', textTransform: 'uppercase',
          color: colors.textDim, marginBottom: 6,
        }}>
          Council
        </div>

        {error && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
            <div style={{ fontSize: 13, color: colors.text, lineHeight: 1.5 }}>
              Couldn't load the Council report — the daemon didn't answer.
            </div>
            <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono }}>{error}</div>
            <Button colors={colors} type="button" onClick={() => load()}>Retry</Button>
          </div>
        )}

        {!error && !data && (
          <div style={{ fontSize: 12, color: colors.textMuted }}>Loading…</div>
        )}

        {data && !report && (
          <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.5 }}>
            No weekly report yet. Turn on The Council under Settings → Features;
            the chat agent can also convene one on demand.
          </div>
        )}

        {report && (
          <>
            <div style={{
              fontSize: 16, fontWeight: 600, color: colors.text,
              lineHeight: 1.3, marginBottom: 8,
            }}>
              {report.headline}
            </div>
            <div style={{
              fontSize: 12, color: colors.textMuted, lineHeight: 1.5,
              whiteSpace: 'pre-wrap', overflow: 'auto', flex: 1, minHeight: 0,
            }}>
              {report.markdown}
            </div>
            <div style={{
              display: 'flex', gap: 10, marginTop: 10, alignItems: 'center',
              fontSize: 11, color: colors.textDim, flexWrap: 'wrap',
            }}>
              <span>{session?.status} · {positions.length} take{positions.length === 1 ? '' : 's'}</span>
              <button
                onClick={() => setOpenTakes(v => !v)}
                style={{
                  background: 'none', border: 'none', padding: 0, cursor: 'pointer',
                  color: colors.cyan, fontFamily: font.body, fontSize: 11,
                }}
              >
                {openTakes ? 'Hide takes' : 'Per-model takes'}
              </button>
              <button
                onClick={() => setInboxOpen(true)}
                style={{
                  background: 'none', border: 'none', padding: 0, cursor: 'pointer',
                  color: colors.cyan, fontFamily: font.body, fontSize: 11,
                }}
              >
                {actions} open action{actions === 1 ? '' : 's'}
              </button>
            </div>
            {openTakes && (
              <div style={{
                marginTop: 8, overflow: 'auto', maxHeight: 180,
                fontSize: 11, color: colors.textMuted, lineHeight: 1.45,
              }}>
                {positions.map(p => (
                  <div key={p.id} style={{ marginBottom: 10 }}>
                    <div style={{ fontWeight: 600, color: colors.text }}>
                      {p.provider} / {p.model} ({p.status})
                    </div>
                    <div style={{ whiteSpace: 'pre-wrap' }}>
                      {p.raw_text || p.error || '—'}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </div>
      {inboxOpen && (
        <DecisionInbox inbox={inbox} onClose={() => setInboxOpen(false)} />
      )}
    </>
  );
}
