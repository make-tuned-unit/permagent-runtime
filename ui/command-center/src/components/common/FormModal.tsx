/**
 * FormModal — the third and last modal shape: a modal you fill in.
 *
 * `DetailModal` is a thing you read (title, body, close) and `ConfirmDialog` is
 * a question you answer. Neither fits a modal whose job is to collect a value
 * and submit it, and the app grew six of those independently — Add a card, New
 * automation, Configure provider, Add custom provider, Meeting recorder, Merge
 * person — each with its own scrim, its own action row, and its own idea of
 * what a failed submit looks like.
 *
 * It is built ON `DetailModal` rather than beside it, exactly as `ConfirmDialog`
 * is, so the a11y floor is inherited and cannot drift: one scrim, one focus
 * trap, one Escape, one `role="dialog"` + `aria-labelledby`, focus returned to
 * whatever opened it.
 *
 * What it adds, and what the six were each missing some of:
 *  - IT IS A REAL `<form>`. Enter submits from any field. Six hand-rolled
 *    modals with a `<div>` and an onClick meant the most ordinary keyboard
 *    gesture there is did nothing, in the one place a user is typing.
 *  - THE SUBMIT RUNS THROUGH THE `Button` CONTRACT. The work is on `onSubmit`,
 *    so the click handler cannot be awaited — the button takes `pending` from
 *    this shell's own in-flight flag, which is the form-submit shape `Button`
 *    documents, and gets the visible pending floor with it.
 *  - A FAILED SUBMIT KEEPS THE MODAL OPEN, WITH THE REASON ON IT. `onSubmit`
 *    resolving `false` (the app's convention) or throwing both land as a
 *    sentence above the action row. Closing is the caller's: it owns the state
 *    that decides whether this is mounted, and it closes on success only.
 *  - CANCEL IS A PEER of the submit, not a de-emphasised escape hatch.
 */

import { useCallback, useId, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { font, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from './Button';
import { DetailModal } from './DetailModal';

export interface FormModalProps {
  /** Names the thing being made or changed — "New automation", not "Form". */
  title: string;
  /** Optional status pill beside the title, same contract as `DetailModal`. */
  badge?: { label: string; color: string; bg: string } | null;
  /** Panel width, passed through to `DetailModal`. A form with a date picker
   *  and a timezone field is a different size from a two-field one. */
  width?: number | string;
  /** The fields. Rendered inside the `<form>`, so Enter submits from any of
   *  them and a `required` attribute is enforced by the browser. */
  children: ReactNode;
  /** The verb — "Create", "Save", "Merge". Never "OK" or "Submit". */
  submitLabel: string;
  cancelLabel?: string;
  /** Opening words of the failure sentence, e.g. "Couldn't create it". */
  failureLabel?: string;
  /** Grey the submit out while the form is incomplete. The reason belongs on
   *  the control that is disabled — see `disabledReason`. */
  submitDisabled?: boolean;
  /** Why the submit is disabled, said out loud. A disabled control with no
   *  explanation is not feedback; it is a dead-looking button. */
  disabledReason?: string;
  /** Does the work. May return a promise; resolving `false` means it failed,
   *  matching the `Button` contract and the store's convention. Resolve
   *  anything else (or nothing) for success — and close from the caller. */
  onSubmit: () => unknown;
  onCancel: () => void;
  /** An extra control on the left of the action row (a "Test" or a "Remove"). */
  secondaryAction?: ReactNode;
}

export function FormModal({
  title,
  badge,
  width,
  children,
  submitLabel,
  cancelLabel = 'Cancel',
  failureLabel = "That didn't go through",
  submitDisabled,
  disabledReason,
  onSubmit,
  onCancel,
  secondaryAction,
}: FormModalProps) {
  const { colors } = useTheme();
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const formId = useId();

  const run = useCallback(async () => {
    if (pending) return;
    setPending(true);
    setFailure(null);
    try {
      const result = await onSubmit();
      if (result === false) setFailure(failureLabel);
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      setFailure(detail ? `${failureLabel} — ${detail}` : failureLabel);
    } finally {
      setPending(false);
    }
  }, [onSubmit, failureLabel, pending]);

  return (
    <DetailModal
      title={title}
      badge={badge}
      width={width}
      onClose={onCancel}
      footer={<>
        {secondaryAction}
        <div style={{ flex: 1 }} />
        <Button colors={colors} variant="ghost" onClick={onCancel} disabled={pending}>
          {cancelLabel}
        </Button>
        {/* `type="submit"` + `form` so the button lives in the footer and still
            submits the form in the body — and so Enter and the click are one
            code path rather than two that can disagree. */}
        <Button
          colors={colors}
          variant="primary"
          type="submit"
          form={formId}
          pending={pending}
          disabled={submitDisabled || pending}
          title={submitDisabled ? disabledReason : undefined}
          style={{ '--pa-btn-weight': 600 } as CSSProperties}
        >
          {submitLabel}
        </Button>
      </>}
    >
      <form
        id={formId}
        onSubmit={e => { e.preventDefault(); void run(); }}
        style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
      >
        {children}
      </form>
      {failure && (
        <div
          role="alert"
          style={{
            marginTop: 12, fontFamily: font.body, ...type.caption,
            color: colors.danger, lineHeight: 1.5,
          }}
        >
          {failure}
        </div>
      )}
      {submitDisabled && disabledReason && !failure && (
        <div style={{
          marginTop: 12, fontFamily: font.body, ...type.caption,
          color: colors.textDim, lineHeight: 1.5,
        }}>
          {disabledReason}
        </div>
      )}
    </DetailModal>
  );
}
