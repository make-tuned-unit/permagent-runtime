import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface BashOutputResultProps {
  stdout?: string;
  stderr?: string;
  exitCode?: number;
}

export function BashOutputResult({ stdout, stderr, exitCode }: BashOutputResultProps) {
  const { colors } = useTheme();
  return (
    <div className="space-y-1.5">
      {stdout && (
        <div>
          <div className="text-[9px] uppercase mb-0.5" style={{ fontFamily: font.mono, color: colors.textMuted }}>stdout</div>
          <pre
            className="rounded p-2 text-[11px] overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap"
            style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.success }}
          >
            {stdout}
          </pre>
        </div>
      )}
      {stderr && (
        <div>
          <div className="text-[9px] uppercase mb-0.5" style={{ fontFamily: font.mono, color: colors.textMuted }}>stderr</div>
          <pre
            className="rounded p-2 text-[11px] overflow-x-auto max-h-[200px] overflow-y-auto whitespace-pre-wrap"
            style={{ fontFamily: font.mono, backgroundColor: colors.codeBg, color: colors.danger }}
          >
            {stderr}
          </pre>
        </div>
      )}
      {exitCode !== undefined && (
        <div className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>
          exit code: <span style={{ color: exitCode === 0 ? colors.success : colors.danger }}>{exitCode}</span>
        </div>
      )}
    </div>
  );
}
