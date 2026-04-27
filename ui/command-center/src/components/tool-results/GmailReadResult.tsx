import { MarkdownContent } from '../chat/MarkdownContent';

interface GmailReadResultProps {
  subject?: string;
  from?: string;
  to?: string;
  date?: string;
  body?: string;
}

export function GmailReadResult({ subject, from, to, date, body }: GmailReadResultProps) {
  return (
    <div className="space-y-2">
      <div className="space-y-0.5">
        {subject && (
          <div className="text-[12px] font-mono font-semibold text-dark-text">{subject}</div>
        )}
        <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-[10px] font-mono text-dark-muted">
          {from && <span>From: {from}</span>}
          {to && <span>To: {to}</span>}
          {date && <span>{date}</span>}
        </div>
      </div>
      {body && (
        <div className="border-t border-dark-border/50 pt-2 text-[12px] font-mono text-dark-text leading-relaxed">
          <MarkdownContent content={body} />
        </div>
      )}
    </div>
  );
}
