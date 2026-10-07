import { useEffect, useRef, type FormEvent, type ReactNode } from 'react';
import { space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { PrimaryButton, WizardHeading, WizardSubhead } from './atoms';

export interface StepPrimary {
  label: string;
  disabled?: boolean;
  pending?: boolean;
  /** Label while pending (default: the label with an ellipsis). */
  pendingLabel?: string;
  /** Start focus on the primary button (a step with nothing to fill in). */
  autoFocus?: boolean;
}

interface Props {
  title: ReactNode;
  why?: ReactNode;
  /** Above the title (the Welcome step's mark). */
  lead?: ReactNode;
  children?: ReactNode;
  primary: StepPrimary;
  /** The primary action — also what Enter does when it is enabled. */
  onPrimary: () => unknown;
  /** Footer, left side: Skip / Decide later / Continue anyway. */
  secondary?: ReactNode;
  /** Content column width. */
  width?: number;
}

/**
 * One step of first-run setup: a `<form>` with a scrolling body and a pinned
 * footer.
 *
 * - The body centres with `margin: auto` inside a scroll container, so a step
 *   taller than the window scrolls from its top instead of losing its title and
 *   its button off both edges (the old `justify-content: center` clip).
 * - The footer is outside the scroll, so the way forward is always on screen at
 *   the minimum window size.
 * - Enter submits the form, which runs the primary action only when it is
 *   enabled; Escape is the shell's Back.
 * - On mount, focus moves to the step's `[data-autofocus]` field, else to its
 *   title, so keyboard and screen-reader users start inside the new step.
 */
export function StepFrame({ title, why, lead, children, primary, onPrimary, secondary, width = 520 }: Props) {
  const { colors } = useTheme();
  const formRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    const form = formRef.current;
    if (!form) return;
    const target = form.querySelector<HTMLElement>('[data-autofocus]')
      ?? form.querySelector<HTMLElement>('[data-step-heading]');
    target?.focus({ preventScroll: true });
  }, []);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (primary.disabled || primary.pending) return;
    void onPrimary();
  };

  return (
    <form ref={formRef} onSubmit={submit} noValidate aria-busy={primary.pending || undefined} style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', display: 'flex', flexDirection: 'column', padding: `${space.xl}px ${space.huge}px` }}>
        <div style={{ margin: 'auto', width: '100%', maxWidth: width, paddingTop: space.md, paddingBottom: space.huge }}>
          {lead}
          <WizardHeading>{title}</WizardHeading>
          {why && <WizardSubhead>{why}</WizardSubhead>}
          <div style={{ marginTop: space.huge, display: 'flex', flexDirection: 'column', gap: space.xl }}>
            {children}
          </div>
        </div>
      </div>
      <div style={{ flexShrink: 0, borderTop: `1px solid ${colors.border}`, padding: `${space.xl}px ${space.huge}px` }}>
        <div style={{ margin: '0 auto', maxWidth: Math.max(width, 560), display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: space.md, minWidth: 0 }}>{secondary}</div>
          {/* Pending is shown in the label and on the form (aria-busy), not by
              disabling the button: a focused button that disables itself
              drops keyboard focus to the page, and the next Enter goes
              nowhere. The submit handler ignores presses while pending. */}
          <PrimaryButton type="submit" disabled={primary.disabled} autoFocus={primary.autoFocus}>
            {primary.pending ? (primary.pendingLabel ?? `${primary.label}…`) : primary.label}
          </PrimaryButton>
        </div>
      </div>
    </form>
  );
}
