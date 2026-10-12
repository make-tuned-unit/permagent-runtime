/**
 * Tooltip — the one floating label primitive.
 *
 * Replaces the native `title=` attribute: styled glass, short delay, keyboard
 * focus, aria-describedby, Escape/scroll dismiss, and viewport-edge flipping.
 * Glass only (D1) — this is a floating control layer, never a content surface.
 *
 * Sidebar row labels keep their own controller (`useSidebarTooltip`) and the
 * browser-pane inversion in `sidebar/tooltipPlacement.ts`; they render through
 * `TooltipBubble` so the material and motion stay one place.
 */

import {
  Children,
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

import { useTheme } from '../../styles/useTheme';
import {
  concentric, duration, ease, font, radius, space, textSize,
} from '../../styles/tokens';
import { glassSurface } from './Glass';
import {
  placeViewportTooltip,
  type TipSize,
  type TooltipSide,
  type ViewportPlacement,
} from './tooltipPlacement';

/** Deliberate on first entry — matches the sidebar's cold delay. */
export const TOOLTIP_COLD_DELAY_MS = 260;
/** Once the pointer is clearly browsing, subsequent tips appear instantly. */
export const TOOLTIP_WARM_WINDOW_MS = 700;

/** Shared across every Tooltip on the page (and the sidebar controller). */
let lastHiddenAt = 0;

/** Test/sidebar seam: record a hide so the next show is warm. */
export function _markTooltipHiddenForTests(at = Date.now()): void {
  lastHiddenAt = at;
}

export function _resetTooltipWarmForTests(): void {
  lastHiddenAt = 0;
}

export function isTooltipWarm(now = Date.now()): boolean {
  return now - lastHiddenAt < TOOLTIP_WARM_WINDOW_MS;
}

export function noteTooltipHidden(now = Date.now()): void {
  lastHiddenAt = now;
}

// ── Bubble (shared glass chrome) ─────────────────────────────────────

const PAD_Y = space.sm; // 6
const PAD_X = space.lg; // 10
/** Outer radius is the floating-glass step; inner chips sit concentrically. */
const OUTER_R = radius.glass; // 9
const INNER_R = concentric(OUTER_R, PAD_Y);

export interface TooltipBubbleProps {
  id: string;
  children: ReactNode;
  /** Measuring seam: the placer needs the bubble's real box to flip honestly. */
  bubbleRef?: (el: HTMLDivElement | null) => void;
  left: number;
  top: number;
  transform: string;
  fromTransform?: string;
  maxWidth?: number;
  shortcut?: string;
  /** When maxWidth is tight (sidebar vs browser), wrap instead of truncating. */
  wrap?: boolean;
  role?: 'tooltip';
  style?: CSSProperties;
}

/**
 * The glass label itself. Portalled by callers; owns material, concentric
 * radius, spring-in, and reduce-motion / reduce-transparency.
 */
export function TooltipBubble({
  id,
  children,
  bubbleRef,
  left,
  top,
  transform,
  fromTransform,
  maxWidth,
  shortcut,
  wrap,
  role = 'tooltip',
  style,
}: TooltipBubbleProps) {
  const { colors, glass, reduceTransparency, reduceMotion } = useTheme();
  const material = glassSurface(glass.glass, reduceTransparency);
  const [entered, setEntered] = useState(reduceMotion || !fromTransform);

  useLayoutEffect(() => {
    if (reduceMotion || !fromTransform) {
      setEntered(true);
      return;
    }
    setEntered(false);
    const raf = requestAnimationFrame(() => setEntered(true));
    return () => cancelAnimationFrame(raf);
  }, [reduceMotion, fromTransform, left, top, transform]);

  const motion: CSSProperties = reduceMotion || !fromTransform
    ? { opacity: 1, transform }
    : {
        opacity: entered ? 1 : 0,
        transform: entered ? transform : fromTransform,
        transition: [
          `opacity ${duration.snappy}ms ${ease.snappy}`,
          `transform ${duration.snappy}ms ${ease.snappy}`,
        ].join(', '),
      };

  return (
    <div
      id={id}
      role={role}
      ref={bubbleRef}
      style={{
        position: 'fixed',
        top,
        left,
        zIndex: 9999,
        pointerEvents: 'none',
        display: 'flex',
        alignItems: 'center',
        gap: space.md,
        flexWrap: wrap ? 'wrap' : undefined,
        maxWidth,
        padding: `${PAD_Y}px ${PAD_X}px`,
        borderRadius: OUTER_R,
        border: `1px solid ${colors.borderHi}`,
        ...material,
        fontFamily: font.body,
        fontSize: textSize.caption,
        fontWeight: 500,
        color: colors.text,
        whiteSpace: wrap ? 'normal' : 'nowrap',
        wordBreak: wrap ? 'break-word' : undefined,
        ...motion,
        ...style,
      }}
    >
      {children}
      {shortcut && (
        <span style={{
          fontSize: textSize.micro,
          fontWeight: 600,
          letterSpacing: '0.04em',
          color: colors.textDim,
          border: `1px solid ${colors.border}`,
          borderRadius: INNER_R,
          padding: `1px ${space.xs}px`,
          whiteSpace: 'nowrap',
        }}>{shortcut}</span>
      )}
    </div>
  );
}

// ── Controller helpers (sidebar reuses delay/warm + dismiss) ─────────

export function useTooltipDismiss(open: boolean, hide: () => void): void {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') hide();
    };
    const onScroll = () => hide();
    window.addEventListener('keydown', onKey);
    // Capture: scroll may fire on any ancestor, not just window.
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open, hide]);
}

