import { useRef, useEffect, useState } from 'react';
import { FiMoreVertical } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

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
      <button
        onClick={() => setOpen(!open)}
        title="More actions"
        style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          width: 28, height: 28, borderRadius: radius.md,
          border: `1px solid ${colors.border}`,
          background: colors.surface,
          color: colors.textMuted, cursor: 'pointer',
          transition: 'all 150ms ease',
        }}
      >
        <FiMoreVertical size={14} />
      </button>

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
            <button
              key={i}
              onClick={() => { setOpen(false); item.onClick(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                width: '100%', padding: '10px 14px',
                background: 'none', border: 'none',
                fontFamily: font.body, fontSize: 13,
                color: item.danger ? colors.danger : colors.text,
                cursor: 'pointer', textAlign: 'left',
                transition: 'background 100ms ease',
              }}
              onMouseEnter={e => (e.currentTarget.style.background = colors.cyanSoft)}
              onMouseLeave={e => (e.currentTarget.style.background = 'none')}
            >
              {item.icon}
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
