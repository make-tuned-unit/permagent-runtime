/**
 * Project Health Grid — Operator Command Center's per-project table (§2.2,
 * §3 `project_health_grid`). Small multiples: at a wide card width, rows
 * render as 2-3 columns rather than one long list (coordinator, Phase-1
 * presentation pass), each with a real 14-day activity sparkline where the
 * activity journal has one, a status chip coloured by meaning, and a
 * plain-words reason for anything not "active". No fabricated sparkline
 * (P12) — a project with no journaled goal activity shows "no history yet",
 * never a flat, misleadingly-confident zero line.
 */
import { radius, space, font, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, CardScrollBody } from '../atoms';
import { useCommandCenter, navigateToTool } from '../../../lib/store';
import { useDashboardInsights, type ProjectHealthRow } from './useDashboardInsights';

const STATUS_LABEL: Record<ProjectHealthRow['status'], string> = {
  active: 'Active',
  stalled: 'Stalled',
  dormant: 'Dormant',
};

export function ProjectHealthGridCard() {
  const { colors } = useTheme();
  const { data, loading, error } = useDashboardInsights();
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);
  const rows = data?.projectHealthGrid ?? [];
  const openProject = (projectId: string) => {
    setPendingProjectNavigation(projectId);
    navigateToTool('projects');
  };

  return (
    <div style={{
      height: '100%', boxSizing: 'border-box',
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      boxShadow: [colors.elevationRaised, colors.cardHighlight].filter(Boolean).join(', '),
      padding: space.xxl,
      display: 'flex', flexDirection: 'column',
      overflow: 'hidden',
    }}>
      <SectionTitle title="Project health" right={rows.length > 0 ? `${rows.length} project${rows.length === 1 ? '' : 's'}` : undefined} />
      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't load project health</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : rows.length === 0 ? (
        <EmptyNote hint="Add a project to see it here">No projects yet</EmptyNote>
      ) : (
        <CardScrollBody testId="project-health-body">
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
            gap: space.md,
          }}>
            {rows.map(row => (
              <ProjectHealthTile key={row.projectId} row={row} onOpen={() => openProject(row.projectId)} />
            ))}
          </div>
        </CardScrollBody>
      )}
    </div>
  );
}

function ProjectHealthTile({ row, onOpen }: { row: ProjectHealthRow; onOpen: () => void }) {
  const { colors } = useTheme();
  return (
    <button
      type="button"
      data-card-row=""
      onClick={onOpen}
      style={{
        display: 'flex', flexDirection: 'column', gap: space.xs, width: '100%',
        padding: space.md, borderRadius: radius.md,
        border: `1px solid ${colors.border}`, background: colors.bg,
        cursor: 'pointer', textAlign: 'left',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: space.sm }}>
        <span style={{
          flex: 1, minWidth: 0, fontFamily: font.body, fontSize: textSize.small, fontWeight: 500,
          color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>{row.projectName}</span>
        <StatusChip status={row.status} />
      </div>
      <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim }}>
        {row.activeGoals} active{row.heldGoals > 0 ? ` · ${row.heldGoals} held` : ''}
      </div>
      {row.reason && (
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.4 }}>
          {row.reason}
        </div>
      )}
      <Sparkline points={row.sparkline} colors={colors} />
    </button>
  );
}

function Sparkline({ points, colors }: { points: ProjectHealthRow['sparkline']; colors: ReturnType<typeof useTheme>['colors'] }) {
  if (points.length < 2) {
    return (
      <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, marginTop: space.xxs }}>
        No activity history yet
      </div>
    );
  }
  const w = 120;
  const h = 24;
  const max = Math.max(1, ...points.map(p => p.count));
  const step = w / (points.length - 1);
  const path = points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${(i * step).toFixed(1)} ${(h - (p.count / max) * h).toFixed(1)}`).join(' ');
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} style={{ marginTop: space.xxs }} aria-hidden>
      <path d={path} fill="none" stroke={colors.cyan} strokeWidth={1.5} />
    </svg>
  );
}

function StatusChip({ status }: { status: ProjectHealthRow['status'] }) {
  const { colors } = useTheme();
  const color = status === 'stalled' ? colors.danger : status === 'active' ? colors.success : colors.textDim;
  return (
    <span style={{
      fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em',
      textTransform: 'uppercase', color, flexShrink: 0,
    }}>{STATUS_LABEL[status]}</span>
  );
}
