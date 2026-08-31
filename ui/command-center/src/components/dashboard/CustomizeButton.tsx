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

import { type CSSProperties } from 'react';
import { FiCheck, FiEdit2 } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { Button } from '../common/Button';

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
    <Button
      colors={colors}
      type="button"
      data-testid="dashboard-customize"
      onClick={onToggle}
      // The title stays as the longer form; it is the elaboration now, not the
      // only thing that names the control.
      title={editing ? 'Finish arranging your cards' : 'Rearrange, resize, add or remove cards'}
      style={{
        '--pa-btn-bg': editing ? colors.cyanSoft : colors.surface,
        '--pa-btn-fg': editing ? colors.cyan : colors.textMuted,
        '--pa-btn-border': editing ? colors.cyan : colors.border,
        '--pa-btn-bg-hover': editing ? colors.cyanSoft : colors.surfaceHi,
        '--pa-btn-fg-hover': editing ? colors.cyan : colors.text,
        '--pa-btn-border-hover': editing ? colors.cyan : colors.borderHi,
        '--pa-btn-bg-active': editing ? colors.cyanGlow : colors.surface,
        '--pa-btn-pad': '5px 14px',
        '--pa-btn-radius': `${radius.md}px`,
        fontFamily: font.body,
        fontSize: 12,
      } as CSSProperties}
    >
      {/* The primitive wraps its children in one span, so the icon and the word
          need their own row to keep the 6px they have always sat at. */}
      <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        {editing ? <><FiCheck size={14} /> Done</> : <><FiEdit2 size={14} /> Customize</>}
      </span>
    </Button>
  );
}
