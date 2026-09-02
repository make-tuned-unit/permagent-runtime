import { useRef, useEffect, useState, type CSSProperties } from 'react';
import { FiMoreVertical } from 'react-icons/fi';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export interface MenuItem {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
}

interface Props {
  items: MenuItem[];
}

export function DashboardOverflowMenu({ items }: Props) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKey);
    };
  }, [open]);

  return (
    <div ref={menuRef} style={{ position: 'relative' }}>
      <Button
        colors={colors}
        type="button"
        onClick={() => setOpen(!open)}
        title="More actions"
        aria-label="More actions"
        style={{
          '--pa-btn-bg': colors.surface,
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-border': colors.border,
          '--pa-btn-bg-hover': colors.surfaceHi,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-border-hover': colors.borderHi,
          '--pa-btn-bg-active': colors.surface,
          '--pa-btn-pad': '0',
          '--pa-btn-radius': `${radius.md}px`,
          width: 28, height: 28,
        } as CSSProperties}
      >
        <FiMoreVertical size={14} />
      </Button>

      {open && (
        <div style={{
          position: 'absolute', top: '100%', right: 0, marginTop: 6,
          minWidth: 200, borderRadius: radius.md,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: colors.cardShadow,
          overflow: 'hidden', zIndex: 20,
        }}>
          {items.map((item, i) => (
            <Button
              key={i}
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => { setOpen(false); item.onClick(); }}
              style={{
                '--pa-btn-bg': 'transparent',
                '--pa-btn-fg': item.danger ? colors.danger : colors.text,
                '--pa-btn-border': 'transparent',
                '--pa-btn-bg-hover': colors.cyanSoft,
                '--pa-btn-fg-hover': item.danger ? colors.danger : colors.text,
                '--pa-btn-bg-active': colors.cyanSoft,
                '--pa-btn-pad': '10px 14px',
                // Square: the menu is a single rounded card that clips its rows.
                '--pa-btn-radius': '0',
                display: 'flex', width: '100%',
                justifyContent: 'flex-start',
                fontFamily: font.body, fontSize: textSize.small,
              } as CSSProperties}
            >
              {/* One row inside the primitive's label span, so the icon keeps
                  its 10px from the label. */}
              <span style={{ display: 'flex', alignItems: 'center', gap: 10, textAlign: 'left' }}>
                {item.icon}
                {item.label}
              </span>
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
