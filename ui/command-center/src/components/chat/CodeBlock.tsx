import { useState, useEffect, useCallback } from 'react';
import { FiCopy, FiCheck } from 'react-icons/fi';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function CodeBlock({ language, code }: { language: string; code: string }) {
  const { colors, theme } = useTheme();
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const shikiTheme = theme === 'silver' ? 'github-light' : 'github-dark';
    import('shiki').then(({ codeToHtml }) => {
      codeToHtml(code, { lang: language, theme: shikiTheme })
        .then(result => {
          if (!cancelled) setHtml(result);
        })
        .catch(() => {});
    });
    return () => { cancelled = true; };
  }, [code, language, theme]);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [code]);

  return (
    <div
      className="relative group rounded-lg my-2 overflow-hidden"
      style={{ backgroundColor: colors.codeBg, border: `1px solid ${colors.border}` }}
    >
      <div
        className="flex items-center justify-between px-3 py-1"
        style={{ backgroundColor: colors.surface, borderBottom: `1px solid ${colors.border}` }}
      >
        <span className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textMuted }}>{language}</span>
        <button
          onClick={handleCopy}
          className="flex items-center gap-1 text-[10px] transition opacity-0 group-hover:opacity-100"
          style={{ fontFamily: font.mono, color: colors.textMuted }}
          onMouseEnter={e => (e.currentTarget.style.color = colors.text)}
          onMouseLeave={e => (e.currentTarget.style.color = colors.textMuted)}
        >
          {copied ? <><FiCheck size={11} style={{ color: colors.success }} /> Copied</> : <><FiCopy size={11} /> Copy</>}
        </button>
      </div>
      {html ? (
        <div
          className="overflow-x-auto p-3 text-[12px] leading-relaxed [&_pre]:!bg-transparent [&_pre]:!m-0 [&_pre]:!p-0 [&_code]:!text-[12px]"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="overflow-x-auto p-3 text-[12px] leading-relaxed" style={{ fontFamily: font.mono, color: colors.text }}>
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
