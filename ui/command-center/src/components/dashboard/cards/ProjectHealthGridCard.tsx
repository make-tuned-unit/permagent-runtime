/**
 * Project Health Grid — Operator Command Center's per-project table (§2.2,
 * §3 `project_health_grid`). One row per project: active/held goal counts and
 * a plain status word. No fabricated sparkline (P12) — a real 14-day activity
 * series needs an activity-journal source Phase 1 doesn't have yet (Phase 2,
 * HOME_TEMPLATES_RESEARCH.md §5); "dormant" is said as a word, not drawn as a
 * flat line that could be misread as "zero today".
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
          {rows.map(row => (
            <button
              key={row.projectId}
              type="button"
              data-card-row=""
              onClick={() => openProject(row.projectId)}
              style={{
                display: 'flex', alignItems: 'center', width: '100%', maxWidth: 720,
                gap: space.lg, padding: `${space.md}px 0`, borderTop: `1px solid ${colors.border}`,
                border: 'none', borderTopWidth: 1, background: 'transparent', cursor: 'pointer',
                textAlign: 'left',
              }}
            >
              <span style={{
                flex: 1, minWidth: 0, fontFamily: font.body, fontSize: textSize.small,
                color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              }}>{row.projectName}</span>
              <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
                {row.activeGoals} active{row.heldGoals > 0 ? ` · ${row.heldGoals} held` : ''}
              </span>
              <StatusChip status={row.status} />
            </button>
          ))}
        </CardScrollBody>
      )}
    </div>
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
