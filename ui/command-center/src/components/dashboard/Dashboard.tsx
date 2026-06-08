import { FiEdit2, FiCheck, FiX, FiPlus, FiRotateCcw } from 'react-icons/fi';

import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { useDashboard } from './useDashboard';
import { useLayout, DEFAULT_LAYOUT, type DashboardLayoutData, type DashboardCardLayout } from './useLayout';
import { CARD_REGISTRY } from './cards/registry';
import { AddCardPicker } from './AddCardPicker';
import { DashboardOverflowMenu } from './DashboardOverflowMenu';
import { ResetConfirmModal } from './ResetConfirmModal';
import { useState, useCallback, useRef } from 'react';

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
  const { data, loading } = useDashboard();
  const { layout, persistLayout } = useLayout();
  const [isEditMode, setIsEditMode] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [showPicker, setShowPicker] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [dragSrcId, setDragSrcId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());

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
    // Ignore clicks on the remove button
    if ((e.target as HTMLElement).closest('button')) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDragSrcId(cardId);
  }, [isEditMode]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragSrcId) return;
    const over = findCardAtPoint(e.clientX, e.clientY);
    setDragOverId(over && over !== dragSrcId ? over : null);
  }, [dragSrcId, findCardAtPoint]);

  const handlePointerUp = useCallback(() => {
    if (dragSrcId && dragOverId) {
      reorderCards(dragSrcId, dragOverId);
    }
    setDragSrcId(null);
    setDragOverId(null);
  }, [dragSrcId, dragOverId, reorderCards]);

  if (loading || !data) {
    return (
      <div style={{ width: '100%', height: '100%', background: colors.bg, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <Mobius size={120} state="thinking" />
      </div>
    );
  }

  const cardDataMap: Record<string, any> = {
    hero: { agent: data.agent },
    stats: { stats: data.stats },
    in_flight: { tasks: data.in_flight },
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

      {/* CSS Grid — reflows natively with the container, no JS width measurement */}
      <div
        onPointerMove={dragSrcId ? handlePointerMove : undefined}
        onPointerUp={dragSrcId ? handlePointerUp : undefined}
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(12, 1fr)',
          gridAutoRows: ROW_HEIGHT,
          gap: GAP,
          touchAction: isEditMode ? 'none' : 'auto',
        }}
      >
        {visibleCards.map(card => {
          const entry = CARD_REGISTRY[card.type];
          if (!entry) return null;
          const Component = entry.component;
          const props = cardDataMap[card.type] || {};
          const { x, y } = card.position;
          const { w, h } = card.size;
          const isDragging = dragSrcId === card.id;
          const isDragTarget = dragOverId === card.id && !isDragging;
          return (
            <div
              key={card.id}
              ref={el => { if (el) cardRefs.current.set(card.id, el); else cardRefs.current.delete(card.id); }}
              onPointerDown={isEditMode ? (e) => handlePointerDown(card.id, e) : undefined}
              style={{
                gridColumn: `${x + 1} / span ${w}`,
                gridRow: `${y + 1} / span ${h}`,
                cursor: isEditMode ? (isDragging ? 'grabbing' : 'grab') : 'default',
                borderRadius: radius.lg,
                outline: isDragTarget
                  ? `2px solid ${colors.cyan}`
                  : isEditMode ? `1px solid ${colors.cyanSoft}` : 'none',
                outlineOffset: -1,
                boxShadow: isEditMode ? `0 0 12px ${colors.cyanGlow}` : 'none',
                opacity: isDragging ? 0.5 : 1,
                transition: 'outline 150ms ease, box-shadow 200ms ease, opacity 150ms ease',
                position: 'relative',
                minHeight: 0,
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
            </div>
          );
        })}
      </div>

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
          e.currentTarget.style.background = 'rgba(239,68,68,0.15)';
          e.currentTarget.style.color = '#EF4444';
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
