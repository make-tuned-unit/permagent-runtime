import { FiEdit2, FiCheck, FiX, FiPlus, FiRotateCcw } from 'react-icons/fi';

import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from './useDashboard';
import { useLiveGoals } from '../../lib/useLiveGoals';
import { useLayout, DEFAULT_LAYOUT, type DashboardLayoutData, type DashboardCardLayout } from './useLayout';
import { CARD_REGISTRY } from './cards/registry';
import { AddCardPicker } from './AddCardPicker';
import { DashboardOverflowMenu } from './DashboardOverflowMenu';
import { ResetConfirmModal } from './ResetConfirmModal';
import { Echo } from './Echo';
import { useState, useCallback, useRef, useMemo } from 'react';

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

/** Recompute grid positions from array order, packing cards into rows. */
function reflow(cards: DashboardCardLayout[]): DashboardCardLayout[] {
  let x = 0;
  let y = 0;
  let rowHeight = 0;
  return cards.map(card => {
    const w = card.size.w;
    const h = card.size.h;
    if (x + w > 12) {
      x = 0;
      y += rowHeight;
      rowHeight = 0;
    }
    const placed = { ...card, position: { x, y } };
    x += w;
    rowHeight = Math.max(rowHeight, h);
    return placed;
  });
}

const ROW_HEIGHT = 60;
const GAP = 16;

