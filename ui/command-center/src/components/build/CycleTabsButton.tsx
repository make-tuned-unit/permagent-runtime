import { type CSSProperties } from 'react';
import { FiChevronRight } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export function CycleTabsButton({ pane, onCycle }: { pane: 'terminal' | 'browser'; onCycle: () => void }) {
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      onClick={onCycle}
      aria-label={`Cycle ${pane} tabs`}
      title={`Cycle ${pane} tabs (Tab when pane is selected)`}
      style={{
        '--pa-btn-fg': colors.textMuted,
        '--pa-btn-fg-hover': colors.cyan,
        '--pa-btn-pad': '6px 10px',
      } as CSSProperties}
    >
      <FiChevronRight size={13} aria-hidden="true" />
    </Button>
  );
}
