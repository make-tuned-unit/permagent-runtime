/**
 * World W2 — one evidence slot, rendering every W1 section value honestly:
 * loading, ready (with truncation), stale, empty (with what empty does NOT
 * mean), error (connection vs daemon error, keeping the last good data),
 * unsupported and setup-required. A state is never collapsed into "nothing".
 */
import type { ReactNode } from 'react';
import { Button } from '../../../common/Button';
import type { ThemeColors } from '../../../../styles/tokens';
import type { EvidenceSlot as SlotSchema } from '../data/placeSchema';
import { sectionFreshness, visibleItems, type Section } from '../data/types';
import { ageLabel, failureLabel } from './format';
import type { PlaceStyles } from './placeStyles';

export interface EvidenceSlotProps<T> {
  slot: SlotSchema;
  section: Section<T> | undefined;
  loading: boolean;
  now: number;
  styles: PlaceStyles;
  colors: ThemeColors;
  /** Render the visible items (ready, or last good data when stale). */
  renderItems(items: readonly T[], stale: boolean): ReactNode;
  /** "Showing N; more exist" — where the full list lives. */
  moreWhere?: string;
  /** Offered for setup_required (e.g. focus the project chooser). */
  onSetup?: () => void;
  onRetry?: () => void;
  /** Collapse into a disclosure (compact layouts). */
  collapsible?: boolean;
}

export function slotStateOf(section: Section<unknown> | undefined, loading: boolean): string {
  if (!section) return loading ? 'loading' : 'missing';
  return section.state;
}

export function EvidenceSlot<T>({ slot, section, loading, now, styles, colors, renderItems, moreWhere, onSetup, onRetry, collapsible }: EvidenceSlotProps<T>) {
  const state = slotStateOf(section as Section<unknown> | undefined, loading);
  const { items, stale, truncated } = section ? visibleItems(section, now) : { items: [] as readonly T[], stale: false, truncated: false };
  const fresh = section ? sectionFreshness(section as Section<unknown>, now) : 'not_applicable';
  const headingId = `world-slot-${slot.id}`;

  let meta: ReactNode = null;
  if (section && (section.state === 'ready' || section.state === 'empty')) meta = <span style={styles.slotMeta}>{fresh === 'stale' ? `Stale · read ${ageLabel(section.fetchedAt, now)}` : `Read ${ageLabel(section.fetchedAt, now)}`}</span>;
  else if (section?.state === 'error') meta = <span style={styles.badge('danger')}>{section.failure.cause === 'disconnected' ? 'Disconnected' : 'Error'}</span>;

  let body: ReactNode;
  if (!section) {
    body = <p style={styles.note} aria-busy={loading || undefined}>{loading ? 'Reading…' : 'Not loaded.'}</p>;
  } else {
    switch (section.state) {
      case 'ready':
        body = <>
          {stale && <p style={styles.note}><span style={styles.badge('warn')}>Stale</span> Last read {ageLabel(section.fetchedAt, now)}; it refreshes on your next visit.</p>}
          {renderItems(items, stale)}
          {truncated && <p style={styles.note}>Showing {items.length}; more exist{moreWhere ? ` — the full list is in ${moreWhere}` : ''}. This is not the complete list.</p>}
        </>;
        break;
      case 'empty':
        body = <p style={styles.note}>{section.copy || slot.emptyCopy}</p>;
        break;
      case 'error':
        body = <>
          <p style={styles.warn} data-failure={section.failure.cause}>{failureLabel(section.failure.cause)}</p>
          {section.lastGood
            ? section.lastGood.items.length
              ? <><p style={styles.note}><span style={styles.badge('warn')}>Stale</span> Last good data from {ageLabel(section.lastGood.fetchedAt, now)}:</p>{renderItems(items, true)}</>
              : <p style={styles.note}><span style={styles.badge('warn')}>Stale</span> Last known answer ({ageLabel(section.lastGood.fetchedAt, now)}): nothing. That may have changed.</p>
            : <p style={styles.note}>No earlier data to show. This is unknown, not empty.</p>}
          {onRetry && <div style={styles.actions}><Button colors={colors} flashSuccess={false} onClick={onRetry}>Try again</Button></div>}
        </>;
        break;
      case 'unsupported':
        body = <p style={styles.note}>Not available here: {section.reason}</p>;
        break;
      case 'setup_required':
        body = <>
          <p style={styles.note}>{section.reason}</p>
          {onSetup && <div style={styles.actions}><Button colors={colors} flashSuccess={false} onClick={onSetup}>{section.action}</Button></div>}
        </>;
        break;
    }
  }

  const head = <span style={styles.slotHead}><span id={headingId} style={styles.slotTitle}>{slot.label}</span>{meta}</span>;
  const common = { 'data-slot': slot.id, 'data-slot-state': state, 'data-stale': stale ? 'true' : 'false' } as const;
  if (collapsible) {
    return (
      <details style={styles.slot} {...common}>
        <summary style={{ cursor: 'pointer' }}>{head}</summary>
        {body}
      </details>
    );
  }
  return (
    <section style={styles.slot} aria-labelledby={headingId} {...common}>
      <h4 style={{ margin: 0 }}>{head}</h4>
      {body}
    </section>
  );
}
