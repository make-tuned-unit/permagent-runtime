import { Children, Fragment, isValidElement } from 'react';
import type { CSSProperties } from 'react';
import { concentric, font, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

/**
 * The Settings visual language, in one file.
 *
 * Every pane is built from `H1`, `Section` and `Row`, so this is where the
 * calm Apple settings feel is either present or absent. It used to be absent
 * in a specific, nameable way: each `Section` was a bordered, filled CARD, and
 * every `Row` — including the first one in a group — drew a full-width rule
 * above itself, which put a hairline directly under the section title. That is
 * three pieces of decoration (card fill, card border, leading rule) doing the
 * job one piece of layout does, and WWDC25/356 is explicit about it:
 * *"Instead of relying on decoration, hierarchy should be expressed through
 * layout and grouping."*
 *
 * So the shape is Apple's grouped inset list:
 *   - a small uppercase section header OUTSIDE the group, inset to the row text;
 *   - one opaque rounded group holding the rows;
 *   - separators BETWEEN rows only, inset past the label column;
 *   - no shadow, no glass.
 *
 * Opaque is not an aesthetic preference here. Settings content is the content
 * layer, and Apple's rule for the content layer has no exceptions: *"Don't use
 * Liquid Glass in the content layer."* Glass belongs to the floating control
 * layer — toolbars, popovers, the sidebar — and the Settings body is none of
 * those. There is deliberately no `backdropFilter` anywhere in this file.
 */

/**
 * The two things inline styles cannot say: a separator on every row but the
 * first, and a hover fill on a row that is actually a target.
 *
 * Injected once, from the module, rather than added to `index.css` — this is
 * the Settings surface's own vocabulary and it should travel with the file that
 * defines it. The custom properties are set per-group inline, so the rules
 * carry the live theme without the stylesheet knowing anything about themes.
 */
const SETTINGS_CSS = `
.pa-set-group > * + * { border-top: 1px solid var(--pa-set-sep); }
.pa-set-row-tap { cursor: pointer; transition: background var(--pa-set-dur) var(--pa-set-ease); }
.pa-set-row-tap:hover { background: var(--pa-set-hover); }
.pa-set-row-tap:active { background: var(--pa-set-active); }
@media (prefers-reduced-motion: reduce) { .pa-set-row-tap { transition: none; } }
`;

function ensureSettingsStyles(): void {
  if (typeof document === 'undefined') return;
  if (document.getElementById('pa-settings-atoms')) return;
  const el = document.createElement('style');
  el.id = 'pa-settings-atoms';
  el.textContent = SETTINGS_CSS;
  document.head.appendChild(el);
}
ensureSettingsStyles();

/**
 * The Settings select/input look. Lived in `SettingsView` until the Guard,
 * Watcher and Librarian panes moved out to Agents and needed the same
 * controls — one definition, so a relocated setting cannot look relocated.
 */
export function selectStyle(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    height: 30, padding: `0 ${space.lg}px`, borderRadius: radius.sm,
    background: colors.inputBg, border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, fontSize: textSize.small,
    minWidth: 240, cursor: 'pointer',
  };
}

/** Whether an Ollama model is loaded, on disk, or absent. Shared by the Models
 *  pane's model table and the Librarian's schedule, wherever it lives. */
export function ModelStateBadge({ state }: { state: 'running' | 'installed' | 'missing' }) {
  const { colors } = useTheme();
  const styles: Record<string, { bg: string; text: string; label: string }> = {
    running: { bg: colors.cyanSoft, text: colors.cyan, label: 'Loaded' },
    installed: { bg: colors.fillSubtle, text: colors.textMuted, label: 'Installed' },
    missing: { bg: colors.fillSubtle, text: colors.danger, label: 'Not installed' },
  };
  const s = styles[state];
  return (
    <span style={{ fontSize: textSize.micro, fontWeight: 600, padding: '2px 8px', borderRadius: radius.pill, background: s.bg, color: s.text }}>
      {s.label}
    </span>
  );
}

/**
 * The pane title. `type.title` rather than a hand-typed 24px: the ramp's
 * `title` is 20/26/600 at -0.01em, which is the macOS large-title proportion
 * for a dense window, and the 24 it replaced was an off-ramp size that existed
 * only here.
 *
 * Left-aligned, and the subtitle sits at a readable measure — Tahoe's
 * typography is *"bolder and left-aligned"*, and centered body copy is now the
 * un-Apple choice (WWDC25/356).
 */
export function H1({ children, sub }: { children: React.ReactNode; sub?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: space.huge }}>
      <div style={{ ...type.title, fontFamily: font.display, color: colors.text }}>{children}</div>
      {sub && (
        <div style={{
          fontSize: textSize.small, color: colors.textMuted,
          marginTop: space.sm, maxWidth: 620, lineHeight: 1.45,
        }}>{sub}</div>
      )}
    </div>
  );
}

/**
 * Give every direct child of a group the group's own padding.
 *
 * The group is edge-to-edge so that separators and hover fills run its full
 * width, which means the padding has to live on the rows. `Row` and `Block`
 * carry it; anything else a caller drops in — a paragraph, a button strip, a
 * grid of theme swatches — is wrapped in a `Block` so it cannot end up flush
 * against the border. Fragments are flattened first, because a fragment of
 * `Row`s is one of the commonest shapes here and wrapping it whole would pad
 * the rows twice.
 *
 * The alternative was making forty-odd call sites each say `<Block>`, and
 * being one `<Block>` short is invisible until someone looks at that pane.
 */
