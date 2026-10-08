/**
 * Home focus mode: one card expanded into focus, every other card compact.
 *
 * Display state only. Nothing here touches the persisted layout — focusing a
 * card changes how the grid is DRAWN for this session, the way Customize's
 * stretch-free preview does, and collapsing puts the overview back exactly as
 * it was. One card is focused at a time; Customize turns focus off.
 *
 * Cards opt into richer behaviour through {@link useCardPresentation}: a card
 * that has its own compact and full renderings (Calendar) reads it and
 * switches; every other card needs nothing — the Dashboard gives it the full
 * width and its uncapped body when focused, and a fixed compact tile (whose own
 * scroll body still works) when another card is.
 */
import {
  createContext, useCallback, useContext, useEffect, useRef, useState, type CSSProperties, type RefObject,
} from 'react';
import { flushSync } from 'react-dom';
import { FiMaximize2, FiMinimize2 } from 'react-icons/fi';
import { font, getReduceMotion, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Tooltip } from '../common/Tooltip';
import './homeFocus.css';

/** How a Home card is being drawn right now. */
export type CardPresentation = 'overview' | 'focused' | 'compact';

export interface CardPresentationValue {
  presentation: CardPresentation;
  /** Expand this card into focus. */
  expand: () => void;
  /** Return to the overview. */
  collapse: () => void;
}

const CardPresentationContext = createContext<CardPresentationValue | null>(null);
export const CardPresentationProvider = CardPresentationContext.Provider;

/** The card's presentation on Home, or null outside the Home grid (tests,
 *  any other host) — callers keep their own default behaviour then. */
export function useCardPresentation(): CardPresentationValue | null {
  return useContext(CardPresentationContext);
}

/** Attribute on each card's expand control; collapse returns focus to it. */
export const FOCUS_TRIGGER_ATTR = 'data-home-focus-trigger';
/** Attribute on the focused card's collapse control; expand moves focus to it. */
export const FOCUS_COLLAPSE_ATTR = 'data-home-focus-collapse';

type StartViewTransition = (cb: () => void) => { finished?: Promise<unknown> };
let latestHomeTransition = 0;

/**
 * Apply a focus change, animated as a morph where View Transitions exist and
 * motion is allowed (homeFocus.css names each cell for the duration), and
 * instantly otherwise. Same contract as `runViewTransition`, but with its own
 * root-transition override: the page around the cards must not slide.
 */
export function runHomeFocusTransition(update: () => void): void {
  const doc = typeof document === 'undefined' ? undefined
    : document as Document & { startViewTransition?: StartViewTransition };
  if (!doc || typeof doc.startViewTransition !== 'function' || getReduceMotion()) {
    update();
    return;
  }
  const html = doc.documentElement;
  const transition = ++latestHomeTransition;
  html.dataset.homeFocusVt = 'true';
  const clear = () => {
    // Finishing an older transition must not remove the newer one's styles.
    if (transition === latestHomeTransition) delete html.dataset.homeFocusVt;
  };
  try {
    const t = doc.startViewTransition(() => flushSync(update));
    if (t.finished) void t.finished.then(clear, clear);
    else clear();
  } catch {
    clear();
    update();
  }
}

/** Whether this engine can morph a focus change (else the CSS fade is used). */
export function supportsFocusMorph(): boolean {
  return typeof document !== 'undefined'
    && typeof (document as Document & { startViewTransition?: unknown }).startViewTransition === 'function';
}

function isTypingTarget(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable;
}

/**
 * The focused card and how to change it.
 *
 * - `expand(id)` focuses one card (replacing any other) and moves keyboard
 *   focus to its Collapse control.
 * - `collapse()` returns to the overview and puts keyboard focus back on the
 *   card's Expand control, so a keyboard user lands where they started.
 * - Escape collapses — only for a key pressed inside the dashboard (or on
 *   the page body), never one already handled (`defaultPrevented`), never
 *   while a modal dialog is open, and never from a text field. It does not
 *   stop propagation, so nothing else's Escape handling is affected.
 * - Focus clears itself when the card leaves the layout or `disabled` turns
 *   on (Customize).
 */
