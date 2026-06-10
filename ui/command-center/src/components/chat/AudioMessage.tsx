import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface AudioMessageProps {
  src: string;
  filename: string;
}

export function AudioMessage({ src, filename }: AudioMessageProps) {
  const { colors } = useTheme();
  return (
    <div className="flex flex-col gap-1 my-1">
      <span className="text-[10px] truncate max-w-[300px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>{filename}</span>
      <audio controls preload="metadata" className="max-w-[400px] h-8">
        <source src={src} />
      </audio>
    </div>
  );
}
