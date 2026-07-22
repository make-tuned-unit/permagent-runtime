import { FiChevronRight } from 'react-icons/fi';
import { useTheme } from '../../styles/useTheme';

export function CycleTabsButton({ pane, onCycle }: { pane: 'terminal' | 'browser'; onCycle: () => void }) {
  const { colors } = useTheme();
  return (
    <button
      type="button"
      onClick={onCycle}
      className="px-2.5 py-1.5 transition-colors"
      style={{ color: colors.textMuted }}
      onMouseEnter={event => { event.currentTarget.style.color = colors.cyan; }}
      onMouseLeave={event => { event.currentTarget.style.color = colors.textMuted; }}
      aria-label={`Cycle ${pane} tabs`}
      title={`Cycle ${pane} tabs (Tab when pane is selected)`}
    >
      <FiChevronRight size={13} aria-hidden="true" />
    </button>
  );
}
