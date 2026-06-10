import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { CodeBlock } from './CodeBlock';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function MarkdownContent({ content }: { content: string }) {
  const { colors } = useTheme();
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        code({ className, children, ...props }) {
          const match = /language-(\w+)/.exec(className || '');
          const inline = !match;
          if (inline) {
            return (
              <code
                className="rounded px-1 py-0.5 text-[12px]"
                style={{ backgroundColor: colors.codeBg, color: colors.codeText, wordBreak: 'break-all', fontFamily: font.mono }}
                {...props}
              >
                {children}
              </code>
            );
          }
          return <CodeBlock language={match[1]} code={String(children).replace(/\n$/, '')} />;
        },
        pre({ children }) {
          return <>{children}</>;
        },
        a({ href, children }) {
          return (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="underline transition-colors"
              style={{ color: colors.cyan }}
              onMouseEnter={e => (e.currentTarget.style.opacity = '0.8')}
              onMouseLeave={e => (e.currentTarget.style.opacity = '1')}
            >
              {children}
            </a>
          );
        },
        table({ children }) {
          return (
            <div className="overflow-x-auto my-2">
              <table className="min-w-full text-[11px]" style={{ border: `1px solid ${colors.border}` }}>{children}</table>
            </div>
          );
        },
        th({ children }) {
          return (
            <th
              className="px-2 py-1 text-left"
              style={{ border: `1px solid ${colors.border}`, backgroundColor: colors.surface, color: colors.textMuted, fontFamily: font.display, fontWeight: 600 }}
            >
              {children}
            </th>
          );
        },
        td({ children }) {
          return <td className="px-2 py-1" style={{ border: `1px solid ${colors.border}` }}>{children}</td>;
        },
        ul({ children }) {
          return <ul className="list-disc pl-4 my-1 space-y-0.5">{children}</ul>;
        },
        ol({ children }) {
          return <ol className="list-decimal pl-4 my-1 space-y-0.5">{children}</ol>;
        },
        blockquote({ children }) {
          return (
            <blockquote
              className="pl-3 my-2 italic"
              style={{ borderLeft: `2px solid ${colors.cyan}66`, color: colors.textMuted }}
            >
              {children}
            </blockquote>
          );
        },
        p({ children }) {
          return <p className="my-1">{children}</p>;
        },
        hr() {
          return <hr className="my-2" style={{ borderColor: colors.border }} />;
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
}
