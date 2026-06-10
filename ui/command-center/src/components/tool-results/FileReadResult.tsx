import { FiFile } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface FileReadResultProps {
  filename?: string;
  content?: string;
  truncated?: boolean;
}

export function FileReadResult({ filename, content, truncated }: FileReadResultProps) {
  const { colors } = useTheme();
  const preview = content ? content.slice(0, 500) : '';

  return (
    <div className="space-y-1.5">
      {filename && (
        <div className="flex items-center gap-1.5 text-[11px]" style={{ fontFamily: font.mono, color: colors.text }}>
          <FiFile size={12} style={{ color: colors.textMuted }} />
          {filename}
        </div>
      )}
      {preview && (
        <pre
          className="rounded p-2 text-[10px] overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap"
          style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.text }}
        >
          {preview}
          {(truncated || (content && content.length > 500)) && (
            <span style={{ color: colors.textMuted }}>... (truncated)</span>
          )}
        </pre>
      )}
    </div>
  );
}
