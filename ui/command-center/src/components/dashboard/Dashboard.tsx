import { useAppNavigationRequests, consumeNavigationRequest } from '../../lib/appNavigationRequests';
import { FiEdit2, FiX, FiPlus, FiRotateCcw, FiCornerRightDown } from 'react-icons/fi';

import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from './useDashboard';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { useDueTodos } from '../../lib/useDueTodos';
import {
  useLayout, reflow, layoutForDisplay, minSpanForWidth, DEFAULT_LAYOUT,
  MASONRY_ROW_UNIT, spanForContent, settleSpan,
  type DashboardLayoutData, type DashboardCardLayout, type SpanSettleState,
} from './useLayout';
import { useCardRegistry } from './cards/useCardRegistry';
import { AddCardPicker } from './AddCardPicker';
import { DashboardOverflowMenu } from './DashboardOverflowMenu';
import { CustomizeButton } from './CustomizeButton';
import { MissingCard } from './MissingCard';
import { Echo } from './Echo';
import { LearnNext } from './LearnNext';
import { isDefaultHomeLayout } from './homeFirstAction';
import { navigateToTool, useCommandCenter } from '../../lib/store';
import { openChatModelSetup } from '../chat/ChatSetupCard';
import { ViewHeader } from '../common/ViewHeader';
import { HeaderAgentStatus } from '../sidebar/NavAgentStatus';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { Button } from '../common/Button';
import { AsOf } from '../common/AsOf';
import { useState, useCallback, useEffect, useRef, useMemo, type CSSProperties } from 'react';
import { FiLayout } from 'react-icons/fi';
import { LayoutsGallery } from './LayoutsGallery';
import { LayoutPreviewOverlay } from './LayoutPreviewOverlay';
import { comingSoonCards, DASHBOARD_TEMPLATES } from './templates';
import { useDashboardLayoutPreviewEvent, clearDashboardLayoutPreview } from './dashboardLayoutPreviewEvent';

import { Tooltip } from '../common/Tooltip';
import { pressable } from '../common/pressable';
import {
  CardPresentationProvider, ExpandCardButton, FocusBar, supportsFocusMorph, useHomeFocus,
  type CardPresentation,
} from './homeFocus';
type SaveState = 'idle' | 'saving' | 'saved' | 'error';

function persistAndNotify(
  persistLayout: (l: DashboardLayoutData) => Promise<void>,
  newLayout: DashboardLayoutData,
  setSaveState: (s: SaveState) => void,
) {
  setSaveState('saving');
  const minVisible = new Promise(r => setTimeout(r, 700));
  Promise.all([persistLayout(newLayout), minVisible])
    .then(() => { setSaveState('saved'); setTimeout(() => setSaveState('idle'), 1000); })
    .catch(() => { setSaveState('error'); setTimeout(() => setSaveState('idle'), 2000); });
}

const ROW_HEIGHT = 60;
const GAP = space.xxl;
/** Height of a compact tile while another card is focused: three of the
 *  old grid rows — room for a card's heading and its first few rows, which
 *  its own scroll body continues below. */
const COMPACT_TILE_PX = 3 * ROW_HEIGHT + 2 * GAP;
/** Compact tiles are this many columns wide where the grid allows it (four
 *  to a row), wider where `minSpanForWidth` says that would be too narrow. */
const COMPACT_TILE_SPAN = 3;