export function Dashboard() {
  const { gradient, colors } = useTheme();
  const { data, loading, error, retry } = useDashboard();
  // One shared live-goal subscription for every "in flight" surface (count,
  // list, header, status) so they always agree. Sessions are a separate stat.
  const { goals: activeGoals, activeCount } = useLiveGoals();
  // Stable props identity for the memoized InFlightCard — only changes when the
  // (deduped) goal list actually changes, so a benign refetch never re-renders it.
  const inFlightProps = useMemo(() => ({ goals: activeGoals }), [activeGoals]);
  const { layout, persistLayout } = useLayout();
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
    const entry = CARD_REGISTRY[type];
    if (!entry) return;
    const newCard: DashboardCardLayout = {
      id: type,
      type,
      position: { x: 0, y: 0 },
      size: { w: entry.defaultSize.w, h: entry.defaultSize.h },
      visible: true,
    };
    persistAndNotify(persistLayout, { cards: reflow([...layout.cards, newCard]) }, setSaveState);
  }, [layout.cards, persistLayout]);

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
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, textAlign: 'center', fontFamily: font.body }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: colors.danger }}>Couldn't load the dashboard</div>
            <div style={{ fontSize: 11, color: colors.textDim, maxWidth: 320, lineHeight: 1.5 }}>
              The daemon didn't respond. Check that it's running, then try again.
            </div>
            <button
              onClick={retry}
              style={{
                marginTop: 6, padding: '5px 14px', borderRadius: 6, cursor: 'pointer',
                background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
                color: colors.cyan, fontSize: 11, fontWeight: 600, fontFamily: font.body,
              }}
            >
              Try again
            </button>
          </div>
        ) : (
          <Mobius size={120} state="thinking" />
        )}
      </div>
    );
  }

  const cardDataMap: Record<string, any> = {
    hero: { agent: data.agent, activeCount },
    stats: { stats: data.stats },
    in_flight: inFlightProps,
    decisions: { activeCount },
    recent: { items: data.recent },
  };

  const visibleCards = layout.cards.filter(c => c.visible);
  const canRemove = visibleCards.length > 1;
  const currentTypes = layout.cards.map(c => c.type);

  return (
    <div
      style={{
        width: '100%', height: '100%', overflowY: 'auto', overflowX: 'hidden',
        background: gradient.workspace,
        padding: '28px 32px 40px',
      }}
    >
      {/* Edit mode toolbar */}
      <div style={{
        position: 'sticky', top: 0, zIndex: 10,
        display: 'flex', justifyContent: 'flex-end', alignItems: 'center',
        gap: 10, marginBottom: 16, height: 32,
      }}>
        <SaveIndicator state={saveState} />
        {isEditMode && (
          <DashboardOverflowMenu items={[
            { label: 'Reset to default', icon: <FiRotateCcw size={14} />, onClick: () => setShowResetConfirm(true) },
          ]} />
        )}
        <button
          onClick={() => setIsEditMode(!isEditMode)}
          title={isEditMode ? 'Done editing' : 'Customize dashboard'}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: isEditMode ? '5px 14px' : '5px 8px',
            borderRadius: radius.md,
            border: `1px solid ${isEditMode ? colors.cyan : colors.border}`,
            background: isEditMode ? colors.cyanSoft : colors.surface,
            color: isEditMode ? colors.cyan : colors.textMuted,
            fontFamily: font.body, fontSize: 12, fontWeight: 500,
            cursor: 'pointer', transition: 'all 150ms ease',
          }}
        >
          {isEditMode ? <><FiCheck size={14} /> Done</> : <FiEdit2 size={14} />}
        </button>
      </div>

      {/* Echo — a forgotten thread from your Brain, resurfaced unprompted.
          Renders itself only when there's a genuinely dormant thread and it
          hasn't shown recently; otherwise nothing. Hidden while arranging cards. */}
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
          const entry = CARD_REGISTRY[card.type];
          if (!entry) return null;
          const Component = entry.component;
          const props = cardDataMap[card.type] || {};
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
                boxShadow: isEditMode ? `0 0 12px ${colors.cyanGlow}` : 'none',
                opacity: isDragging ? 0.5 : 1,
                transition: isResizing ? 'none' : 'outline 150ms ease, box-shadow 200ms ease, opacity 150ms ease',
                position: 'relative',
                minHeight: 0,
                overflow: 'hidden',
                userSelect: isEditMode ? 'none' : 'auto',
              }}
            >
              <Component {...props} />
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
        const entry = srcCard ? CARD_REGISTRY[srcCard.type] : null;
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
            fontFamily: font.body, fontSize: 12, fontWeight: 600,
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
            gap: 8, cursor: 'pointer',
            transition: 'border-color 150ms ease, background 150ms ease',
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
          <span style={{ fontFamily: font.body, fontSize: 13, color: colors.textMuted }}>
            Add card
          </span>
        </div>
      )}

      {showPicker && (
        <AddCardPicker
          currentCardTypes={currentTypes}
          onSelect={addCard}
          onClose={() => setShowPicker(false)}
        />
      )}

      {showResetConfirm && (
        <ResetConfirmModal
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
    <button
      onClick={e => { e.stopPropagation(); if (!disabled) onClick(); }}
      title={disabled ? 'Dashboard needs at least one card' : 'Remove this card'}
      style={{
        position: 'absolute', top: 8, right: 8, zIndex: 5,
        width: 24, height: 24, borderRadius: '50%',
        border: 'none',
        background: disabled ? colors.cyanSoft : colors.surface,
        color: disabled ? colors.textDim : colors.textMuted,
        cursor: disabled ? 'not-allowed' : 'pointer',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        transition: 'background 100ms ease, color 100ms ease',
      }}
      onMouseEnter={e => {
        if (!disabled) {
          e.currentTarget.style.background = colors.danger + '26';
          e.currentTarget.style.color = colors.danger;
        }
      }}
      onMouseLeave={e => {
        e.currentTarget.style.background = disabled ? colors.cyanSoft : colors.surface;
        e.currentTarget.style.color = disabled ? colors.textDim : colors.textMuted;
      }}
    >
      <FiX size={14} />
    </button>
  );
}

function ResizeHandle({ onPointerDown }: { onPointerDown: (e: React.PointerEvent) => void }) {
  const { colors } = useTheme();
  return (
    <div
      onPointerDown={onPointerDown}
      style={{
        position: 'absolute', bottom: 0, right: 0, width: 20, height: 20,
        cursor: 'nwse-resize', zIndex: 5,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        <path d="M9 1L1 9M9 5L5 9M9 9L9 9" stroke={colors.textDim} strokeWidth={1.5} strokeLinecap="round" />
      </svg>
    </div>
  );
}

function SaveIndicator({ state }: { state: SaveState }) {
  const { colors } = useTheme();
  if (state === 'idle') return null;
  const config: Record<Exclude<SaveState, 'idle'>, { label: string; color: string }> = {
    saving: { label: 'Saving...', color: colors.textMuted },
    saved: { label: 'Saved', color: colors.success },
    error: { label: 'Save failed', color: colors.danger },
  };
  const c = config[state];
  return (
    <span style={{
      fontFamily: font.body, fontSize: 11, fontWeight: 500,
      color: c.color, transition: 'opacity 200ms ease',
    }}>
      {c.label}
    </span>
  );
}