function padGroupChildren(children: React.ReactNode): React.ReactNode {
  return Children.map(children, child => {
    if (child == null || typeof child === 'boolean') return child;
    if (isValidElement(child)) {
      if (child.type === Fragment) {
        return padGroupChildren((child.props as { children?: React.ReactNode }).children);
      }
      if (child.type === Row || child.type === Block) return child;
    }
    return <Block>{child}</Block>;
  });
}

/**
 * A grouped inset list: header above, opaque group below.
 *
 * `sub` is the group's description and stays with the header rather than
 * moving into a footer, because several of these say something you need before
 * you touch the control ("Set both boxes together, or neither").
 */
export function Section({ title, sub, children }: { title: string; sub?: string; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: space.huge }}>
      <div style={{ ...type.label, color: colors.textDim, padding: `0 ${space.xl}px ${space.md}px` }}>{title}</div>
      {sub && (
        <div style={{
          fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5,
          padding: `0 ${space.xl}px ${space.lg}px`, maxWidth: 620,
        }}>{sub}</div>
      )}
      <div
        className="pa-set-group"
        style={{
          '--pa-set-sep': colors.border,
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          overflow: 'hidden',
        } as CSSProperties}
      >
        {padGroupChildren(children)}
      </div>
    </div>
  );
}

/**
 * One row of a group.
 *
 * `onClick` makes the whole row the target, and only then does it get the
 * pointer ladder — a hover fill on a row that cannot be pressed is a lie about
 * what is clickable, and Mac users read a hover highlight as "this is a
 * button". Rows that merely HOLD a control leave the feedback to the control.
 */
export function Row({ label, hint, children, onClick }: {
  label: string; hint?: string; children?: React.ReactNode; onClick?: () => void;
}) {
  const { colors } = useTheme();
  return (
    <div
      className={onClick ? 'pa-set-row-tap' : undefined}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } } : undefined}
      style={{
        display: 'flex', alignItems: 'flex-start', gap: space.huge,
        padding: `${space.xl}px ${space.xxl}px`,
        '--pa-set-hover': colors.fillHover,
        '--pa-set-active': colors.fillActive,
        '--pa-set-dur': '160ms',
        '--pa-set-ease': 'var(--pa-ease-smooth, cubic-bezier(0.22, 1, 0.36, 1))',
      } as CSSProperties}
    >
      <div style={{ width: 196, flexShrink: 0, paddingTop: space.xs }}>
        <div style={{ fontSize: textSize.small, fontWeight: 500, color: colors.text }}>{label}</div>
        {hint && <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xs, lineHeight: 1.45 }}>{hint}</div>}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>{children}</div>
    </div>
  );
}

/** Free-form content inside a group — prose, a button strip, a table. Carries
 *  the group's own padding so a caller never has to guess it. */
export function Block({ children }: { children: React.ReactNode }) {
  return <div style={{ padding: `${space.xl}px ${space.xxl}px` }}>{children}</div>;
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
        width: '100%', padding: multi ? space.lg : `${space.md}px ${space.lg}px`,
        background: colors.inputBg, border: `1px solid ${colors.border}`,
        borderRadius: radius.sm, color: colors.text,
        fontFamily: mono ? font.mono : font.body,
        fontSize: mono ? textSize.caption : textSize.small, outline: 'none',
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
        '--pa-btn-bg-hover': on ? colors.cyanSoft : colors.fillHover,
        '--pa-btn-fg-hover': on ? colors.cyan : colors.text,
        '--pa-btn-border-hover': on ? colors.cyan : colors.borderHi,
        '--pa-btn-pad': '6px 12px',
        '--pa-btn-radius': `${radius.pill}px`,
        '--pa-btn-weight': 500,
        fontFamily: font.body,
        fontSize: textSize.caption,
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
    <div style={{ display: 'flex', alignItems: 'center', gap: space.xl }}>
      <input type="range" min={min} max={max} value={value} disabled={disabled}
        onChange={e => onChange?.(Number(e.target.value))}
        style={{ flex: 1, accentColor: colors.cyan, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.55 : 1 }} />
      <span style={{ fontFamily: font.mono, fontSize: textSize.caption, color: colors.textMuted, minWidth: 50, textAlign: 'right' }}>{value}{suffix}</span>
    </div>
  );
}

export function Kbd({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <span style={{
      display: 'inline-block', padding: '2px 7px',
      fontFamily: font.mono, fontSize: textSize.micro, color: colors.text,
      background: colors.fillSubtle,
      border: `1px solid ${colors.border}`,
      borderRadius: radius.xs, minWidth: 22, textAlign: 'center',
    }}>{children}</span>
  );
}

// ── Card primitives (migrated from the Governance surface when its panels
//    were folded into Settings) — shared by the Spend / Sovereignty / Models
//    panes so their data-dense views read as one surface. ─────────────────

/** Outer padding of a `Card`. Named because `StatRow` derives its own corner
 *  from it: `r_inner = r_outer - padding` (WWDC25/356). */
const CARD_PAD = space.xxl;

export function Card({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      padding: CARD_PAD,
    }}>
      {children}
    </div>
  );
}

export function SectionLabel({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return <div style={{ ...type.label, fontFamily: font.body, color: colors.textDim }}>{children}</div>;
}

/** A labeled row: primary text + optional sub-line on the left, a value node on
 *  the right. Its corner is concentric with the `Card` it sits in. */
export function StatRow({ left, sub, right }: { left: React.ReactNode; sub?: React.ReactNode; right: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: space.xl,
      padding: `${space.lg}px ${space.xl}px`,
      borderRadius: concentric(radius.lg, CARD_PAD),
      background: colors.fillSubtle,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {left}
        </div>
        {sub != null && (
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
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
        fontSize: textSize.small,
      } as CSSProperties}
    >
      {saving ? 'Saving...' : 'Save'}
    </Button>
  );
}
