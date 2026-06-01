import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { CodeBlock } from './CodeBlock';
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
              <code className="rounded bg-black/30 px-1 py-0.5 text-accent text-[12px]" {...props}>
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
            <a href={href} target="_blank" rel="noopener noreferrer" className="text-accent underline hover:text-accent/80">
              {children}
            </a>
          );
        },
        table({ children }) {
          return (
            <div className="overflow-x-auto my-2">
              <table className="min-w-full text-[11px] border border-dark-border">{children}</table>
            </div>
          );
        },
        th({ children }) {
          return <th className="border border-dark-border px-2 py-1 text-left font-semibold text-dark-muted" style={{ backgroundColor: colors.surface }}>{children}</th>;
        },
        td({ children }) {
          return <td className="border border-dark-border px-2 py-1">{children}</td>;
        },
        ul({ children }) {
          return <ul className="list-disc pl-4 my-1 space-y-0.5">{children}</ul>;
        },
        ol({ children }) {
          return <ol className="list-decimal pl-4 my-1 space-y-0.5">{children}</ol>;
        },
        blockquote({ children }) {
          return <blockquote className="border-l-2 border-accent/40 pl-3 my-2 text-dark-muted italic">{children}</blockquote>;
        },
        p({ children }) {
          return <p className="my-1">{children}</p>;
        },
        hr() {
          return <hr className="border-dark-border my-2" />;
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
}