export function useHomeFocus({ ids, disabled, rootRef, getCell }: {
  ids: readonly string[];
  disabled: boolean;
  rootRef: RefObject<HTMLElement>;
  getCell: (id: string) => HTMLElement | undefined;
}) {
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const focusedRef = useRef<string | null>(null);
  focusedRef.current = focusedId;

  const moveFocus = useCallback((id: string, attr: string) => {
    requestAnimationFrame(() => {
      const target = getCell(id)?.querySelector<HTMLElement>(`[${attr}]`);
      target?.focus({ preventScroll: attr === FOCUS_TRIGGER_ATTR });
      // The focused card is drawn first; bring its top into view.
      if (attr === FOCUS_COLLAPSE_ATTR) getCell(id)?.scrollIntoView?.({ block: 'nearest', behavior: getReduceMotion() ? 'auto' : 'smooth' });
    });
  }, [getCell]);

  const expand = useCallback((id: string) => {
    if (disabled || focusedRef.current === id) return;
    runHomeFocusTransition(() => {
      setFocusedId(id);
      // View-transition updates are asynchronous: schedule focus only after
      // the DOM change has been requested, not while the old snapshot remains.
      moveFocus(id, FOCUS_COLLAPSE_ATTR);
    });
  }, [disabled, moveFocus]);

  const collapse = useCallback(() => {
    const was = focusedRef.current;
    if (!was) return;
    runHomeFocusTransition(() => {
      setFocusedId(null);
      moveFocus(was, FOCUS_TRIGGER_ATTR);
    });
  }, [moveFocus]);

  // Never a stale focus: the card was removed, or Customize took over.
  const idsKey = ids.join('\u0000');
  useEffect(() => {
    if (focusedId && (disabled || !ids.includes(focusedId))) setFocusedId(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- idsKey stands for ids
  }, [focusedId, disabled, idsKey]);

  useEffect(() => {
    if (!focusedId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      const target = e.target as Element | null;
      const root = rootRef.current;
      const fromHome = target === document.body || (!!root && !!target && root.contains(target));
      if (!fromHome || isTypingTarget(target)) return;
      if (document.querySelector('[aria-modal="true"]')) return;
      e.preventDefault();
      collapse();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [focusedId, rootRef, collapse]);

  return { focusedId, expand, collapse };
}

/** The Expand control over a card's corner. */
export function ExpandCardButton({ name, cellId, onExpand }: {
  name: string; cellId: string; onExpand: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Tooltip content={`Expand ${name}`}>
      <Button
        colors={colors}
        variant="bare"
        type="button"
        flashSuccess={false}
        className="pa-home-expand"
        aria-label={`Expand ${name}`}
        aria-expanded={false}
        aria-controls={cellId}
        {...{ [FOCUS_TRIGGER_ATTR]: '' }}
        onClick={e => { e.stopPropagation(); onExpand(); }}
        style={{
          '--pa-btn-bg': colors.surface,
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-border': colors.border,
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-border-hover': colors.borderHi,
          '--pa-btn-pad': '0',
          '--pa-btn-radius': `${radius.sm}px`,
          position: 'absolute', top: space.md, right: space.md, zIndex: 4,
          width: 26, height: 26,
          boxShadow: colors.elevationRaised,
        } as CSSProperties}
      >
        <FiMaximize2 size={13} />
      </Button>
    </Tooltip>
  );
}

/** The strip above a focused card: what it is, how to get back. */
export function FocusBar({ name, cellId, onCollapse }: {
  name: string; cellId: string; onCollapse: () => void;
}) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: space.lg,
      padding: `0 ${space.xs}px ${space.md}px`,
      fontFamily: font.body,
    }}>
      <span style={{ fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase', color: colors.textDim }}>
        Focused
      </span>
      <span style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.text, minWidth: 0, overflowWrap: 'anywhere' }}>{name}</span>
      <span style={{ fontSize: textSize.micro, color: colors.textDim, marginLeft: 'auto' }}>Esc to return</span>
      <Button
        colors={colors}
        type="button"
        flashSuccess={false}
        aria-label={`Collapse ${name}`}
        aria-expanded
        aria-controls={cellId}
        {...{ [FOCUS_COLLAPSE_ATTR]: '' }}
        onClick={onCollapse}
        style={{
          '--pa-btn-bg': colors.surface,
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-border': colors.border,
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-border-hover': colors.borderHi,
          '--pa-btn-pad': `${space.xs}px ${space.lg}px`,
          '--pa-btn-radius': `${radius.md}px`,
          fontSize: textSize.caption, flexShrink: 0,
        } as CSSProperties}
      >
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: space.sm }}>
          <FiMinimize2 size={13} /> Collapse
        </span>
      </Button>
    </div>
  );
}
