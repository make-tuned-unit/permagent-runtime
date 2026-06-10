import { FiX, FiFile } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface AttachmentChipProps {
  filename: string;
  onRemove: () => void;
}

export function AttachmentChip({ filename, onRemove }: AttachmentChipProps) {
  const { colors } = useTheme();
  return (
    <div
      className="flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] max-w-[200px]"
      style={{ fontFamily: font.mono, color: colors.text, backgroundColor: colors.surface, border: `1px solid ${colors.border}` }}
    >
      <FiFile size={12} className="shrink-0" style={{ color: colors.textMuted }} />
      <span className="truncate">{filename}</span>
      <button
        onClick={onRemove}
        className="transition shrink-0 ml-0.5"
        style={{ color: colors.textMuted }}
        onMouseEnter={e => (e.currentTarget.style.color = colors.danger)}
        onMouseLeave={e => (e.currentTarget.style.color = colors.textMuted)}
      >
        <FiX size={12} />
      </button>
    </div>
  );
}
