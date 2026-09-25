/**
 * Overnight Digest — Morning Briefing's ambient tail (§2.1, §3
 * `overnight_digest`). Deterministic bucket counts since local midnight; a
 * quiet night is said as one, not rendered as four blank rows (P8 calm tech —
 * this card is small and stays quiet unless there's something to report).
 */
import { radius, space } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { SectionTitle, EmptyNote, StatCompact } from '../atoms';
import { useDashboardInsights } from './useDashboardInsights';

export function OvernightDigestCard() {
  const { colors } = useTheme();
  const { data, loading, error } = useDashboardInsights();
  const digest = data?.overnightDigest;

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
      <SectionTitle title="Overnight" />
      {error ? (
        <EmptyNote hint="Couldn't reach the daemon">Couldn't load the overnight digest</EmptyNote>
      ) : loading && !data ? (
        <EmptyNote>Loading…</EmptyNote>
      ) : !digest || digest.quiet ? (
        <EmptyNote>Quiet night</EmptyNote>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
          {digest.newDecisions > 0 && (
            <StatCompact label="New decisions" value={digest.newDecisions} />
          )}
          {digest.stalledSince > 0 && (
            <StatCompact label="Newly stalled" value={digest.stalledSince} />
          )}
        </div>
      )}
    </div>
  );
}
