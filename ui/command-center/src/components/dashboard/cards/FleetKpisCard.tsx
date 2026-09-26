/**
 * Fleet KPI strip — Operator Command Center's top row (§2.2, §3
 * `fleet_kpis`). Plain cross-project counts; a metric with zero underlying
 * projects reads "—", never a fabricated "0%" (P12).
 */
import { radius, space, font, textSize, tabularNums } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { EmptyNote } from '../atoms';
import { useDashboardInsights } from './useDashboardInsights';

export function FleetKpisCard() {
  const { colors } = useTheme();
  const { data, loading, error } = useDashboardInsights();
  const kpis = data?.fleetKpis;

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: `${space.lg}px ${space.xxl}px`,
      display: 'flex', alignItems: 'center',
      overflow: 'hidden',
    }}>
      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't load fleet stats</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : !kpis || kpis.projectCount === 0 ? (
        <EmptyNote hint="Add a project to see fleet-wide stats">No projects yet</EmptyNote>
      ) : (
        <div style={{ display: 'flex', width: '100%', gap: space.xxl }}>
          <Kpi label="Projects" value={kpis.projectCount} colors={colors} />
          <Kpi label="Active goals" value={kpis.activeGoalCount} colors={colors} />
          {/* Coloured by MEANING, not just "has a number": stalled/waiting
              are attention-worthy (danger/cyan), never the same neutral text
              as a plain count (coordinator, Phase-1 presentation pass). */}
          <Kpi label="Stalled" value={kpis.stalledGoalCount} tone={kpis.stalledGoalCount > 0 ? 'danger' : undefined} colors={colors} />
          <Kpi label="Decisions waiting" value={kpis.pendingDecisionCount} tone={kpis.pendingDecisionCount > 0 ? 'cyan' : undefined} colors={colors} />
          <Kpi label="Sessions today" value={kpis.sessionsToday} colors={colors} />
        </div>
      )}
    </div>
  );
}

function Kpi({ label, value, tone, colors }: { label: string; value: number; tone?: 'cyan' | 'danger'; colors: ReturnType<typeof useTheme>['colors'] }) {
  const color = tone === 'danger' ? colors.danger : tone === 'cyan' ? colors.cyan : colors.text;
  return (
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{
        fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.08em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xxs,
        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
      }}>{label}</div>
      <div style={{
        fontFamily: font.display, fontSize: textSize.title, fontWeight: 600,
        ...tabularNums, color,
      }}>{value}</div>
    </div>
  );
}