export function Dashboard() {
  const { gradient, colors, reduceMotion } = useTheme();
  const { data, loading, error, lastOkAt, failing, retry, refresh } = useDashboard();
  const currentModel = useCommandCenter(s => s.currentModel);
  const chatRoute = useCommandCenter(s => s.chatResolvedRoute);
  const providersError = useCommandCenter(s => s.providersError);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const configRev = useCommandCenter(s => s.configRev);
  // Home can be the first surface after setup, before Chat has ever mounted.
  // Confirm configuration here rather than treating the store's null as empty.
  useEffect(() => { void loadProviders(); }, [loadProviders, configRev]);
  const hasModel = Boolean(currentModel || (chatRoute?.source !== 'unconfigured' && chatRoute?.model));
  // One shared live-goal subscription for every "in flight" surface (count,
  // list, header, status) so they always agree. Sessions are a separate stat.
  const { goals: activeGoals, activeCount } = useLiveGoals();
  // Stable props identity for the memoized InFlightCard — only changes when the
  // (deduped) goal list actually changes, so a benign refetch never re-renders it.
  const inFlightProps = useMemo(() => ({ goals: activeGoals }), [activeGoals]);
  // Dated to-dos across every board. Fetched once here and passed down, like
  // the goal subscription above — the card must not fetch per render.
  const dueTodos = useDueTodos();
  const todosProps = useMemo(() => ({ todos: dueTodos }), [dueTodos]);
  const { layout, layoutLoaded, persistLayout, revertLayout } = useLayout();
  // Rendered registry = first-party code cards + daemon-served manifest cards.
  const { registry, status: registryStatus } = useCardRegistry();
  const [isEditMode, setIsEditMode] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [showPicker, setShowPicker] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [showLayouts, setShowLayouts] = useState(false);
  // dagK home-templates lane: the agent's preview_dashboard_layout tool
  // proposes via a `dashboard_layout_preview` event rather than the gallery —
  // this surfaces it the same way the gallery's own "Try" does (P9: always an
  // inspectable overlay, never a silent write).
  const agentPreview = useDashboardLayoutPreviewEvent();
  const agentPreviewTemplate = agentPreview?.templateId
    ? DASHBOARD_TEMPLATES.find(t => t.id === agentPreview.templateId)
    : undefined;
  const [dragSrcId, setDragSrcId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [dragPos, setDragPos] = useState<{ x: number; y: number } | null>(null);
  const dragGhostSize = useRef<{ w: number; h: number }>({ w: 0, h: 0 });
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const gridRef = useRef<HTMLDivElement>(null);
  // Content-driven row spans (masonry via CSS grid; coordinator, 2560px round
  // 2). Each card's rendered span (in MASONRY_ROW_UNIT-tall tracks) tracks its
  // OWN measured height, capped at what its configured `h` would reserve
  // under the old fixed-row grid — see `spanForContent`'s doc. `spans` is
  // what render reads; `spanSettle`/`pendingHeights`/`flushHandle` are the
  // measurement pipeline behind it, kept in refs so a ResizeObserver firing
  // mid-frame never triggers more than one state update per animation frame.
  const [spans, setSpans] = useState<Record<string, number>>({});
  const spanSettle = useRef<Map<string, SpanSettleState>>(new Map());
  const pendingHeights = useRef<Map<string, number>>(new Map());
  const flushHandle = useRef<number | null>(null);
  // Each visible card's CURRENT configured `h`, refreshed every render — read
  // by `measureCard` (stable identity; never re-created) instead of being
  // captured in a closure, so a resize in Customize is reflected the next
  // time a measurement lands without giving the callback a changing identity
  // (which would otherwise mean tearing down and rebuilding every card's
  // ResizeObserver on every render).
  const configuredHeights = useRef<Map<string, number>>(new Map());
  const cellObservers = useRef<Map<string, ResizeObserver>>(new Map());
  const cellRefFns = useRef<Map<string, (el: HTMLDivElement | null) => void>>(new Map());

  // Focus mode (homeFocus.tsx): one card expanded, the rest compact. Display
  // only — never persisted. Customize turns it off.
  const rootRef = useRef<HTMLDivElement>(null);
  const getCell = useCallback((id: string) => cardRefs.current.get(id), []);
  const visibleIds = layout.cards.filter(c => c.visible).map(c => c.id);
  const { focusedId, expand, collapse } = useHomeFocus({
    ids: visibleIds, disabled: isEditMode, rootRef, getCell,
  });
  const focusMode = focusedId !== null && !isEditMode;
  const requestedFocus = useAppNavigationRequests(s => s.homeFocus);
  const [navigationNotice, setNavigationNotice] = useState<string | null>(null);
  useEffect(() => {
    // Wait for the real layout and grid; never focus an initial DEFAULT_LAYOUT card.
    if (!requestedFocus || !layoutLoaded || loading || !data) return;
    // Bundled code cards and collapse do not depend on the manifest request.
    // Wait only when the requested renderer could still arrive from it.
    if (!isEditMode && requestedFocus.section !== 'overview' && !registry[requestedFocus.section] && registryStatus === 'loading') return;
    const section = requestedFocus.section;
    if (isEditMode) {
      setNavigationNotice('Finish Customize before focusing a Home card. Your layout was not changed.');
    } else if (section === 'overview') {
      setNavigationNotice(null);
      collapse();
    } else {
      const card = layout.cards.find(c => c.visible && c.type === section);
      if (!card || !registry[section]) {
        setNavigationNotice(`The ${registry[section]?.name ?? section} card is not available in this Home layout. No card was added.`);
      } else {
        setNavigationNotice(null);
        expand(card.id);
      }
    }
    consumeNavigationRequest('homeFocus', requestedFocus);
  }, [requestedFocus, layoutLoaded, loading, data, registryStatus, isEditMode, layout.cards, registry, expand, collapse]);

  // While focused, cells are drawn at focus/compact sizes that say nothing
  // about their overview height — measuring them would feed the masonry
  // spans (whose settle state only ever grows once frozen) heights they will
  // not have again. Measurement pauses, and the settle state starts fresh
  // when the overview comes back.
  const focusActive = useRef(false);
  if (focusActive.current && !focusMode) spanSettle.current.clear();
  focusActive.current = focusMode;

  const measureCard = useCallback((id: string, heightPx: number) => {
    if (focusActive.current) return;
    pendingHeights.current.set(id, heightPx);
    if (flushHandle.current != null) return;
    flushHandle.current = requestAnimationFrame(() => {
      flushHandle.current = null;
      const updates: Record<string, number> = {};
      let changed = false;
      pendingHeights.current.forEach((h, cardId) => {
        const configuredH = configuredHeights.current.get(cardId) ?? 1;
        const raw = spanForContent(h, configuredH, MASONRY_ROW_UNIT, GAP, ROW_HEIGHT);
        const prevSettle = spanSettle.current.get(cardId);
        const next = settleSpan(prevSettle, raw);
        spanSettle.current.set(cardId, next);
        if (!prevSettle || prevSettle.span !== next.span) { updates[cardId] = next.span; changed = true; }
      });
      pendingHeights.current.clear();
      if (changed) setSpans(s => ({ ...s, ...updates }));
    });
  }, []);

  // Stable per-card ref callback: caches one function per id so the DOM node
  // is never torn down and re-observed just because a re-render passed a new
  // inline arrow function (the pitfall with callback refs). `measureCard`
  // above is itself stable, so a cached function keeps working forever.
  const getCellRef = useCallback((id: string) => {
    let fn = cellRefFns.current.get(id);
    if (fn) return fn;
    fn = (el: HTMLDivElement | null) => {
      if (el) {
        cardRefs.current.set(id, el);
        if (typeof ResizeObserver !== 'undefined') {
          let ro = cellObservers.current.get(id);
          if (!ro) {
            ro = new ResizeObserver(entries => {
              const h = entries[0]?.contentRect.height;
              if (h != null) measureCard(id, h);
            });
            cellObservers.current.set(id, ro);
          }
          ro.observe(el);
        }
      } else {
        cardRefs.current.delete(id);
        const ro = cellObservers.current.get(id);
        if (ro) { ro.disconnect(); cellObservers.current.delete(id); }
      }
    };
    cellRefFns.current.set(id, fn);
    return fn;
  }, [measureCard]);

  useEffect(() => {
    return () => {
      cellObservers.current.forEach(ro => ro.disconnect());
      cellObservers.current.clear();
      if (flushHandle.current != null) cancelAnimationFrame(flushHandle.current);
    };
  }, []);
  // Resize state
  const [resizeId, setResizeId] = useState<string | null>(null);
  const resizeStart = useRef<{ pointerId: number; startX: number; startY: number; origW: number; origH: number; colPx: number }>({ pointerId: 0, startX: 0, startY: 0, origW: 0, origH: 0, colPx: 0 });
  const [resizePreview, setResizePreview] = useState<{ w: number; h: number } | null>(null);
  // The grid's own width, so cards can be drawn no narrower than is readable
  // (see minSpanForWidth). Observed rather than read once: the chat dock and
  // sidebar change it without a window resize.
  const [gridWidth, setGridWidth] = useState(0);
  const gridMounted = !loading && !!data;
  useEffect(() => {
    const el = gridRef.current;
    if (!el) return;
    setGridWidth(el.getBoundingClientRect().width);
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(entries => {
      const w = entries[0]?.contentRect.width;
      if (w) setGridWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [gridMounted]);

  const reorderCards = useCallback((srcId: string, targetId: string) => {
    if (srcId === targetId) return;
    const cards = [...layout.cards];
    const srcIdx = cards.findIndex(c => c.id === srcId);
    const tgtIdx = cards.findIndex(c => c.id === targetId);
    if (srcIdx === -1 || tgtIdx === -1) return;
    const [moved] = cards.splice(srcIdx, 1);
    cards.splice(tgtIdx, 0, moved);
    persistAndNotify(persistLayout, { cards: reflow(cards) }, setSaveState);
  }, [layout.cards, persistLayout]);

  const removeCard = useCallback((id: string) => {
    const remaining = layout.cards.filter(c => c.id !== id);
    if (remaining.length === 0) return;
    persistAndNotify(persistLayout, { cards: remaining }, setSaveState);
  }, [layout.cards, persistLayout]);

  const addCard = useCallback((type: string) => {
    const entry = registry[type];
    if (!entry) return;
    const newCard: DashboardCardLayout = {
      id: type,
      type,
      position: { x: 0, y: 0 },
      size: { w: entry.defaultSize.w, h: entry.defaultSize.h },
      visible: true,
    };
    persistAndNotify(persistLayout, { cards: reflow([...layout.cards, newCard]) }, setSaveState);
  }, [layout.cards, persistLayout, registry]);

  const resetToDefault = useCallback(() => {
    setShowResetConfirm(false);
    persistAndNotify(persistLayout, DEFAULT_LAYOUT, setSaveState);
  }, [persistLayout]);

  // Pointer-event drag: find which card the pointer is over
  const findCardAtPoint = useCallback((clientX: number, clientY: number): string | null => {
    for (const [id, el] of cardRefs.current) {
      const rect = el.getBoundingClientRect();
      if (clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom) {
        return id;
      }
    }
    return null;
  }, []);

  const handlePointerDown = useCallback((cardId: string, e: React.PointerEvent) => {
    if (!isEditMode) return;
    if ((e.target as HTMLElement).closest('button')) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dragGhostSize.current = { w: rect.width, h: rect.height };
    setDragSrcId(cardId);
    setDragPos({ x: e.clientX, y: e.clientY });
  }, [isEditMode]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragSrcId) return;
    setDragPos({ x: e.clientX, y: e.clientY });
    const over = findCardAtPoint(e.clientX, e.clientY);
    setDragOverId(over && over !== dragSrcId ? over : null);
  }, [dragSrcId, findCardAtPoint]);

  const handlePointerUp = useCallback(() => {
    if (dragSrcId && dragOverId) {
      reorderCards(dragSrcId, dragOverId);
    }
    setDragSrcId(null);
    setDragOverId(null);
    setDragPos(null);
  }, [dragSrcId, dragOverId, reorderCards]);

  const handleResizeDown = useCallback((cardId: string, e: React.PointerEvent) => {
    e.stopPropagation();
    const card = layout.cards.find(c => c.id === cardId);
    if (!card || !gridRef.current) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const gridWidth = gridRef.current.getBoundingClientRect().width;
    const colPx = (gridWidth - 11 * GAP) / 12;
    resizeStart.current = { pointerId: e.pointerId, startX: e.clientX, startY: e.clientY, origW: card.size.w, origH: card.size.h, colPx };
    setResizeId(cardId);
    setResizePreview({ w: card.size.w, h: card.size.h });
  }, [layout.cards]);

  const handleResizeMove = useCallback((e: React.PointerEvent) => {
    if (!resizeId) return;
    const { startX, startY, origW, origH, colPx } = resizeStart.current;
    const rowPx = ROW_HEIGHT + GAP;
    const dCols = Math.round((e.clientX - startX) / colPx);
    const dRows = Math.round((e.clientY - startY) / rowPx);
    const newW = Math.max(1, Math.min(12, origW + dCols));
    const newH = Math.max(1, origH + dRows);
    setResizePreview({ w: newW, h: newH });
  }, [resizeId]);

  const handleResizeUp = useCallback(() => {
    if (!resizeId || !resizePreview) { setResizeId(null); setResizePreview(null); return; }
    const cards = layout.cards.map(c =>
      c.id === resizeId ? { ...c, size: { w: resizePreview.w, h: resizePreview.h } } : c
    );
    persistAndNotify(persistLayout, { cards: reflow(cards) }, setSaveState);
    setResizeId(null);
    setResizePreview(null);
  }, [resizeId, resizePreview, layout.cards, persistLayout]);

  if (loading || !data) {
    return (
      <div style={{ width: '100%', height: '100%', background: colors.bg, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        {!loading && error ? (
          // Initial load failed with nothing to show — an explicit, recoverable
          // dead-end instead of a spinner that never resolves.
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: space.sm, textAlign: 'center', fontFamily: font.body }}>
            <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.danger }}>Couldn't load the dashboard</div>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, maxWidth: 320, lineHeight: 1.5 }}>
              The daemon didn't respond. Check that it's running, then try again.
            </div>
            <Button
              colors={colors}
              type="button"
              onClick={retry}
              style={{
                '--pa-btn-bg': colors.cyanSoft,
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-border': colors.borderHi,
                '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-fg-hover': colors.cyan,
                '--pa-btn-border-hover': colors.cyan,
                '--pa-btn-bg-active': colors.cyanGlow,
                '--pa-btn-pad': '5px 14px',
                '--pa-btn-radius': `${radius.sm}px`,
                '--pa-btn-weight': 600,
                marginTop: space.sm, fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Try again
            </Button>
          </div>
        ) : (
          <Mobius size={120} state="thinking" />
        )}
      </div>
    );
  }

  const cardDataMap: Record<string, any> = {
    stats: { stats: data.stats },
    in_flight: inFlightProps,
    decisions: { activeCount },
    recent: { items: data.recent },
    todos: todosProps,
  };

  const untouched = !hasModel && chatRoute?.source === 'unconfigured' && !providersError && !failing
    && data.stats.sessions_total === 0 && data.stats.memory_count === 0
    && data.recent.length === 0 && data.in_flight.length === 0 && activeCount === 0 && dueTodos.todos.length === 0;
  const firstAction = untouched && layoutLoaded && !isEditMode;
  const visibleCards = firstAction && isDefaultHomeLayout(layout) ? [] : layout.cards.filter(c => c.visible);
  // Where each card is DRAWN. The stored x/y were a shelf packing that left
  // holes; this is a dense first-fit pack in the user's order, stretched into
  // any hole nothing else fits (display-only). Customize shows true sizes —
  // no stretch — so the resize grip edits what you see.
  const cells = layoutForDisplay(
    visibleCards.map(c => (resizeId === c.id && resizePreview ? { id: c.id, size: resizePreview } : c)),
    { minSpan: minSpanForWidth(gridWidth, GAP), fill: !isEditMode },
  );
  const canRemove = visibleCards.length > 1;
  const currentTypes = layout.cards.map(c => c.type);
  // Refreshed every render (not an effect) so `measureCard` — stable, never
  // re-created — always reads the CURRENT configured `h` for a card, even one
  // just resized in Customize, without needing `h` in its own dependencies.
  // An ambient manifest readout (layout 'compact': weather, system, status)
  // has no inner scroller to give a height cap anything to scroll, so capping
  // it at its configured `h` just slices the card — QA 2026-10-07: the
  // weather forecast row was cut off at h:2. Such a card is drawn at its full
  // content height; scrolling cards keep the cap.
  // 'decisions' (Needs You) is the same kind of card: a short stack of lines with
  // no scroller. With six lines its natural height passes the h:4 cap and the
  // clip cut off "Review →" (QA Q11), so it is drawn at its content's height.
  // 'calendar' draws its views at their own fixed heights (a month grid is
  // six rows tall) and scrolls the time grid itself, so an h cap would slice it.
  const growsToFit = (type: string) => registry[type]?.manifest?.layout === 'compact' || type === 'decisions' || type === 'calendar';
  visibleCards.forEach(c => configuredHeights.current.set(c.id, growsToFit(c.type) ? Math.max(c.size.h, 12) : c.size.h));

  return (
    <div
      ref={rootRef}
      style={{
        width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
        background: gradient.workspace,
      }}
    >
      {navigationNotice && <div role="status" style={{ padding: `${space.sm}px ${space.xl}px`, fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted }}>{navigationNotice}</div>}
      {/* Home had no title at all — the one view of the four that never named
          itself, and its controls were sticky INSIDE the scroller. Both now
          match every other view. */}
      <ViewHeader
        title="Home"
        // The agent's status, beside the name of the view it belongs to. It
        // used to live in the rail: a second line under the Home row when the
        // sidebar was open, and a loose green dot beside the icon when it was
        // closed — which marked a row without saying anything. Here it has
        // room for the name and the word, not just the colour.
        afterTitle={<HeaderAgentStatus data={data} error={error} />}
        // A failed poll keeps the last good payload, which is right — and
        // silent, which is not. Once the figures below stop being refreshed the
        // header says how old they are and offers the way back. Said in the
        // app's one staleness rendering, so it reads the same here as it does
        // on a three-month-old memory in the Brain.
        subtitle={failing ? (
          <AsOf
            data-testid="dashboard-freshness"
            asOf={lastOkAt}
            prefix="Updated"
            suffix="reconnecting"
            unknownLabel="Can't reach the daemon"
            // Failing is stale at any age: the figures stopped being confirmed
            // the moment the poll did.
            staleAfterMs={0}
            dot
          />
        ) : undefined}
        actions={<>
        {failing && (
          // No success tick: the caption above is the confirmation — it goes
          // away when the poll lands, and stays when it doesn't.
          <Button
            colors={colors}
            type="button"
            flashSuccess={false}
            onClick={() => refresh()}
          >
            Retry
          </Button>
        )}
        <SaveIndicator state={saveState} />
        <DashboardOverflowMenu items={[
          {
            label: 'Revert to previous layout',
            icon: <FiRotateCcw size={14} />,
            onClick: () => {
              setSaveState('saving');
              revertLayout()
                .then(() => { setSaveState('saved'); setTimeout(() => setSaveState('idle'), 1000); })
                .catch(() => { setSaveState('error'); setTimeout(() => setSaveState('idle'), 2000); });
            },
          },
          ...(isEditMode
            ? [{ label: 'Reset to default', icon: <FiRotateCcw size={14} />, onClick: () => setShowResetConfirm(true) }]
            : []),
        ]} />
        <Button
          colors={colors}
          type="button"
          onClick={() => setShowLayouts(true)}
          style={{
            '--pa-btn-bg': colors.surface,
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-border': colors.border,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-pad': '5px 14px',
            '--pa-btn-radius': `${radius.md}px`,
            fontFamily: font.body, fontSize: textSize.caption,
          } as CSSProperties}
        >
          <span style={{ display: 'flex', alignItems: 'center', gap: space.sm }}>
            <FiLayout size={14} /> Layouts
          </span>
        </Button>
        <CustomizeButton
          editing={isEditMode}
          onToggle={() => setIsEditMode(!isEditMode)}
          colors={colors}
        />
        </>}
      />

      {isEditMode && (
        // The resize/reorder affordances are only discoverable once you know
        // they exist — the corner grip is small and drag-to-move has no
        // chrome. This names both so "shape cards into columns or rows" is not
        // a hidden feature.
        <div style={{
          flexShrink: 0, padding: `${space.md}px ${space.huge}px`,
          background: colors.cyanSoft, borderBottom: `1px solid ${colors.border}`,
          fontFamily: font.body, fontSize: textSize.caption, color: colors.cyan,
          display: 'flex', alignItems: 'center', gap: space.md,
        }}>
          <FiEdit2 size={12} />
          <span>
            Drag a card to move it · drag the corner grip to resize — wider for a
            row, taller for a column. Changes save automatically.
          </span>
        </div>
      )}

      {/* 28/32/40 measured close to `huge` (24) but past its ceiling — the
          space scale caps at panel-scale padding, and this is exactly that:
          round to the ceiling rather than add a step for one caller. */}
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden', padding: `${space.huge}px` }}>

      {/* The banner slot (C8): ONE of these is on screen at a time, never both.
          Echo resurfaces a dormant Brain thread; Learn next coaches an untried
          capability. They are unrelated features that were rendering
          simultaneously in identical shells, which read as one banner split in
          two — `bannerSlot` now hands the slot to whichever has something to
          say, Learn next first. Both are hidden while arranging cards. */}
      {firstAction && <section aria-label="Get started" style={{
        padding: space.huge, marginBottom: space.xxxl, borderRadius: radius.lg,
        border: `1px solid ${colors.border}`, background: colors.surface, fontFamily: font.body,
      }}>
        <h2 style={{ margin: 0, fontFamily: font.display, fontSize: textSize.title, color: colors.text }}>Start with a model</h2>
        <p style={{ margin: `${space.xl}px 0`, fontSize: textSize.body, lineHeight: 1.5, color: colors.textMuted }}>
          Choose a hosted model or one on this Mac to start working with your agent.
        </p>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
          <Button colors={colors} variant="primary" flashSuccess={false} onClick={() => { void openChatModelSetup(); }}>Choose a model</Button>
          <Button colors={colors} variant="ghost" flashSuccess={false} onClick={() => { navigateToTool('world'); }}>Explore World</Button>
        </div>
        <p style={{ margin: `${space.xxl}px 0 0`, fontSize: textSize.small, color: colors.textMuted }}>
          No runs or memories yet. Your activity will appear here as you work.
        </p>
      </section>}
      {!isEditMode && <LearnNext modelConfigured={hasModel} />}
      {!isEditMode && <Echo />}

      {/* CSS Grid — reflows natively with the container, no JS width measurement */}
      <div
        ref={gridRef}
        onPointerMove={dragSrcId ? handlePointerMove : resizeId ? handleResizeMove : undefined}
        onPointerUp={dragSrcId ? handlePointerUp : resizeId ? handleResizeUp : undefined}
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(12, 1fr)',
          // Customize keeps the original fixed-row grid — the resize grip
          // drags in whole `h` units of exactly this height. Outside it, a
          // fine row unit plus dense auto-flow is the masonry technique
          // (coordinator, 2560px round 2): each card below sets only a row
          // SPAN (no explicit start), so the browser's own dense packing
          // decides where it lands, backfilling a shorter neighbour's leftover
          // tracks instead of leaving them reserved underneath it.
          // Focus mode leaves masonry: the focused card takes the full width
          // at its full height and the compact tiles a fixed one, so plain
          // content-sized rows are exact and need no measured spans.
          gridAutoRows: isEditMode ? ROW_HEIGHT : focusMode ? 'auto' : MASONRY_ROW_UNIT,
          gridAutoFlow: isEditMode ? 'row' : 'row dense',
          gap: GAP,
          touchAction: isEditMode ? 'none' : 'auto',
          userSelect: (dragSrcId || resizeId) ? 'none' : 'auto',
        }}
      >
        {visibleCards.map((card, index) => {
          const entry = registry[card.type];
          // A card in the layout with no registry entry used to `return null`:
          // it left a hole in the grid and no explanation. Two of the default
          // layout's own cards (Calendar, Council) are manifest-served, so this
          // fired every time the manifest fetch was slow or down — and Reset to
          // default put both of them back, where they rendered as nothing.
          const Component = entry?.component;
          // Manifest cards self-fetch and only need their manifest; first-party
          // code cards read pre-fetched data from cardDataMap.
          const props = entry?.manifest ? { manifest: entry.manifest } : (cardDataMap[card.type] || {});
          const { x, y, w, h } = cells[index];
          const isResizing = resizeId === card.id;
          const isDragging = dragSrcId === card.id;
          const isDragTarget = dragOverId === card.id && !isDragging;
          // The row span this card is actually drawn at, outside Customize:
          // whatever `measureCard` last settled on, or — before any
          // measurement has landed (first paint, or a card just added/resized)
          // — the same ceiling `spanForContent` would cap it at, so a card
          // never flashes at a single row before its first real measurement.
          const contentSpan = spans[card.id] ?? spanForContent(0, h, MASONRY_ROW_UNIT, GAP, ROW_HEIGHT);
          const presentation: CardPresentation = !focusMode ? 'overview' : focusedId === card.id ? 'focused' : 'compact';
          const name = entry?.name ?? card.type;
          const cellId = `home-card-${index}`;
          // Focus-mode geometry, drawn without explicit starts so the dense
          // flow packs the compact tiles under the focused card (CSS `order`
          // lifts it to the top without reordering the DOM or the layout).
          const focusGeometry: CSSProperties | null = presentation === 'focused'
            ? { gridColumn: '1 / -1', gridRow: 'auto', order: -1, maxHeight: undefined, display: 'flex', flexDirection: 'column' }
            : presentation === 'compact'
              ? { gridColumn: `span ${Math.max(COMPACT_TILE_SPAN, minSpanForWidth(gridWidth, GAP, 220))}`, gridRow: 'auto', height: COMPACT_TILE_PX, maxHeight: undefined }
              : null;
          return (
            <div
              key={card.id}
              id={cellId}
              data-testid={`dashboard-cell-${card.id}`}
              data-presentation={presentation}
              data-home-expandable={!isEditMode && presentation !== 'focused' && Component ? '' : undefined}
              className={`pa-home-cell${presentation === 'focused' && !supportsFocusMorph() ? ' pa-home-focus-enter' : ''}`}
              role={presentation === 'focused' ? 'region' : undefined}
              aria-label={presentation === 'focused' ? `${name}, expanded` : undefined}
              ref={getCellRef(card.id)}
              onPointerDown={isEditMode && !resizeId ? (e) => handlePointerDown(card.id, e) : undefined}
              style={{
                '--pa-home-vt-name': `pa-home-cell-${index}`,
                gridColumn: `${x + 1} / span ${w}`,
                // Customize: explicit start AND span, on the fixed ROW_HEIGHT
                // grid above — the resize grip drags `h` directly, so this
                // must be exactly what it edits. Outside Customize: span
                // ONLY, no explicit start, on the fine masonry grid above —
                // that is what lets `grid-auto-flow: dense` place this card
                // wherever its column range is free for `contentSpan` tracks,
                // backfilling a shorter neighbour's leftover space instead of
                // stacking strictly under whichever card came before it.
                gridRow: isEditMode ? `${y + 1} / span ${h}` : `span ${contentSpan}`,
                // Rows size to content, with the configured `h` as a MAX, not
                // a fixed height (coordinator, 2560px pass: In Flight/Needs
                // You/To-dos sat 50%+ empty; ambient status strips left a
                // void band under them). Outside Customize, every card is
                // top-aligned instead of stretched to the row's full track
                // height, and capped at the pixel height its own `h` would
                // have reserved — content shorter than that shrinks the box
                // (no more empty band); content at or past it hits the cap,
                // which gives the card's own CardScrollBody/overflow a
                // DEFINITE height to fill-then-scroll against, unchanged from
                // before. Customize keeps the true stretched box so the
                // resize grip still edits what you see (layoutForDisplay's
                // `fill: false` does the matching thing for width). This box
                // cap is independent of (and unchanged by) the row-TRACK fix
                // above: that fix is what stops the now-shrunk box from
                // leaving a reserved-but-empty band in the grid underneath it.
                alignSelf: isEditMode ? undefined : 'start',
                maxHeight: isEditMode || growsToFit(card.type) ? undefined : h * ROW_HEIGHT + (h - 1) * GAP,
                cursor: isEditMode ? (isDragging ? 'grabbing' : 'grab') : 'default',
                borderRadius: radius.lg,
                outline: isDragTarget
                  ? `2px solid ${colors.cyan}`
                  : isResizing ? `2px solid ${colors.cyan}`
                  : isEditMode ? `1px solid ${colors.cyanSoft}` : 'none',
                outlineOffset: -1,
                // Lift affordance: picking a card up raises it off the grid
                // (elevationFloating — the same step a toast sits at) with a
                // hair of scale; dropping settles it back with a softer
                // spring. `reduceMotion` drops the scale outright rather than
                // just shortening it — a card that visibly grows and shrinks
                // is exactly the motion that setting asks to lose.
                boxShadow: isDragging
                  ? colors.elevationFloating
                  : isEditMode ? `0 0 ${space.xl}px ${colors.cyanGlow}` : 'none',
                // 'none' at rest, never 'scale(1)': any transform makes the card
                // the containing block for fixed descendants, so a modal opened
                // inside it would be trapped in the card (DetailModal also portals).
                transform: !reduceMotion && isDragging ? 'scale(1.02)' : 'none',
                opacity: isDragging ? 0.5 : 1,
                transition: reduceMotion || isResizing
                  ? 'none'
                  : isDragging
                    ? `outline ${duration.fast}ms ${ease.out}, box-shadow ${duration.snappy}ms ${ease.snappy}, transform ${duration.snappy}ms ${ease.snappy}, opacity ${duration.fast}ms ${ease.out}`
                    : `outline ${duration.fast}ms ${ease.out}, box-shadow ${duration.smooth}ms ${ease.smooth}, transform ${duration.smooth}ms ${ease.smooth}, opacity ${duration.fast}ms ${ease.out}`,
                position: 'relative',
                minHeight: 0,
                overflow: 'hidden',
                // Outside Customize the cell is capped by `maxHeight`, not
                // sized, so a card's `height: 100%` has nothing to resolve
                // against and a long card (the Council report) painted at its
                // full content height and was clipped mid-line by this cell
                // with no way to scroll. A flex column lets the card shrink to
                // the cap, which gives its own scroll body a real height.
                ...(isEditMode ? null : { display: 'flex', flexDirection: 'column' }),
                userSelect: isEditMode ? 'none' : 'auto',
                ...focusGeometry,
              } as CSSProperties}
            >
              {presentation === 'focused' && (
                <FocusBar name={name} cellId={cellId} onCollapse={collapse} />
              )}
              <CardPresentationProvider value={{ presentation, expand: () => expand(card.id), collapse }}>
                {/* Keep the card mounted as its presentation changes. In
                    overview/compact, contents preserves its original box and
                    height:100% sizing against the cell. ResizeObserver
                    measures the real outer cell, never this boxless wrapper.
                    Focus needs a flex body below the focus bar. */}
                <div data-home-card-body="" style={presentation === 'focused'
                  ? { flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }
                  : { display: 'contents' }}>
                  {Component
                    ? <Component {...props} />
                    : <MissingCard type={card.type} status={registryStatus} />}
                </div>
              </CardPresentationProvider>
              {!isEditMode && presentation !== 'focused' && Component && (
                <ExpandCardButton name={name} cellId={cellId} onExpand={() => expand(card.id)} />
              )}
              {isEditMode && (
                <RemoveButton
                  disabled={!canRemove}
                  onClick={() => removeCard(card.id)}
                />
              )}
              {isEditMode && (
                <ResizeHandle
                  onPointerDown={(e) => handleResizeDown(card.id, e)}
                />
              )}
            </div>
          );
        })}
      </div>

      {/* Drag ghost — follows pointer during reorder */}
      {dragSrcId && dragPos && (() => {
        const srcCard = visibleCards.find(c => c.id === dragSrcId);
        const entry = srcCard ? registry[srcCard.type] : null;
        if (!entry) return null;
        const gh = dragGhostSize.current;
        return (
          <div style={{
            position: 'fixed',
            left: dragPos.x - gh.w / 2,
            top: dragPos.y - 20,
            width: gh.w,
            height: Math.min(gh.h, 80),
            background: colors.surface,
            border: `1px solid ${colors.cyan}`,
            borderRadius: radius.lg,
            opacity: 0.85,
            pointerEvents: 'none',
            zIndex: 100,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600,
            color: colors.cyan,
            boxShadow: colors.elevationOverlay,
          }}>
            {entry.name}
          </div>
        );
      })()}

      {/* Add card placeholder — edit mode only */}
      {isEditMode && (
        <div
          {...pressable(() => setShowPicker(true))}
          style={{
            marginTop: space.xxl,
            height: 80, borderRadius: radius.lg,
            border: `1px dashed ${colors.border}`,
            background: colors.cyanSoft,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            gap: space.md, cursor: 'pointer',
            transition: reduceMotion ? 'none' : `border-color ${duration.fast}ms ${ease.out}, background ${duration.fast}ms ${ease.out}`,
          }}
          onMouseEnter={e => {
            e.currentTarget.style.borderColor = colors.cyan;
            e.currentTarget.style.background = colors.cyanSoft;
          }}
          onMouseLeave={e => {
            e.currentTarget.style.borderColor = colors.border;
            e.currentTarget.style.background = colors.cyanSoft;
          }}
        >
          <FiPlus size={16} style={{ color: colors.textMuted }} />
          <span style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted }}>
            Add card
          </span>
        </div>
      )}

      </div>

      {/* Overlays sit OUTSIDE the scroller so they are not clipped by it. */}
      {showPicker && (
        <AddCardPicker
          registry={registry}
          currentCardTypes={currentTypes}
          onSelect={addCard}
          onClose={() => setShowPicker(false)}
        />
      )}

      {showResetConfirm && (
        /* Losing a hand-arranged dashboard has no undo, which is the tier that
           earns a modal. It had its own chrome — and with it no focus trap, no
           dialog role, and no focus returned to the button that opened it. */
        <ConfirmDialog
          title="Reset dashboard?"
          consequence="This restores the default layout. The arrangement you built — which cards are on the board, and where — is lost."
          confirmLabel="Reset"
          failureLabel="Couldn't reset the dashboard"
          onConfirm={resetToDefault}
          onCancel={() => setShowResetConfirm(false)}
        />
      )}

      {showLayouts && (
        <LayoutsGallery
          currentLayout={layout}
          onApply={async (next) => { await persistLayout(next); }}
          onClose={() => setShowLayouts(false)}
        />
      )}

      {/* The agent's preview_dashboard_layout tool proposed a layout — shown
          as the same overlay the gallery's own "Try" uses (R7), regardless of
          whether Layouts is open. Never applies on its own (P9). */}
      {!showLayouts && agentPreview && (
        <LayoutPreviewOverlay
          current={layout}
          proposed={agentPreview.layout}
          reason={agentPreview.reason}
          comingSoon={agentPreviewTemplate ? comingSoonCards(agentPreviewTemplate) : []}
          onApply={async () => { await persistLayout(agentPreview.layout); clearDashboardLayoutPreview(); }}
          onKeepCurrent={() => clearDashboardLayoutPreview()}
        />
      )}
    </div>
  );
}

