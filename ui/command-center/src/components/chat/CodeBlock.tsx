import { useState, useEffect, useCallback } from 'react';
import { FiCopy, FiCheck } from 'react-icons/fi';

export function CodeBlock({ language, code }: { language: string; code: string }) {
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    import('shiki').then(({ codeToHtml }) => {
      codeToHtml(code, { lang: language, theme: 'github-dark' })
        .then(result => {
          if (!cancelled) setHtml(result);
        })
        .catch(() => {
          // Language not supported, leave unhighlighted
        });
    });
    return () => { cancelled = true; };
  }, [code, language]);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [code]);

  return (
    <div className="relative group rounded-lg bg-[#0d1117] border border-dark-border my-2 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-1 bg-[#161b22] border-b border-dark-border">
        <span className="text-[10px] font-mono text-dark-muted">{language}</span>
        <button
          onClick={handleCopy}
          className="flex items-center gap-1 text-[10px] font-mono text-dark-muted hover:text-dark-text transition opacity-0 group-hover:opacity-100"
        >
          {copied ? <><FiCheck size={11} className="text-emerald-400" /> Copied</> : <><FiCopy size={11} /> Copy</>}
        </button>
      </div>
      {html ? (
        <div
          className="overflow-x-auto p-3 text-[12px] leading-relaxed [&_pre]:!bg-transparent [&_pre]:!m-0 [&_pre]:!p-0 [&_code]:!text-[12px]"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto p-3 text-[12px] leading-relaxed font-mono text-slate-300">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
