/**
 * World W2 — the shell every place panel shares.
 *
 * Header (place name, host agent, state line), exactly ONE primary action,
 * the secondary actions, the place-level empty state, then the place's own
 * evidence. Keyboard-first: the primary action takes focus when the panel
 * opens; Tab reaches everything; Escape closes the topmost layer of THIS panel
 * only and returns focus to whatever opened it. No global listeners.
 *
 * The panel root is a non-modal `role="dialog"`: it is the accessible,
 * non-spatial equivalent of selecting the place's object in the scene, and
 * ForumView's own Escape handler already leaves dialog keys alone.
 */
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from 'react';
import { Button } from '../../../common/Button';
import type { ThemeColors } from '../../../../styles/tokens';
import type { PlaceSchema } from '../data/placeSchema';
import { useForumReducedMotion } from '../useForumReducedMotion';
import { createLayerStack, LayerContext } from './layers';
import type { PlaceEvent } from './integration';
import type { PlaceStyles } from './placeStyles';

export interface PlaceActionSpec {
  id: string;
  label: string;
  description: string;
  onActivate(): void;
  disabled?: boolean;
  /** Shown beside a disabled action: why it is not available yet. */
  disabledReason?: string;
  /** A clearly labelled demonstration, separate from real evidence. */
  demonstration?: boolean;
  /** For toggles that open an inline layer. */
  expanded?: boolean;
  ref?: RefObject<HTMLButtonElement>;
}

export interface PlacePanelProps {
  schema: PlaceSchema;
  hostName: string;
  /** e.g. "no agents-API record" — said, never hidden. */
  hostNote?: string | null;
  stateLine: { text: string; tone: 'ok' | 'stale' | 'error' | 'setup' | 'loading' };
  /** Extra header facts (Arrival's four-fact orchestrator state). */
  headerExtra?: ReactNode;
  primary: PlaceActionSpec;
  secondary: PlaceActionSpec[];
  /** True when every evidence slot is empty or needs setup. */
  placeEmpty: boolean;
  compact: boolean;
  colors: ThemeColors;
  styles: PlaceStyles;
  onClose(): void;
  onEvent?(event: PlaceEvent): void;
  returnFocusTo?: HTMLElement | null;
  children: ReactNode;
}

export function PlacePanel({
  schema, hostName, hostNote, stateLine, headerExtra, primary, secondary, placeEmpty, compact, colors, styles,
  onClose, onEvent, returnFocusTo, children,
}: PlacePanelProps) {
  const titleId = useId();
  const hintId = useId();
  const stack = useMemo(() => createLayerStack(), []);
  const rootRef = useRef<HTMLDivElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const ownPrimaryRef = useRef<HTMLButtonElement>(null);
  const primaryRef = primary.ref ?? ownPrimaryRef;
  const reduced = useForumReducedMotion();
  const [shown, setShown] = useState(false);
  // The element that opened the panel, captured before the panel takes focus.
  const [opener] = useState<HTMLElement | null>(() => (typeof document !== 'undefined' && document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null));
  const returnTo = useRef<HTMLElement | null>(null);
  returnTo.current = returnFocusTo ?? opener;
  const placeId = schema.id;
  const eventRef = useRef(onEvent);
  eventRef.current = onEvent;

  useEffect(() => {
    setShown(true);
    eventRef.current?.({ type: 'place_opened', placeId });
    // Keyboard-first: the one primary action is where focus starts.
    const target = primaryRef.current && !primaryRef.current.disabled ? primaryRef.current : headingRef.current;
    target?.focus({ preventScroll: true });
    return () => {
      eventRef.current?.({ type: 'place_closed', placeId });
      const back = returnTo.current;
      if (back && back.isConnected) back.focus({ preventScroll: true });
    };
    // Mount/unmount only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Escape' || e.defaultPrevented || e.nativeEvent.isComposing) return;
    e.preventDefault();
    e.stopPropagation();
    if (!stack.closeTop()) onClose();
  };

  const disabledNotes = [primary, ...secondary].filter((a) => a.disabled && a.disabledReason);

  return (
    <LayerContext.Provider value={stack}>
      <div
        ref={rootRef}
        role="dialog"
        aria-modal="false"
        aria-labelledby={titleId}
        aria-describedby={hintId}
        data-world-place={schema.id}
        data-compact={compact ? 'true' : 'false'}
        data-reduced-motion={reduced ? 'true' : 'false'}
        onKeyDown={onKeyDown}
        style={{ ...styles.panel, ...styles.enter(shown, reduced) }}
      >
        <header style={styles.header}>
          <div style={styles.headerText}>
            <p style={styles.eyebrow}>{schema.interactionObject}</p>
            <h2 id={titleId} ref={headingRef} tabIndex={-1} style={styles.title}>{schema.name}</h2>
            <p style={styles.hostLine}>Host: {hostName}{hostNote ? ` — ${hostNote}` : ''}</p>
            <p style={styles.stateLine} role={stateLine.tone === 'error' ? 'alert' : 'status'} data-state-tone={stateLine.tone}>
              {stateLine.tone !== 'loading' && <span aria-hidden="true" style={styles.dot(stateLine.tone)} />}
              <span>{stateLine.text}</span>
            </p>
          </div>
          <Button colors={colors} variant="bare" flashSuccess={false} aria-label={`Close ${schema.name}`} onClick={onClose}>×</Button>
        </header>
        {headerExtra}
        <div style={styles.actions}>
          <Button
            ref={primaryRef}
            colors={colors}
            variant="primary"
            flashSuccess={false}
            data-place-primary="true"
            data-action={primary.id}
            aria-describedby={hintId}
            aria-expanded={primary.expanded}
            disabled={primary.disabled}
            onClick={() => primary.onActivate()}
          >{primary.label}</Button>
          <div role="group" aria-label="More actions" style={styles.actions}>
            {secondary.map((a) => (
              <Button
                key={a.id}
                ref={a.ref}
                colors={colors}
                variant="ghost"
                flashSuccess={false}
                data-action={a.id}
                title={a.description}
                aria-expanded={a.expanded}
                disabled={a.disabled}
                onClick={() => a.onActivate()}
              >{a.label}{a.demonstration ? ' (demonstration)' : ''}</Button>
            ))}
          </div>
          <p id={hintId} style={styles.primaryHint}>{primary.description} Escape closes the most recent layer.</p>
          {disabledNotes.map((a) => <p key={a.id} style={styles.primaryHint} data-disabled-reason={a.id}>{a.label}: {a.disabledReason}</p>)}
        </div>
        {placeEmpty && (
          <section style={styles.slot} data-place-empty="true" aria-label={schema.emptyState.title}>
            <p style={styles.strong}>{schema.emptyState.title}</p>
            <p style={styles.note}>{schema.emptyState.body}</p>
          </section>
        )}
        {children}
      </div>
    </LayerContext.Provider>
  );
}
