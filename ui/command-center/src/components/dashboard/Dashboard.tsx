import { FiEdit2, FiX, FiPlus, FiRotateCcw, FiCornerRightDown } from 'react-icons/fi';

import { concentric, duration, ease, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from './useDashboard';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { useDueTodos } from '../../lib/useDueTodos';
import { useLayout, reflow, DEFAULT_LAYOUT, type DashboardLayoutData, type DashboardCardLayout } from './useLayout';
import { useCardRegistry } from './cards/useCardRegistry';
import { AddCardPicker } from './AddCardPicker';
import { DashboardOverflowMenu } from './DashboardOverflowMenu';
import { CustomizeButton } from './CustomizeButton';
import { MissingCard } from './MissingCard';
import { Echo } from './Echo';
import { LearnNext } from './LearnNext';
import { ViewHeader } from '../common/ViewHeader';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { Button } from '../common/Button';
import { AsOf } from '../common/AsOf';
import { useState, useCallback, useRef, useMemo, type CSSProperties } from 'react';

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

export function Dashboard() {
  const { gradient, colors, reduceMotion } = useTheme();
  const { data, loading, error, lastOkAt, failing, retry, refresh } = useDashboard();
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
  const { layout, persistLayout } = useLayout();
  // Rendered registry = first-party code cards + daemon-served manifest cards.
  const { registry, status: registryStatus } = useCardRegistry();
  const [isEditMode, setIsEditMode] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [showPicker, setShowPicker] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [dragSrcId, setDragSrcId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [dragPos, setDragPos] = useState<{ x: number; y: number } | null>(null);
  const dragGhostSize = useRef<{ w: number; h: number }>({ w: 0, h: 0 });
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const gridRef = useRef<HTMLDivElement>(null);
  // Resize state
  const [resizeId, setResizeId] = useState<string | null>(null);
  const resizeStart = useRef<{ pointerId: number; startX: number; startY: number; origW: number; origH: number; colPx: number }>({ pointerId: 0, startX: 0, startY: 0, origW: 0, origH: 0, colPx: 0 });
  const [resizePreview, setResizePreview] = useState<{ w: number; h: number } | null>(null);

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
                marginTop: 6, fontSize: textSize.micro, fontFamily: font.body,
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

  const visibleCards = layout.cards.filter(c => c.visible);
  const canRemove = visibleCards.length > 1;
  const currentTypes = layout.cards.map(c => c.type);

  return (
    <div
      style={{
        width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
        background: gradient.workspace,
      }}
    >
      {/* Home had no title at all — the one view of the four that never named
          itself, and its controls were sticky INSIDE the scroller. Both now
          match every other view. */}
      <ViewHeader
        title="Home"
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
        {isEditMode && (
          <DashboardOverflowMenu items={[
            { label: 'Reset to default', icon: <FiRotateCcw size={14} />, onClick: () => setShowResetConfirm(true) },
          ]} />
        )}
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
      {!isEditMode && <LearnNext />}
      {!isEditMode && <Echo />}

      {/* CSS Grid — reflows natively with the container, no JS width measurement */}
      <div
        ref={gridRef}
        onPointerMove={dragSrcId ? handlePointerMove : resizeId ? handleResizeMove : undefined}
        onPointerUp={dragSrcId ? handlePointerUp : resizeId ? handleResizeUp : undefined}
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(12, 1fr)',
          gridAutoRows: ROW_HEIGHT,
          gap: GAP,
          touchAction: isEditMode ? 'none' : 'auto',
          userSelect: (dragSrcId || resizeId) ? 'none' : 'auto',
        }}
      >
        {visibleCards.map(card => {
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
          const { x, y } = card.position;
          const isResizing = resizeId === card.id;
          const w = isResizing && resizePreview ? resizePreview.w : card.size.w;
          const h = isResizing && resizePreview ? resizePreview.h : card.size.h;
          const isDragging = dragSrcId === card.id;
          const isDragTarget = dragOverId === card.id && !isDragging;
          return (
            <div
              key={card.id}
              ref={el => { if (el) cardRefs.current.set(card.id, el); else cardRefs.current.delete(card.id); }}
              onPointerDown={isEditMode && !resizeId ? (e) => handlePointerDown(card.id, e) : undefined}
              style={{
                gridColumn: `${x + 1} / span ${w}`,
                gridRow: `${y + 1} / span ${h}`,
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
                transform: !reduceMotion && isDragging ? 'scale(1.02)' : 'scale(1)',
                opacity: isDragging ? 0.5 : 1,
                transition: reduceMotion || isResizing
                  ? 'none'
                  : isDragging
                    ? `outline ${duration.fast}ms ${ease.out}, box-shadow ${duration.snappy}ms ${ease.snappy}, transform ${duration.snappy}ms ${ease.snappy}, opacity ${duration.fast}ms ${ease.out}`
                    : `outline ${duration.fast}ms ${ease.out}, box-shadow ${duration.smooth}ms ${ease.smooth}, transform ${duration.smooth}ms ${ease.smooth}, opacity ${duration.fast}ms ${ease.out}`,
                position: 'relative',
                minHeight: 0,
                overflow: 'hidden',
                userSelect: isEditMode ? 'none' : 'auto',
              }}
            >
              {Component
                ? <Component {...props} />
                : <MissingCard type={card.type} status={registryStatus} />}
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
          onClick={() => setShowPicker(true)}
          style={{
            marginTop: 16,
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
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={e => { e.stopPropagation(); if (!disabled) onClick(); }}
      title={disabled ? 'Dashboard needs at least one card' : 'Remove this card'}
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
        position: 'absolute', top: 8, right: 8, zIndex: 5,
        width: 24, height: 24,
        cursor: disabled ? 'not-allowed' : 'pointer',
      } as CSSProperties}
    >
      <FiX size={14} />
    </Button>
  );
}

function ResizeHandle({ onPointerDown }: { onPointerDown: (e: React.PointerEvent) => void }) {
  const { colors, reduceMotion } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <div
      onPointerDown={onPointerDown}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title="Drag to resize — wider for a row, taller for a column"
      style={{
        position: 'absolute', bottom: 4, right: 4, width: 22, height: 22,
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
