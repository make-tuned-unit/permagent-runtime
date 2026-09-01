import { font, radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Mobius, type MobiusState } from '../../mobius/Mobius';
import type { DashboardAgent } from '../useDashboard';

interface Props {
  agent: DashboardAgent;
  /** Live count of active goals (Ready/InProgress/Review) — the unit of work
   *  the user means by "thing Henry is working on", not raw sessions. */
  activeCount: number;
}

export function HeroCard({ agent, activeCount }: Props) {
  const { gradient, showHeroMobius, colors } = useTheme();
  const mobiusState = (agent.state === 'thinking' ? 'thinking' : 'idle') as MobiusState;

  return (
    <div style={{
      position: 'relative', overflow: 'hidden',
      padding: space.huge, borderRadius: radius.lg,
      background: gradient.card,
      border: `1px solid ${colors.border}`,
      // The one card with no shadow at all — the elevation ladder gives every
      // card the same raised step (D1) instead of leaving this one flat.
      boxShadow: colors.elevationRaised,
      height: '100%', boxSizing: 'border-box',
      display: 'flex', alignItems: 'center', gap: space.huge,
    }}>
      <div style={{ flex: 1 }}>
        <div style={{
          fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
          letterSpacing: '0.14em', textTransform: 'uppercase',
          color: colors.cyan, marginBottom: 12,
        }}>
          Status — {agent.state}
        </div>
        <div style={{
          fontFamily: font.display, fontSize: 24, fontWeight: 600,
          letterSpacing: '-0.02em', lineHeight: 1.2, marginBottom: 10,
          color: colors.text,
        }}>
          {activeCount > 0 ? (
            <>{agent.name} is working on<br /><span style={{ color: colors.cyan }}>{activeCount} {activeCount === 1 ? 'thing' : 'things'}</span> for you</>
          ) : (
            <>{agent.name} is<br />ready</>
          )}
        </div>
        <div style={{ fontSize: textSize.body, color: colors.textMuted, lineHeight: 1.5, maxWidth: 360 }}>
          {activeCount > 0
            ? `${activeCount} active goal${activeCount > 1 ? 's' : ''} in flight`
            : 'Ready when you are.'}
        </div>
      </div>
      {showHeroMobius && (
        <div style={{ flex: '0 0 auto' }}>
          <Mobius size={96} state={mobiusState} glow={1} />
        </div>
      )}
    </div>
  );
}
