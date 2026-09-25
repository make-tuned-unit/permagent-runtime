/**
 * Home card for the latest Council of LLMs weekly report.
 */

import { useCallback, useEffect, useId, useRef, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { api, type CouncilLatest } from '../../../lib/api';
import { Button } from '../../common/Button';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { MarkdownContent } from '../../chat/MarkdownContent';
import { CardScrollBody } from '../atoms';

/**
 * The report body is markdown (the Council writes `**Verdict:**`, `####`
 * project headings, lists). It was dropped into a `pre-wrap` div as a string,
 * so Home showed the asterisks and hashes literally. It now goes through the
 * chat's own renderer. Tailwind's preflight already makes every heading
 * inherit the body's size, which is what a card wants (an h4 at page size
 * would dwarf the headline); these variants only give headings weight, the
 * body text colour, and a little air above, so a heading still reads as one.
 */
// Spelled out in full: Tailwind finds classes by scanning source text, so a
// class assembled at run time would never be generated.
const COUNCIL_MD_CLASS = [
  '[&_h1]:font-semibold [&_h1]:mt-2 [&_h1]:mb-1',
  '[&_h2]:font-semibold [&_h2]:mt-2 [&_h2]:mb-1',
  '[&_h3]:font-semibold [&_h3]:mt-2 [&_h3]:mb-1',
  '[&_h4]:font-semibold [&_h4]:mt-2 [&_h4]:mb-1',
  '[&_h5]:font-semibold [&_h5]:mt-2 [&_h5]:mb-1',
  '[&_h6]:font-semibold [&_h6]:mt-2 [&_h6]:mb-1',
].join(' ');

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
          boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
          padding: `${space.xxxl}px`,
          display: 'flex', flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        <div style={{
          fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
          letterSpacing: '0.10em', textTransform: 'uppercase',
          color: colors.textDim, marginBottom: space.sm,
        }}>
          Council
        </div>

        {error && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: space.md, alignItems: 'flex-start' }}>
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
              lineHeight: 1.3, marginBottom: space.md,
            }}>
              {report.headline}
            </div>
            <CardScrollBody
              testId="council-report-body"
              className={COUNCIL_MD_CLASS}
              style={{
                fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5,
                overflowWrap: 'anywhere',
              }}
            >
              <MarkdownContent content={report.markdown} />
            </CardScrollBody>
            <div style={{
              display: 'flex', gap: space.lg, marginTop: space.lg, alignItems: 'center',
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
                marginTop: space.md, overflow: 'auto', maxHeight: 180,
                fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.45,
              }}>
                {positions.map(p => (
                  <div key={p.id} style={{ marginBottom: space.lg }}>
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