function RemoveButton({ disabled, onClick }: { disabled: boolean; onClick: () => void }) {
  const { colors } = useTheme();
  return (
    // `disabled` is deliberately NOT passed to the primitive: this control has
    // always stayed enabled and explained itself through its title, and the
    // guard lives in the click handler. What the pair of mouse handlers used to
    // do by hand — redden on hover, and only when removable — is now the
    // primitive's `--pa-btn-*-hover` pair, held at the resting values when the
    // last card can't be removed.
    <Tooltip content={disabled ? 'Dashboard needs at least one card' : 'Remove this card'}>
      <Button
        colors={colors}
        variant="bare"
        type="button"
        onClick={e => { e.stopPropagation(); if (!disabled) onClick(); }}
        aria-label="Remove this card"
        style={{
          '--pa-btn-bg': disabled ? colors.cyanSoft : colors.surface,
          '--pa-btn-fg': disabled ? colors.textDim : colors.textMuted,
          '--pa-btn-border': 'transparent',
          '--pa-btn-bg-hover': disabled ? colors.cyanSoft : colors.danger + '26',
          '--pa-btn-fg-hover': disabled ? colors.textDim : colors.danger,
          '--pa-btn-bg-active': disabled ? colors.cyanSoft : colors.danger + '26',
          '--pa-btn-pad': '0',
          '--pa-btn-radius': '50%',
          position: 'absolute', top: space.md, right: space.md, zIndex: 5,
          width: 24, height: 24,
          cursor: disabled ? 'not-allowed' : 'pointer',
        } as CSSProperties}
      >
        <FiX size={14} />
      </Button>
    </Tooltip>
  );
}

