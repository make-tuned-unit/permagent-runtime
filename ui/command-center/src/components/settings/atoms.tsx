import type { CSSProperties } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
      <div style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, letterSpacing: '-0.01em', marginBottom: sub ? 4 : 16 }}>{title}</div>
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

export function TextInput({ value, onChange, placeholder, mono, multi, disabled = false }: {
  value: string; onChange?: (v: string) => void; placeholder?: string; mono?: boolean; multi?: boolean; disabled?: boolean;
}) {
  const { colors } = useTheme();
  const Tag = multi ? 'textarea' : 'input';
  return (
    <Tag
      value={value} placeholder={placeholder} disabled={disabled}
      onChange={e => onChange?.(e.target.value)}
      style={{
        width: '100%', padding: multi ? 12 : '8px 12px',
        background: colors.inputBg, border: `1px solid ${colors.border}`,
        borderRadius: radius.md, color: colors.text,
        fontFamily: mono ? font.mono : font.body,
        fontSize: mono ? 12 : 13, outline: 'none',
        minHeight: multi ? 80 : 'auto', resize: multi ? 'vertical' : 'none',
        cursor: disabled ? 'not-allowed' : 'text', opacity: disabled ? 0.55 : 1,
      } as React.CSSProperties}
    />
  );
}

/* The resting look is unchanged — padding, pill radius, type and the on/off
   palette all ride across as `--pa-btn-*` declarations. What is new is that
   pressing a chip now looks different from not pressing one: an inline `style`
   object could express neither :hover nor :active, so a filter chip gave no
   acknowledgement at all until the list underneath it changed. */
export function Chip({ on, onClick, children }: { on: boolean; onClick?: () => void; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant={on ? 'ghostOn' : 'ghost'}
      onClick={onClick}
      style={{
        '--pa-btn-bg': on ? colors.cyanSoft : 'transparent',
        '--pa-btn-fg': on ? colors.cyan : colors.textMuted,
        '--pa-btn-border': on ? colors.borderHi : colors.border,
        '--pa-btn-bg-hover': on ? colors.cyanSoft : colors.surfaceHi,
        '--pa-btn-fg-hover': on ? colors.cyan : colors.text,
        '--pa-btn-border-hover': on ? colors.cyan : colors.borderHi,
        '--pa-btn-pad': '6px 12px',
        '--pa-btn-radius': `${radius.pill}px`,
        '--pa-btn-weight': 500,
        fontFamily: font.body,
        fontSize: 12,
      } as CSSProperties}
    >
      {children}
    </Button>
  );
}

/* Toggle moved to `components/common/Toggle.tsx`. It was the only atom here
   with a server round trip behind it, and having no busy phase and no failure
   path it made six call sites hand-roll their own optimistic flip and revert.
   The primitive owns that contract now; import it from common. */

export function Slider({ value, onChange, min = 0, max = 100, suffix, disabled = false }: {
  value: number; onChange?: (v: number) => void; min?: number; max?: number; suffix?: string; disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
      <input type="range" min={min} max={max} value={value} disabled={disabled}
        onChange={e => onChange?.(Number(e.target.value))}
        style={{ flex: 1, accentColor: colors.cyan, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.55 : 1 }} />
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

// ── Card primitives (migrated from the Governance surface when its panels
//    were folded into Settings) — shared by the Spend / Sovereignty / Models
//    panes so their data-dense views read as one surface. ─────────────────

export function Card({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      padding: '18px 20px',
    }}>
      {children}
    </div>
  );
}

export function SectionLabel({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      fontFamily: font.body, fontSize: 11, fontWeight: 600,
      letterSpacing: '0.10em', textTransform: 'uppercase', color: colors.textDim,
    }}>
      {children}
    </div>
  );
}

/** A labeled row: primary text + optional sub-line on the left, a value node on
 *  the right. */
export function StatRow({ left, sub, right }: { left: React.ReactNode; sub?: React.ReactNode; right: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 12,
      padding: '10px 12px', borderRadius: radius.md,
      background: colors.bgDeeper, border: `1px solid ${colors.border}`,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {left}
        </div>
        {sub != null && (
          <div style={{ fontSize: 11, color: colors.textMuted, fontFamily: font.mono, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {sub}
          </div>
        )}
      </div>
      <div style={{ flexShrink: 0 }}>{right}</div>
    </div>
  );
}

export function SaveButton({ onClick, disabled, saving }: {
  onClick: () => void; disabled: boolean; saving: boolean;
}) {
  const { colors } = useTheme();
  return (
    // `saving` is the caller's own in-flight flag and the work runs off in the
    // caller's handler, so it arrives as `pending` (the form-submit shape): the
    // button reads as busy rather than merely unavailable while it writes, and
    // the pending floor keeps a 30ms save from flashing past unreadably.
    <Button
      colors={colors}
      variant="primary"
      onClick={onClick}
      disabled={disabled}
      pending={saving}
      style={{
        '--pa-btn-bg': disabled ? colors.cyanSoft : colors.cyan,
        // `textOnCyan`, not `textOnAccent`: the fill here is flat `colors.cyan`,
        // and the token's own rule is that white on flat cyan fails contrast
        // (and inverts to near-white on the silver theme). This is the same
        // colour `variant="primary"` already sets — the override was undoing it.
        '--pa-btn-fg': disabled ? colors.textDim : colors.textOnCyan,
        '--pa-btn-bg-hover': disabled ? colors.cyanSoft : colors.cyan,
        '--pa-btn-bg-active': disabled ? colors.cyanSoft : colors.cyan,
        '--pa-btn-pad': '8px 20px',
        '--pa-btn-radius': `${radius.md}px`,
        fontFamily: font.body,
        fontSize: 13,
      } as CSSProperties}
    >
      {saving ? 'Saving...' : 'Save'}
    </Button>
  );
}
