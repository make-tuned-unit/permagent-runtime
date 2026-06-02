import { useEffect } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  onConfirm: () => void;
  onCancel: () => void;
}

export function ResetConfirmModal({ onConfirm, onCancel }: Props) {
  const { colors } = useTheme();

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel();
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [onCancel]);

  return (
    <div
      onClick={onCancel}
      style={{
        position: 'fixed', inset: 0, zIndex: 100,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 360, borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: colors.cardShadow,
          padding: '24px',
        }}
      >
        <div style={{
          fontFamily: font.display, fontSize: 16, fontWeight: 600,
          color: colors.text, marginBottom: 10,
        }}>
          Reset dashboard?
        </div>
        <div style={{
          fontFamily: font.body, fontSize: 13, color: colors.textMuted,
          lineHeight: 1.5, marginBottom: 24,
        }}>
          This will restore the default layout. Your current arrangement will be lost.
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
          <button
            onClick={onCancel}
            style={{
              padding: '7px 16px', borderRadius: radius.md,
              border: `1px solid ${colors.border}`,
              background: 'none',
              fontFamily: font.body, fontSize: 13, color: colors.textMuted,
              cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            onClick={() => { onConfirm(); }}
            style={{
              padding: '7px 16px', borderRadius: radius.md,
              border: 'none',
              background: colors.ribbonGradient,
              fontFamily: font.body, fontSize: 13, fontWeight: 500,
              color: colors.textOnAccent, cursor: 'pointer',
            }}
          >
            Reset
          </button>
        </div>
      </div>
    </div>
  );
}
