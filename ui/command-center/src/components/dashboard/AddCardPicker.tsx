import { FiPlus, FiX } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { CARD_REGISTRY } from './cards/registry';

interface Props {
  currentCardTypes: string[];
  onSelect: (type: string) => void;
  onClose: () => void;
}

export function AddCardPicker({ currentCardTypes, onSelect, onClose }: Props) {
  const { colors } = useTheme();
  const available = Object.entries(CARD_REGISTRY).filter(
    ([type]) => !currentCardTypes.includes(type)
  );

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 100,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 340, maxHeight: 400,
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: colors.cardShadow,
          overflow: 'hidden',
          display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '14px 18px',
          borderBottom: `1px solid ${colors.border}`,
        }}>
          <span style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text }}>
            Add card
          </span>
          <button
            onClick={onClose}
            style={{
              background: 'none', border: 'none', color: colors.textMuted,
              cursor: 'pointer', padding: 4, display: 'flex',
            }}
          >
            <FiX size={16} />
          </button>
        </div>

        {/* List */}
        <div style={{ padding: '8px 0', overflow: 'auto', flex: 1 }}>
          {available.length === 0 ? (
            <div style={{
              padding: '24px 18px', textAlign: 'center',
              fontSize: 13, color: colors.textMuted,
            }}>
              All card types are already added
            </div>
          ) : (
            available.map(([type, entry]) => (
              <button
                key={type}
                onClick={() => { onSelect(type); onClose(); }}
                style={{
                  display: 'flex', alignItems: 'center', gap: 12,
                  width: '100%', padding: '10px 18px',
                  background: 'none', border: 'none',
                  cursor: 'pointer', textAlign: 'left',
                  transition: 'background 100ms ease',
                }}
                onMouseEnter={e => (e.currentTarget.style.background = colors.cyanSoft)}
                onMouseLeave={e => (e.currentTarget.style.background = 'none')}
              >
                <div style={{
                  width: 28, height: 28, borderRadius: radius.sm,
                  background: colors.cyanSoft,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  flexShrink: 0,
                }}>
                  <FiPlus size={14} style={{ color: colors.cyan }} />
                </div>
                <div>
                  <div style={{ fontFamily: font.body, fontSize: 13, fontWeight: 500, color: colors.text }}>
                    {entry.name}
                  </div>
                  <div style={{ fontFamily: font.body, fontSize: 11, color: colors.textDim, marginTop: 1 }}>
                    {entry.description}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
