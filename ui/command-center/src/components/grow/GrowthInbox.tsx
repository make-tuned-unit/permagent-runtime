/**
 * The deterministic growth inbox: this week's ranked moves and the "keep doing"
 * wins strip, plus the priority tint they share.
 *
 * Split out of GrowView.tsx (R9), unchanged. All content comes from the backend
 * ranker (grow.rs) with NO model in the loop — this file only presents it, with
 * honest loading / error / empty states.
 */

import { radius, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { useCommandCenter } from '../../lib/store';
import { Button } from '../common/Button';
import { growChip } from './growStyles';
import { growCard, growLabel } from './growChrome';
import { CARD_PAD, CARD_R, ROW_PAD, ROW_R } from './growGeometry';
import { ErrorState, SkeletonCards } from './GrowStateBlocks';
import type { GrowthInboxData, GrowthMove, GrowthWin, LoadState, MovePriority } from './growTypes';

// ── Growth inbox (Analytics lens headline) ───────────────────────────────────
// The deterministic inbox rendered atop the analytics lens: this week's ranked
// moves + a "keep doing" wins strip. All content comes from the backend ranker
// (grow.rs) — this component only presents it, with honest loading / error /
// empty states. No Henry drafting hand-offs here (those belong to GrowView's
// prompt seams); the inbox is informational.

export function priorityMeta(priority: MovePriority, colors: ThemeColors): { label: string; color: string } {
  switch (priority) {
    case 'high': return { label: 'High priority', color: colors.warning };
    case 'medium': return { label: 'Medium priority', color: colors.cyan };
    default: return { label: 'Low priority', color: colors.textDim };
  }
}

export function GrowthInboxSection({
  colors, state, inbox, onRetry, projectName,
}: {
  colors: ThemeColors;
  state: LoadState;
  inbox: GrowthInboxData | null;
  onRetry: () => void;
  /** Grounds the hand-off prompt each move card offers. */
  projectName: string;
}) {
  // Defensive against a partial payload. This section previously only rendered
  // inside the Analytics lens; it is now the top of Actions, the default tab,
  // so a malformed response would crash the first thing the user sees.
  const signal = inbox?.signal;
  const moves = inbox?.moves ?? [];
  const wins = inbox?.wins ?? [];
  const hasSignal = !!signal && ((signal.posts ?? 0) > 0 || (signal.shipped ?? 0) > 0);
  const empty = !!inbox && moves.length === 0 && wins.length === 0;

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: space.lg, margin: `0 0 ${space.xl}px`, flexWrap: 'wrap' }}>
        <h3 style={{ ...growLabel(colors), margin: 0 }}>
          Your growth moves this week
        </h3>
        {hasSignal && signal && (
          <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
            from {signal.posts} {signal.posts === 1 ? 'post' : 'posts'} · {signal.shipped} shipped
          </span>
        )}
      </div>

      {state === 'error' ? (
        <ErrorState colors={colors} inline message="Couldn't load your growth moves." onRetry={onRetry} />
      ) : state === 'loading' ? (
        <SkeletonCards colors={colors} count={3} height={68} />
      ) : !inbox ? null : empty ? (
        <div style={{
          border: `1px dashed ${colors.border}`, borderRadius: CARD_R, padding: space.huge,
          textAlign: 'center', fontSize: textSize.caption, color: colors.textDim, lineHeight: 1.6,
        }}>
          Not enough signal yet. Publish a post or ship a goal and I'll start surfacing your 2-3
          highest-leverage growth moves here each week — ranked, no guesswork.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
          {moves.length > 0 ? (
            moves.map((m) => <MoveCard key={m.title} move={m} colors={colors} projectName={projectName} />)
          ) : (
            <div style={{
              ...growCard(colors, { r: CARD_R, pad: CARD_PAD }),
              fontSize: textSize.caption, color: colors.textMuted,
            }}>
              You're on track — no urgent moves this week. Keep doing what's working below.
            </div>
          )}
          {wins.length > 0 && <WinsStrip wins={wins} colors={colors} />}
        </div>
      )}
    </section>
  );
}

/**
 * A growth move: the first thing on the Actions tab, and — until now — the one
 * card type in the whole lens with nothing to click.
 *
 * Every other card offers a way to act on it. This one presented "your 2–3
 * highest-leverage growth moves this week", explained why each mattered, and
 * then stopped. A recommendation you cannot do anything with reads as a
 * newsletter, which is exactly what the tab is not for.
 *
 * It gets the hand-off the rest of the surface already uses: open the chat
 * dock and send a prompt grounded in this move and this project. Same
 * mechanism, same wording, no new concept.
 */
function MoveCard({ move, colors, projectName }: {
  move: GrowthMove;
  colors: ThemeColors;
  projectName: string;
}) {
  const meta = priorityMeta(move.priority, colors);
  const agentName = useCommandCenter((st) => st.agentName);
  const sendMessage = useCommandCenter((st) => st.sendMessage);
  const openChatDock = useCommandCenter((st) => st.openChatDock);
  const setActivePanel = useCommandCenter((st) => st.setActivePanel);

  const discuss = () => {
    // setActivePanel('chat') only dismisses an overlay; since chat went
    // dock-first the dock has to be opened explicitly or the prompt goes to a
    // conversation nobody can see. Same two lines as every other hand-off here.
    setActivePanel('chat');
    openChatDock();
    void sendMessage(
      `Let's work on this growth move for ${projectName}: "${move.title}". `
      + `The reason it came up: ${move.why}`,
    );
  };

  return (
    <div style={{
      display: 'flex', flexDirection: 'column', gap: space.sm,
      ...growCard(colors, { r: ROW_R, pad: ROW_PAD }),
      borderLeft: `3px solid ${meta.color}`,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
        <span style={{
          ...growLabel(colors, meta.color),
          border: `1px solid ${meta.color}`, borderRadius: radius.pill, padding: `1px ${space.md}px`,
        }}>{meta.label}</span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: textSize.micro, color: colors.textDim, fontVariantNumeric: 'tabular-nums' }}>
          {move.evidenceCount} {move.evidenceCount === 1 ? 'signal' : 'signals'}
        </span>
      </div>
      <div style={{ fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{move.title}</div>
      <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5 }}>{move.why}</div>
      <div style={{ display: 'flex', marginTop: space.xs / 2 }}>
        <Button
          colors={colors}
          type="button"
          data-testid="move-discuss"
          onClick={discuss}
          style={growChip()}
        >
          Discuss with {agentName}
        </Button>
      </div>
    </div>
  );
}

function WinsStrip({ wins, colors }: { wins: GrowthWin[]; colors: ThemeColors }) {
  return (
    <div style={{ marginTop: space.xs }}>
      <div style={{ ...growLabel(colors), marginBottom: space.md }}>
        Keep doing
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
        {wins.map((w) => (
          <div key={w.title} style={{
            display: 'flex', alignItems: 'flex-start', gap: space.md,
            ...growCard(colors, { r: ROW_R, pad: ROW_PAD }),
            borderLeft: `3px solid ${colors.success}`,
          }}>
            <span aria-hidden style={{ color: colors.success, fontSize: textSize.small, lineHeight: '18px' }}>✓</span>
            <div>
              <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text }}>{w.title}</div>
              <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, marginTop: space.xs / 2 }}>{w.why}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
