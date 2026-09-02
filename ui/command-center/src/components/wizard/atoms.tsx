import { useState, useRef, useEffect, useMemo, type CSSProperties, type ReactNode, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { FiArrowLeft, FiChevronDown } from 'react-icons/fi';
import { font, ease, duration, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

// ── WizardHeading / WizardSubhead ──────────────────────────────────────
// One heading scale for every moment (audit #603: sizes drifted 32/28/22 and
// weight 700 vs 600 across screens). All moments render the title + subtitle
// through these so the flow reads as one designed object in all three themes.
export function WizardHeading({ children, style = {} }: { children: ReactNode; style?: CSSProperties }) {
  const { colors } = useTheme();
  return (
    <h1 style={{
      fontFamily: font.display, fontSize: 28, fontWeight: 700, lineHeight: 1.15,
      letterSpacing: '-0.02em', color: colors.text, margin: 0, textAlign: 'center',
      ...style,
    }}>{children}</h1>
  );
}

export function WizardSubhead({ children, style = {} }: { children: ReactNode; style?: CSSProperties }) {
  const { colors } = useTheme();
  return (
    <p style={{
      fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.5, color: colors.textMuted,
      margin: '8px 0 0', textAlign: 'center', maxWidth: 400,
      ...style,
    }}>{children}</p>
  );
}

// ── PrimaryButton ──────────────────────────────────────────────────────
export function PrimaryButton({
children, disabled, onClick, full, style = {} }: {
  children: ReactNode; disabled?: boolean; onClick?: () => void; full?: boolean; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  // The fill and the text colour move onto the primitive's custom properties so
  // this button finally has a press give and a focus ring. The glow does not:
  // there is no `--pa-btn-shadow`, and a `box-shadow` written inline cannot be
  // changed by a `:hover` rule — so the hover flag stays, driving only the glow.
  // (Rule: mouse handlers that set React state are kept.)
  const [hover, setHover] = useState(false);
  return (
    <Button
      colors={colors}
      variant="primary"
      type="button"
      onClick={onClick}
      disabled={disabled}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        '--pa-btn-fg': disabled ? colors.textDim : colors.textOnAccent,
        '--pa-btn-bg': disabled ? colors.purpleSoft : colors.purple,
        '--pa-btn-bg-hover': colors.purpleBright,
        '--pa-btn-bg-active': colors.purple,
        '--pa-btn-border': 'transparent',
        '--pa-btn-border-hover': 'transparent',
        '--pa-btn-pad': '12px 20px',
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 600,
        fontFamily: font.body, fontSize: textSize.body, letterSpacing: '-0.01em',
        height: 44, minWidth: 140, width: full ? '100%' : 'auto',
        boxShadow: disabled ? 'none' : hover
          ? `0 0 0 4px ${colors.purpleSoft}, 0 8px 24px ${colors.purpleGlow}`
          : `0 4px 14px ${colors.purpleGlow}`,
        ...style,
      } as CSSProperties}
    >{children}</Button>
  );
}

// ── GhostLink ──────────────────────────────────────────────────────────
export function GhostLink({
children, onClick, style = {} }: {
  children: ReactNode; onClick?: () => void; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  // The muted-to-full hover this kept a `hover` state for is exactly what
  // `--pa-btn-fg-hover` expresses, so the state and its two handlers go.
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={onClick}
      style={{
        '--pa-btn-fg': colors.textMuted,
        '--pa-btn-fg-hover': colors.text,
        '--pa-btn-bg-hover': 'transparent',
        '--pa-btn-bg-active': 'transparent',
        '--pa-btn-pad': '6px 2px',
        '--pa-btn-radius': '0',
        '--pa-btn-weight': 500,
        fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5,
        ...style,
      } as CSSProperties}
    >{children}</Button>
  );
}

// ── Input ──────────────────────────────────────────────────────────────
export function Input({
value, onChange, placeholder, type = 'text', onKeyDown, onBlur, autoFocus, ariaLabel, style = {} }: {
  value: string; onChange: (v: string) => void; placeholder?: string; type?: string;
  onKeyDown?: (e: ReactKeyboardEvent<HTMLInputElement>) => void;
  onBlur?: () => void; autoFocus?: boolean; ariaLabel?: string; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  return (
    <input type={type} value={value} onChange={e => onChange(e.target.value)}
      placeholder={placeholder} spellCheck={false} autoFocus={autoFocus}
      aria-label={ariaLabel} onKeyDown={onKeyDown}
      onFocus={() => setFocus(true)} onBlur={() => { setFocus(false); onBlur?.(); }}
      style={{
        width: '100%', fontFamily: font.body, fontSize: textSize.body, fontWeight: 400,
        color: colors.text,
        background: colors.inputBg,
        border: focus ? `1px solid ${colors.cyan}` : `1px solid ${colors.border}`,
        borderRadius: radius.md, padding: '13px 14px', outline: 'none',
        boxShadow: focus ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `all ${duration.fast}ms ${ease.out}`, ...style,
      }}
    />
  );
}

// ── Textarea ───────────────────────────────────────────────────────────
// Multi-line sibling of Input. Carries the SAME focus-ring treatment (cyan
// border + glow) so the wizard's textareas stop re-implementing it inline
// (audit #603: three duplicate focus-glow implementations) and every field
// gets a visible focus indicator for keyboard users.
export function Textarea({
  value, onChange, placeholder, rows = 3, style = {} }: {
  value: string; onChange: (v: string) => void; placeholder?: string; rows?: number; style?: CSSProperties;
}) {
  const { colors } = useTheme();
  const [focus, setFocus] = useState(false);
  return (
    <textarea value={value} onChange={e => onChange(e.target.value)}
      placeholder={placeholder} rows={rows} spellCheck={false}
      onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
      style={{
        width: '100%', fontFamily: font.body, fontSize: textSize.body, fontWeight: 400,
        color: colors.text, background: colors.inputBg, resize: 'none', lineHeight: 1.6,
        border: focus ? `1px solid ${colors.cyan}` : `1px solid ${colors.border}`,
        borderRadius: radius.md, padding: '13px 14px', outline: 'none',
        boxShadow: focus ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
        transition: `all ${duration.fast}ms ${ease.out}`, ...style,
      }}
    />
  );
}

/**
 * Trait-add validation (MomentMeet). Pure so it's unit-testable and the moment
 * can surface a real reason instead of silently swallowing the input (audit
 * #603: trait-add had no validation feedback). Case-insensitive dedupe.
 */
export function validateTrait(existing: string[], candidate: string): { ok: boolean; reason?: string } {
  const t = candidate.trim();
  if (!t) return { ok: false, reason: 'Type a trait first.' };
  if (t.length > 24) return { ok: false, reason: 'Keep traits short (under 24 characters).' };
  if (existing.some(x => x.toLowerCase() === t.toLowerCase())) {
    return { ok: false, reason: `"${t}" is already there.` };
  }
  return { ok: true };
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
      {/* A listbox trigger, not an action: `Button` would flatten the
          aria-haspopup / aria-expanded pairing that says what it does, and fold
          the label and the chevron into one span. It keeps the element and
          takes the shared `.pa-btn` interaction rules — it had no hover or
          pressed state at all, so it read as inert next to the inputs. */}
      <button type="button" aria-haspopup="listbox" aria-expanded={open} className="pa-btn" onClick={() => setOpen(x => !x)} style={{
        '--pa-btn-bg': colors.inputBg,
        '--pa-btn-fg': colors.text,
        '--pa-btn-border': open ? colors.cyan : colors.border,
        '--pa-btn-bg-hover': colors.inputBg,
        '--pa-btn-border-hover': open ? colors.cyan : colors.borderHi,
        '--pa-btn-bg-active': colors.inputBg,
        '--pa-btn-pad': '13px 14px',
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 500,
        width: '100%', justifyContent: 'space-between',
        fontFamily: font.body, fontSize: textSize.body, lineHeight: 1.5,
        boxShadow: open ? `0 0 0 3px ${colors.cyanGlow}` : 'none',
      } as CSSProperties}>
        <span style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          {current.dot && <span style={{ width: 8, height: 8, borderRadius: '50%', background: current.dot }} />}
          {current.label}
        </span>
        <FiChevronDown size={10} style={{ opacity: 0.55, transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 180ms' }} />
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
                display: 'flex', alignItems: 'center', gap: 10, padding: '10px 10px', borderRadius: radius.sm,
                fontFamily: font.body, fontSize: textSize.small, color: colors.text,
                background: selected ? colors.cyanSoft : 'transparent', cursor: 'pointer',
              }}
              onMouseEnter={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = colors.surfaceHi; }}
              onMouseLeave={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
              onFocus={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = colors.surfaceHi; }}
              onBlur={e => { if (!selected) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
            >
              {o.dot && <span style={{ width: 8, height: 8, borderRadius: '50%', background: o.dot }} />}
              <span style={{ flex: 1 }}>{o.label}</span>
              {o.note && <span style={{ fontSize: textSize.micro, color: colors.textMuted }}>{o.note}</span>}
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
          width: i === current ? 18 : 6, height: 6, borderRadius: radius.pill,
          background: i === current ? colors.cyan : colors.textDim,
          boxShadow: i === current ? `0 0 12px ${colors.cyanGlow}` : 'none',
          transition: `all ${duration.slow}ms ${ease.out}`,
        }} />
      ))}
    </div>
  );
}

