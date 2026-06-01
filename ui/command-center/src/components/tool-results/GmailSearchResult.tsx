import { FiMail } from 'react-icons/fi';
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
    return <div className="text-[11px] text-dark-muted font-mono">No emails found.</div>;
  }

  return (
    <div className="space-y-1">
      {emails.map((email, i) => (
        <div
          key={email.id || i}
          className="flex items-start gap-2 rounded-md px-2 py-1.5 border border-dark-border/50"
          style={{ backgroundColor: email.unread ? colors.surface : 'transparent' }}
        >
          <div className="mt-0.5 shrink-0">
            <FiMail size={13} className={email.unread ? 'text-accent' : 'text-dark-muted'} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-2">
              <span className={`text-[11px] font-mono truncate ${email.unread ? 'text-dark-text font-semibold' : 'text-dark-muted'}`}>
                {email.from || 'Unknown'}
              </span>
              {email.date && (
                <span className="text-[9px] font-mono text-dark-muted/60 shrink-0">{email.date}</span>
              )}
            </div>
            <div className="text-[11px] font-mono text-dark-text truncate">{email.subject || '(no subject)'}</div>
            {email.snippet && (
              <div className="text-[10px] font-mono text-dark-muted truncate">{email.snippet}</div>
            )}
          </div>
          {email.unread && (
            <div className="w-1.5 h-1.5 rounded-full bg-accent shrink-0 mt-1.5" />
          )}
        </div>
      ))}
    </div>
  );
}