// ── Wrapper primitive ────────────────────────────────────────────────

export interface TooltipProps {
  /** Label body. Empty / null disables the tip without unmounting children. */
  content: ReactNode;
  children: ReactElement;
  placement?: TooltipSide;
  /** Override cold delay (ms). Focus always opens immediately. */
  delayMs?: number;
  disabled?: boolean;
  shortcut?: string;
}

/**
 * Wrap a single element. Clones the child (no extra DOM box) so layout and
 * test queries that expect the trigger as `firstElementChild` keep working.
 * Strips a native `title` if the child still carries one, so a11y does not
 * double-announce via the OS chrome.
 */
export function Tooltip({
  content,
  children,
  placement = 'top',
  delayMs = TOOLTIP_COLD_DELAY_MS,
  disabled = false,
  shortcut,
}: TooltipProps) {
  const reactId = useId();
  const tipId = `pa-tip-${reactId.replace(/:/g, '')}`;
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  // The bubble's own box. Null for exactly one frame (before it is laid out),
  // during which `placeViewportTooltip` falls back to its size-free heuristic.
  // Without this the placer guessed 48px of needed clearance, so a label that
  // wrapped to two lines was placed "above" a trigger 50px down the window and
  // rendered up through the chat header onto the icon row (2026-09-18).
  const [tipSize, setTipSize] = useState<TipSize | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const triggerRef = useRef<HTMLElement | null>(null);

  const clear = useCallback(() => {
    clearTimeout(timer.current);
  }, []);

  const hide = useCallback(() => {
    clear();
    setOpen(prev => {
      if (prev) noteTooltipHidden();
      return false;
    });
    setAnchor(null);
    // Next open may carry different content, so never place it on a stale box.
    setTipSize(null);
  }, [clear]);

  /**
   * Ref on the portalled bubble: the one place its real size exists. A zero
   * box means "not laid out" (jsdom always, a browser never once painted), and
   * is dropped so the fallback heuristic stays in charge rather than the placer
   * flipping on a measurement that never happened.
   */
  const measureBubble = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    if (width <= 0 || height <= 0) return;
    setTipSize(prev => (
      prev && Math.abs(prev.width - width) < 0.5 && Math.abs(prev.height - height) < 0.5
        ? prev
        : { width, height }
    ));
  }, []);

  const commit = useCallback((el: HTMLElement) => {
    setAnchor(el.getBoundingClientRect());
    setOpen(true);
  }, []);

  const show = useCallback((el: HTMLElement | null, immediate: boolean) => {
    if (!el || disabled || content == null || content === false || content === '') return;
    clear();
    if (immediate || isTooltipWarm() || delayMs <= 0) {
      commit(el);
      return;
    }
    timer.current = setTimeout(() => commit(el), delayMs);
  }, [disabled, content, clear, commit, delayMs]);

  useEffect(() => () => clear(), [clear]);
  useTooltipDismiss(open, hide);

  // The anchor box is read once, when the tooltip opens. A control that opens
  // while its container is still settling (a modal's close button, focused on
  // mount) moves right after, leaving the bubble stranded far from it — "Close"
  // floated ~100px under the X in the card dialog. Re-read the box while open.
  useEffect(() => {
    if (!open) return;
    let raf = 0;
    let frames = 0;
    const resync = () => {
      const el = triggerRef.current;
      if (el) {
        const next = el.getBoundingClientRect();
        setAnchor(prev => (
          prev && Math.abs(prev.top - next.top) < 0.5 && Math.abs(prev.left - next.left) < 0.5
            && Math.abs(prev.width - next.width) < 0.5 && Math.abs(prev.height - next.height) < 0.5
            ? prev
            : next
        ));
      }
      if (++frames < 30) raf = requestAnimationFrame(resync);
    };
    raf = requestAnimationFrame(resync);
    return () => cancelAnimationFrame(raf);
  }, [open]);

  const child = Children.only(children);
  const childProps = child.props as Record<string, unknown>;

  const onMouseEnter = (e: MouseEvent<HTMLElement>) => {
    (childProps.onMouseEnter as ((ev: MouseEvent<HTMLElement>) => void) | undefined)?.(e);
    show(e.currentTarget, false);
  };
  const onMouseLeave = (e: MouseEvent<HTMLElement>) => {
    (childProps.onMouseLeave as ((ev: MouseEvent<HTMLElement>) => void) | undefined)?.(e);
    hide();
  };
  const onFocus = (e: FocusEvent<HTMLElement>) => {
    (childProps.onFocus as ((ev: FocusEvent<HTMLElement>) => void) | undefined)?.(e);
    // Keyboard focus: open immediately so tabbing is not waiting on the hover delay.
    show(e.currentTarget, true);
  };
  const onBlur = (e: FocusEvent<HTMLElement>) => {
    (childProps.onBlur as ((ev: FocusEvent<HTMLElement>) => void) | undefined)?.(e);
    hide();
  };

  const existingDescribedBy = typeof childProps['aria-describedby'] === 'string'
    ? childProps['aria-describedby']
    : undefined;
  const describedBy = open
    ? [existingDescribedBy, tipId].filter(Boolean).join(' ')
    : existingDescribedBy;

  const mergedRef = (node: HTMLElement | null) => {
    triggerRef.current = node;
    const r = (child as { ref?: unknown }).ref;
    if (typeof r === 'function') r(node);
    else if (r && typeof r === 'object' && 'current' in (r as object)) {
      (r as { current: HTMLElement | null }).current = node;
    }
  };

  let placementPos: ViewportPlacement | null = null;
  if (open && anchor) {
    placementPos = placeViewportTooltip(anchor, placement, undefined, undefined, tipSize ?? undefined);
  }

  return (
    <>
      {cloneElement(child, {
        ref: mergedRef,
        onMouseEnter,
        onMouseLeave,
        onFocus,
        onBlur,
        'aria-describedby': describedBy,
        // Never leave a native title competing with the glass tip.
        title: undefined,
      } as Record<string, unknown>)}
      {open && placementPos && content != null && content !== false && content !== '' && createPortal(
        <TooltipBubble
          id={tipId}
          bubbleRef={measureBubble}
          left={placementPos.left}
          top={placementPos.top}
          transform={placementPos.transform}
          fromTransform={placementPos.fromTransform}
          shortcut={shortcut}
        >
          {content}
        </TooltipBubble>,
        document.body,
      )}
    </>
  );
}
