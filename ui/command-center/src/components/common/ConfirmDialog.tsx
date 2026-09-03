/**
 * ConfirmDialog — the app's Tier-3 destructive confirmation.
 *
 * The destructive-action ruling gives each risk tier one pattern: reversible
 * actions get no confirmation at all (an undo affordance instead), destructive
 * but recoverable ones get a two-step confirm inline on the row, and only the
 * unrecoverable / high-blast-radius ones — rotating a live credential, deleting
 * something with no undo — earn a full-attention modal. This is that modal, and
 * the only one.
 *
 * It replaces three native OS dialogs, which bypassed the theme, the focus ring
 * and the interface voice all at once. What is different here is not the chrome:
 *  - It states the SPECIFIC consequence. "Are you sure?" tells the user nothing
 *    they didn't already know; "ingestion fails with 401 until you redeploy"
 *    is the sentence they actually need.
 *  - The affirmative runs through the `Button` contract — a pending phase you
 *    can see, and no success tick unless the work landed.
 *  - A failure keeps the dialog open with the reason on it. An OS dialog closes
 *    the moment you click, so a failed action looked exactly like a done one.
 *
 * It is built on `DetailModal`, the app's one modal shell, so it inherits that
 * shell's scrim, Escape handling and dialog semantics rather than growing a
 * second set that can drift.
 *
 * Boring and correct: no red scare-styling, no persuasion. The consequence
 * sentence carries the weight, and Cancel is a peer of the affirmative rather
 * than a de-emphasised escape hatch.
 */

import { useCallback, useState } from 'react';
import type { ReactNode } from 'react';
import { font, space, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from './Button';
import { DetailModal } from './DetailModal';

export interface ConfirmDialogProps {
  /** The question, named concretely — "Delete \"Nightly sweep\"?" */
  title: string;
  /** What happens if they go through with it. The whole reason this tier gets
   *  a modal: state the consequence, not a generic warning. */
  consequence: ReactNode;
  /** The affirmative's label — the verb, never "OK" or "Yes". */
  confirmLabel: string;
  cancelLabel?: string;
  /** Opening words of the failure sentence, e.g. "Couldn't delete it". */
  failureLabel?: string;
  /** Runs the action. May return a promise; resolving `false` means it failed,
   *  matching the `Button` contract. Closing is the caller's — it holds the
   *  state that decides whether the dialog is mounted at all. */
  onConfirm: () => unknown;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  consequence,
  confirmLabel,
  cancelLabel = 'Cancel',
  failureLabel = "That didn't go through",
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { colors } = useTheme();
  const [failure, setFailure] = useState<string | null>(null);

  const run = useCallback(async () => {
    setFailure(null);
    try {
      const result = await onConfirm();
      if (result === false) { setFailure(failureLabel); return false; }
      return result;
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      setFailure(detail ? `${failureLabel} — ${detail}` : failureLabel);
      return false;
    }
  }, [onConfirm, failureLabel]);

  return (
    <DetailModal
      title={title}
      onClose={onCancel}
      footer={<>
        <Button colors={colors} variant="ghost" onClick={onCancel}>{cancelLabel}</Button>
        <Button colors={colors} variant="primary" onClick={run}>{confirmLabel}</Button>
      </>}
    >
      <div style={{
        fontFamily: font.body, ...type.small,
        color: colors.textMuted, lineHeight: 1.55, whiteSpace: 'pre-line',
      }}>
        {consequence}
      </div>
      {failure && (
        <div
          role="alert"
          style={{
            marginTop: space.xl, fontFamily: font.body, ...type.caption,
            color: colors.danger, lineHeight: 1.5,
          }}
        >
          {failure}
        </div>
      )}
    </DetailModal>
  );
}
