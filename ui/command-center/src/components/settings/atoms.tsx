import { font, ease, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function H1({ children, sub }: { children: React.ReactNode; sub?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: 28 }}>
      <div style={{ fontFamily: font.display, fontSize: 24, fontWeight: 600, letterSpacing: '-0.02em', color: colors.text }}>{children}</div>
      {sub && <div style={{ fontSize: 13, color: colors.textMuted, marginTop: 6, maxWidth: 580, lineHeight: 1.55 }}>{sub}</div>}
    </div>
  );
}

export function Section({ title, sub, children }: { title: string; sub?: string; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: 28, padding: 24, borderRadius: radius.md, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
      <div style={{ fontFamily: font.display, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em', marginBottom: sub ? 4 : 16 }}>{title}</div>
      {sub && <div style={{ fontSize: 12, color: colors.textMuted, marginBottom: 18, lineHeight: 1.5 }}>{sub}</div>}
      {children}
    </div>
  );
}

export function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 24, padding: '14px 0', borderTop: `1px solid ${colors.border}` }}>
      <div style={{ width: 200, flexShrink: 0, paddingTop: 6 }}>
        <div style={{ fontSize: 13, fontWeight: 500, color: colors.text }}>{label}</div>
        {hint && <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 4, lineHeight: 1.5 }}>{hint}</div>}
      </div>
      <div style={{ flex: 1 }}>{children}</div>
    </div>
  );
}

export function TextInput({ value, onChange, placeholder, mono, multi }: {
  value: string; onChange?: (v: string) => void; placeholder?: string; mono?: boolean; multi?: boolean;
}) {
  const { colors } = useTheme();
  const Tag = multi ? 'textarea' : 'input';
  return (
    <Tag
      value={value} placeholder={placeholder}
      onChange={e => onChange?.(e.target.value)}
      style={{
        width: '100%', padding: multi ? 12 : '8px 12px',
        background: colors.inputBg, border: `1px solid ${colors.border}`,
        borderRadius: 8, color: colors.text,
        fontFamily: mono ? font.mono : font.body,
        fontSize: mono ? 12 : 13, outline: 'none',
        minHeight: multi ? 80 : 'auto', resize: multi ? 'vertical' : 'none',
      } as React.CSSProperties}
    />
  );
}

export function Chip({ on, onClick, children }: { on: boolean; onClick?: () => void; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <button onClick={onClick} style={{
      padding: '6px 12px', borderRadius: 999, cursor: 'pointer',
      fontFamily: font.body, fontSize: 12, fontWeight: 500,
      background: on ? colors.cyanSoft : 'transparent',
      border: `1px solid ${on ? colors.borderHi : colors.border}`,
      color: on ? colors.cyan : colors.textMuted,
    }}>{children}</button>
  );
}

export function Toggle({ on, onChange }: { on: boolean; onChange?: (v: boolean) => void }) {
  const { colors } = useTheme();
  return (
    <button onClick={() => onChange?.(!on)} style={{
      width: 36, height: 22, borderRadius: 999, padding: 2,
      background: on ? colors.cyan : colors.surfaceHi,
      border: 'none', cursor: 'pointer', position: 'relative',
      transition: `background 160ms ${ease.out}`,
      boxShadow: on ? `0 0 8px ${colors.cyanGlow}` : 'none',
    }}>
      <div style={{
        width: 18, height: 18, borderRadius: '50%', background: '#fff',
        transform: on ? 'translateX(14px)' : 'translateX(0)',
        transition: `transform 180ms cubic-bezier(0.34, 1.56, 0.64, 1)`,
      }} />
    </button>
  );
}

export function Slider({ value, onChange, min = 0, max = 100, suffix }: {
  value: number; onChange?: (v: number) => void; min?: number; max?: number; suffix?: string;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
      <input type="range" min={min} max={max} value={value}
        onChange={e => onChange?.(Number(e.target.value))}
        style={{ flex: 1, accentColor: colors.cyan }} />
      <span style={{ fontFamily: font.mono, fontSize: 12, color: colors.textMuted, minWidth: 50, textAlign: 'right' }}>{value}{suffix}</span>
    </div>
  );
}

export function Kbd({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <span style={{
      display: 'inline-block', padding: '2px 7px',
      fontFamily: font.mono, fontSize: 11, color: colors.text,
      background: colors.border,
      border: `1px solid ${colors.border}`,
      borderRadius: 5, minWidth: 22, textAlign: 'center',
    }}>{children}</span>
  );
}

export function SaveButton({ onClick, disabled, saving }: {
  onClick: () => void; disabled: boolean; saving: boolean;
}) {
  const { colors } = useTheme();
  return (
    <button onClick={onClick} disabled={disabled} style={{
      fontFamily: font.body, fontSize: 13, fontWeight: 600,
      padding: '8px 20px', borderRadius: radius.md,
      background: disabled ? colors.cyanSoft : colors.cyan,
      color: disabled ? colors.textDim : colors.textOnAccent,
      border: 'none', cursor: disabled ? 'default' : 'pointer',
      transition: `all 200ms ${ease.out}`,
      opacity: disabled ? 0.5 : 1,
    }}>
      {saving ? 'Saving...' : 'Save'}
    </button>
  );
}
