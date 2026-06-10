import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { MarkdownContent } from '../chat/MarkdownContent';

interface GmailReadResultProps {
  subject?: string;
  from?: string;
  to?: string;
  date?: string;
  body?: string;
}

export function GmailReadResult({ subject, from, to, date, body }: GmailReadResultProps) {
  const { colors } = useTheme();
  return (
    <div className="space-y-2">
      <div className="space-y-0.5">
        {subject && (
          <div className="text-[12px]" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>{subject}</div>
        )}
        <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>
          {from && <span>From: {from}</span>}
          {to && <span>To: {to}</span>}
          {date && <span>{date}</span>}
        </div>
      </div>
      {body && (
        <div className="pt-2 text-[12px] leading-relaxed" style={{ borderTop: `1px solid ${colors.border}`, fontFamily: font.body, color: colors.text }}>
          <MarkdownContent content={body} />
        </div>
      )}
    </div>
  );
}
