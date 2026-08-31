import { useEffect, type CSSProperties } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
          <Button
            colors={colors}
            type="button"
            onClick={onCancel}
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-border': colors.border,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-active': colors.surface,
              '--pa-btn-pad': '7px 16px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 400,
              fontFamily: font.body, fontSize: 13,
            } as CSSProperties}
          >
            Cancel
          </Button>
          <Button
            colors={colors}
            variant="primary"
            type="button"
            onClick={() => { onConfirm(); }}
            style={{
              '--pa-btn-bg': colors.ribbonGradient,
              '--pa-btn-fg': colors.textOnAccent,
              '--pa-btn-border': 'transparent',
              '--pa-btn-bg-hover': colors.ribbonGradient,
              '--pa-btn-border-hover': 'transparent',
              '--pa-btn-bg-active': colors.ribbonGradient,
              '--pa-btn-pad': '7px 16px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 500,
              fontFamily: font.body, fontSize: 13,
            } as CSSProperties}
          >
            Reset
          </Button>
        </div>
      </div>
    </div>
  );
}
