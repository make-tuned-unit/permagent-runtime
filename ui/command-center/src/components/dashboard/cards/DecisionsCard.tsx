/** One summary and one review entry point for the canonical decision queue. */
import { useEffect, useState } from 'react';
import { font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useDecisions } from '../decisions/useDecisions';
import { DecisionInbox } from '../decisions/DecisionInbox';
import { summarizeDecisions } from '../decisions/summary';
import { useCommandCenter } from '../../../lib/store';
import { Button } from '../../common/Button';
import { CardScrollBody, EmptyNote, SectionTitle } from '../atoms';

interface Props { activeCount?: number }

export function DecisionsCard({ activeCount }: Props = {}) {
  const { colors } = useTheme();
  const inbox = useDecisions();
  const { data, error } = inbox;
  const [open, setOpen] = useState(false);
  const summary = summarizeDecisions(data, 'Your agent', activeCount);
  const pendingOpen = useCommandCenter(s => s.pendingDecisionInbox);
  const clearPendingDecisionInbox = useCommandCenter(s => s.clearPendingDecisionInbox);
  useEffect(() => {
    if (!pendingOpen) return;
    setOpen(true);
    clearPendingDecisionInbox();
  }, [pendingOpen, clearPendingDecisionInbox]);

  return (
    <>
      <section data-testid="decisions-card" aria-label="Decisions" style={{
        height: '100%', minHeight: 0, boxSizing: 'border-box',
        borderRadius: radius.lg, background: colors.surface,
        border: `1px solid ${colors.border}`,
        boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
        padding: space.xxl, display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        <SectionTitle title="Decisions" right={data ? `${summary.count} waiting` : undefined} />
        <CardScrollBody testId="decisions-card-body">
          {error ? (
            <EmptyNote>Couldn't load decisions</EmptyNote>
          ) : !data ? (
            <EmptyNote>Checking decisions…</EmptyNote>
          ) : (
            <>
              {summary.count > 0 ? (
                <div data-testid="decisions-card-top-item" style={{
                  fontFamily: font.display, fontSize: textSize.heading, fontWeight: 600,
                  lineHeight: 1.4, color: colors.text, overflowWrap: 'anywhere', maxWidth: 800,
                }}>
                  {summary.topItemLabel ?? 'Your decisions are ready to review'}
                </div>
              ) : <EmptyNote>No decisions waiting</EmptyNote>}
              {summary.oldestLabel && <p style={{ margin: `${space.sm}px 0 0`, fontSize: textSize.caption, color: colors.textMuted }}>{summary.oldestLabel}</p>}
              {summary.attention > 0 && <p style={{ margin: `${space.sm}px 0 0`, fontSize: textSize.caption, color: colors.warning }}>
                {summary.attention} {summary.attention === 1 ? 'goal needs' : 'goals need'} your attention. Details are in the inbox.
              </p>}
            </>
          )}
        </CardScrollBody>
        <div data-testid="decisions-card-review" style={{ flexShrink: 0, paddingTop: space.lg }}>
          {error ? (
            <Button colors={colors} type="button" flashSuccess={false} onClick={() => void inbox.refresh()}>Retry</Button>
          ) : data ? (
            <Button colors={colors} type="button" variant={summary.count > 0 ? 'primary' : 'ghost'}
              aria-label="Open the decision inbox" flashSuccess={false} onClick={() => setOpen(true)}>
              {summary.count > 0 ? 'Review decisions' : 'Open inbox'}
            </Button>
          ) : null}
        </div>
      </section>
      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}
