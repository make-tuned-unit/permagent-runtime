import { useState, useRef, useEffect, useMemo, type CSSProperties, type ReactNode } from 'react';
import { font, ease, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// ── PrimaryButton ──────────────────────────────────────────────────────
export function PrimaryButton({
children, disabled, onClick, full, style = {} }: {
  children: ReactNode; disabled?: boolean; onClick?: () => void; full?: boolean; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <button onClick={onClick} disabled={disabled}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        fontFamily: font.body, fontSize: 14, fontWeight: 600, letterSpacing: '-0.01em',
        color: disabled ? colors.textDim : colors.textOnAccent,
        background: disabled ? colors.purpleSoft : hover ? colors.purpleBright : colors.purple,
        border: 'none', borderRadius: radius.md, padding: '12px 20px', height: 44,
        minWidth: 140, width: full ? '100%' : 'auto',
        cursor: disabled ? 'not-allowed' : 'pointer',
        boxShadow: disabled ? 'none' : hover
          ? `0 0 0 4px ${colors.purpleSoft}, 0 8px 24px ${colors.purpleGlow}`
          : `0 4px 14px ${colors.purpleGlow}`,
        transition: `all 200ms ${ease.out}`, ...style,
      }}
    >{children}</button>
  );
}

// ── GhostLink ──────────────────────────────────────────────────────────
export function GhostLink({
children, onClick, style = {} }: {
  children: ReactNode; onClick?: () => void; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <button onClick={onClick}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        fontFamily: font.body, fontSize: 13, fontWeight: 500,
        color: hover ? colors.text : colors.textMuted,
        background: 'transparent', border: 'none', padding: '6px 2px',
        cursor: 'pointer', transition: `color 160ms ${ease.out}`, ...style,
      }}
    >{children}</button>
  );
}