function ResizeHandle({ onPointerDown }: { onPointerDown: (e: React.PointerEvent) => void }) {
  const { colors, reduceMotion } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <Tooltip content="Drag to resize — wider for a row, taller for a column">
      <span tabIndex={0} style={{ outline: 'none' }}>
        <div
          onPointerDown={onPointerDown}
          onMouseEnter={() => setHover(true)}
          onMouseLeave={() => setHover(false)}
          style={{
            position: 'absolute', bottom: space.xs, right: space.xs, width: 22, height: 22,
            cursor: 'nwse-resize', zIndex: 5,
            // Concentric to the card's own radius.lg corner, 4px in — the one
            // clean corner-offset relationship in this file (D4).
            borderRadius: concentric(radius.lg, 4),
            background: hover ? colors.cyanGlow : colors.cyanSoft,
            border: `1px solid ${colors.cyan}`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            transition: reduceMotion ? 'none' : `background ${duration.fast}ms ${ease.out}`,
          }}
        >
          {/* Was a hand-drawn <svg> (two diagonal strokes). Feather has no
              resize-handle glyph, but a corner arrow is the conventional
              substitute and IS a glyph Feather has a name for — the
              one-icon-system gate's own test: "ask whether Feather would have
              a word for it; if it would, use Feather's." */}
          <FiCornerRightDown size={12} color={colors.cyan} />
        </div>
      </span>
    </Tooltip>
  );
}

function SaveIndicator({ state }: { state: SaveState }) {
  const { colors, reduceMotion } = useTheme();
  if (state === 'idle') return null;
  const config: Record<Exclude<SaveState, 'idle'>, { label: string; color: string }> = {
    saving: { label: 'Saving...', color: colors.textMuted },
    saved: { label: 'Saved', color: colors.success },
    error: { label: 'Save failed', color: colors.danger },
  };
  const c = config[state];
  return (
    <span style={{
      fontFamily: font.body, fontSize: textSize.micro, fontWeight: 500,
      color: c.color,
      transition: reduceMotion ? 'none' : `opacity ${duration.base}ms ${ease.out}`,
    }}>
      {c.label}
    </span>
  );
}