// ── BackChevron ────────────────────────────────────────────────────────
export function BackChevron({
onClick }: { onClick: () => void }) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={onClick}
      style={{
        '--pa-btn-fg': colors.textMuted,
        '--pa-btn-fg-hover': colors.text,
        '--pa-btn-bg-hover': 'transparent',
        '--pa-btn-bg-active': 'transparent',
        '--pa-btn-pad': '6px 10px 6px 6px',
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': 500,
        fontFamily: font.body, fontSize: textSize.small, lineHeight: 1.5,
      } as CSSProperties}
    >
      {/* `Button` folds its children into one label span, so the arrow and the
          word need their own row to keep the 6px that used to come from the
          button's own `display:flex`. */}
      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
        <FiArrowLeft size={14} />
        Back
      </span>
    </Button>
  );
}

// ── Particles ──────────────────────────────────────────────────────────
// Custom-property carrier: lets `--pa-dx` sit in a typed style object without a
// blanket @ts-expect-error (audit #603 #9). The animation is gated on the
// user's reduce-motion preference — no drifting particles when motion is off.
type ParticleStyle = CSSProperties & { '--pa-dx': string };

export function Particles({ density = 28 }: { density?: number }) {
  const { colors, reduceMotion } = useTheme();
  const seeds = useMemo(() => Array.from({ length: density }).map((_, i) => ({
    i, x: Math.random() * 100, y: Math.random() * 100,
    r: 0.5 + Math.random() * 1.6, d: 18 + Math.random() * 28,
    delay: -Math.random() * 30, drift: (Math.random() - 0.5) * 30,
    hue: Math.random() < 0.85 ? 'cyan' : 'purple' as const,
  })), [density]);
  // Respect reduce-motion: skip the floating field entirely (the shell keeps its
  // static brand backdrop) rather than animating against the user's preference.
  if (density === 0 || reduceMotion) return null;
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
            '--pa-dx': `${s.drift}vw`,
            animation: `pa-float ${s.d}s linear ${s.delay}s infinite`,
            opacity: 0,
          } as ParticleStyle} />
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
