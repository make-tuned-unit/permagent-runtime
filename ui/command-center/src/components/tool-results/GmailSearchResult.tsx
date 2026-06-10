import { FiMail } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface EmailEntry {
  id?: string;
  from?: string;
  subject?: string;
  snippet?: string;
  date?: string;
  unread?: boolean;
}

interface GmailSearchResultProps {
  emails: EmailEntry[];
}

export function GmailSearchResult({ emails }: GmailSearchResultProps) {
  const { colors } = useTheme();
  if (!emails || emails.length === 0) {
    return <div className="text-[11px]" style={{ color: colors.textMuted, fontFamily: font.mono }}>No emails found.</div>;
  }

  return (
    <div className="space-y-1">
      {emails.map((email, i) => (
        <div
          key={email.id || i}
          className="flex items-start gap-2 rounded-md px-2 py-1.5"
          style={{ backgroundColor: email.unread ? colors.surface : 'transparent', border: `1px solid ${colors.border}` }}
        >
          <div className="mt-0.5 shrink-0">
            <FiMail size={13} style={{ color: email.unread ? colors.cyan : colors.textMuted }} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <span
                className="text-[11px] truncate"
                style={{ fontFamily: font.mono, color: email.unread ? colors.text : colors.textMuted, fontWeight: email.unread ? 600 : 400 }}
              >
                {email.from || 'Unknown'}
              </span>
              {email.date && (
                <span className="text-[9px] shrink-0" style={{ fontFamily: font.mono, color: colors.textDim }}>{email.date}</span>
              )}
            </div>
            <div className="text-[11px] truncate" style={{ fontFamily: font.mono, color: colors.text }}>{email.subject || '(no subject)'}</div>
            {email.snippet && (
              <div className="text-[10px] truncate" style={{ fontFamily: font.mono, color: colors.textMuted }}>{email.snippet}</div>
            )}
          </div>
          {email.unread && (
            <div className="w-1.5 h-1.5 rounded-full shrink-0 mt-1.5" style={{ backgroundColor: colors.cyan }} />
          )}
        </div>
      ))}
    </div>
  );
}
