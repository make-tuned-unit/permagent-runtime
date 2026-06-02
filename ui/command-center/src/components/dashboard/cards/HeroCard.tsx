import { font, radius } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Mobius, type MobiusState } from '../../mobius/Mobius';
import type { DashboardAgent } from '../useDashboard';

interface Props {
  agent: DashboardAgent;
}

export function HeroCard({ agent }: Props) {
  const { gradient, showHeroMobius, colors } = useTheme();
  const mobiusState = (agent.state === 'thinking' ? 'thinking' : 'idle') as MobiusState;

  return (
    <div style={{
      position: 'relative', overflow: 'hidden',
      padding: 24, borderRadius: radius.lg,
      background: gradient.card,
      border: `1px solid ${colors.border}`,
      height: '100%', boxSizing: 'border-box',
      display: 'flex', alignItems: 'center', gap: 24,
    }}>
      <div style={{ flex: 1 }}>
        <div style={{
          fontFamily: font.body, fontSize: 11, fontWeight: 600,
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
          {agent.active_count > 0 ? (
            <>{agent.name} is working on<br /><span style={{ color: colors.cyan }}>{agent.active_count} {agent.active_count === 1 ? 'thing' : 'things'}</span> for you</>
          ) : (
            <>{agent.name} is<br />ready</>
          )}
        </div>
        <div style={{ fontSize: 14, color: colors.textMuted, lineHeight: 1.5, maxWidth: 360 }}>
          {agent.active_count > 0
            ? `Working across ${agent.active_count} session${agent.active_count > 1 ? 's' : ''}`
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
