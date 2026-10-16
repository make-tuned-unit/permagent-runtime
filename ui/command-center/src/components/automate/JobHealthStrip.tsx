/**
 * One line at the top of Automate: paused schedules, reported problems, or
 * digest availability. See `jobHealthModel.ts`.
 */
import type { CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { withAlpha } from '../dashboard/decisions/format';
import type { HealthProblem, HealthSummary } from './jobHealthModel';

const KIND_WORD: Record<HealthProblem['kind'], string> = {
  failing: 'Failing',
  missed: 'Missed',
  never: 'Never ran',
  wedged: 'Stuck',
  dead_letters: 'Undelivered',
};

export function JobHealthStrip({ summary, onOpen, canOpen }: {
  summary: HealthSummary | null;
  /** Open a schedule's detail. */
  onOpen?: (id: string) => void;
  /** Which problem ids are schedules on this page. Built-in loops (the Guard,
   *  the Librarian) have no detail here, so they are not buttons. */
  canOpen?: (id: string) => boolean;
}) {
  const { colors } = useTheme();
  if (!summary) return null;
  const tone = summary.state === 'ok' ? colors.success : summary.state === 'problems' ? colors.warning : colors.textDim;
  return (
    <div
      role="status"
      data-testid="job-health-strip"
      data-state={summary.state}
      style={{
        display: 'flex', alignItems: 'flex-start', gap: space.lg, flexWrap: 'wrap',
        padding: summary.state === 'ok' ? `${space.xs}px 0` : `${space.md}px ${space.xxl}px`, borderRadius: radius.md, marginBottom: space.xl,
        background: summary.state === 'ok' ? 'transparent' : withAlpha(tone, 0.08), border: summary.state === 'ok' ? 'none' : `1px solid ${withAlpha(tone, 0.22)}`,
        fontFamily: font.body,
      }}
    >
      <span style={{ width: 8, height: 8, borderRadius: '50%', background: tone, flexShrink: 0, marginTop: 5 }} />
      <span style={{ fontSize: textSize.caption, fontWeight: summary.state === 'ok' ? 400 : 600, color: summary.state === 'ok' ? colors.textMuted : colors.text, minWidth: 0 }}>{summary.message}</span>
      {summary.state === 'problems' && (
        <span style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, flex: 1, minWidth: 0 }}>
          {summary.problems.map(p => {
            const color = p.kind === 'failing' || p.kind === 'wedged' ? colors.danger : colors.warning;
            const body = (
              <>
                <span style={{ fontFamily: font.mono, fontSize: 10, color }}>{KIND_WORD[p.kind]}</span>
                <span style={{ fontSize: textSize.micro, color: colors.text, fontWeight: 600 }}>{p.label}</span>
                <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>{p.reason}</span>
              </>
            );
            const chip: CSSProperties = {
              display: 'inline-flex', alignItems: 'baseline', gap: space.sm,
              padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.sm,
              background: colors.surface, border: `1px solid ${colors.border}`,
            };
            return onOpen && canOpen?.(p.id) ? (
              <button
                key={`${p.kind}:${p.id}`}
                type="button"
                className="pa-btn pa-btn--composite"
                data-testid="job-health-problem"
                onClick={() => onOpen(p.id)}
                style={{
                  ...chip,
                  '--pa-btn-bg': colors.surface, '--pa-btn-bg-hover': colors.surfaceHi,
                  '--pa-btn-border': colors.border, '--pa-btn-border-hover': colors.borderHi,
                  '--pa-btn-pad': `${space.xxs}px ${space.md}px`, '--pa-btn-radius': `${radius.sm}px`,
                } as CSSProperties}
              >{body}</button>
            ) : (
              <span key={`${p.kind}:${p.id}`} data-testid="job-health-problem" style={chip}>{body}</span>
            );
          })}
        </span>
      )}
    </div>
  );
}
