/**
 * Home's door into edit mode.
 *
 * Behind this one control sits the whole customize system — drag to reorder,
 * a corner grip to resize, add and remove cards, reset to default, and an
 * in-context help banner that is genuinely well written. All of it was gated
 * behind a bare pencil glyph whose only label was a hover `title`, which is
 * not a label: a control that is hovered is a control someone already decided
 * to investigate, and nothing on the page gave them a reason to.
 *
 * So the word is on the button at rest. Not a new feature — the same feature,
 * finally saying what it is.
 */

import { FiCheck, FiEdit2 } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';

export function CustomizeButton({
  editing,
  onToggle,
  colors,
}: {
  editing: boolean;
  onToggle: () => void;
  colors: ThemeColors;
}) {
  return (
    <button
      type="button"
      data-testid="dashboard-customize"
      onClick={onToggle}
      // The title stays as the longer form; it is the elaboration now, not the
      // only thing that names the control.
      title={editing ? 'Finish arranging your cards' : 'Rearrange, resize, add or remove cards'}
      style={{
        display: 'flex', alignItems: 'center', gap: 6,
        padding: '5px 14px',
        borderRadius: radius.md,
        border: `1px solid ${editing ? colors.cyan : colors.border}`,
        background: editing ? colors.cyanSoft : colors.surface,
        color: editing ? colors.cyan : colors.textMuted,
        fontFamily: font.body, fontSize: 12, fontWeight: 500,
        cursor: 'pointer', transition: 'all 150ms ease',
      }}
    >
      {editing ? <><FiCheck size={14} /> Done</> : <><FiEdit2 size={14} /> Customize</>}
    </button>
  );
}