// ── Input ──────────────────────────────────────────────────────────────
export function Input({
value, onChange, placeholder, type = 'text', style = {} }: {
  value: string; onChange: (v: string) => void; placeholder?: string; type?: string; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  return (
    <input type={type} value={value} onChange={e => onChange(e.target.value)}
      placeholder={placeholder} spellCheck={false}
      onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
      style={{
        width: '100%', fontFamily: font.body, fontSize: 14, fontWeight: 400,
        color: colors.text,
        background: colors.inputBg,
        border: focus ? `1px solid ${colors.cyan}` : `1px solid ${colors.border}`,
        borderRadius: radius.md, padding: '13px 14px', outline: 'none',
        boxShadow: focus ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `all 160ms ${ease.out}`, ...style,
      }}
    />
  );
}

// ── Select ─────────────────────────────────────────────────────────────
export interface SelectOption { value: string; label: string; dot?: string; note?: string }

export function Select({
value, onChange, options, style = {} }: {
  value: string; onChange: (v: string) => void; options: SelectOption[]; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDoc = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener('mousedown', onDoc);
    return () => document.removeEventListener('mousedown', onDoc);
  }, []);
  const current = options.find(o => o.value === value) || options[0];
  return (
    <div
      ref={ref}
      style={{ position: 'relative', ...style }}
      onKeyDown={e => { if (e.key === 'Escape' && open) setOpen(false); }}
    >
      <button type="button" aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen(x => !x)} style={{
        width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        fontFamily: font.body, fontSize: 14, fontWeight: 500, color: colors.text,
        background: colors.inputBg,
        border: open ? `1px solid ${colors.cyan}` : `1px solid ${colors.border}`,
        borderRadius: radius.md, padding: '13px 14px', cursor: 'pointer',
        boxShadow: open ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `all 160ms ${ease.out}`,
      }}>
        <span style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          {current.dot && <span style={{ width: 8, height: 8, borderRadius: '50%', background: current.dot }} />}
          {current.label}
        </span>
        <svg width="10" height="6" viewBox="0 0 10 6" fill="none" style={{ opacity: 0.55, transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 180ms' }}>
          <path d="M1 1l4 4 4-4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </button>
      {open && (
        <div role="listbox" style={{
          position: 'absolute', top: 'calc(100% + 6px)', left: 0, right: 0,
          background: colors.surface, border: `1px solid ${colors.border}`,
          borderRadius: radius.md, padding: 6, boxShadow: colors.cardShadow, zIndex: 30,
        }}>
          {options.map(o => {
            const selected = o.value === value;
            const choose = () => { onChange(o.value); setOpen(false); };
            return (
            <div
              key={o.value}
              role="option"
              aria-selected={selected}
              tabIndex={0}
              onClick={choose}
              // Keyboard-operable (was a bare div onClick): Enter/Space selects,
              // Escape closes (handled on the container). Focus mirrors hover.
              onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); choose(); } }}
              style={{
                display: 'flex', alignItems: 'center', gap: 10, padding: '10px 10px', borderRadius: 6,
                fontFamily: font.body, fontSize: 13, color: colors.text,
                background: selected ? colors.cyanSoft : 'transparent', cursor: 'pointer',
              }}
              onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = colors.surfaceHi; }}
              onMouseLeave={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
              onFocus={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = colors.surfaceHi; }}
              onBlur={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
            >
              {o.dot && <span style={{ width: 8, height: 8, borderRadius: '50%', background: o.dot }} />}
              <span style={{ flex: 1 }}>{o.label}</span>
              {o.note && <span style={{ fontSize: 11, color: colors.textMuted }}>{o.note}</span>}
            </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── ProgressDots ───────────────────────────────────────────────────────
export function ProgressDots({ count = 4, current = 0, style = {} }: {
  count?: number; current?: number; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center', ...style }}>
      {Array.from({ length: count }).map((_, i) => (
        <div key={i} style={{
          width: i === current ? 18 : 6, height: 6, borderRadius: 999,
          background: i === current ? colors.cyan : colors.textDim,
          boxShadow: i === current ? `0 0 12px ${colors.cyanGlow}` : 'none',
          transition: `all 320ms ${ease.out}`,
        }} />
      ))}
    </div>
  );
}

// ── BackChevron ────────────────────────────────────────────────────────
export function BackChevron({
onClick }: { onClick: () => void }) {
  const { colors } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <button onClick={onClick}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        background: 'transparent', border: 'none', padding: '6px 10px 6px 6px',
        color: hover ? colors.text : colors.textMuted,
        fontFamily: font.body, fontSize: 13, fontWeight: 500,
        cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 6,
        borderRadius: 8, transition: `color 160ms ${ease.out}`,
      }}>
      <svg width="14" height="10" viewBox="0 0 14 10" fill="none">
        <path d="M5 1L1 5l4 4M1 5h12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
      Back
    </button>
  );
}

// ── Particles ──────────────────────────────────────────────────────────
export function Particles({ density = 28 }: { density?: number }) {
  const { colors } = useTheme();
  const seeds = useMemo(() => Array.from({ length: density }).map((_, i) => ({
    i, x: Math.random() * 100, y: Math.random() * 100,
    r: 0.5 + Math.random() * 1.6, d: 18 + Math.random() * 28,
    delay: -Math.random() * 30, drift: (Math.random() - 0.5) * 30,
    hue: Math.random() < 0.85 ? 'cyan' : 'purple' as const,
  })), [density]);
  if (density === 0) return null;
  return (
    <>
      <style>{`
        @keyframes pa-float {
          0% { transform: translate(0,0); opacity: 0; }
          10% { opacity: 0.18; }
          90% { opacity: 0.18; }
          100% { transform: translate(var(--pa-dx), -120vh); opacity: 0; }
        }
      `}</style>
      <div style={{ position: 'absolute', inset: 0, overflow: 'hidden', pointerEvents: 'none' }}>
        {seeds.map(s => (
          <div key={s.i} style={{
            position: 'absolute', left: `${s.x}%`, top: `${s.y}%`,
            width: s.r * 2, height: s.r * 2, borderRadius: '50%',
            background: s.hue === 'cyan' ? colors.cyan : colors.purple,
            filter: 'blur(0.4px)',
            // @ts-expect-error CSS custom properties
            '--pa-dx': `${s.drift}vw`,
            animation: `pa-float ${s.d}s linear ${s.delay}s infinite`,
            opacity: 0,
          }} />
        ))}
      </div>
    </>
  );
}

// ── Glass ──────────────────────────────────────────────────────────────
export function Glass({ children, r = 14, padding = 18, style = {} }: {
  children: ReactNode; r?: number; padding?: number; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  return (
    <div style={{
      position: 'relative',
      background: colors.surface,
      backdropFilter: 'blur(24px) saturate(140%)',
      WebkitBackdropFilter: 'blur(24px) saturate(140%)',
      border: `1px solid ${colors.borderHi}`,
      borderRadius: r, padding,
      boxShadow: colors.cardShadow,
      ...style,
    }}>
      {children}
    </div>
  );
}
