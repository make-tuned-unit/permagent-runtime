/**
 * Home card for the latest Council of LLMs weekly report.
 */

import { useCallback, useEffect, useId, useRef, useState, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../../styles/tokens';
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
  const takesId = useId();
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
          fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
          letterSpacing: '0.10em', textTransform: 'uppercase',
          color: colors.textDim, marginBottom: 6,
        }}>
          Council
        </div>

        {error && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'flex-start' }}>
            <div style={{ fontSize: textSize.small, color: colors.text, lineHeight: 1.5 }}>
              Couldn't load the Council report — the daemon didn't answer.
            </div>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono }}>{error}</div>
            <Button colors={colors} type="button" onClick={() => load()}>Retry</Button>
          </div>
        )}

        {!error && !data && (
          <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>Loading…</div>
        )}

        {data && !report && (
          <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5 }}>
            No weekly report yet. Turn on The Council under Settings → Features;
            the chat agent can also convene one on demand.
          </div>
        )}

        {report && (
          <>
            <div style={{
              fontSize: textSize.heading, fontWeight: 600, color: colors.text,
              lineHeight: 1.3, marginBottom: 8,
            }}>
              {report.headline}
            </div>
            <div style={{
              fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5,
              whiteSpace: 'pre-wrap', overflow: 'auto', flex: 1, minHeight: 0,
            }}>
              {report.markdown}
            </div>
            <div style={{
              display: 'flex', gap: 10, marginTop: 10, alignItems: 'center',
              fontSize: textSize.micro, color: colors.textDim, flexWrap: 'wrap',
            }}>
              <span>{session?.status} · {positions.length} take{positions.length === 1 ? '' : 's'}</span>
              {/* Disclosure toggle: it opens the takes list right below and
                  there is nothing to await, so it takes the shared `.pa-btn`
                  interaction rules rather than the Button primitive's
                  pending/success machinery. */}
              <button
                type="button"
                className="pa-btn hover:underline"
                aria-expanded={openTakes}
                aria-controls={takesId}
                onClick={() => setOpenTakes(v => !v)}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': 'transparent',
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-fg-hover': colors.cyan,
                  '--pa-btn-bg-active': 'transparent',
                  '--pa-btn-pad': '0',
                  '--pa-btn-radius': '0',
                  '--pa-btn-weight': 400,
                  fontFamily: font.body, fontSize: textSize.micro,
                } as CSSProperties}
              >
                {openTakes ? 'Hide takes' : 'Per-model takes'}
              </button>
              <Button
                colors={colors}
                variant="bare"
                type="button"
                className="hover:underline"
                onClick={() => setInboxOpen(true)}
                style={{
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-fg-hover': colors.cyan,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-bg-active': 'transparent',
                  '--pa-btn-pad': '0',
                  '--pa-btn-radius': '0',
                  '--pa-btn-weight': 400,
                  fontFamily: font.body, fontSize: textSize.micro,
                } as CSSProperties}
              >
                {actions} open action{actions === 1 ? '' : 's'}
              </Button>
            </div>
            {openTakes && (
              <div id={takesId} style={{
                marginTop: 8, overflow: 'auto', maxHeight: 180,
                fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.45,
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
