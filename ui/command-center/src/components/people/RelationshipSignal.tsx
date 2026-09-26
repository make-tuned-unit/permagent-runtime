import type { RelationshipSignal as Signal } from '../projects/types';
import { useTheme } from '../../styles/useTheme';
import { space, radius, textSize } from '../../styles/tokens';
import { Tooltip } from '../common/Tooltip';
import { AsOf } from '../common/AsOf';

export function isFading(signal?: Signal | null): boolean {
  return signal?.bucket === 'fading' || signal?.bucket === 'dormant';
}

export function RelationshipMeter({ signal }: { signal?: Signal | null }) {
  const { colors } = useTheme();
  const label = !signal ? 'Relationship signal unavailable' : signal.bucket === 'no_signal'
    ? 'No interactions recorded yet' : `${signal.bucket[0].toUpperCase()}${signal.bucket.slice(1)} · ${signal.score}/100`;
  return <Tooltip content={label}><span role="img" aria-label={label} style={{ display: 'inline-flex', gap: space.xs }}>
    {[25, 50, 75, 100].map((level, i) => <span key={level} aria-hidden style={{
      width: space.sm, height: space.lg, borderRadius: radius.xs,
      background: signal && signal.score > i * 25 ? colors.cyan : colors.border,
    }} />)}
  </span></Tooltip>;
}

export function RelationshipBreakdown({ signal }: { signal?: Signal | null }) {
  const { colors } = useTheme();
  if (!signal) return null;
  return <section aria-label="Relationship activity" style={{ display: 'grid', gap: space.md, fontSize: textSize.caption }}>
    <strong>Relationship activity</strong>
    <RelationshipMeter signal={signal} />
    {signal.bucket === 'no_signal' ? <span>No interactions recorded yet</span> :
      (Object.entries(signal.signals) as [keyof Signal['signals'], Signal['signals']['meetings']][]).map(([key, value]) =>
        <div key={key} style={{ display: 'flex', gap: space.md, flexWrap: 'wrap', color: colors.textMuted }}>
          <span>{{ meetings: 'Meetings', decisions: 'Decisions', memories: 'Memories', last_contact: 'Recorded contact' }[key]}: {value.count}</span>
          {value.most_recent && <AsOf asOf={value.most_recent} prefix="Latest" />}
        </div>)}
    <span style={{ color: colors.textMuted }}>Based on recorded activity over the last 90 days.</span>
  </section>;
}
