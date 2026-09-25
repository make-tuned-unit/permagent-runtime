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
          <Kpi label="Stalled" value={kpis.stalledGoalCount} accent={kpis.stalledGoalCount > 0} colors={colors} />
          <Kpi label="Decisions waiting" value={kpis.pendingDecisionCount} accent={kpis.pendingDecisionCount > 0} colors={colors} />
          <Kpi label="Sessions today" value={kpis.sessionsToday} colors={colors} />
        </div>
      )}
    </div>
  );
}

function Kpi({ label, value, accent, colors }: { label: string; value: number; accent?: boolean; colors: ReturnType<typeof useTheme>['colors'] }) {
  return (
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{
        fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.08em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xxs,
        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
      }}>{label}</div>
      <div style={{
        fontFamily: font.display, fontSize: textSize.title, fontWeight: 600,
        ...tabularNums, color: accent ? colors.cyan : colors.text,
      }}>{value}</div>
    </div>
  );
}
