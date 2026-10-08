import { Children, Fragment, isValidElement } from 'react';
import type { CSSProperties } from 'react';
import { font, radius, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { ToggleLabelContext } from '../common/Toggle';

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
 * The label column of a `Row`, and the group width below which a row stops
 * being two columns and stacks its label above its control.
 *
 * The threshold is a CONTAINER width, not a viewport width. How wide the
 * settings column is depends on whether the chat panel is docked beside it,
 * not on the window: at 1162px with chat docked the column is under 400px, and
 * a 196px label plus a 24px gap plus the row padding left the control ~50px —
 * inputs vanished off the card's edge and selects ran past it. 520 is the
 * label column, the gap and the padding plus room for a 240px select.
 */
const ROW_LABEL_WIDTH = 196;
const ROW_STACK_BELOW = 520;

/**
 * The things inline styles cannot say: a separator on every row but the
 * first, a hover fill on a row that is actually a target, and a row layout
 * that answers to the width of the group it sits in (`@container`, with the
 * group as the container — an inline `display: flex` could not be overridden
 * by the query, so the row's layout lives here rather than on the element).
 *
 * `.pa-set-egress` is the Privacy pane's egress-audit table on the same rule:
 * four fixed columns needed ~460px and, in a narrow column, scrolled sideways
 * inside the card so its RESULT pills read as clipped. Narrow, it drops the
 * Kind column (`.pa-set-wide-only`; the kind moves into the row's tooltip).
 *
 * Injected once, from the module, rather than added to `index.css` — this is
 * the Settings surface's own vocabulary and it should travel with the file that
 * defines it. The custom properties are set per-group inline, so the rules
 * carry the live theme without the stylesheet knowing anything about themes.
 */
const SETTINGS_CSS = `
.pa-set-group { container-type: inline-size; }
.pa-set-body { padding: 32px 40px 60px; }
@container (max-width: 560px) { .pa-set-body { padding: ${space.huge}px ${space.xxxl}px 40px; } }
.pa-set-group > * + * { border-top: 1px solid var(--pa-set-sep); }
.pa-set-row { display: flex; align-items: flex-start; gap: ${space.huge}px; }
.pa-set-row-label { width: ${ROW_LABEL_WIDTH}px; flex-shrink: 0; padding-top: ${space.xs}px; }
.pa-set-row-control { flex: 1; min-width: 0; }
.pa-set-egress { display: grid; grid-template-columns: 150px minmax(0, 1fr) 90px 80px; gap: ${space.xl}px; align-items: center; }
@container (max-width: ${ROW_STACK_BELOW}px) {
  .pa-set-row { flex-direction: column; align-items: stretch; gap: ${space.md}px; }
  .pa-set-row-label { width: auto; padding-top: 0; }
  .pa-set-egress { grid-template-columns: 64px minmax(0, 1fr) 80px; gap: ${space.md}px; }
  .pa-set-wide-only { display: none; }
}
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
    // `min(…, 100%)`: a select never asks for more room than its row has. A
    // bare 240 pushed "Open on launch" past the card edge in a narrow column.
    minWidth: 'min(240px, 100%)', maxWidth: '100%', cursor: 'pointer',
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
    <span style={{ fontSize: textSize.micro, fontWeight: 600, padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.pill, background: s.bg, color: s.text }}>
      {s.label}
    </span>
  );
}

/** `H1` graduated to `components/common/H1.tsx` (#1177, #1185) — imported
 *  wherever the old settings pane title is still needed. */

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
export function Row({ label, hint, children, onClick, dim }: {
  label: string; hint?: string; children?: React.ReactNode; onClick?: () => void;
  /** Fades the row for something never seen / not yet real — R3: honest, not
   *  hidden. A never-paired device or an inert integration is still LISTED,
   *  just visually secondary to the rows that are actually happening. */
  dim?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <div
      className={onClick ? 'pa-set-row pa-set-row-tap' : 'pa-set-row'}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } } : undefined}
      style={{
        padding: `${space.xl}px ${space.xxl}px`,
        opacity: dim ? 0.6 : 1,
        '--pa-set-hover': colors.fillHover,
        '--pa-set-active': colors.fillActive,
        '--pa-set-dur': '160ms',
        '--pa-set-ease': 'var(--pa-ease-smooth, cubic-bezier(0.22, 1, 0.36, 1))',
      } as CSSProperties}
    >
      <div className="pa-set-row-label">
        <div style={{ fontSize: textSize.small, fontWeight: 500, color: colors.text }}>{label}</div>
        {hint && <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xs, lineHeight: 1.45 }}>{hint}</div>}
      </div>
      <div className="pa-set-row-control">
        <ToggleLabelContext.Provider value={label}>{children}</ToggleLabelContext.Provider>
      </div>
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
      display: 'inline-block', padding: `${space.xxs}px ${space.sm}px`,
      fontFamily: font.mono, fontSize: textSize.micro, color: colors.text,
      background: colors.fillSubtle,
      border: `1px solid ${colors.border}`,
      borderRadius: radius.xs, minWidth: 22, textAlign: 'center',
    }}>{children}</span>
  );
}

// `Card`, `SectionLabel` and `StatRow` graduated to `components/common/Card.tsx`
// (#1177, #1185) — `history/SpendPanel` needed them from outside `settings/`.

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
      // An ordinary "nothing to save yet" disabled state, not a dead preview
      // control — the settings-surface fitness scan (settingsPalette /
      // SettingsView.preview-controls.test.tsx) only allows a disabled
      // control that says why, so every `SaveButton` caller gets that for
      // free instead of re-deriving it per panel.
      data-disabled-reason={disabled ? (saving ? 'saving' : 'no changes to save') : undefined}
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

/**
 * Technical detail behind a disclosure (R3, UX_STANDARD.md).
 *
 * Several Section `sub` paragraphs used to be the whole explanation — crate
 * names, socket requirements, scanner internals — in the ONE line every
 * visitor reads before deciding whether the section is theirs. A plain-language
 * `sub` on the caller stays the default; this holds what a curious operator
 * (not everyone) needs. Native `<details>`/`<summary>`: free keyboard support
 * (Enter/Space toggle, Tab reaches it) and a platform disclosure triangle
 * rather than a hand-rolled chevron plus ARIA.
 */
export function Details({ summary, children }: { summary: string; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <details style={{ marginTop: space.md }}>
      <summary
        style={{
          cursor: 'pointer', fontSize: textSize.micro, color: colors.textMuted,
          fontFamily: font.body, userSelect: 'none',
        }}
      >
        {summary}
      </summary>
      <div style={{ marginTop: space.sm, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.55 }}>
        {children}
      </div>
    </details>
  );
}

/**
 * One row of a list of named, statused things — a paired device, a background
 * worker, a dispatch persona, a messaging gateway. Converges the shape that
 * `AgentsPanel`'s worker/persona rows and `DevicesPanel`'s device rows had
 * each hand-rolled slightly differently (portrait-or-not, chip-or-not,
 * clickable-or-not): one row, one set of states, so a 4th list does not
 * invent a 4th look (R6 — "one list-row style per list type").
 *
 * `onOpen` makes the row itself the target (button semantics, full keyboard
 * support); omit it for a row that is display-only and let `trailing` carry
 * whatever action it needs instead.
 */
export function ListRow({ icon, title, chip, status, detail, trailing, onOpen, tone, testId }: {
  icon?: React.ReactNode;
  title: string;
  /** A short status pill next to the title — "on"/"off", "connected", etc. */
  chip?: React.ReactNode;
  /** The status text at the row's trailing edge (top line) — live state. */
  status?: React.ReactNode;
  /** The plain-language sentence under the title. */
  detail?: React.ReactNode;
  /** Buttons or controls on their own line, below detail. */
  trailing?: React.ReactNode;
  onOpen?: () => void;
  /** Dims the row for something never seen / not yet real (R3 — honest, not hidden). */
  tone?: 'default' | 'dim' | 'problem';
  testId?: string;
}) {
  const { colors } = useTheme();
  const border = tone === 'problem' ? colors.danger : colors.border;
  const body = (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: space.lg, opacity: tone === 'dim' ? 0.6 : 1 }}>
      {icon}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', flexWrap: 'wrap', justifyContent: 'space-between', columnGap: space.xl, rowGap: space.xxs, marginBottom: space.xs }}>
          <span style={{ display: 'flex', alignItems: 'center', gap: space.md, minWidth: 0 }}>
            <span style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{title}</span>
            {chip}
          </span>
          {status}
        </div>
        {detail && <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.45 }}>{detail}</div>}
        {trailing && <div style={{ marginTop: space.md }}>{trailing}</div>}
      </div>
    </div>
  );
  const shared: CSSProperties = {
    padding: `${space.xl}px 14px`, borderRadius: radius.md, background: colors.bgDeeper,
    border: `1px solid ${border}`, fontFamily: font.body,
  };
  if (!onOpen) return <div data-testid={testId} style={shared}>{body}</div>;
  return (
    <button type="button" onClick={onOpen} data-testid={testId} style={{ ...shared, display: 'block', width: '100%', textAlign: 'left', cursor: 'pointer' }}>
      {body}
    </button>
  );
}
