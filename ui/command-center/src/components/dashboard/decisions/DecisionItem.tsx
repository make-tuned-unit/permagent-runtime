/**
 * Decision Inbox — the action bar (⑨), its keycaps and the shortcut sheet.
 *
 * One button language (U5): the shared `Button` in three roles only.
 *  - primary — flat cyan, the ONE tinted control in the view (D8). Trailing.
 *  - ghost   — the alternatives, beside it.
 *  - bare    — Later and the `?` help.
 *  - danger  — a ghost with the `danger` label, parked alone at the leading
 *              edge, with a trailing ellipsis because it opens ConfirmDialog
 *              (U13; HIG: never the default, never beside benign options).
 *
 * The labels come from the same `DecisionPlan` record as the "If you choose"
 * rows, so a row and its button cannot disagree (U3). The retired local `Btn`
 * adapter (ribbon-gradient primary, radius.md) was a second button language.
 *
 * S2: every string from the daemon renders as a React text node.
 */
import type { CSSProperties, ReactNode, RefObject } from 'react';
import { font, radius, space, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { Tooltip } from '../../common/Tooltip';
import type { DecisionPlan, VerdictAction } from './decisionEffects';
import { SHORTCUT_ROWS } from './DecisionReview.keymap';

export { pushedRejectWarning, toolArgumentsText } from './decisionEffects';

/**
 * A keycap. There is no `Kbd` in common/ yet (requested in the lane report);
 * this is token-only so it can move there unchanged.
 */
export function Kbd({ children, onAccent = false }: { children: ReactNode; onAccent?: boolean }) {
  const { colors } = useTheme();
  return (
    <kbd
      aria-hidden="true"
      style={{
        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
        minWidth: type.micro.fontSize + space.xs, boxSizing: 'border-box',
        fontFamily: font.mono, ...type.micro,
        padding: `0 ${space.xs}px`,
        borderRadius: radius.xs,
        border: `1px solid ${onAccent ? colors.textOnCyan : colors.border}`,
        background: onAccent ? 'transparent' : colors.fillSubtle,
        color: onAccent ? colors.textOnCyan : colors.textDim,
        opacity: onAccent ? 0.7 : 1,
      }}
    >
      {children}
    </kbd>
  );
}

/** Bare and ghost share one hit size: the bare variant's own 2px 5px pad
 *  would leave Later under the 24px target (WCAG 2.5.8). */
const BAR_PAD = `${space.sm}px ${space.lg}px`;

interface BarProps {
  plan: DecisionPlan;
  /** The action whose inline field is open, if any. */
  fieldAction: VerdictAction | null;
  fieldText: string;
  fieldRef: RefObject<HTMLTextAreaElement>;
  onFieldText: (text: string) => void;
  onCloseField: () => void;
  onSubmitField: () => void;
  /** A click on any verdict. The review decides: open a field, confirm, or hold. */
  onAction: (action: VerdictAction) => void;
  onLater: () => void;
  shortcutsOpen: boolean;
  onToggleShortcuts: () => void;
  /** A warning to show above an open field (e.g. already-pushed work). */
  fieldWarning?: string | null;
  disabled?: boolean;
}

/** Ghost order, left to right: bar order reads as the numbers do. With a
 *  primary, the highest key sits furthest from it (…3, 2, [1]); without one
 *  (a choice), "None of these" leads and the options run 1, 2, 3. */
function orderGhosts(ghosts: VerdictAction[], hasPrimary: boolean): VerdictAction[] {
  const rank = (a: VerdictAction) => (a.key === '0' ? 10 : Number(a.key ?? 99));
  if (hasPrimary) return [...ghosts].sort((a, b) => rank(b) - rank(a));
  const none = ghosts.filter(a => a.key === '0');
  return [...none, ...ghosts.filter(a => a.key !== '0').sort((a, b) => rank(a) - rank(b))];
}

export function DecisionActionBar({
  plan, fieldAction, fieldText, fieldRef, onFieldText, onCloseField, onSubmitField,
  onAction, onLater, shortcutsOpen, onToggleShortcuts, fieldWarning, disabled = false,
}: BarProps) {
  const { colors } = useTheme();
  // While a field is open its send button is the one tint (D8); the plan's
  // own primary steps down to ghost.
  const roleOf = (a: VerdictAction): VerdictAction['role'] =>
    fieldAction ? (a.id === fieldAction.id ? 'primary' : a.role === 'danger' ? 'danger' : 'ghost') : a.role;
  const dangers = plan.actions.filter(a => roleOf(a) === 'danger');
  const primaryAction = plan.actions.find(a => roleOf(a) === 'primary') ?? null;
  const ghosts = orderGhosts(plan.actions.filter(a => roleOf(a) === 'ghost'), !!primaryAction);
  const fieldRequired = fieldAction?.field === 'input' || fieldAction?.field === 'edit';
  const canSubmitField = !fieldRequired || !!fieldText.trim();

  const verdictButton = (a: VerdictAction) => {
    const role = roleOf(a);
    const isField = fieldAction?.id === a.id;
    const label = isField ? (a.fieldSubmitLabel ?? a.label) : a.label;
    const showReturn = role === 'primary' && (isField || a.returnDefault);
    const style: CSSProperties = role === 'danger'
      ? ({
        '--pa-btn-fg': colors.danger, '--pa-btn-fg-hover': colors.danger,
        '--pa-btn-pad': BAR_PAD,
      } as CSSProperties)
      : ({ '--pa-btn-pad': role === 'primary' ? `${space.sm}px ${space.xl}px` : BAR_PAD } as CSSProperties);
    return (
      <Button
        key={a.id}
        colors={colors}
        variant={role === 'primary' ? 'primary' : 'ghost'}
        type="button"
        flashSuccess={false}
        disabled={disabled || (isField && !canSubmitField)}
        aria-keyshortcuts={[a.key, showReturn ? 'Enter' : null].filter(Boolean).join(' ') || undefined}
        data-action={a.id}
        onClick={() => (isField ? onSubmitField() : onAction(a))}
        style={{ ...style, fontFamily: font.body }}
      >
        {label}
        {a.key && <Kbd onAccent={role === 'primary'}>{a.key}</Kbd>}
        {showReturn && <Kbd onAccent>↩</Kbd>}
      </Button>
    );
  };

  return (
    <div tabIndex={-1} style={{ position: 'relative', display: 'flex', flexDirection: 'column', gap: space.lg, width: '100%', minWidth: 0, outline: 'none' }}>
      {shortcutsOpen && <ShortcutSheet />}
      {fieldAction && (
        <div className="pa-enter" style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
          {fieldWarning && (
            <div role="alert" style={{ ...type.small, color: colors.warning }}>{fieldWarning}</div>
          )}
          <div style={{
            display: 'flex', alignItems: 'flex-end', gap: space.md,
            padding: `${space.sm}px ${space.sm}px ${space.sm}px ${space.xl}px`,
            background: colors.inputBg, border: `1px solid ${colors.borderHi}`, borderRadius: radius.md,
          }}>
            <label htmlFor={`field-${fieldAction.id}`} className="sr-only">{fieldAction.fieldPlaceholder ?? fieldAction.label}</label>
            <textarea
              id={`field-${fieldAction.id}`}
              ref={fieldRef}
              rows={fieldAction.field === 'edit' ? 4 : 1}
              value={fieldText}
              placeholder={fieldAction.fieldPlaceholder}
              onChange={e => onFieldText(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); onCloseField(); return; }
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing && fieldAction.field !== 'edit') {
                  e.stopPropagation(); e.preventDefault();
                  if (canSubmitField) onSubmitField();
                }
              }}
              style={{
                flex: 1, minWidth: 0, boxSizing: 'border-box', resize: 'none', border: 'none', outline: 'none',
                background: 'transparent', color: colors.text, fontFamily: font.body, ...type.body,
                padding: `${space.xs}px 0`,
              }}
            />
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={onCloseField}
              style={{ '--pa-btn-pad': BAR_PAD, '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': colors.fillHover, fontFamily: font.body } as CSSProperties}
            >
              Cancel <Kbd>esc</Kbd>
            </Button>
          </div>
        </div>
      )}
      <div role="toolbar" aria-label="Answer this decision" style={{ display: 'flex', alignItems: 'center', gap: space.md, flexWrap: 'wrap', minWidth: 0 }}>
        <Tooltip content="Keyboard shortcuts" shortcut="?">
          <Button
            colors={colors}
            variant="bare"
            type="button"
            aria-label="Keyboard shortcuts"
            aria-expanded={shortcutsOpen}
            onClick={onToggleShortcuts}
            style={{ '--pa-btn-pad': BAR_PAD, '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': colors.fillHover, '--pa-btn-bg-active': colors.fillActive, fontFamily: font.mono } as CSSProperties}
          >
            ?
          </Button>
        </Tooltip>
        {dangers.map(verdictButton)}
        <span style={{ flex: 1 }} />
        <Button
          colors={colors}
          variant="bare"
          type="button"
          aria-keyshortcuts="H"
          data-action="later"
          disabled={disabled}
          onClick={onLater}
          style={{ '--pa-btn-pad': BAR_PAD, '--pa-btn-fg': colors.textMuted, '--pa-btn-fg-hover': colors.text, '--pa-btn-bg-hover': colors.fillHover, '--pa-btn-bg-active': colors.fillActive, fontFamily: font.body } as CSSProperties}
        >
          {plan.later.label}
          <Kbd>H</Kbd>
        </Button>
        {ghosts.map(verdictButton)}
        {primaryAction && verdictButton(primaryAction)}
      </div>
    </div>
  );
}

/** The `?` sheet: a small popover above the bar's leading edge. */
export function ShortcutSheet() {
  const { colors } = useTheme();
  return (
    <div
      role="dialog"
      aria-label="Keyboard shortcuts"
      className="pa-enter"
      style={{
        position: 'absolute', left: 0, bottom: `calc(100% + ${space.md}px)`, zIndex: 2,
        width: 'min(320px, 100%)', boxSizing: 'border-box',
        padding: space.xl, borderRadius: radius.md,
        background: colors.surfaceHi, border: `1px solid ${colors.border}`,
        boxShadow: colors.elevationOverlay,
        display: 'grid', gridTemplateColumns: 'max-content 1fr', columnGap: space.xl, rowGap: space.sm,
        alignItems: 'center', fontFamily: font.body,
      }}
    >
      {SHORTCUT_ROWS.map(row => (
        <div key={row.label} style={{ display: 'contents' }}>
          <span style={{ display: 'inline-flex', gap: space.xs }}>
            {row.keys.map(k => <Kbd key={k}>{k}</Kbd>)}
          </span>
          <span style={{ ...type.small, color: colors.textMuted }}>{row.label}</span>
        </div>
      ))}
    </div>
  );
}
