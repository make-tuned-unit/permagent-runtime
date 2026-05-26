import { GridLayout, noCompactor, verticalCompactor, type Layout } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
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
import { useRef, useState, useEffect, useCallback } from 'react';

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

export function Dashboard() {
  const { gradient, colors } = useTheme();
  const { data, loading } = useDashboard();
  const { layout, persistLayout } = useLayout();
  const containerRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(1200);
  const [isEditMode, setIsEditMode] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const [showPicker, setShowPicker] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);

  useEffect(() => {
    if (!containerRef.current) return;
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) setWidth(entry.contentRect.width);
    });
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, []);

  const handleLayoutChange = useCallback((newGridLayout: Layout) => {
    if (!isEditMode) return;
    const compacted = verticalCompactor.compact(newGridLayout, 12);
    const updatedCards: DashboardCardLayout[] = layout.cards.map(card => {
      const item = compacted.find((l: any) => l.i === card.id);
      if (!item) return card;
      return { ...card, position: { x: item.x, y: item.y }, size: { w: item.w, h: item.h } };
    });
    persistAndNotify(persistLayout, { cards: updatedCards }, setSaveState);
  }, [isEditMode, layout.cards, persistLayout]);

  const removeCard = useCallback((id: string) => {
    const remaining = layout.cards.filter(c => c.id !== id);
    if (remaining.length === 0) return;
    persistAndNotify(persistLayout, { cards: remaining }, setSaveState);
  }, [layout.cards, persistLayout]);

  const addCard = useCallback((type: string) => {
    const entry = CARD_REGISTRY[type];
    if (!entry) return;
    const maxY = layout.cards.reduce((max, c) => Math.max(max, c.position.y + c.size.h), 0);
    const newCard: DashboardCardLayout = {
      id: type,
      type,
      position: { x: 0, y: maxY },
      size: { w: entry.defaultSize.w, h: entry.defaultSize.h },
      visible: true,
    };
    persistAndNotify(persistLayout, { cards: [...layout.cards, newCard] }, setSaveState);
  }, [layout.cards, persistLayout]);

  const resetToDefault = useCallback(() => {
    setShowResetConfirm(false);
    persistAndNotify(persistLayout, DEFAULT_LAYOUT, setSaveState);
  }, [persistLayout]);

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
  const gridLayout = visibleCards.map(card => ({
    i: card.id, x: card.position.x, y: card.position.y, w: card.size.w, h: card.size.h,
  }));
  const canRemove = visibleCards.length > 1;
  const currentTypes = layout.cards.map(c => c.type);

  return (
    <div
      ref={containerRef}
      style={{
        width: '100%', height: '100%', overflowY: 'auto',
        background: gradient.workspace,
        padding: '28px 32px 40px',
        position: 'relative',
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

      <GridLayout
        layout={gridLayout}
        width={width - 64}
        gridConfig={{
          cols: 12, rowHeight: 60,
          margin: [16, 16] as const,
          containerPadding: [0, 0] as const,
        }}
        dragConfig={{ enabled: isEditMode, bounded: false, threshold: 3 }}
        resizeConfig={{ enabled: isEditMode, handles: ['se'] }}
        compactor={noCompactor}
        onDragStop={handleLayoutChange}
        onResizeStop={handleLayoutChange}
      >
        {visibleCards.map(card => {
          const entry = CARD_REGISTRY[card.type];
          if (!entry) return <div key={card.id} />;
          const Component = entry.component;
          const props = cardDataMap[card.type] || {};
          return (
            <div
              key={card.id}
              style={{
                cursor: isEditMode ? 'move' : 'default',
                borderRadius: radius.lg,
                outline: isEditMode ? `1px solid ${colors.cyanSoft}` : 'none',
                outlineOffset: -1,
                boxShadow: isEditMode ? `0 0 12px ${colors.cyanGlow}` : 'none',
                transition: 'outline 200ms ease, box-shadow 200ms ease',
                position: 'relative',
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
      </GridLayout>

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
